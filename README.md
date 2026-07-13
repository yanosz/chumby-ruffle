# chumby-ruffle — a Ruffle fork that runs the Chumby Classic control panel

This is a fork of [Ruffle](https://ruffle.rs) that runs the **Chumby
Classic control panel** — the original `controlpanel.swf` from the
device firmware — as if it were still running on chumby hardware. It is
the player half of the [chumby-pi](https://github.com/yanosz/chumby-pi)
project, which turns a Raspberry Pi with a small touchscreen into a
Chumby; that repository holds the fixtures, packaging, and documentation
and pins this fork as a submodule.

> **Note:** an AI-generated hobby project, not actively maintained. See
> [chumby-pi](https://github.com/yanosz/chumby-pi) for the whole picture.

## How it works

The control panel was written for chumby's own Flash player, and it
expects to reach the device through three channels that ordinary Ruffle
knows nothing about:

- **Vendor functions** — a table of about 140 chumby-specific calls for
  volume, brightness, the bend sensor, the filesystem, shell access, the
  audio player, and more.
- **Shell commands** — the panel runs commands and reads their output.
- **The device filesystem** — it keeps its state as files under paths
  like `/psp` and `/tmp`.

The fork answers all three from a swappable **virtual chumby**: vendor
calls, command output, and files are served from a fixtures directory
rather than from real hardware. Only where a canned answer would lie does
the player consult the machine it runs on — the network diagnostics read
live kernel state, and audio really plays. The result behaves like a real
chumby: the clock plays, alarms ring, radio streams, and settings persist.

## The code

All chumby code lives in `core/src/chumby/`:

| file | what it does |
|------|--------------|
| `host.rs` | the `ChumbyHost` trait — the boundary between the panel and its environment |
| `fixture.rs` | the fixtures-backed host and its virtual filesystem |
| `real_net.rs` | overlays the network calls with live kernel state |
| `avm.rs` | the vendor-function table |
| `navigator.rs` | intercepts the panel's `exec://` and chumby.com requests |
| `audio.rs` | plays audio through mpv |
| `brightness.rs` | drives a real backlight (kernel sysfs, or an owner-configured executable) |
| `input.rs` | feeds simulated bend-sensor and pointer input |
| `ui_policy.rs` | dims and disables panel controls the host platform can't support |

Everything else is a handful of small registration hooks in upstream
files (grep for `chumby`). `claude-docs/design.md` §8 lists them and is
the guide for rebasing onto new upstream releases.

For the engineering record behind this fork — what the panel demands of a
player, why the host boundary looks the way it does, and how to build,
verify and merge upstream — see `claude-docs/`
([requirements](claude-docs/requirements.md),
[design](claude-docs/design.md),
[development](claude-docs/development.md)).

## The host boundary

Everything the panel asks of its environment passes through one trait,
`ChumbyHost`, with a method per kind of traffic: vendor calls, shell
execution, URL fetches, and filesystem access. `FixtureHost` answers all
four from a fixtures directory (`--chumby-fixtures`). Because the boundary
is this narrow, a host that reports something real can be layered on top of
it: `RealNetHost` does exactly that, overriding the network calls with live
kernel state and delegating the rest. The display backlight crosses the
same seam: the panel's brightness writes land on a real
`/sys/class/backlight` device.

Every host call is logged, and that is the project's working loop: when
the panel asks for something the fixtures don't answer, **the log line
names the file to create.**

## Audio

The panel drives a chumby audio daemon through a family of vendor calls.
The fork maps that onto an **mpv** process it controls over a socket:
play, pause, volume, and mute all become mpv commands, and playback
state is reported back in the values the panel expects. If mpv isn't
installed the player runs silently but otherwise works — the state
machine still answers correctly.

## Input

Chumby's signature control is the **bend sensor** — you squeeze the top
of the case. The panel watches it constantly and reacts to a
squeeze-and-release (it summons the button bar, snoozes alarms). No PC
or Pi has one, so the fork simulates it: you send commands on standard
input, or through a named pipe with `--chumby-control`:

```text
bend | tap           squeeze once and release
bend down / bend up  hold / release
click X Y            click at a point
drag X1 Y1 X2 Y2     press, drag, release (for sliders)
```

Physical input maps onto the same actions: the Home key acts as a
squeeze, and on a touchscreen a tap becomes a click while a short
press-and-hold in one spot becomes a squeeze.

## UI policy

Some panel controls make no sense on the host platform — on the Pi the
operating system owns the timezone and network time, so the clock
screen's timezone picker and "set time from the internet" toggle should
be visible but inert, while the 12/24-hour switch stays live. Rather
than editing the panel or silently swallowing its input, the fork
disables such controls from the outside: `core/src/chumby/ui-policy.toml`
maps controls to actions (`hide`, `disable`, `readonly`),
reapplied continuously so re-entering a screen can't bring a control
back. The rules are compiled into the player — this fork runs one SWF, so
which of its controls are dead belongs here. Format and mechanism: the
header comment of `ui_policy.rs` and `claude-docs/design.md` §5.

## Building and running

```sh
cargo build -p ruffle_desktop
```

`controlpanel.swf` is copyrighted chumby firmware and is not distributed
here — take it from your own chumby (or a backup of one) and put it in
`swf-assets/`. Then:

```sh
./run-controlpanel.sh
```

which wraps:

```sh
target/debug/ruffle_desktop \
  --load-behavior blocking \
  --filesystem-access-mode allow \
  --chumby-fixtures fixtures \
  --chumby-control /tmp/chumby-ctl \
  -PlocalCache=1 \
  swf-assets/controlpanel.swf
```

The `fixtures/` tree in this repo is the virtual chumby: the panel's
filesystem, the canned output of the commands it runs, and the chumby.com
responses it expects. `chumby-ctl bend` squeezes the (virtual) bend sensor
to summon the control-panel bar. Useful log targets: `chumby_host` (all
environment traffic), `chumby_audio`, and `chumby_pick` (a click-target
diagnostic).

## Keeping up with upstream

The `chumby` branch is kept linear: the current upstream `master` with
the fork's work on top. Upstream releases are absorbed by rebasing onto
the new tip, using the hook list in `claude-docs/design.md` §8 to resolve
conflicts. Work happens on a feature branch per session, squashed on
merge. The real test after a merge is not that it compiles but that
`controlpanel.swf` boots to the panel — the hooks can break silently.

---

## ASnative(5,N) reference

Every index the control panel (or a companion SWF) binds, with its
purpose, arguments, return value, and what `FixtureHost` currently
answers. Names are the panel's own `_wrapper` globals (frame 2 of
`controlpanel.swf`); indices 72/74 are bound namelessly, so their
names here are project-invented. Semantics come from the panel's call
sites and from the chumby wiki, which is still online:
[ChumbyNative](https://wiki.chumby.com/index.php?title=ChumbyNative)
documents most of the table, and
[Controlling BTplay](https://wiki.chumby.com/index.php?title=Controlling_BTplay)
covers the audio-player family (this table was cross-checked against
both on 2026-07-07). Rows marked *undocumented* are bound but never
explained anywhere we found. Where the wiki and the SWF disagree, the
SWF wins — it is what actually runs — and the disagreement is noted
inline (one known case: the time family, see 5,176–178).

Fixture-behavior shorthand:

- **rootfs** — resolved inside the virtual rootfs (`fixtures/rootfs/`).
- **store** *(default D)* — the name-keyed get/set store: `_setX(v)`
  stores its first argument under `X` and returns `undefined`;
  `_getX()` returns the stored value, or the default D (no default
  listed = `undefined`).
- **stub** — logged under `chumby_host`, returns `undefined`. The
  panel tolerates `undefined` on every observed path.

Without `--chumby-fixtures` no host is registered and **every** index
returns `undefined`, like stock Ruffle.

### Touchscreen & calibration

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,10 | `_rawX()` | raw (uncalibrated) touch X — calibration screen | stub |
| 5,11 | `_rawY()` | raw touch Y | stub |
| 5,12 | `_setCalibration(xoffset, xscale, yoffset, yscale)` | apply a touchscreen calibration | store (never read back — no `_getCalibration` exists); desktop/Pi input is already calibrated |
| 5,13 | `_writeCalibration()` | persist the calibration | stub |
| 5,43 | `_getTouchClick()` | click-sound-on-touch setting → 0/1 | store (default 0) |
| 5,44 | `_setTouchClick(on)` | set the click-sound toggle | store |
| 5,203 | `_getUsingTsdev()` | is the tsdev touchscreen driver in use → 0/1 | store |
| 5,204 | `_getTSCalibrationPath()` | path of the calibration file | store |

### Sensors & power

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,14 | `_bendLevel()` | raw analog bend-sensor level | stub |
| 5,15 | `_lightLevel()` | ambient-light sensor reading | stub |
| 5,16 | `_dcVolts()` | DC supply voltage — panel checks it in power handling | `12.0` (nominal) |
| 5,25 | `_bent()` | bend sensor pressed → 0/1; **polled every frame**, edges fire onBend/onUnbend (summons the button bar, snoozes alarms) | live simulated bend state (control channel `bend`, Home key, touch long-press) |
| 5,26 | `_getBendThreshold()` | bend trigger threshold | store |
| 5,27 | `_setBendThreshold(n)` | set it | store |
| 5,28 | `_bendAverage()` | averaged bend reading | stub |
| 5,38 | `_headphonesIn()` | headphones plugged in → 0/1 | `0` |
| 5,39 | `_batteryVolts()` | battery voltage | stub |
| 5,40 | `_powerDown(when[, secondsToPowerUp])` | 1 = power off on player exit, 2 = now; optional wake-up delay | stub (a real `PiHost` could map this to systemd poweroff) |
| 5,41 | `_powerSource()` | 0 = battery, 1 = external | `1` |
| 5,60 | `_accelerometer(index)` | accelerometer reading, component by index | index 0 (version probe) → 1; other indexes → 2048, the level raw-axis value (intro.swf's ball page polls 5/6) |
| 5,61 | `_accelerometerSigned(index)` | signed variant (name from the SWF binding; not on the wiki) | stub |

The panel also binds `ASnative(4,39)` as `_batteryPower` — category 4
collides with Ruffle's `ASSetNative`, and the panel never calls it, so
the fork leaves category 4 alone.

### Display & LCD

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,19 | `_getLCDMute()` | LCD blanked → 0/1 | store (default 0) |
| 5,20 | `_setLCDMute(state)` | LCD on/dim/off (0/1/2) — `setDim` drives it on hw 3.6/3.7 | store; with `brightness_ctl` configured, runs the executable with the level as argument (brightness.rs) |
| 5,21 | `_getLCDBrightness()` | backlight level 0–65535 | store (default 65536) |
| 5,22 | `_setLCDBrightness(n)` | set backlight. NB: the panel actually writes `/proc/sys/sense1/brightness` via `_putFile` instead — that write drives a detected kernel backlight (brightness.rs) | store |
| 5,23 | `_getBrightnessThresholdForRange(r)` | auto-brightness threshold for a light range | store |
| 5,24 | `_setBrightnessThresholdForRange(r,n)` | set it | store (first arg only) |
| 5,200 | `_getScreenWidth()` | framebuffer width — panel hardcodes its 320×240 layout regardless | store |
| 5,201 | `_getScreenHeight()` | framebuffer height | store |
| 5,300 | `_getBitmapSmoothing()` | bitmap-scaling filter enabled → 0/1 | store |
| 5,301 | `_setBitmapSmoothing(on)` | set it | store |
| 5,380 | `_setBackgroundAlpha(a)` | framebuffer background alpha | store |

### Filesystem (virtual rootfs)

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,50 | `_getFile(path)` | read file → contents as string, `undefined` if missing | rootfs read |
| 5,51 | `_putFile(path, data)` | write string to file → `undefined` | rootfs write (parent dirs created; confined to the root) |
| 5,53 | `_fileExists(path)` | → 1/0 | rootfs |
| 5,54 | `_fileSize(path)` | → size in bytes, 0 if missing | rootfs |
| 5,55 | `_unlink(path)` | delete file → `undefined` | rootfs |
| 5,320 | `_getDirectoryEntry(obj, path, index)` | fill `obj` with directory entry `index` → status (1 = entry valid, 0 = no more entries, −1 = bad path) | real: `RootFs::dir_entry`, name-sorted so ascending-index iteration is stable; fills `_name`, `_path` (normalized panel-space join), `_isDir`, `_isDirLink`, `_isFile` — drives Music → My Music Files and the mp3files alarm browser |

### Shell

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,52 | `_backtick(cmd)` | synchronous shell execution → stdout as string | canned response from `fixtures/exec/manifest.txt` (longest command prefix wins); unknown commands log loudly and return `""`. Real semantics, not fixtures, for `rm /psp/ifalarm` (backup-alarm dismiss) and `…/scripts/enable_intro` / `disable_intro` (toggle `/psp/disable_intro` in the rootfs) |

(The asynchronous variant is the `exec://` URL scheme, answered from
the same fixture store by `ChumbyNavigator`.)

### Master/slave widget system

On real hardware, widgets/intro/ads run in a *second player instance*
("slave") on an overlay framebuffer, remote-controlled by the panel.
This fork runs with `-PlocalCache=1`, so widgets load in-movie via
`loadMovie` and the whole family can stay stubbed — except the slave
variables, which the panel also uses as its own handshake channel.

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,70 | `_csccd(…)` | *undocumented* (bound by preload.swf; possibly crypto-processor access) | stub |
| 5,71 | `_gsccd(…)` | *undocumented*, get counterpart of 70 | stub |
| 5,72 | `_slaveSetting72(n)` | *undocumented*; `WidgetPlayer.prepareSlaveSettings` calls it with `(1)` | stub |
| 5,74 | `_slaveSetting74(n)` | *undocumented*; called with `(0)` next to 72 | stub |
| 5,80 | `_setSlaveVar(name, value)` | set a variable inside the slave movie | name-keyed slave-var store |
| 5,81 | `_getSlaveVar(name)` | read a slave variable → string | store; `_chumby_widget_done` defaults to `"true"` so intro/widget handoffs never hang |
| 5,82 | `_routeUIEvents(mask)` | route input events to master/slave | stub |
| 5,83 | `_setDisplay(i)` | select render target (main/overlay) | store |
| 5,84 | `_startSlave(path, params)` | load a SWF into the slave player → instance id | stub (widgets load in-movie instead) |
| 5,85 | `_stopSlave(id)` | stop a slave instance | stub |
| 5,86 | `_pauseResumeSlave(id, mode)` | pause/resume a slave instance | stub |
| 5,87 | `_getDefaultSlaveInstance()` | handle of the default slave | store |
| 5,88 | `_getDisplay()` | current render target | store |
| 5,89 | `_getSlaveLoadStatus()` | slave movie load progress/state | store — the panel's handoff logic relies on `_chumby_widget_done` (5,81) instead |
| 5,100 | `_expireCache()` | flush the player's HTTP cache | stub |
| 5,101 | `_expireCacheFiltered(pat)` | flush cache entries matching a pattern | stub |
| 5,110 | `_setOverlayVisibility(opacity)` | overlay framebuffer alpha, 0–255 | store |
| 5,111 | `_getOverlayVisibility()` | read it | store |
| 5,112 | `_setOverlayBlendingEnabled(on)` | overlay alpha blending | store |
| 5,113 | `_getOverlayBlendingEnabled()` | read it | store |
| 5,114 | `_setOverlayChromaBlendingEnabled(on)` | chroma-key blending | store |
| 5,115 | `_getOverlayChromaBlendingEnabled()` | read it | store |
| 5,116 | `_setOverlayChromaBlendColor(c)` | chroma-key color | store |
| 5,117 | `_getOverlayChromaBlendColor()` | read it | store |
| 5,118 | `_enableMasterUpdates(on)` | start/stop rendering the master movie | stub |
| 5,119 | `_enableSlaveUpdates(on)` | same for the slave | stub |
| 5,210 | `_setSlaveAS3Var(name, value)` | AS3 variant of 5,80 | store |
| 5,211 | `_setURLEncodedVars(s)` | inject URL-encoded variables into the slave | store |
| 5,220 | `_enableMasterUpdatesPriv(on)` | privileged variant of 5,118 | stub |
| 5,330 | `_slaveMouseDown(x, y)` | inject a mouse press into the slave | stub |
| 5,331 | `_slaveMouseMove(x, y)` | inject pointer motion | stub |
| 5,332 | `_slaveMouseUp(x, y)` | inject a release | stub |
| 5,333 | `_getLastGesture()` | last recognized touch gesture | store |
| 5,360 | `_grantTempSlavePrivileges(…)` | temporarily raise slave privileges | stub |
| 5,361 | `_revokeTempSlavePrivileges(…)` | revoke them | stub |
| 5,362 | `_grantSlavePrivileges(…)` | grant persistent privileges | stub |
| 5,363 | `_revokeSlavePrivileges(…)` | revoke them | stub |
| 5,364 | `_getEffectivePrivileges()` | current privilege set | store |
| 5,370 | `_resetTransform()` | reset the slave display transform | stub |
| 5,371 | `_addRotation(deg)` | compose a rotation onto it | stub |
| 5,372 | `_addScale(f)` | compose a scale | stub |
| 5,373 | `_addTranslate(x, y)` | compose a translation | stub |
| 5,381 | `_getSWFDimensions(…)` | dimensions of a loaded SWF | store |
| 5,382 | `_mapMovieToViewportSpace(x, y)` | movie → viewport coordinates | stub |
| 5,383 | `_mapViewportToMovieSpace(x, y)` | viewport → movie coordinates | stub |
| 5,384 | `_fillFrameBufferPixels(…)` | fill a framebuffer region (pixels) | stub |
| 5,385 | `_fillFrameBufferBytes(…)` | fill a framebuffer region (raw bytes) | stub |
| 5,386 | `_setDisplayRect(x, y, w, h)` | confine the slave display to a rectangle | store |
| 5,387 | `_setDisplayRectEventTranslate(…)` | ditto, also translating input events | store |
| 5,420 | `_SetOnLocationCallback(…)` | *undocumented* | stub (note the capital S — it bypasses the `_set…` store) |

### USB keyboard / mouse / gamepad

Bound by the panel and by `preload.swf` for USB input devices; never
called by the control panel itself.

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,90 | `_getKeyboardEventMask()` | which key events the movie receives | store |
| 5,91 | `_hasKeyboard()` | USB keyboard present → 0/1 | stub |
| 5,92 | `_setKeyboardEventMask(m)` | set the key-event mask | store |
| 5,93 | `_keyboardGetScanCode()` | last raw scancode | stub |
| 5,94 | `_getMouseButtonState()` | current button state | store |
| 5,95 | `_getNextMouseEvent()` | dequeue a mouse event | store |
| 5,96 | `_getGamepadState()` | current gamepad state | store |
| 5,97 | `_getNextGamepadEvent()` | dequeue a gamepad event | store |
| 5,98 | `_getKeyEventOfferMask()` | which key events are offered to the movie first | store |
| 5,99 | `_setKeyEventOfferMask(m)` | set it | store |

### Audio player (btplay → mpv)

The original controls a `btplay` daemon — wiki:
[Controlling BTplay](https://wiki.chumby.com/index.php?title=Controlling_BTplay) —
and the fork maps the family onto a spawned mpv process (`audio.rs`).
State constants the panel expects: IDLE −1, PAUSED 0, WAITING 1,
PLAYING 2.

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,130 | `_getAudioPlayerPID()` | pid of the btplay daemon | store |
| 5,131 | `_getAudioPlayerState()` | playback state (constants above; the wiki documents only −1/0/2) | polls mpv: stopped → −1, paused → 0, playing → 2 (never 1 — the panel's watchdog kills streams not PLAYING within 5 s) |
| 5,132 | `_pauseAudioPlayer()` | pause playback | mpv pause via IPC |
| 5,133 | `_resumeAudioPlayer()` | resume playback | mpv unpause |
| 5,134 | `_stopAudioPlayer()` | stop playback | mpv quit + reap |
| 5,135 | `_getAudioPlayerTrackAttributes()` | current-track metadata (MP3 tags etc.) → object | store |
| 5,140 | `_isAudioPlayerAvailable()` | audio backend usable → 0/1 | `1` if mpv is installed, else `0` (silent-stub mode) |
| 5,141 | `_terminateAudioPlayer()` | kill the daemon | same as stop |
| 5,142 | `_startAudioPlayer()` | start the daemon | stub — mpv is spawned per `_playAudio`, there is no daemon |
| 5,143 | `_restartAudioPlayer()` | restart the daemon | stub, same reason |
| 5,144 | `_playAudio(url, …)` | play a URL or file | spawns mpv at the current system volume; absolute chumby paths (e.g. `/usr/chumby/alarmtones/…`) resolve inside the rootfs |
| 5,145 | `_playAudioNext(url)` | cue the following track | stub |
| 5,146 | `_playAudioLoopCount(n)` | loop count for the next `_playAudio` — alarms use it | stored, consumed by the next play |
| 5,147 | `_skipAudioNext()` | skip forward in the playlist | stub |
| 5,148 | `_skipAudioPrev()` | skip backward | stub |
| 5,149 | `_playAudioAddPlaylist(mimeType, paths…)` | append files to the playlist (`"*"` = autodetect type) | stub |
| 5,150 | `_playAudioResetPlaylist()` | clear the playlist | stub |
| 5,151 | `_playAudioNow(path)` | play the given file (the wiki's plain "PlayAudio"; the panel uses 5,144 instead) | stub |
| 5,152 | `_sendMediaPlayerCommand(cmd)` | raw command string to the player daemon | stub |
| 5,340 | `_pauseMediaPlayer()` | media(video)-player variant of pause | stub |
| 5,341 | `_resumeMediaPlayer()` | resume | stub |
| 5,342 | `_stopMediaPlayer()` | stop | stub |

### System volume

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,17 | `_getSpeakerMute()` | speaker muted → 0/1 (legacy) | store (default 0) |
| 5,18 | `_setSpeakerMute(on)` | mute the speaker | store |
| 5,180 | `_getSystemVolume()` | master volume 0–100 | store, seeded from `/psp/volume` at startup (default 60) |
| 5,181 | `_setSystemVolume(n)` | set master volume | clamped 0–100, persisted to `/psp/volume`, applied live to mpv unless muted |
| 5,182 | `_getSystemBalance()` | balance −100…100 | store (default 0) |
| 5,183 | `_setSystemBalance(n)` | set balance | store |
| 5,184 | `_getSystemMute()` | muted → 0/1 | store (default 0) |
| 5,185 | `_setSystemMute(on)` | mute/unmute | stored; drives mpv volume to 0 / back to the stored volume |

### Encoding & crypto

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,160 | `_base64Encode(s)` | → base64 string | real implementation, in-table |
| 5,161 | `_base64Decode(s)` | → decoded string | real implementation |
| 5,162 | `_md5Sum(s)` | → md5 hex digest (GUID hashing) | real implementation (`real_ident::md5_hex`) |
| 5,163 | `_blowfishEncrypt(plain, key[, mode])` | encrypt (mp3tunes credential store) | stub (the service is dead) |
| 5,164 | `_blowfishDecrypt(crypto, key[, mode])` | decrypt | stub |

### Time & network configuration

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,170 | `_getstalt(key)` | query a system property by key → number (chumby's Gestalt) | store |
| 5,172 | `_getNetworkConnectTimeout()` | player HTTP connect timeout | store |
| 5,173 | `_setNetworkConnectTimeout(s)` | set it | store |
| 5,174 | `_getNetworkTimeout()` | player HTTP read timeout | store |
| 5,175 | `_setNetworkTimeout(s)` | set it | store |
| 5,176 | `_setSystemTime(secs)` | set the system clock (epoch seconds). Wiki discrepancy: the ChumbyNative page lists the time family under category 103; the SWF binds it here at 5,176–178, and the SWF wins | store (never applied; with the shipped ui-policy the NTP toggle is disabled and `use_ntp=1` keeps the manual set-time/set-date screens hidden, so nothing calls this) |
| 5,177 | `_getTimeZone()` | current timezone string | reads `/psp/timezone` from the rootfs (default `"UTC"`) |
| 5,178 | `_setTimeZone(tz)` | set the timezone | writes `/psp/timezone` in the rootfs (trimmed, matching the seeded fixture format), so set → get via 5,177 round-trips as on real hardware. (The panel separately persists `/psp/timezone_city` via `_putFile`, which is unaffected.) |

### Identity & player state

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,42 | `_fscommand2(cmd, var)` | Flash-Lite `fscommand2` shim; the panel feeds its heartbeat with `GetTotalPlayerMemory`/`GetFreePlayerMemory` | stub (tolerated — the heartbeat file is still written) |
| 5,120 | `_exitOpportunity()` | panel signals "now is a safe moment to restart the player" (memory hygiene on real hardware) | stub |
| 5,121 | `_secondsBeforeRestart()` | countdown to the player's scheduled restart | stub |
| 5,122 | `_restartNow()` | restart the player immediately | stub |
| 5,202 | `_getPlatform()` | hardware config name (`"ironforge"`, `"falconwing"`, …) | `"ironforge"` — the fork emulates a Chumby Classic |
| 5,205 | `_getEnvironment(name)` | read an environment variable | `LANGUAGE` → `"en_US"`, `CONFIGNAME` → `"ironforge"`, anything else → `""` |
| 5,207 | `_getpid()` | player process id | store |
| 5,208 | `_getTotalPlayerMemory()` | total player memory | store |
| 5,209 | `_getFreePlayerMemory()` | free player memory | store |

### Pipes

The `_pipe*` family (wiki-documented) talks to an external process
over a pipe; the control panel never calls any of them.

| idx | native | purpose / return | fixture behavior |
|-----|--------|------------------|------------------|
| 5,190 | `_pipeDaemon(cmd)` | spawn a daemon connected by pipe → handle | stub |
| 5,191 | `_pipeOpen(cmd[, mode])` | open a pipe to a command → handle | stub |
| 5,192 | `_pipeSetInput(handle, listener)` | register an input listener on the pipe | stub |
| 5,193 | `_pipeRead(handle)` | read from the pipe → string | stub |
| 5,194 | `_pipeWrite(handle, s)` | write to the pipe | stub |
| 5,195 | `_pipeClose(handle)` | close the pipe | stub |

---

*The original upstream Ruffle README is preserved as [README.ruffle.md](README.ruffle.md).*
