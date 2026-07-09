# Requirements

What this fork must do, and the constraints it must do it under. The
authority for every functional claim below is `controlpanel.swf` version
**2.8.87b3** (the panel the real device downloaded and ran) as decompiled
with ffdec, cross-checked against wiki.chumby.com and against a real Chumby
Classic used as a read-only oracle. Where the three disagree, the SWF wins.

Line references written `F2:n` point into the ffdec script export
`frame_2/DoAction.as`; sprite ids are written `DS<n>`.

This repository is the **player**. The appliance around it — packaging,
fixture data, kiosk, Raspberry Pi bring-up — lives in
[chumby-pi](https://github.com/yanosz/chumby-pi), which pins this fork as a
submodule. Product scope ("which screens does the appliance offer") is
decided there; this document records what the *panel* demands of a player,
and which of those demands we satisfy.

---

## 1. Functional requirements

### FR1 — Run the original control panel unmodified

`controlpanel.swf` is never edited, recompiled, or bytecode-patched. All
behavior is exerted from the Rust side: what native calls return, what
commands print, what files contain, what URLs answer, and what properties
display objects carry. This is the project's first hard rule and it
constrains every design choice below.

### FR2 — Provide the six player extensions the panel expects

The panel was written for chumby's own Flash Lite player. Beyond stock
Ruffle it needs exactly these mechanisms; everything else in this document
is routing data for them.

| # | Mechanism | What the player must provide |
|---|-----------|------------------------------|
| M1 | `ASnative(5,N)` table | ~140 vendor functions, registered before frame 1 runs. Stray `ASnative(4,39)` (`_batteryPower`) is bound but never called. |
| M2 | `exec://` URL scheme | `XML.load("exec://CMD")` runs CMD; stdout becomes the loaded document (parsed as XML, or consumed raw via `onData`). |
| M3 | `_backtick(cmd)` = `ASnative(5,52)` | Synchronous shell exec returning stdout as a string. Same command space as M2. |
| M4 | `file://` loads | Directory listing (`XML.load("file://DIR")` → `<directory><file name=…/></directory>`, used by the music file finder and the geek browser) and plain file reads (`file:////LICENSES/gpl.txt`). Note the quirky multi-slash forms the panel emits: `file:///`, `file:////usr/...`. |
| M5 | FlashVars injection | `-Pname=value` root variables plus a player-supplied `$version`. |
| M6 | Master/slave dual-movie system | Widgets and the intro run as a *second player instance* on an overlay, driven by natives 72–89, 110–119, 210–211, 330–332, 360–364, 380–387, with variables exchanged through `_setSlaveVar`/`_getSlaveVar`. **We do not implement this** — see FR7. |

Outside the SWF the panel also assumes a heartbeat contract: it writes
`/tmp/movieheartbeat` itself every 15 s via `_putFile` (F2:174) to satisfy a
cron watchdog, and `fscommand("quit")` is honored only when the player was
started with `-Q`.

### FR3 — Answer the vendor-call table

Every index the panel or a companion SWF binds must *exist* — the table is
built unconditionally at startup, so a missing entry only fails at call time,
but several unused indices are probed by widgets rather than the panel. The
per-index reference (purpose, arguments, return value, what the fixture host
answers) is the fork's [`README.md`](../README.md) §"ASnative(5,N)
reference", which is the maintained source; it is not duplicated here.

The families that must behave *sensibly*, not merely exist:

- **Filesystem** — `_getFile` (5,50), `_putFile` (5,51), `_fileExists`
  (5,53), `_fileSize` (5,54), `_unlink` (5,55), `_getDirectoryEntry`
  (5,320). 59 + 50 + 63 static call sites respectively; used on every screen.
- **Shell** — `_backtick` (5,52), 50 call sites.
- **Slave lifecycle** — 5,80–89, 110–119. `_getSlaveVar("_chumby_widget_done")`
  **must eventually return `"true"`** or the panel hangs waiting for the
  intro/widget to finish.
- **Audio** — `_getAudioPlayerState` (5,131) and pause/resume/stop
  (5,132–134), `_isAudioPlayerAvailable` (5,140), `_playAudio` (5,144),
  `_playAudioLoopCount` (5,146). The state constants are the SWF's, not the
  wiki's: IDLE −1, PAUSED 0, WAITING 1, PLAYING 2.
- **Volume** — `_get/_setSystemVolume`, `…Balance`, `…Mute` (5,180–185),
  plus the `chumby_set_volume|pan|mute` backticks that do the same job.
- **Pure functions** — `_base64Encode/Decode` (5,160/161), `_md5Sum`
  (5,162), `_blowfishEncrypt/Decrypt` (5,163/164). Real implementations, no
  fixtures.
- **Identity** — `_getPlatform` (5,202) → `ironforge`, `_getEnvironment`
  (5,205) → `LANGUAGE`, `CONFIGNAME`.
- **Time** — `_setSystemTime` (5,176), `_getTimeZone` (5,177),
  `_setTimeZone` (5,178). These must round-trip: set → get returns what was
  set (verified against real hardware).

The panel does **not** use the brightness natives; it writes
`/proc/sys/sense1/brightness` (0–65535) directly via `_putFile` (F2:9119).

### FR4 — Answer the shell-command catalog

The panel's command strings are the keys; no script by these names needs to
exist. The ones on the boot path or on a screen in scope:

| command | via | site | must return |
|---------|-----|------|-------------|
| `guidgen.sh` | `exec://` | F2:204 | GUID text (chomped, uppercased) |
| `macgen.sh` | `exec://` | F2:245 | MAC text |
| `network_status.sh` | `exec://` | F2:286 | `<network><configuration type ssid auth encryption/><interface ip netmask gateway nameserver1 nameserver2>[<error/>]</interface></network>` |
| `signal_strength` | `exec://` + backtick | F2:8110, 27226 | `<wifi connected linkquality signalstrength/>` |
| `chumby_version -h/-s/-f/-n` | backtick | F2:30551–66 | version strings |
| `md5sum /tmp/.guidhash` | backtick | F2:1946 | `<md5>  <file>` |
| `chumby_set_volume\|pan\|mute [n]` | backtick | F2:9456… | nothing, or the current value |
| `sync_time_state.sh 0\|1` | `exec://` | F2:16755 | nothing |
| `dcid -o` | backtick | F2:9597 | DCID XML |

The rest of the catalog — `ap_scan`, `start_network`, `restart_network`,
`network_adapter_list.sh`, the firmware-update machinery, the intercom
stack, the iPod/FM daemon probes — belongs to screens the appliance does not
offer. Those must fail *cleanly*: an unknown command logs loudly and returns
an empty response, which the panel tolerates everywhere we have observed.
Several skip decisions are implemented exactly this way — a music source
whose probe fails cleanly removes itself from the source list.

`<error/>` inside `<interface>` is what makes the panel believe the network
is down. A healthy `network_status.sh` answer is what keeps the wifi wizard
from ever appearing, and what sets `Object._chumby.hasNetwork`, which in
turn gates the dashboard thumbnail and the Info screen's registration block.

### FR5 — Present the filesystem the panel assumes

All persistence is plain files — there is no SharedObject and no AMF
anywhere in the panel, so no AMF0 fixtures are needed. Reads and writes must
be confined to a virtual root (the panel can be directed at arbitrary paths
through `/mnt/usb/externalmusic.xml`, so confinement is a security
requirement, not a convenience).

- **`/psp`** (persistent): `firsttime` (writing `"0"` ends the wizard),
  `clock_format`, `touchclick`, `dimlevel` (`"2"` boots into night mode),
  `nooverlay` (`"1"` = single framebuffer — required for us),
  `timezone_city` (`"City\tCountry"`), `timezone`, `use_ntp`, `alarms`
  (XML, default written if missing), `alarm_volume`, `url_streams`,
  `hostname`, `volume`, `profile.xml`, and ~20 more.
- **`/tmp`** (volatile): `movieheartbeat`, `nightmode`, `.guidhash`,
  `channel_names`, `widget_names`, `currentProfileID`, `currentProfileName`,
  `controlpanelversion`, `change_profile` (polled — an external
  profile-switch request), `widgetcache/`.
- **Device files**: `/proc/sys/sense1/brightness` (write, 0–65535),
  `/var/run/btplay.pid`, `/etc/{hardware,software}_version`,
  `/etc/firmware_build`, `/LICENSES/{gpl,lgpl}.txt`,
  `/usr/chumby/alarmtones/<name>.mp3`, `/usr/widgets/intro.swf`.
- **`/mnt`**: `usb`, `usb2..4`, `storage` existence probes and their
  contents (alarm sounds, `post_alarm_action`, update images, widget cache).

### FR6 — Answer the panel's HTTP traffic in-process

`makeURL` prefixes `baseURL` (default `http://xml.chumby.com`) onto
`/`-rooted paths; `makeWidgetsURL` does the same with `widgets.chumby.com`.
Both bases can be **rewritten at runtime** by the `/xml/chumbies` response
(attributes `baseurl`/`widgetsurl`, F2:4186) — the hook zurk's offline
firmware exploited.

Nothing may reach the live chumby.com. The boot path needs
`/xml/chumbies`, `/xml/authorize`, `/xml/profiles`, and
`update.chumby.com/update` (answering "no update"); the ad, stats, social
and music-directory endpoints may all return empty or error. The parameters
`defaultUpdateTime=9999` / `defaultProfileTime=9999` are the panel's own
documented way to quiet its polling loops.

The profile schema is loose and forward-compatible: the panel reads only the
named nodes it knows (`<widget>`, `<mode>`, `<movie href>`,
`<thumbnail href>`), ignores unknown elements, and reads absent ones as
`undefined` rather than crashing. `file://` hrefs are accepted for widget
movies and thumbnails.

### FR7 — Reach the main screen without a server, without the slave player

Two decisions collapse most of the work:

- **`localCache=1`**: the panel then renders widgets *in-movie* via
  `loadMovie` instead of handing them to a slave player instance. The
  master/slave system (FR2 M6) therefore stays a set of logging stubs, with
  `_getSlaveVar("_chumby_widget_done")` returning `"true"` so handoffs never
  hang.
- **`builtin=1`**: the panel's own dispatcher reaches the `main` frame once
  (a) network status reports healthy and (b) the clock is sane. Both are
  environment answers. No Rust-side frame control or variable injection is
  needed — the wizard skip is "the panel believes the network is up".

`--load-behavior blocking` is mandatory: stock Ruffle's default streaming
load makes `gotoAndStop("builtin")` fail with *frame label not found*,
because `dispatch()` runs from frame 2 before later frames are parsed. The
real chumby player loads the whole file first, so blocking is also the
faithful behavior.

### FR8 — Keep the environment swappable

Everything the panel can observe about the outside world goes through one
trait, so the same player binary can be backed by canned data on a desktop
and by the real system on a Pi, and so a single log line names the fixture
that would answer a given call.

### FR9 — Neutralize controls the host platform cannot support

Some panel controls are meaningless or harmful on a Pi: the timezone picker
and NTP toggle (the OS owns time), the network and touchscreen-calibration
settings, brightness while it is unwired, channel management while there is
only one local channel, and the social buttons. They must be *visibly*
disabled rather than silently ignored — a control that looks live but does
nothing is the worst outcome, especially around alarms.

Requirements on the mechanism:

- **Stable element identification.** Instance paths are not uniformly
  stable: unnamed instances get AVM1 auto-names (`instanceN`) from a global
  counter that depends on navigation history. Depths, in contrast, are
  authored into the SWF. So each element needs a list of selector
  alternatives, tried in order, first match wins; an unresolved primary
  selector is a logged WARNING, never a silent no-op.
- **Type-generic actions**, operating at the display-object level so no
  assumption is made about the control's class: `hide` (`_visible = false`),
  `disable` (`enabled = false` on the target *and its children*, because
  AVM1 `enabled` does not cascade and the hit handlers sit on inner clips —
  plus a lowered `_alpha` so the state reads as "shown, not changeable"),
  `readonly` (input TextField → dynamic/unselectable), and `tint` (an AVM1
  color transform).
- **Idempotent re-application**, because the SWF re-initializes controls on
  screen entry (`fixButtons()`).
- **Rules are declarative data, owned by this repository.** Which of the
  panel's controls are dead is a fact about the panel and about what this
  player can honour, not about whoever packages it — this fork exists to run
  one SWF. They are compiled in, so the player always has them.

### FR10 — Report real device state on status surfaces

The Info screen and the dashboard signal meter are diagnostics: the user
reads them to find out why the network is broken. Every field on them must
be **derived from live state**. This is a requirement born from a defect —
a hardcoded `type="lan"` kept the page showing "Ethernet" with a blue bar
after the device moved to wifi.

Concretely, the network touchpoints must report the real default-route
interface, its real type (wireless iff `/sys/class/net/<if>/wireless/`
exists), real IPv4, netmask, gateway, DNS and MAC; and `signal_strength`
must report real link quality on wifi and real link state on ethernet. When
nothing is connected, the reader yields nothing and the fixture answers
instead, so a desktop or CI run with no usable network behaves as before.

> **Open defect.** `real_net.rs` still emits a constant `type="lan"` and a
> constant `connected=1 linkquality=100`, and the ethernet tint is applied
> by a static rule. See [design.md](design.md) §7 for what exists today and
> what has to change.

### FR11 — Audio

`_playAudio` and the btplay control family drive a real audio backend
(mpv, as a spawned process over its JSON IPC socket). The alarm ring-through
and stream playback are the flows that must work audibly. Where no backend
is available (CI, a headless dev box) the player degrades to a silent stub
and says so in the log.

### FR12 — Input on a device with no keyboard and no mouse

- **Touch** must work: upstream Ruffle ignores Wayland touch entirely
  (a touch is not a pointer), so touch events are mapped to left-button
  mouse events.
- **The bend sensor** — the chumby's squeeze gesture that summons the
  control-panel bar — is raised by a stationary long-press (≥1 s, ≤12 px
  movement), by the Home key, and by a control command. The panel polls
  `_bent` every frame, so one press/release pair per gesture is what it
  needs.
- **A control channel** (a FIFO) accepts `bend`, `bend down|up`,
  `click X Y`, `drag X1 Y1 X2 Y2`, so the panel can be driven from a script
  over SSH. Pointer commands go through the same window→movie coordinate
  mapping as real input, one action per event-loop iteration (sliders need
  the sequence spread across ticks).

---

## 2. Non-functional requirements

### NFR1 — The patch stays clean and rebasable

All chumby-specific Rust lives in one module, `core/src/chumby/`, which has
no upstream counterpart and rebases without conflict. Upstream files carry
only registration hooks, and **every hook site contains the word `chumby`
in a comment**, so `grep -rn chumby` over a touched file finds the patch to
re-apply. The current patch surface is enumerated in
[design.md](design.md) §8.

The chumby code is **always compiled**. The `chumby` cargo feature and every
`#[cfg(feature = "chumby")]` gate were removed once it became clear the fork
is never used without them. Re-introducing gating during an upstream rebase
would be a deliberate re-decision, not drift resolution.

### NFR2 — Reimplement device touchpoints in Rust, not in shell

The exec keys (`network_status.sh`, `signal_strength`, …) are the SWF's
command strings, not scripts to ship. Read `/proc`, `/sys`, netlink, or call
`getifaddrs` — do not shell out to `iw`/`ip`, and do not install helper
scripts on the device. This is a *principle*: deviate when there is a
convincing reason, and say why.

### NFR3 — Degrade, don't crash

Errors are normal results. Unknown natives return category-sensible defaults
(getters → `0`/`""`, setters → no-op); unknown commands return empty; a URL
we do not claim passes through to the real navigator; a live reader that
finds nothing falls back to the fixture. The panel was written to tolerate a
hostile environment and does so.

### NFR4 — Observability: the log line is the fixture name

Every host call is logged under the `chumby_host` target with its arguments
and result, keyed by the panel's own request string (command line, URL path,
filesystem path). There is no translation layer between what the panel asked
for and what a fixture file is called, so a log line tells you exactly which
file to create. Stock Ruffle gives **zero** signal here — an unmapped
`ASnative` category returns `Undefined` silently at any log level — so this
logging is the only runtime visibility into native usage.

### NFR5 — Targets

Linux amd64 (desktop development) and aarch64 (Raspberry Pi 3B+). The wasm
build must keep compiling: platform-specific code is `#[cfg(unix)]`-gated
with a stub for everything else. Nothing may hardcode the display
resolution — the panel hardcodes its own layout at 320×240 (F2:2278) but
forwards `System.capabilities.screenResolutionX/Y` to widgets, and the real
device runs 480×320.

### NFR6 — No traffic to chumby.com, ever

Not on the boot path, not in CI, not in a desktop run. The device GUID must
not leak.

### NFR7 — Performance headroom

The content was authored for a 350 MHz ARM9; on a Pi 3B+ at 480×320 and
12 fps the player must stay near one core. The floor is set by software
rasterization, not by the SWF.

### NFR8 — Acceptance is "the movie runs"

A build that compiles proves nothing — an upstream merge can compile clean
and still break the ASnative hooks. The gate is: the player starts
`controlpanel.swf`, executes a chumby native, and is still alive at a
timeout, with no panic. This runs in CI on every push.

---

## 3. Known gaps

Carried forward, in the order they are expected to land.

| Gap | Note |
|-----|------|
| Geek + intro buttons still live | The Info screen's `piButton` (the "π" geek trigger, `frame_2` ~27145) and `introButton` were recorded as disabled but no such rules were ever written. Geek is reachable, and the intro button cannot do anything on the localCache path. Two `disable` rules are owed. |
| Network type hardcoded (FR10) | `real_net.rs` reports constant `type="lan"` and constant full signal; the ethernet tint is a static rule. Must derive from `/sys/class/net/<if>/wireless/`; wifi SSID needs nl80211 (sysfs has none), signal comes from `/proc/net/wireless`. |
| Static-field audit (FR10) | Sweep the whole `network_status.sh` + `signal_strength` output for any other value that is not read from live state. |
| `_getDirectoryEntry` (5,320) | `RootFs::dir_entry` exists; the native still stubs "end of listing". Needed for USB/local-file music browsing. |
| Brightness | The panel's `/proc/sys/sense1/brightness` writes and `_setLCDMute` (5,20) are not mapped to a real backlight. Blocked on display hardware that can dim. |
| Intro widget | `playIntro` (F2:5289) loads `intro.swf` only through `_startSlave`, which we do not run. Since we own the interpreter, the fix is VM-level interception rather than reviving the slave system or editing the SWF. |
| `clock_format` live update | The 12/24h toggle persists to `/psp/clock_format`, but a running widget only reads it at start. Real hardware pushes `_setSlaveVar("_chumby_clock_format", …)` every heartbeat; the in-movie path has no slave-var bridge. Recorded, no fix planned. |
