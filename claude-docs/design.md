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
- `exec` — longest-prefix match against a manifest; a few commands
  (`chumby_set_volume N`, `md5sum`) get small dynamic handlers instead of
  static files. Unknown → logged loudly, empty response.
- `fetch` — host-allowlisted static files under `http/<host>/<path>`.
  In-process interception, no local HTTP server.
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

### One piece of AVM1 surgery

`chumby/avm.rs` deletes `WidgetPlayer.prototype.onPress` once the panel
defines it. That click-stats handler puts the widget container into AS2
button mode and swallows every widget click on the in-movie `localCache`
path. It is harmless on real hardware, where widgets play in a separate
slave player. This is the "revisit if a widget misbehaves" case that the
localCache decision explicitly foresaw.

## 4. URL interception

`ChumbyNavigator` decorates whatever `NavigatorBackend` the frontend built,
and claims four kinds of URL:

- **`exec://CMD`** → `host.exec()`, stdout returned as the loaded document.
- **chumby HTTP hosts** → `host.fetch()`, answered from fixture files
  in-process. Never leaves the machine.
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
action    = "disable"                    # hide | disable | readonly | tint
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
an inner clip (`box` on a checkbox, `b` on a sprite button). `tint` applies
a flat color transform, the same `new Color(mc).setRGB(…)` idiom the panel
uses on its own buttons.

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
12/24h clock format needs a restart, and why the intro widget needs
interpreter-level work rather than a fixture.

The dashboard preview picture is compatible with this: it is a *static*
thumbnail, `loadMovie`'d from a `<thumbnail href>` in the profile, not a
second live render. `loadMovie` decodes a JPEG as readily as a SWF.

## 7. Real network diagnostics

`signal_strength` drives the dashboard's only network element, a five-bar
`WifiIndicator` meter (level = `(linkquality − 50) × 2`, hidden when
`connected != 1`). There is **no ethernet icon anywhere in the SWF**, so on a
wired link the meter is repurposed: full bars, recoloured to a distinct
non-wifi blue by a `tint` rule on the parent indicator (tinting a child bar
would be undone by its own `gotoAndStop`). The Info screen states
`type: Ethernet` alongside, so the display is honest rather than clever.

**This is where the open defect lives.** The type is currently a constant.
The reader must instead decide wireless-vs-wired from the default-route
interface (`/sys/class/net/<if>/wireless/` exists ⟺ wireless), report
`type="wlan"` or `type="lan"` accordingly, and let both the indicator colour
and the Info screen's signal line follow that. On wifi, SSID needs nl80211 —
sysfs does not expose it — and link quality comes from `/proc/net/wireless`.
The static `wired-eth-bar` tint rule has to become conditional on the same
reading. See requirements.md §3.

## 8. The patch surface

New code lives in `core/src/chumby/`, file by file in
[development.md](development.md) §2. Upstream files carry only this:

| File | Change |
|------|--------|
| `core/src/lib.rs` | `pub mod chumby;` |
| `core/src/avm1/globals/asnative.rs` | `5 => chumby::avm::method` match arm (+ the `ASnative(4,39)` collision note) |
| `core/src/player.rs` | click-target diagnostic in `run_mouse_pick`, silent unless `chumby_pick=debug`; body split into `run_mouse_pick_inner` |
| `core/Cargo.toml` | `toml` (ui-policy parsing) and target-gated `libc` (getifaddrs) |
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
