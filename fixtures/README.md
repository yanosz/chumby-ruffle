# fixtures — how to change any mocked result

This directory is the `FixtureHost`'s answer corpus. Fixture keys are the
panel's **own request strings** — when a run logs a `MISSING` line under the
`chumby_host` target, that line names the file to create.

Canonical launcher: `./run-controlpanel.sh` (repo root) — adds the
`/tmp/chumby-ctl` control FIFO for the simulated bend sensor and tees logs
to `/tmp/chumby-run.log`. The underlying command it runs (from
`ruffle/`):

```sh
RUST_LOG=warn,chumby_host=info,avm_trace=info \
cargo run -p ruffle_desktop -- \
  --load-behavior blocking \
  --filesystem-access-mode allow \
  --chumby-fixtures <repo>/fixtures \
  --chumby-control /tmp/chumby-ctl \
  -PlocalCache=1 \
  <repo>/swf-assets/controlpanel.swf
```

(`--load-behavior blocking` is required: Ruffle's default streaming load
breaks `gotoAndStop` to late frame labels — `claude-docs/requirements.md`
FR7.)

Audio (`_playAudio` family) plays through **mpv** if installed; without it
the state machine still answers correctly but stays silent. Set
`CHUMBY_AUDIO_DEVICE` (an mpv device name, e.g.
`alsa/plughw:CARD=UACDemoV10`; list with `mpv --audio-device=help`) to
route mpv to a specific output — used on the Pi for the USB card.
Screen-flow regression check: `./verify-screens.sh all` (xdotool;
screenshots into `claude-docs/images/`).

Fixture bodies (exec and http) may contain the token `{FIXTURES}`; the
FixtureHost expands it to the absolute fixtures directory at read time.
Use it wherever a fixture must reference a file by path (the channel
profile's widget `file://` URLs do) so the same tree works on the dev box
and on the Pi.

## Layout

| Path | Answers | How to change |
|------|---------|---------------|
| `rootfs/` | `_getFile`/`_putFile`/`_fileExists`/… (ASnative 5,50-55,320) — the panel's whole filesystem view (`/psp`, `/tmp`, `/mnt`, …) — AND panel-hardcoded `file://` data loads (e.g. the licenses viewer's `file:////LICENSES/gpl.txt`) | edit/create the file at the same path under `rootfs/`; panel writes land here too (e.g. `tmp/movieheartbeat`, `proc/sys/sense1/brightness`) |
| `exec/manifest.txt` | `_backtick` (5,52) and `exec://` commands | TAB-separated `command-prefix<TAB>response-file`; longest prefix wins; comment lines start with `#` |
| `exec/*.{txt,xml}` | command stdout bodies | edit the file named in the manifest |
| `http/<host>/<path>` | HTTP fetches to chumby hosts (`*.chumby.com`, `127.0.0.1`, `localhost`) | file at query-stripped, trailing-slash-stripped path; e.g. `http://xml.chumby.com/xml/chumbies/?id=…` → `http/xml.chumby.com/xml/chumbies` |
| (in code) | stateful natives (volume, mute, balance, touchclick, brightness, slave vars, platform=ironforge, env vars) | `core/src/chumby/fixture.rs` in the ruffle fork — `native()` match |

## Current contents

- `rootfs/psp/`: first-boot done (`firsttime=0`), single-framebuffer mode
  (`nooverlay=1`), 12h clock, Oslo timezone, NTP on, hostname `chumbypi`;
  volume 60, touch-click off, alarm list + alarm volume, and
  `url_streams` with one My Streams entry (SomaFM Groove Salad) — all
  panel-writable, so edits made in the UI persist here across runs.
- `rootfs/usr/chumby/alarmtones/`: the 7 stock alarm MP3s from the device
  backup (the SWF `_fileExists`-checks them before offering them).
- `rootfs/LICENSES/`: `gpl.txt`, `lgpl.txt`, `README` copied verbatim from the
  device backup — the SOFTWARE LICENSE viewer (Settings → Chumby Info →
  SOFTWARE LICENSE) loads `gpl.txt`/`lgpl.txt` via `file:////LICENSES/…`.

Format gotchas (cost a debugging round each on 2026-07-06):
- `psp/alarms`: the `time` attribute is **minutes** since midnight
  (`13:35` → `815`), not seconds — a seconds value parses fine but the
  alarm never fires (`Alarm.setHours(_time/60)`, frame_2).
- `psp/url_streams`: every `<stream>` needs a `mimetype` attribute
  (`audio/mpeg`, or `audio/x-mpegurl` / `audio/x-scpls` for playlists) —
  without it `DirectURLPlayer.playStream()` silently does nothing.
- `exec/`: GUID, MAC, healthy wired `network_status.sh`, full-bars
  `signal_strength`, `chumby_version` h/s/f/n = 3.8 / 1.7.2 / 1.7.2 /
  CHUMBYPI001, minimal `dcid`, quiet handlers for headphone_mgr /
  widgetcache / musicsource / alarm-dismiss (`rm /psp/ifalarm`,
  `reload_backup_alarm`) commands.
- `http/`: authorize + chumbies + the widget-channel profile
  (`xml/profiles`). The profile is **generated** from the widget sidecars
  by `chumby-widget-channel` (repo root), not hand-edited — see
  `claude-docs/design.md` §2. Plus "no update", empty
  external music sources, inert FM radio.
- `widgets/`: one `<name>.swf` per widget plus its `<name>.widget.xml`
  metadata sidecar (the `<widget>` element the panel reads). The generator
  wraps every sidecar into the single channel = all installed widgets,
  ordered by widget id (Unsubscribed Clock, then builtinclock). SWFs are
  referenced from the profile via `file://` + `{FIXTURES}` hrefs. An
  optional `<thumbnail href>` sidecar child points at an 80×60 `<name>.jpg`
  preview the panel `loadMovie`s into the dashboard bar (W2); the JPEGs are
  chumby widget artwork, so — like the SWFs — they are gitignored and
  provided locally (render one from the widget with the `exporter` tool).
  A widget without a thumbnail sidecar simply shows no preview.

With these, the unmodified 2.8.87b3 panel boots the real device path
(authorize → validate → normal operation) and plays the Unsubscribed Clock.
The control panel bar is summoned with the simulated bend sensor: type
`bend` (or `tap`) + Enter in the launch terminal, `echo bend >
/tmp/chumby-ctl`, or press Home with the window focused. Verified
reachable on fixtures: alarms (B5), Music → My Streams (C0/C2),
Settings → Volume (E0/E1) — see `verify-screens.sh`.
