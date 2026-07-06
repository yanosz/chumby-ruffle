# chumby-ruffle — architecture of the chumby fork

This fork makes [Ruffle](https://ruffle.rs) run the **Chumby Classic
control panel** (`controlpanel.swf`, an AVM1/Flash-Lite-era movie) as if
it were running on chumby hardware. It is the player half of the
[chumby-pi](https://github.com/yanosz/chumby-pi) project, which turns a
Raspberry Pi with a small touchscreen into a Chumby; that repo carries
the fixtures, packaging, docs, and pins this fork as a git submodule.

Everything here is additive and feature-gated: built **without**
`--features chumby` the tree compiles to stock Ruffle with zero
behavioral difference.

## Why a fork at all

The chumby shipped a customized Flash Lite player
(`chumbyflashplayer`) whose movies talk to the device through three
non-standard channels:

1. **`ASnative(5, N)`** — a vendor function table with ~140 entries:
   volume, brightness, bend sensor, filesystem, shell, audio player,
   master/slave widget control, crypto helpers, …
2. **`exec://` URLs** — `loadVariables("exec://mount", …)` runs a shell
   command and loads its stdout as the document.
3. **The device filesystem** — the panel persists state via
   `_getFile`/`_putFile` under `/psp`, `/tmp`, `/mnt/usb`.

Stock Ruffle returns `undefined` for unknown natives and knows nothing
of `exec://`, so the panel hangs at boot. The fork answers all three
channels from a pluggable host environment instead of the real system —
a **virtual chumby**: fetches of chumby.com endpoints are answered from
a fixture corpus, shell commands from canned responses, and the
filesystem from a confined virtual rootfs. The "syscall surface" of the
original player is replaced wholesale; nothing the movie does reaches
the host system directly.

## Layout: one module, thin hooks

All chumby code lives in `core/src/chumby/`, compiled only with the
`chumby` cargo feature (`core/Cargo.toml` declares it,
`desktop/Cargo.toml` forwards it). Upstream files carry only small
cfg-gated registration hooks. The complete upstream patch surface:

| Hook | File | Change |
|------|------|--------|
| H1 | `core/src/avm1/globals/asnative.rs` | route category `5` to `chumby::avm::method` |
| H2 | `core/src/lib.rs` | `pub mod chumby;` declaration |
| H4 | `desktop/src/player.rs` | wrap the navigator in `ChumbyNavigator` |
| H5 | `desktop/src/cli.rs`, `main.rs` | `--chumby-fixtures <DIR>` flag + host init |
| H6 | `core/Cargo.toml`, `desktop/Cargo.toml` | feature declaration |
| H7 | `desktop/src/app.rs` | Home key → bend sensor (as on chumby's falconwing port) |
| H8 | `desktop/src/cli.rs`, `main.rs` | `--chumby-control <FIFO>` flag + reader threads |
| H9 | `core/src/player.rs` | opt-in click-target diagnostic in `run_mouse_pick` |
| H10 | `desktop/src/app.rs` | drain simulated-pointer queue in `about_to_wait` |
| H11 | `desktop/src/app.rs` | `WindowEvent::Touch` → mouse events; long-press → bend |

Every hook is `#[cfg(feature = "chumby")]`. The chumby-pi repo's
`docs/patch-notes.md` tracks this table against upstream drift and is
the rebase guide.

## The `ChumbyHost` trait (`host.rs`)

One trait method per category of environment traffic, small enough to
implement twice:

```rust
pub trait ChumbyHost: Send + Sync {
    /// ASnative(5,N) calls that need host state (volume, platform, …).
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue;
    /// Shell execution: `_backtick` (5,52) and the exec:// URL scheme.
    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError>;
    /// URL fetch interception. None = not ours, pass through.
    fn fetch(&self, url: &str) -> Option<Result<Vec<u8>, HostError>>;
    /// The virtual rootfs (get/put/exists/size/unlink/dir_entry).
    fn fs(&self) -> &dyn ChumbyFs;
}
```

The shipped implementation is `FixtureHost` (`fixture.rs`), which
answers from a fixtures directory (`--chumby-fixtures`); a future
`PiHost` can map the same calls onto real hardware (backlight, ALSA,
clock) without touching the AVM or navigator layers.

The host is reached through a process-global `OnceLock` registry
(`chumby::set_host`) rather than a `PlayerBuilder` field — a deliberate
deviation that keeps the upstream patch surface at two lines per hook.
The desktop frontend runs one player per process; revisit if
multi-player embedding ever matters.

Every host call is logged under the tracing target `chumby_host` with
args and result. This is the project's discovery loop: **the log line
for an unanswered request names the fixture file to create.**
High-frequency polls (`_bent` runs every frame) are deduplicated —
consecutive identical calls collapse into an `× N` line.

## The ASnative(5,N) table (`avm.rs`)

Upstream's `asnative.rs` dispatches category 5 to
`chumby::avm::method`, a `TableNativeFunction` keyed by index. The
canonical index → wrapper-name table (from the chumby-pi environment
contract, `docs/reference/03-environment-contract.md` there) covers the
~140 natives the panel and its libraries reference.

Dispatch layers, in order:

- **Filesystem natives** (50/51/53/54/55/320) go straight to
  `host.fs()` — the virtual rootfs.
- **`_backtick`** (52) goes to `host.exec()`.
- **Pure functions** are implemented in-table and never reach the
  host: base64 encode/decode (160/161); `_md5Sum` (162) currently
  returns a constant marker digest (observed call sites only compare
  a digest with itself).
- **Everything else** goes to `host.native()`, where `FixtureHost`
  keeps stateful get/set pairs (volume, mute, balance, touchclick,
  LCD brightness…) in a name-keyed store, seeds volume from the
  persisted `/psp/volume`, answers identity queries
  (`_getPlatform` → `"ironforge"`), drives the audio player, and
  returns category-sensible defaults for the rest.
- **Master/slave widget natives** stay logging stubs by design: the
  panel runs with `-PlocalCache=1` so widgets load in-movie via
  `loadMovie` instead of chumby's dual-player master/slave system.
  `_getSlaveVar("_chumby_widget_done")` answers `"true"` so
  intro/widget handoffs never hang.

Two quirks worth knowing:

- `ASnative(4,39)` collides with Ruffle's existing category 4
  (`ASSetNative`). Chumby's `_batteryPower` there is never called by
  the panel, so category 4 is left untouched.
- The table performs one piece of AS2 surgery: it deletes
  `WidgetPlayer.prototype.onPress` (a click-stats handler) once the
  panel defines it. On real hardware widgets run in a slave player and
  never see it; in our in-movie mode it would put the widget container
  into AS2 button mode and swallow every click meant for the widget's
  own buttons.

## URL interception: `ChumbyNavigator` (`navigator.rs`)

A `NavigatorBackend` decorator wrapped around the desktop navigator
(hook H4). Its `fetch()`:

- **`exec://<command>`** — the part after the scheme is a raw shell
  command, *not* a URL (never parse it). Answered by `host.exec()`;
  stdout becomes the document body.
- **chumby hosts** (`chumby.com`, `*.chumby.com`, localhost daemons) —
  answered by `host.fetch()` from the fixture corpus at
  `fixtures/http/<host>/<path>`. A *missing* fixture for a chumby host
  is reported as a clean HTTP failure rather than being allowed to
  escape to the real (long-dead) chumby.com; the panel handles
  `onLoad(false)` everywhere.
- **Everything else** passes through to the wrapped backend unchanged —
  real internet-radio streams can flow while chumby.com stays mocked.

## The virtual rootfs (`fixture.rs::RootFs`)

All panel filesystem traffic (`_getFile`, `_putFile`, `_fileExists`,
`_fileSize`, `_unlink`, `_getDirectoryEntry`) resolves inside
`fixtures/rootfs/`, which is pre-seeded with the state files a real
chumby would have (`/psp/firsttime`, alarms XML, timezone, …). Paths
are normalized (the panel emits messy multi-slash paths like
`//mnt/usb`), `..` components are rejected, and writes are confined to
the root — the movie cannot read or write outside it. `/tmp` and
`/mnt` are plain subdirectories. Fixture bodies may contain a
`{FIXTURES}` token that expands to the absolute fixtures path, so
profile XML can reference widget SWFs by `file://` URL portably.

Shell fixtures live next door: `fixtures/exec/manifest.txt` maps
command prefixes (TAB-separated, longest prefix wins) to response
files. Unknown commands log loudly and return empty output, which the
panel tolerates everywhere observed.

## Audio: mpv backend (`audio.rs`)

The chumby played audio through a `btplay` daemon controlled via the
`ASnative(5,131–152)` family. `FixtureHost` maps that state machine
onto a spawned **mpv** process controlled over its Unix-socket IPC
(`/tmp/chumby-mpv.sock`): `_playAudio` spawns mpv with the resolved
URL (absolute chumby paths resolve inside the virtual rootfs — alarm
tones live at `/usr/chumby/alarmtones`), `_setSystemVolume`/mute
translate to live IPC volume changes, `_getAudioPlayerState` polls the
process and answers in the SWF's own constants (IDLE −1, PAUSED 0,
WAITING 1, PLAYING 2 — the panel's `TrackedBTPlayer` watchdog kills
any stream not reporting PLAYING within 5 s, so this mapping is
load-bearing). If mpv is not installed the player degrades to a
silent stub: the state machine still answers correctly, the UI works,
no sound plays.

## Simulated input: control channel (`input.rs`) and touch

Chumby's signature input is the **bend sensor** (the squeezable top
button); the panel polls `ASnative(5,25) _bent` every frame and fires
onBend/onUnbend on edges (release summons the main button bar,
snoozes alarms). Neither a dev box nor a Pi has one, so the fork
accepts line commands on ruffle's stdin and, with
`--chumby-control <PATH>`, on a FIFO any shell can write to:

```text
bend | tap           press for one poll, then release
bend down / bend up  hold / release
click X Y            left-click at window coordinates
drag X1 Y1 X2 Y2     press, glide, release (sliders)
```

Pointer commands are queued and drained by the frontend event loop
**one action per iteration** — press-tracking widgets (sliders) need
the down/move/up sequence spread over player ticks. Coordinates take
the same `window_to_movie_position` path as real mouse input.

Physical inputs map onto the same primitives: the Home key taps the
bend (H7), and on a touchscreen (H11 — upstream ignores
`WindowEvent::Touch` entirely; Wayland touch is not a pointer)
single-touch becomes left-button mouse events, while a stationary
(≤12 px) hold of ≥1 s becomes a bend tap — the touch stand-in for the
squeeze.

## Building and running

```sh
cargo build -p ruffle_desktop --features chumby
```

`chumby` is **not** a default feature — a plain build compiles but
contains none of this. Run against the chumby-pi fixture tree:

```sh
target/debug/ruffle_desktop \
  --load-behavior blocking \
  --filesystem-access-mode allow \
  --chumby-fixtures <chumby-pi>/fixtures \
  --chumby-control /tmp/chumby-ctl \
  -PlocalCache=1 \
  controlpanel.swf
```

(`--load-behavior blocking` works around an upstream
GoToLabel/streaming issue and matches the original player's behavior;
`--filesystem-access-mode allow` lets the movie load widget SWFs via
`file://`; see the chumby-pi repo — its `run-controlpanel.sh` wraps
all of this.)
`controlpanel.swf` itself is copyrighted and not distributed here or
in chumby-pi.

Useful log targets: `chumby_host` (all host traffic),
`chumby_audio` (mpv), `chumby_pick=debug` (click-target diagnostic,
H9).

## Branch discipline

The `chumby` branch is kept **linear**: current upstream `master` with
the whole fork applied on top as a single squashed commit. Upstream
merges are done by rebasing/cherry-picking that one commit onto the new
upstream tip; the hook table above (mirrored in chumby-pi's
`docs/patch-notes.md`) is the checklist for resolving drift. The
acceptance test after any upstream merge is not "it compiles" but
"controlpanel.swf boots to the panel" — the hooks can break silently.
