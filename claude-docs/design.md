# Design

How the fork satisfies [requirements.md](requirements.md). This is the
internal engineering view: the host boundary, the interception points, why
the alternatives were rejected, and what the patch surface against upstream
Ruffle actually is.

The repo's [`README.md`](../README.md) is the public overview and carries
the per-index `ASnative(5,N)` reference; it is not repeated here. The
appliance that wraps this player is
[chumby-pi](https://github.com/yanosz/chumby-pi).

---

## 1. The shape of the thing

The panel reaches for the outside world through three channels: vendor
functions, shell commands, and the filesystem (plus HTTP, which is ordinary
Flash). Stock Ruffle knows none of the first three. Rather than teaching
Ruffle about chumby, the fork inserts **one boundary** — the `ChumbyHost`
trait — and routes all three channels plus URL loads across it. Behind the
boundary sits either canned data or the real system.

```
        controlpanel.swf  (never modified)
                │
     ASnative(5,N)  exec://  _backtick  file://  http://
                │
        core/src/chumby/          ← the entire patch
         avm.rs   navigator.rs   ui_policy.rs  input.rs  audio.rs
                │
            ChumbyHost  ────────────┐
                │                   │
          FixtureHost         RealNetHost (decorator)
          disk fixtures       /proc, /sys, getifaddrs
```

Everything else in upstream Ruffle is untouched except for a handful of
registration lines (§8).

## 2. The host boundary

```rust
pub trait ChumbyHost: Send + Sync {
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue;
    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError>;
    fn fetch(&self, url: &str) -> Option<Result<HostResponse, HostError>>;
    fn fs(&self) -> &dyn ChumbyFs;
}
```

One method per category of touchpoint, small enough to implement twice.
`name` is the panel's own wrapper name (`"_getPlatform"`), so implementations
read like the contract rather than like a jump table. `fetch` returning
`None` means "not ours" — the call falls through to the wrapped navigator,
which is what lets real internet-radio streams flow while chumby.com stays
mocked.

Pure functions (md5, base64, blowfish) never reach the host; they are
implemented directly in the AVM table.

The host is reached from AVM natives through a process-global `OnceLock`
registry in `host.rs`, not through a `PlayerBuilder` field. This is a
deliberate deviation from the original design: it shrinks the upstream patch
in `desktop/src/player.rs` to the navigator wrap alone.

### FixtureHost

Backs every category from a fixtures directory whose *keys are the panel's
own request strings* (NFR4). No translation layer.

- `native` — a defaults table, plus category-sensible fallbacks. Stateful
  pairs that must round-trip (volume, balance, mute, touchclick, timezone)
  are backed by files in the virtual rootfs, in the same place the panel
  itself persists them, so panel and host cannot disagree.
- `exec` — longest-prefix match against a manifest; a few commands get
  dynamic handlers instead of static files (the backup-alarm dismissal in
  `FixtureHost::exec`, the identity commands — `guidgen.sh`,
  `chumby_version -n`, `md5sum` — in `RealNetHost`). Unknown → logged
  loudly, empty response. (`chumby_set_volume|pan|mute` have no handler at
  all — on our path the panel drives volume through `_setSystemVolume`, and
  an unmatched backtick's empty answer is within FR4's contract.)
- `fetch` — host-allowlisted static files under `http/<host>/<path>`.
  In-process interception, no local HTTP server. One carve-out: with
  `access_chumby_com=1` the two music-proxy hosts (`shoutcast.chumby.com`,
  `bor.chumby.com`) return `None` instead — pass through to the real
  navigator (requirements FR15). The SHOUTcast tune-in redirect to
  `yp.shoutcast.com` is followed by the real backend; no shoutcast-specific
  code exists on our side. `xml.chumby.com` is never passed through.
- `fs` — rooted at `rootfs/`, writes confined to the root (`..` rejected).
  This confinement is what neutralizes the `externalmusic.xml`
  arbitrary-path hazard.

Paths in fixture data may contain a `{FIXTURES}` token, expanded to the
absolute fixtures directory at serve time. That is what lets the same
profile XML name widget SWFs on both the dev box and the Pi.

The fixture rootfs is **read-write** — all of the panel's persistence
(`/psp/alarms`, `/psp/volume`, `/psp/url_streams`, `/psp/clock_format`)
lands there. A desktop run therefore mutates `fixtures/rootfs/`; check
`git status` before concluding a fixture changed by itself.

### Directory enumeration: USB / local music

`_getDirectoryEntry` (5,320) is the one native that mutates an AVM1 object
argument, so its arm lives inline in `avm.rs` (`dispatch` has the
`activation` and the raw object; objects cannot cross `HostValue`). The
host side is `ChumbyFs::dir_entry` → `RootFs`: `read_dir`, name-sort,
index. The sort is not cosmetic — the panel iterates ascending indices
resumably across frames (`FileFinderPOSIX` re-posts `[dir, index]` at 200
entries/frame), so an unstable order would skip or duplicate entries.
`metadata()` (follows symlinks) decides `_isDir`/`_isFile`;
`symlink_metadata()` sets `_isDirLink`, the panel's recursion loop guard.
`_path` is rebuilt as a normalized panel-space join (`panel_path_join`) —
the callers pass `//mnt/usb/`-style paths, and the value feeds the
breadcrumb, `_fileExists`, and `_playAudio`, whose `resolve_url` maps it
into the rootfs for mpv. So mounting media at `<rootfs>/mnt/usb` makes
browsing *and* playback work with no further plumbing.

Each call re-lists the directory — O(n²) per scan, accepted knowingly
(proportionality): the browser lists ≤100 entries and Play All caps at
2500 files spread across frames. Revisit with a one-entry cache only if
the on-device Play All scan proves slow.

### The widget channel

Real chumby fetched its channel — a list of widget instances — from
chumby.com. We generate one from the widgets we ship. Each widget carries a
`*.widget.xml` sidecar next to its SWF holding exactly the `<widget>`
element the panel consumes (name, description, version, mode, access,
`<movie href>`, optional `<thumbnail href>`); `chumby-widget-channel`
wraps them in the `<widget_instance>`/`<profile>` envelope and writes
`fixtures/http/xml.chumby.com/xml/profiles`. The schema the panel accepts is
loose (requirements FR6); the dashboard preview thumbnail is §6.

### RealNetHost

A decorator, not a sibling: it wraps `FixtureHost`, overrides the network
exec touchpoints (`network_status.sh`, `signal_strength`, `macgen.sh`) from
live kernel state, and delegates everything else. It is always active. A
read that finds no connected interface returns `None`, so the inner fixture
answers and an offline desktop or CI run is unchanged.

Sources: default-route interface and gateway from `/proc/net/route`; the
interface's IPv4 and netmask from `getifaddrs`; DNS from `/etc/resolv.conf`;
MAC from `/sys/class/net/<if>/address`. No shell (NFR2). `getifaddrs` needs
`libc`, added target-gated under `[target.'cfg(unix)'.dependencies]`, with a
`#[cfg(not(unix))]` stub. (The gating predates the decision to drop the wasm
target, NFR5; `audio.rs` is ungated and is what actually breaks wasm.)

Earlier revisions used a UDP `local_addr` probe for the IP and a
`/proc/net/route` hex parse for the netmask. Both were brittle;
`getifaddrs` is the canonical source and handles the edge cases they didn't.

### Player configuration

`config.rs` reads `<fixtures>/player.toml` once in `FixtureHost::new`
(requirements FR14) — no new CLI flag, `--chumby-fixtures` already locates
it, and the fixtures root is outside the panel-writable rootfs. Parsing is
a manual `toml::Table` walk like ui-policy's; unknown keys and bad values
warn and keep defaults.

The volume cap is applied inside `AudioPlayer` (`effective_volume`, used
by both `play` and the IPC `set_volume`), so one choke point covers every
caller: `_playAudio`, `_setSystemVolume`, unmute, alarm fade-ins. The
backup alarm's exemption from the cap is structural, not a flag: its tone
child never passes through `AudioPlayer` (§9). `access_chumby_com` is
stored on the host (`FixtureHost::config()`) for the remote-channels
milestone to gate on; nothing reads it yet.

## 3. Vendor calls: the AVM table

Upstream's `core/src/avm1/globals/asnative.rs` dispatches `ASnative(a,b)` by
category `a`. Category 5 was unmapped, so every chumby native silently
returned `Undefined` — no warning at any log level. A single match arm
routes the whole category into `chumby::avm::method`.

`ASnative(4,39)` (`_batteryPower`) collides with Ruffle's own category 4
(`ASSetNative`). The panel never calls it, so category 4 stays upstream and
the collision is documented rather than resolved.

Dispatch inside `avm.rs` has three tiers: pure functions computed inline;
calls that need host state, forwarded across the boundary; and everything
else, a shared logging stub. Each arm carries its index *and* its wrapper
name in a comment, in both directions — nobody remembers ASnative indices by
heart.

`avm::method` is also where the per-frame work rides. The panel polls
`_bent` every frame, so the UI policy (§5) re-applies at frame cadence for
free, with no new upstream hook.

### Three pieces of AVM1 surgery

`chumby/avm.rs` deletes `WidgetPlayer.prototype.onPress` once the panel
defines it. That click-stats handler puts the widget container into AS2
button mode and swallows every widget click on the in-movie `localCache`
path. It is harmless on real hardware, where widgets play in a separate
slave player. This is the "revisit if a widget misbehaves" case that the
localCache decision explicitly foresaw.

`intro.rs` replaces `WidgetPlayer.prototype.playIntro` and
`introAdvanceTimerHandler` with Rust-native functions once frame 2 defines
them. The panel's own `playIntro` (F2:5283) never attempts the intro under
localCache — that branch substitutes the built-in clock; only the
`_startSlave` branch names `intro.swf` — so there is nothing to intercept
at (5,84) and the method itself is replaced. The replacement restages the
slave branch's semantics onto the panel's own widgetProxy path (an init
object with `_chumby_movie_url`, `attachMovie`, the `g_playingIntro`
globals); the handler poll reads `widgetProxy.proxy._chumby_widget_done`
like every localCache path does. The handler must be replaced *on the
prototype*, not merely installed as `onEnterFrame`: closing the info screen
runs `setState` (F2:3642), which re-installs the handler from the
prototype — the original would then poll `_getSlaveVar` and a stale
`"true"` in the slave-var store ends the intro instantly. While
`g_playingIntro` is true, the intro's `fscommand("quit")` (its frame 12;
on real hardware it kills the slave player) is swallowed by a chumby hook
in `avm1/fscommand.rs`; standalone runs and the panel's own quit paths
keep upstream behavior.

`music_sources.rs` splices unsupported sources out of
`MusicPlayer.musicSources` (requirements FR15), one-shot with retry until
frame 2 defines the array. The array was chosen over the per-player
`exists()` prototypes because it is the single choke point — the Music
list, the alarm audio list and `sourceForSelector` all read it — and
because iPod's force-show bypasses `exists()` entirely. `reorderSources`
and externalmusic.xml only permute or insert, never resurrect a removed
entry, so one shot is enough. Which selectors are hidden depends on
`access_chumby_com` and `enable_lyrion` (config.rs).

## 4. URL interception

`ChumbyNavigator` decorates whatever `NavigatorBackend` the frontend built,
and claims four kinds of URL:

- **`exec://CMD`** → `host.exec()`, stdout returned as the loaded document.
- **chumby HTTP hosts** → `host.fetch()`, answered from fixture files
  in-process. Never leaves the machine — *except* the opt-in passthroughs
  under `access_chumby_com` (`fixture.rs::fetch` returns `None`, so the
  request falls through to the inner navigator and reaches the live
  revived chumby.com): the music-proxy hosts (`is_music_host`), and the
  two registration endpoints `/xml/authorize` + `/xml/registerchumby`
  (`is_identity_endpoint`). The identity passthrough additionally requires
  a real hardware serial (`real_ident::serial().is_some()`), so a dev/CI
  box on the random GUID can never present a registrable identity (NFR6).
  Everything else on those hosts stays fixture-answered.
- **`file://`** → resolved against the virtual rootfs first. The licenses
  viewer hardcodes `file:////LICENSES/gpl.txt`, a chumby rootfs path we
  cannot change (FR1); the stock navigator would look for `/LICENSES` on the
  real disk and find nothing. A rootfs miss returns `None` and falls through,
  so `{FIXTURES}`-expanded real-disk paths (widget SWFs, thumbnails) are
  unaffected. This also covers any future panel-hardcoded `file://` read.
- Everything else → the inner navigator.

## 5. UI policy

Rules live in `core/src/chumby/ui-policy.toml` and are compiled in with
`include_str!` (FR9), parsed on first use, applied from `avm::method` at
frame cadence, idempotent. The cost of compiling them in is that editing a
rule needs a rebuild; `test_embedded_policy_parses` catches a typo at test
time rather than as a control that silently stays live on the device.

```toml
[[rule]]
id        = "clock-ntp-toggle"
action    = "disable"                    # hide | disable | readonly
selectors = [
  "controlPanel/depth:15/depth:1/name:timePanel/name:ntpButton",
  "controlPanel/depth:15/depth:1/depth:1/depth:46",
]
```

A selector is a path of segments from a stable anchor (`_root.controlPanel`),
each segment resolving a child either `name:`-wise or `depth:`-wise. Named
instances come from `PlaceObject` tags and are stable. Unnamed ones are not:
AVM1 auto-names them `instanceN` from a global counter that depends on how
many unnamed instances were created earlier in the session — i.e. on the
user's navigation history. Depths *are* authored into the SWF, so a
depth path is the reliable fallback. Alternatives are tried in order, first
match wins.

Only the **primary** selector's failure raises "screen present but control
missing — selectors stale?". Depth-based fallbacks legitimately walk into
other frames of the same container (the settings grid reuses depths across
frames) and would otherwise cry wolf. Logging is transition-damped per rule:
acquired / parent-only / gone.

`disable` sets `enabled = false` on the target **and its direct children**,
then dims `_alpha` to ~45. The child pass is not belt-and-braces: AVM1
`enabled` does not cascade, and every chumby button puts its hit handler on
an inner clip (`box` on a checkbox, `b` on a sprite button). (A fourth
action, `tint`, existed for I3's blue "wired ethernet" bar and was removed
with it — see §7.)

**Why properties and not something else.** Bytecode patching at load is
fragile across panel variants and violates the spirit of FR1. Renderer-side
graying is cosmetic — the control stays live. Making the underlying natives
no-ops leaves a UI that looks functional and silently ignores input, which
around alarms is the worst possible failure. Setting properties at runtime is
the same mechanism as the wizard skip, costs nothing per frame, keeps the SWF
untouched, and keeps each policy entry declarative rather than code.

## 6. Widget playback: the localCache path

The panel supports two widget architectures. On real hardware it hands each
widget to a **slave player instance** on an overlay framebuffer and talks to
it through `_setSlaveVar`/`_getSlaveVar`. With `-PlocalCache=1` it instead
loads the widget *into itself* with `loadMovie`.

We take the second. It was proven to work under stock Ruffle before any
patch existed, it collapses roughly forty natives into logging stubs, and it
means one player process. The cost is that anything the panel does only over
the slave-var channel does not reach a running widget — which is why the
12/24h clock format needs a restart, and why the intro widget needed
interpreter-level work rather than a fixture (the `playIntro` replacement,
§3).

The dashboard preview picture is compatible with this: it is a *static*
thumbnail, `loadMovie`'d from a `<thumbnail href>` in the profile, not a
second live render. `loadMovie` decodes a JPEG as readily as a SWF.

## 7. Real network diagnostics

`signal_strength` drives the dashboard's only network element, a five-bar
`WifiIndicator` meter (level = `(linkquality − 50) × 2`, hidden when
`connected != 1`), and the Info screen's "link quality" line. `RealNetHost`
decides wireless-vs-wired from the default-route interface
(`/sys/class/net/<if>/wireless/` exists ⟺ wireless) and reports
`type="wlan"` or `type="lan"` accordingly.

On wifi: link quality, signal and noise come from `/proc/net/wireless`
(brcmfmac reports quality on a 0–70 scale, converted to the percent the
panel expects; noise is the driver's value even when it is the −256
"unknown" sentinel). The SSID comes from the `SIOCGIWESSID`
wireless-extensions ioctl — *not* nl80211 as once planned: wext compat is
the same cfg80211 layer that populates `/proc/net/wireless`, verified on
the Pi's brcmfmac. `auth`/`encryption` stay empty; the panel renders a bare
ssid line for any auth value it does not recognize, and reading the
security mode would cost nl80211 plumbing for a decorative suffix.

On a wired link the answer is `connected="0"` and the meter hides. There is
**no ethernet icon anywhere in the SWF** — the meter is a wifi meter — so
the wired diagnostics (type Ethernet, IP, gateway, DNS) live on the Info
screen instead. This replaces I3's earlier repurposing (full bars tinted
blue by a `wired-eth-bar` rule), which was unconditional — the bar stayed
blue after the device moved to wifi — and whose honest fix (a runtime
condition plus tint un-apply in the policy engine) was judged
disproportionate for a minor indicator (user 2026-07-10). The `tint` action
went with it.

The readers are always on, with no flag or configuration: whenever a default
route exists they answer, and with no route they return `None` and the
fixture answers, so a desktop/CI run with no usable network behaves as
before. The panel caches `networkType`/`ssid` from the boot-time
`gotNetworkStatus`, so a network change under a running panel shows on the
Info screen only after a player restart.

`RealNetHost::exec` also carries the **device identity** touchpoints
(requirements FR10), implemented in `real_ident.rs` with the same
real-else-fixture contract: `guidgen.sh` (salted-md5 GUID of the machine
serial), `chumby_version -n` (model tag + serial, e.g. `RPI3B-…`), and an
honest `md5sum <path>` computed from the virtual rootfs — the panel md5s
`/tmp/.guidhash` for its chumby.com auth parameters, all intercepted
in-process.

## 8. The patch surface

New code lives in `core/src/chumby/`, file by file in
[development.md](development.md) §2. Upstream files carry only this:

| File | Change |
|------|--------|
| `core/src/lib.rs` | `pub mod chumby;` |
| `core/src/avm1/globals/asnative.rs` | `5 => chumby::avm::method` match arm (+ the `ASnative(4,39)` collision note) |
| `core/src/avm1.rs` | `pub use function::FunctionObject;` (native fns for the prototype surgery) |
| `core/src/avm1/fscommand.rs` | `chumby::intro::swallow_fscommand_quit` guard before the provider dispatch |
| `core/src/player.rs` | click-target diagnostic in `run_mouse_pick`, silent unless `chumby_pick=debug`; body split into `run_mouse_pick_inner` |
| `core/Cargo.toml` | `toml` (ui-policy parsing) and target-gated `libc` (getifaddrs, SIOCGIWESSID) |
| `desktop/src/player.rs` | `ChumbyNavigator` wrap before `.with_navigator(…)` |
| `desktop/src/cli.rs` | `--chumby-fixtures <PATH>`, `--chumby-control <FIFO>` |
| `desktop/src/main.rs` | host init, `input::spawn` |
| `desktop/src/app.rs` | Home key → bend; `WindowEvent::Touch` arm (upstream ignores touch) with the ≥1 s stationary hold → bend; control-FIFO pointer-command drain in `about_to_wait` |

Hooks are not numbered. An earlier `H1…H11` scheme carried no information
and was dropped; the comment at each site contains the word `chumby`, which
is what a rebase actually needs.

## 9. Audio

`audio.rs` spawns mpv and drives it over its Unix-socket JSON IPC. Three
bugs found by ear on the device shaped it, and each is a trap worth
remembering:

- `_getAudioPlayerState` must return the SWF's constants (IDLE −1, PAUSED 0,
  WAITING 1, PLAYING 2). Returning `1` for "playing" made the panel's own
  `TrackedBTPlayer` watchdog kill every stream after five seconds.
- The IPC socket is created lazily by mpv, and on a loaded Pi that takes
  more than a second. A one-shot connect left volume stuck at spawn level,
  so alarms rang silently at fade-in volume 0. `send_ipc` reconnects lazily.
- Killing mpv without reaping it leaves zombies.

Where mpv is absent the backend logs "audio will be a silent stub" and
continues.

### The backup alarm

`backup_alarm.rs` is chumbalarmd reduced to its contract (requirements
FR13): a thread polls `<rootfs>/psp/ifalarm` every 2 s; if the time in it
passes while the file exists, delete the file and sound a Klaxon for the
configured duration, via a dedicated mpv child. Polling the file — instead
of arming a long sleep when the panel says so — is the design: the file is
the single source of truth, so boot-time missed alarms and re-arms need no
extra paths, and a wall-clock step (the Pi has no RTC) can never strand a
computed sleep. Two exec commands are intercepted ahead of the fixture
manifest in `FixtureHost::exec`: dismissal (`rm /psp/ifalarm; …`) really
deletes the file and kills a sounding tone; the bare `reload_backup_alarm`
is a no-op.

The tone child is deliberately not `AudioPlayer`'s: no shared `child` slot,
no IPC socket, no network source — the primary alarm failing (the mpv
stall-on-WLAN-loss behaviour measured 2026-07-10: silent from cache-drain,
alive for its 60 s network timeout, then exit; `paused-for-cache` over IPC
is the live stall signal if we ever want a faster fallback) must not be
able to take the beep down with it. PipeWire mixes the two if both are
audible.

## 10. Input

Upstream's winit `app.rs` has no touch handling at all — a Wayland touch is
not a pointer. The `WindowEvent::Touch` arm synthesizes MouseMove/Down/Up
from single-touch, and a stationary hold (≤12 px for ≥1 s) raises the bend
sensor.

The control FIFO drains at the top of `about_to_wait`, one action per loop
iteration, converting `click`/`drag` commands into real `PlayerEvent`s
through the same `window_to_movie_position` as physical input. Spreading the
sequence across ticks is what makes sliders draggable from a script.

A client-side attempt to hide the kiosk mouse cursor (`set_cursor_visible`)
was implemented, deployed, and did nothing: with no pointer device entering
the window, the client never owns a cursor surface — the arrow is the
*server-drawn* seat cursor. Reverted. The real fix is a udev rule in the
appliance repo.

## 11. Bootstrap

There is none, and that is the design. The plan once called for a Rust-side
controller that would set `_root` variables and jump frames. It turned out
the panel's own dispatcher reaches `main` given only two honest answers
(healthy network, sane clock), so the wizard skip is an environment answer
rather than a frame hack. If a later screen ever needs real frame control,
the documented insertion point is `Player::update` with a queued-action
approach.

## 12. Remote channels, registration & identity

Roadmap item 5. Off by default (NFR6); the owner opts in with
`access_chumby_com`. Everything here is gated on **both** that flag and a
stable identity — see below.

**Identity model.** The panel presents a single GUID; there is no crypto
signature on the wire (proven by reverse-engineering a real registered
reference chumby: its `dcid` tool only reads a `<skin>` branding value from
`/dev/dcid`, while device identity came from a separate crypto processor via
`cpi`/`guidgen.sh` — which our hardware doesn't have). Two external sources
confirm this split, both found 2026-07-12:

- **DCID is branding, not identity.** A chumby-forum thread
  ([post 3193](https://forum.chumby.com/viewtopic.php?id=3193)) describes
  the DCID as a "daughtercard id" — a tag-based binary structure (≤768 bytes)
  in a small flash on a daughterboard, read/written by the `dcid` tool. A
  stock US unit is just `<chum><skin>0000</skin></chum>`; regional variants
  add distributor/language nodes. It selects skin and localized content —
  no key, no signature. Matches what we saw on the reference box.
- **The GUID is the crypto chip's, and unreproducible off-device.** Chumby
  released the crypto-processor tool
  ([github.com/sutajiokousagi/cpi](https://github.com/sutajiokousagi/cpi),
  © Chumby Industries 2007-8). `cpi` talks to a *serial-attached* RSA chip
  (`/dev/ttyS2` on ironforge, our platform — a prebuilt `arm-linux-ironforge`
  binary is even checked in); `cpi -p` calls `cpi_get_putative_id` and prints
  a key's "putative ID", which is exactly the string `guidgen.sh` (`cpi.sh -p`)
  feeds the panel as the GUID. The RSA private key never leaves the chip
  (the production tests need `Crypt-OpenSSL-RSA` + `Digest-SHA1`), so the
  original GUID cannot be regenerated without the hardware — which is why we
  *synthesise* a stand-in rather than emulate the chip. If a future need ever
  demanded genuine crypto-processor semantics, this released source is the
  starting point; for registration against the revived chumby.com it is not
  needed (the account claims whatever GUID the box presents).

So we synthesise the GUID (`real_ident::resolve_guid`), in priority order:

1. `device_guid` from player.toml (an explicit owner-set UUID),
2. else a salted MD5 of the SoC serial (a Raspberry Pi — stable across
   reflashes/SD swaps, recomputed identically each boot),
3. else a random per-box GUID persisted at `/psp/guid` (dev/CI fallback).

`real_ident::has_wire_identity(config)` is true only for (1) or (2) — a
*stable, owner-anchored* identity. The random dev GUID (3) never qualifies,
so a plain dev box or CI run stays structurally offline for identity even
with the flag on (NFR6). This is the single gate the passthrough and the
UI-policy lift share. **Never commit a real device serial or GUID** — the
salt is public, so the serial reproduces the GUID, and the GUID (via
`device_guid`) impersonates the device.

**Passthrough (`fixture.rs::is_using_host`).** Under flag + identity, the
whole "using" surface passes through to the live service: `xml.chumby.com`
(authorize, registerchumby, chumbies, profiles, setprofile, movie_files,
thumbnails) and `widgets.chumby.com`. `update.chumby.com` is deliberately
**not** included — the device never pulls firmware from chumby.com. Flag off
(or no identity): all of it stays fixture-answered, so the boot-generated
local channel remains the way to configure a channel without chumby.com.

**Registration wizard.** Built into the SWF (`register` frame,
`DefineSprite_708`: `gotosite` → `dogrid` → `dopolling` → `dosuccess`); we
build nothing, only answer `/xml/authorize` (boot gate + the wizard's 5 s
activation poll) and `/xml/registerchumby?id=…&hash=<tapped-oval-pattern>`.
An unregistered box boots into the wizard; the owner taps the oval pattern
and claims the GUID on chumby.com, and the poll flips to `main`. A `Later`
button escapes to `main` so it is never a lockout. Registration leaves
nothing to persist — the stable GUID re-authorises every boot. Caveat: the
`dosuccess` OK button runs the original's clean-slate reset, unlinking a set
of `/psp` alarm/music prefs (`alarms`, `alarm_volume`, `pandora_*`,
`shoutcast_search`, `url_streams`, `widget_shuffle`, `fmradiostation`,
`mp3files_order`, `music_order`, `music_timer_duration`, `slimserver_ip`).

**Widget download & cache.** `WidgetCache.canCache` is hardwired on for
ironforge, so the panel always caches. It downloads each widget SWF by
shelling out `exec://…curl '<movie_url>' > /tmp/widgetcache/<id>; echo $?` —
and we ship no shell. `navigator.rs` recognises exactly that command
(`parse_widget_curl`), fetches the SWF through the real backend, and writes
the bytes into the virtual rootfs at the cache path (echoing `0`). The panel
then md5/size-verifies the cached file and loads it. The cache is keyed
`id`+`version` with a `widgetcache.xml` manifest, so later rotations hit the
cache instead of chumby.com — gentle on the old server. Downloaded SWFs are
copyrighted and gitignored (`/fixtures/rootfs/**/widgetcache/`); note that
our rootfs `/tmp` persists across reboots (unlike real tmpfs), so the cache
is more durable than on hardware.

**Loading a cached widget.** `loadMovie` requests the cached SWF as a
*scheme-less absolute path with a query string* —
`/tmp/widgetcache/<id>?_chumby_widget_instance_index=…` — the widget
parameters riding as the query. `navigator.rs::intercept` therefore treats
both `file://` URLs and scheme-less `/…` paths as rootfs candidates, keyed
on the path with the query stripped; a rootfs miss falls through so
real-disk paths (the controlpanel SWF, fixture widgets) still load. The
response URL for a scheme-less hit is rewritten to `file://…`, because
`SwfMovie::append_parameters_from_url` extracts the query into the loaded
movie's `_root` vars only when `Url::parse` succeeds on the response URL —
a raw `/tmp/…` fails silently and every widget parameter is dropped,
`_chumby_clock_format` the visible casualty (found on-device 2026-07-12;
desktop fixture widgets load over `file://` hrefs and never hit it).

**UI policy.** `main-channel` and `main-delete` are dead-ends without the
remote service, so their disable rules carry `only_without_chumby_access`
and lift exactly when `has_wire_identity` + the flag hold (a serial-less box
with no `device_guid` keeps them disabled — enabled-but-broken avoided).
`main-send`/`main-rate` stay disabled **permanently**: the social surface
(add-widget catalog, rating, send/mail) is out of scope (Jan, 2026-07-11),
so those controls are dead-ends and their endpoints are never passed
through. This is the last of item 5; there is no Phase 3.
