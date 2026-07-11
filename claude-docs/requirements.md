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
| M4 | `file://` loads | Plain file reads (`file:////LICENSES/gpl.txt`) — implemented. Note the quirky multi-slash forms the panel emits: `file:///`, `file:////usr/...`. Directory listing (`XML.load("file://DIR")` → `<directory><file name=…/></directory>`) we do **not** implement: its only callers are the geek browser (out of scope) and `MP3FilesPanelScanner`, which sits on a dead music-panel frame (`xmp3files`, renamed away) — the *live* music finder is `FileFinderPOSIX` on `_getDirectoryEntry` (confirmed in the tag dump, 2026-07-10). |
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
  `_getDirectoryEntry(obj, path, index)` fills `obj` with `_name`, `_path`,
  `_isDir`, `_isDirLink`, `_isFile` and returns 1 / 0 / −1 (entry / end of
  listing / invalid path). The panel iterates ascending indices, resumable
  across frames (`FileFinderPOSIX`, 200 per frame), so listing order must be
  stable call-to-call; all filtering — dotfiles, directory symlinks
  (`_isDirLink`, its loop protection), `usb-*` mounts, music extensions —
  is the panel's own. This native is what drives Music → My Music Files
  and the mp3files alarm browser (USB/local music, delivered 2026-07-11).
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
| `guidgen.sh` | `exec://` | F2:204 | GUID text (chomped, uppercased) — real, derived from the machine serial (FR10) |
| `macgen.sh` | `exec://` | F2:245 | MAC text |
| `network_status.sh` | `exec://` | F2:286 | `<network><configuration type ssid auth encryption/><interface ip netmask gateway nameserver1 nameserver2>[<error/>]</interface></network>` |
| `signal_strength` | `exec://` + backtick | F2:8110, 27226 | `<wifi connected linkquality signalstrength/>` |
| `chumby_version -h/-s/-f/-n` | backtick | F2:30551–66 | version strings (`-h/-s/-f` fixtures = platform identity; `-n` real, FR10) |
| `md5sum /tmp/.guidhash` | backtick | F2:1946 | `<md5>  <file>` — computed for real from the rootfs (FR10) |
| `chumby_set_volume\|pan\|mute [n]` | backtick | F2:9456… | nothing, or the current value |
| `sync_time_state.sh 0\|1` | `exec://` | F2:16755 | nothing |
| `dcid -o` | backtick | F2:9597 | DCID XML |
| `reload_backup_alarm` | backtick | F2:11981 | nothing (the watcher polls the file — FR13) |
| `rm /psp/ifalarm; reload_backup_alarm` | backtick | F2:11986 | nothing, but **must really delete the file** (FR13) |

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
requirement, not a convenience). `externalmusic.xml` itself — the panel's
plugin hook: a stick may declare extra music sources, each with a SWF panel
the player loads — **stays faithful** (decision 2026-07-10, Jan): it only
affects the user's own stick, and confinement plus the fixed exec catalog
bound what such a panel can reach. The chumby treats USB as read-only media
(no `_putFile`/`_unlink` to `/mnt` anywhere in the panel; the
widgetcache-on-USB opt-in is not our path), so a read-only mount is
faithful.

- **`/psp`** (persistent): `firsttime` (writing `"0"` ends the wizard),
  `clock_format`, `touchclick`, `dimlevel` (`"2"` boots into night mode),
  `nooverlay` (`"1"` = single framebuffer — required for us),
  `timezone_city` (`"City\tCountry"`), `timezone`, `use_ntp`, `alarms`
  (XML, default written if missing), `alarm_volume`, `url_streams`,
  `hostname`, `volume`, `profile.xml`, and ~20 more.
- **`/tmp`** (volatile): `movieheartbeat`, `nightmode`, `.guidhash`,
  `channel_names`, `widget_names`, `currentProfileID`, `currentProfileName`,
  `controlpanelversion`, `change_profile` (polled — an external
  profile-switch request), `widgetcache/`. Real hardware's `/tmp` is a
  ramdisk; our rootfs persists it, and the difference is observable:
  a surviving `/tmp/musicsource` makes the Music panel offer a resume
  PLAY that is inert for mp3files (`resumeFrom()` replays an in-memory
  track list a fresh process doesn't have — found 2026-07-11). The host
  deletes `musicsource` at start; full `/tmp` volatility is a §3 gap.
- **Device files**: `/proc/sys/sense1/brightness` (write, 0–65535),
  `/var/run/btplay.pid`, `/etc/{hardware,software}_version`,
  `/etc/firmware_build`, `/LICENSES/{gpl,lgpl}.txt`,
  `/usr/chumby/alarmtones/<name>.mp3`, `/usr/widgets/intro.swf`.
- **`/mnt`**: `usb`, `usb2..4`, `storage` existence probes and their
  contents (music files, alarm sounds, `post_alarm_action`, update images,
  widget cache). `mnt/usb/` in the fixture tree carries generated sine-tone
  MP3s (nothing copyrighted — committed) so desktop and CI runs always have
  a browsable music tree; on the Pi the appliance replaces it with a symlink
  to the real USB automount. `usb2..4`/`storage` stay honestly absent.

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

One decision collapses most of the work: **`localCache=1`**. The panel then
renders widgets *in-movie* via `loadMovie` instead of handing them to a
slave player instance. The master/slave system (FR2 M6) therefore stays a
set of logging stubs, with `_getSlaveVar("_chumby_widget_done")` returning
`"true"` so handoffs never hang.

Nothing else is injected. The panel's own dispatcher takes its normal device
path (`validate` → `main`) once (a) network status reports healthy and
(b) the clock is sane — both environment answers, so the wizard skip is "the
panel believes the network is up" rather than any Rust-side frame control.
(`-Pbuiltin=1` exists in the panel and jumps straight to its offline clock,
bypassing `validate`. It is a debug shortcut, not the path we ship.)

`--load-behavior blocking` is mandatory: stock Ruffle's default streaming
load makes `dispatch()`'s `gotoAndStop` fail with *frame label not found*,
because it runs from frame 2 before later frames are parsed. The real chumby
player loads the whole file first, so blocking is also the faithful
behavior.

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

Requirements on the mechanism (how it is built: [design.md](design.md) §5):

- **Stable element identification**, robust against the AVM1 auto-names that
  depend on navigation history. An element that cannot be resolved is a
  logged WARNING, never a silent no-op.
- **Type-generic actions**, operating at the display-object level so no
  assumption is made about the control's class: `hide`, `disable`,
  `readonly`. A disabled control must read as "shown, not
  changeable".
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
exists), real IPv4, netmask, gateway, DNS and MAC; on wifi,
`signal_strength` must report real link quality, signal and SSID. On a
wired link `signal_strength` answers `connected="0"` — the dashboard meter
is a wifi meter and hides; the Info screen carries the wired diagnostics
(decision 2026-07-10, replacing the earlier blue-tinted-bars repurposing as
disproportionate). When nothing is connected, the reader yields nothing and
the fixture answers instead, so a desktop or CI run with no usable network
behaves as before.

Static-field audit (2026-07-10): every attribute of `network_status.sh` and
`signal_strength` is now live except `auth`/`encryption`, which are
deliberately empty — the panel renders a bare ssid line for unrecognized
auth values, and reading the security mode would need nl80211 for a
decorative suffix. Mechanism: [design.md](design.md) §7.

**Device identity** (2026-07-10, pulled forward from the registration
milestone — registration itself stays out, and the GUID still never leaves
the process): the Info screen's `id:` and `HW#:` lines are real, replacing
the crypto processor the original hardware read (`guidgen.sh` =
`cpi.sh -p`, `chumby_version -n`). The seed is the SoC serial from
`/proc/device-tree/serial-number`; a machine without one (dev box, CI)
gets a random v4 GUID instead, generated at first start and persisted as
`/psp/guid` in the virtual rootfs (gitignored) — per-box, stable across
runs (decision 2026-07-10; a shared fixed GUID and, before that, an
`/etc/machine-id` seed were both rejected). In-player generation survived
the registration milestone unchanged (2026-07-11): rather than move it, the
identity passthrough is gated on `real_ident::serial().is_some()`, so the
random dev GUID is structurally unable to reach the wire and a CI run can
never present a registrable identity (NFR6). The Pi registers with its
serial-derived GUID, which is stable across reboots by construction — so
registration needs nothing persisted locally. The fixture GUID remains the last resort
when entropy or the rootfs write fails — a fixed answer beats one that
changes every boot. GUID = salted md5 of the serial as an uppercase
8-4-4-4-12 string; `HW#` = `<model>-<serial>` where model is "RPI3B"-style
from `/proc/device-tree/model`, or "PC" elsewhere — the panel has no model
field, so the tag rides on the serial line. `md5sum <path>` is computed
for real from the virtual rootfs. `hardware_version` stays `3.8` — platform
identity gates panel behaviour and is not display text. Versions are
platform identity too, answered from fixtures; ground truth for the values
is the original device's `chumby_version` Perl script: `-s` prints
`/etc/software_version` (`1.7.2`), `-f` prints the **third dot-field** of
`/etc/firmware_build` (`1.7.1830` → `1830`) — a wrong `version_f.txt` had
the Info screen showing 1.7.2 twice.

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

### FR13 — Backup alarm: the dead-man beep

Real hardware ran `chumbalarmd`, a standalone daemon whose job was to wake
the user even when the primary alarm's content failed — its help text:
*"will wake you with a loud beep if you don't respond to your primary
alarm."* The typical failure it guards on the Pi: WLAN loss while a net
stream is the alarm (mpv plays its ~5 s cache, sits silent for its 60 s
network timeout, exits; the panel never notices — measured 2026-07-10).

The contract is one file, owned by the panel (AlarmSet, F2:11952):
`/psp/ifalarm` holds the fire time in epoch seconds (primary alarm +
per-alarm `backupDelay` minutes); answering the ring screen deletes it. If
the time passes while the file exists, the host **must** sound a loud tone —
one that shares no fate with the primary playback: its own mpv child, a
local tone source, no network on the path (`backup_alarm.rs`).

Decisions (Jan, 2026-07-10, scoping A2 of the options ladder):
- **In-process**, not a separate daemon: covers content failure, crash
  (systemd restarts the player) and power loss (boot check), accepting that
  a *hung* player kills the watcher with it. Proportionality: hangs have not
  been observed on our player; chumbalarmd's full fate-independence bought
  insurance against a flakiness we don't have.
- **Missed-alarm boot path is in scope**: a fire time found already in the
  past sounds immediately — unless older than **1 hour**, then it is a
  stale leftover and is cleared silently.
- **Fixed duration**, not until-dismissed: `/psp/backup_alarm_duration`
  seconds (default 60) — the knob chumbalarmd also read; dismissal during
  the tone stops it early.
- Tone is `Klaxon.mp3` from the shipped alarmtones (local file), falling
  back to an mpv-generated sine if missing; volume from
  `/psp/backup_alarm_volume` (default 100), through mpv only — no
  sink/hardware volume writes until the on-device check says it is too
  quiet.

### FR14 — Player configuration file

Owner-level knobs live in `<fixtures>/player.toml`, read **once at player
start**; there is no write support and no reload. The file sits at the
fixtures root, deliberately outside `rootfs/`: everything under the virtual
rootfs is reachable by the panel's own `_putFile`, and the panel must not be
able to reconfigure the player. Missing file means defaults; a broken one
logs and answers defaults (NFR3).

| key | default | meaning |
|-----|---------|---------|
| `volume_cap` | 100 | Percent. **A scale, not a clamp** (Jan, 2026-07-11): the panel's 100 % maps to the cap, proportionally below. Panel space stays honest 0–100 everywhere the panel reads it back (`/psp/volume`, `_getSystemVolume`); only what reaches the audio backend is scaled. The backup-alarm Klaxon (FR13) deliberately ignores the cap — it keeps its own `/psp/backup_alarm_volume` knob at full range (Jan, 2026-07-11). |
| `access_chumby_com` | 0 | Opt-in chumby.com traffic. Today it gates exactly the music proxies (FR15): the SHOUTcast / blue octy radio / Sleep Sounds sources appear and their hosts pass through. Off (the default), NFR6 holds unconditionally. The remote-channels milestone will widen it. |
| `enable_lyrion` | 0 | Shows the Squeezebox Server source (FR15). The player side is complete; the server side is unverified and out of scope (Jan, 2026-07-11). |

The committed template is `fixtures/player.toml.example`; the live file is
gitignored.

### FR15 — Music sources policy

The panel ships fourteen music sources (`MusicPlayer.musicSources`,
F2:12798); a source is listed iff its player's `exists()` probe passes —
except iPod, which `availableSources` force-shows on every platform but
`insignia3.5` regardless of its probe (F2:12809). Scope re-decided
2026-07-11 against live endpoints:

- **Working locally**: My Streams (`directurl`), My Music Files
  (`mp3files`). **Self-hiding** through their own failing probes: FM Radio,
  MP3tunes, Internode, and the internal `alarm`/`user` pseudo-sources.
- **Hidden always** (spliced out of `musicSources` at VM level,
  `music_sources.rs`): `ipod` (no daemon, and the probe bypass makes it
  otherwise unhideable), `noaa` (Wunderground sunset the wxradio relay —
  the VHF service lives, its internet directory doesn't) and `cbspodcasts`
  — NOAA and CBS confirmed non-working on a real chumby (Jan, 2026-07-11).
- **Hidden unless `access_chumby_com=1`**: `shoutcast`, `chumbcast`,
  `sleepcast`. The revived chumby.com (Blue Octy) still operates their
  proxies — verified live 2026-07-11, including in-panel SHOUTcast
  directory browsing and audible station playback. With the flag on, the
  two music hosts (`shoutcast.chumby.com`, `bor.chumby.com`) pass through
  the navigator; `xml.chumby.com` and every other chumby host stay
  fixture-answered. The requests carry `config=ironforge`, station ids and
  `ssi` — an obfuscated *timestamp* (F2:2282), not a device identity; the
  GUID never leaves the process.
- **Hidden unless `enable_lyrion=1`**: Squeezebox Server (`slimserver`) —
  LAN-only (`http://<ip>:9000/stream.mp3`, IP persisted in
  `/psp/slimserver_ip`), no chumby.com involvement. The player side is
  complete: playIP feeds the same `_playAudio` → mpv path the proven
  sources use. Whether Lyrion 9.x still answers the legacy HTTP-player
  stream is unconfirmed and out of scope (Jan, 2026-07-11); see §3.

The alarm audio list iterates the same array, so a hidden source is gone
from the alarm wizard too. `/psp/music_order` (the panel's own file) only
reorders and cannot remove; externalmusic.xml sources are unaffected.

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

Linux amd64 (desktop development) and aarch64 (Raspberry Pi 3B+). Upstream's
wasm target is **not** one of ours — `audio.rs` needs a Unix socket for mpv,
so `ruffle_core` no longer builds for `wasm32` (development.md §5). Nothing
may hardcode the display resolution — the panel hardcodes its own layout at 320×240 (F2:2278) but
forwards `System.capabilities.screenResolutionX/Y` to widgets, and the real
device runs 480×320.

### NFR6 — No traffic to chumby.com, ever

Not on the boot path, not in CI, not in a desktop run. This is the
unconditional default. The *owner* may relax it with `access_chumby_com=1`
in player.toml, which opens two slices: the music proxies (FR14/FR15,
2026-07-11), which carry no identity; and — since the registration
milestone (2026-07-11) — the two registration endpoints `/xml/authorize`
and `/xml/registerchumby`, which *do* carry the device GUID to chumby.com.
That identity slice is doubly gated: the flag must be on **and** the machine
must have a real hardware serial (`real_ident::serial().is_some()`), so a
dev box or CI run — which has no serial and would present the random dev
GUID — stays structurally offline for identity and can never register
(verified locally: flag on + no serial → authorize fixture-answered, boots
to main). Everything else on chumby.com hosts stays fixture-answered in
every configuration; the GUID leaves the process only on the owner's
explicit opt-in from real hardware.

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
| `/tmp` volatility | Real hardware's `/tmp` is a ramdisk, wiped per boot; the virtual rootfs persists it. The one observable consequence (stale `/tmp/musicsource` → inert resume PLAY) is fixed pointwise — the host deletes that file at start. Clearing all of `/tmp` at start would be faithful, but collides with the boot machinery: `chumby-widget-channel` pre-writes `currentProfileID/Name` there before the player starts, and seven committed fixtures live there. A session of its own, if ever. |
| Brightness | The panel's `/proc/sys/sense1/brightness` writes and `_setLCDMute` (5,20) are not mapped to a real backlight. Blocked on display hardware that can dim. |
| Boot-time intro | The in-panel half shipped 2026-07-11: the INTRO button plays `intro.swf` on the localCache path via prototype replacement (`chumby/intro.rs`, design §3), and the `enable_intro`/`disable_intro` backticks really toggle `/psp/disable_intro` in the rootfs. Player-side readiness **fully verified 2026-07-12** (development.md §5): the standalone ending — exit → control screen → both flag buttons writing/removing the flag → `fscommand("quit")` actually exiting the process (the swallow stands down without `Object._chumby`) — and a same-session INTRO replay both work. What remains is purely the *boot* entry point: real hardware's `rcS` runs `start_intro` — a standalone player on `intro.swf`, every boot until `/psp/disable_intro` exists — *before* the panel. Undecided (Jan): a pre-panel player invocation in the appliance launcher (faithful, chumby-pi side) vs. skipping it. |
| `clock_format` live update | The 12/24h toggle persists to `/psp/clock_format`, but a running widget only reads it at start. Real hardware pushes `_setSlaveVar("_chumby_clock_format", …)` every heartbeat; the in-movie path has no slave-var bridge. Recorded, no fix planned. |
| Squeezebox / Lyrion | Out of scope (Jan, 2026-07-11); the source hides behind `enable_lyrion=0`. The player side is done — what is open is purely server-side: a scratchpad-extracted Lyrion 9.1.1 booted and answered JSON-RPC, but its `/stream.mp3` 404'd with "invalid skin", an artifact of the improvised install, so whether Lyrion 9.x still speaks the legacy HTTP-player stream was never settled. If ever revisited: a properly installed LMS, and note the dev box's port 9000 belongs to ThinLinc (the panel hardcodes that port). |
| Social surface — **out of scope** (Jan, 2026-07-11) | The add-widget catalog browse (`/xml/categories`), rating (`/xml/ratewidgetinstance`), and send-to-a-friend / mail (`/xml/sendwidgetinstance`, `/xml/sendmail`) are deliberately not implemented. The `main-send`/`main-rate` bar controls stay permanently disabled (ui-policy, unconditional) and those endpoints are never passed through. This closes roadmap item 5; registration + remote channels (Phases 0–2) shipped and verified live 2026-07-11 — mechanism in [design.md](design.md) §12, device deploy in chumby-pi's development.md. Not a gap, recorded so the decision isn't re-litigated. |
