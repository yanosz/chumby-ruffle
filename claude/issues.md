# Open issues

One block per issue: Number, Timestamp, Title, Status, Description.

---

Number: 1
Timestamp: 2026-07-14, 23:00
Title: Rendering regression in upstream.
Status: open
Description: After rebasing to upstream (8328af42d / 2026-07-12), the keyboard
in the SHOUTcast search could no longer be used; the control panel feels
sluggish. Cause unknown — not bisected. None of the 59 commits since 7f62f5dbf
touches the renderer or AVM1, but that does not rule a commit out. It may also
be that llvmpipe on one thread has no headroom left and any slightly heavier
binary tips it over. Not reportable upstream in this configuration. Workaround:
upstream rebased to 7f62f5dbf / 2026-07-06, shipped as 0.9.2. Recheck the
keyboard on every future rebase.

---

Number: 2
Timestamp: 2026-07-26, 22:45
Title: tiny-skia and wgpu render the panel visibly differently.
Status: open, and much narrower — the high-contrast/bright-fringe half of the
report turned out to be the SPI bus, not this renderer. What remains for the
player is a single outline; both leading hypotheses for it are measured and
refuted, cause not yet identified.
Description: Jan compared the two backends on the 3B+ / ILI9486 SPI TFT and
reports three distinct looks. This is a fidelity gap the spike's own metric
missed: it scored 93-98.8 % of pixels within 16 levels and called that good,
but a one-pixel fringe around every glyph is a tiny fraction of pixels and
extremely visible.

NOT OURS, resolved 2026-07-26: the "super-high contrast, small bright border
around every symbol" half of the report was the appliance's SPI overclock, not
this backend. Holding tiny-skia constant and walking the panel's SPI clock
(chumby-pi claude/issues.md #4) reproduces the artifacts at 28.6 MHz and above
and loses them below; at a clean 22.2 MHz the same renderer draws the same
screen without them. Discount that description when hunting the remaining bug.

Observed (Jan, at the screen):
- **tiny-skia** — "some kind of frame / outline on the right hand side of the
  spaceclock", which SURVIVES at the clean clock and is therefore ours. It was
  originally reported together with contrast artifacts since removed from
  scope; "a bit like anti-aliasing in the wrong color" described that half.
- **wgpu** (`CHUMBY_QUALITY=low`) — "blurred", the border is gone, the
  spaceclock numbers look "pixel-like": "a bit like no anti-aliasing is
  applied".
- **Real chumby hardware** — anti-aliasing looks correct; the low resolution
  makes it look "poor" but not wrong.

MEASURED AND REFUTED, both of them (`CHUMBY_TS_STATS=1`, 1128 consecutive
frames of the panel's default screen). Every frame reads:

    SpikeStats { shapes: 167, bitmaps: 0, rects: 92, lines: 0, masks: 47,
                 alpha_masks: 0, blends: 0, offscreen: 0, bitmap_fills: 0 }

- `alpha_masks: 0` — the unimplemented `render_alpha_mask` (lib.rs:823, "Spike:
  no clipping — draw the maskee unclipped") is NOT the cause of the spaceclock
  outline. The spike's assumption that the panel emits none holds for this
  screen.
- `bitmaps: 0`, `bitmap_fills: 0` — the known dropped RGB tinting on bitmap
  paints (lib.rs:419-421, 712-714) is NOT the cause of the bright border. This
  screen draws no bitmaps at all.

What the screen actually consists of, then, is vector geometry: 167 shapes,
92 rects and 47 regular clip masks per frame, no blends, no offscreen passes.
So whatever the difference is, it lives in path rasterisation, clipping or
colour handling — not in the two gaps that were already known about.

Candidates that survive, none yet tested:
1. **The A/B is not like-for-like.** tiny-skia sets `anti_alias: true` on every
   path unconditionally; the wgpu side ran at `CHUMBY_QUALITY=low`, which
   disables MSAA. So part of what was compared is a *configuration* difference,
   not a backend one, and it explains wgpu's "no anti-aliasing" directly. The
   cheap control is wgpu at `--quality high`.
2. **Colour-space / gamma.** `sk_color` (lib.rs:292) passes 8-bit sRGB straight
   into tiny-skia, which blends in gamma space; the wgpu path branches on
   `surface_format.is_srgb()` with separate shader entry points
   (desktop/src/gui/movie.rs:110). Same coverage value, different edge pixel.
   Affects every AA edge and every alpha blend — fits "super-high contrast" as
   a whole-image impression.
3. **Mask-edge double coverage.** 47 clip masks per frame, and both the mask
   (`mask.fill_path(..., true, ...)`, lib.rs:704) and the content it clips are
   anti-aliased. Coverage then multiplies at the boundary — a seam along every
   clip edge, which is the shape of "a frame on the right hand side" of a
   widget. Whether it reads bright or dark depends on the colours involved.

Still to do: Jan's observation of tiny-skia at `speed=24000000` (running as of
2026-07-26 22:40) to rule the 40 MHz SPI overclock in or out as a contributor —
a corrupted bus should look like noise and tearing rather than structure that
recurs around every symbol, but that is an argument, not a measurement.

TRAP, cost time here: the `CHUMBY_TS_STATS` output does NOT reach the journal.
The crate logs through the `log` facade while the desktop player installs a
`tracing_subscriber` registry writing through `tracing_appender::non_blocking`;
the records land in **`/home/pi/.cache/ruffle/log/ruffle.log`**. `journalctl -u
chumby-player` shows nothing at all from the player.

---

Number: 3
Timestamp: 2026-08-20, 22:10
Title: The built-in clock renders without digits.
Status: cause found, fixed in the appliance build (chumby-pi issue 6);
awaiting a rebuilt deb on the box. Full record: claude/clock-digits-plan.md
Description: With no widgets installed the panel falls back to its own
built-in clock (FR17, the empty-channel -> bi_clock path from #26). On the
new box — Pi 3B+, Waveshare 5" DSI panel at 1024x600, chumby-player 0.9.3,
CHUMBY_RENDERER=tiny-skia, CHUMBY_QUALITY=low — the clock shows no digits.
Not reproduced on the desktop and not narrowed at all yet.
Two cheap probes first, in this order. (1) `CHUMBY_RENDERER=wgpu` in
/etc/default/chumby-player, one restart: if the digits appear, this is a
tiny-skia text/glyph gap and joins issue 2's fidelity findings; if they are
missing there too, the renderer is not the cause. (2) The geometry: 1024x600
is the first panel above 640x480 used in this project, so the stage is scaled
further than on anything tested before — worth checking whether the digits
come back at a smaller mode.

Triaged 2026-08-20 (plan and full findings: `claude/clock-digits-plan.md`).
The digits are not text: each strip is an 11-frame sprite whose frames 2-11
hold one solid-fill `DefineShape` per digit, in the same colour as the colon
next to them, and `setDigit(d)` is `gotoAndStop(d + 2)` — so a backend that
draws the colon cannot fail on the digits, and probe (1) above is not the
discriminating test it looked like. Probe (2) is also answered: the desktop
player at `--renderer tiny-skia --quality low --width 1024 --height 600`
draws the clock complete. What the device shows *besides* the digits is what
separates the remaining causes; that question is CHECKPOINT 2 in the plan.

Cause (2026-08-20): not the player. The deb's `ruffle_desktop` was built in
the same cargo invocation as `exporter`, which asks `ruffle_core` for
`deterministic`; cargo unifies features, so `locale::get_current_date_time()`
was frozen at 2001-02-03 04:05:06 (and `default_font` with it — the leak
was two features; see chumby-pi issue 6 for the font consequence). Frozen seconds mean `BuiltinClock.update()`
runs exactly once — from the constructor, before the digit strips are
class-linked — so all six `setDigit` calls no-op and the strips stay blank,
while `clockFormat` is still undefined there and the 24-hour branch clears
`ampm`. February, no digits and no a.m./p.m. are one fault. Fixed in
chumby-pi `0049563`.

---

Number: 4
Timestamp: 2026-08-21, 22:30
Title: chumby.com still serves external music sources the source filter never sees.
Status: open — measured on real hardware; unhandled in the fork, consequence
for us not yet verified.
Description: A real Chumby Classic (firmware 1.7.2 `ironforge`, running the
same panel we ship — `/tmp/controlpanel.swf` md5 `21d54cd7…` = our
`swf-assets/controlpanel.swf`) still gets two *external* music sources from
the revived chumby.com at every boot. Its own trace log, verbatim:

    22:14:15 MusicPlayer.loadExternalPlayers()
    22:14:15 ExternalMusicSources.loadLocal()
    22:14:15 ExternalMusicSources.load()
    22:14:16 ExternalMusicSources.load(): load succeeded
    22:14:16 ExternalMusicSources.loaded() new source pandora
    22:14:16 ExternalMusicSources.loaded() new source iheartradio
    22:14:22 ExternalMusicSource.assignPlayer(): initializing pandora
    22:14:24 ExternalMusicSource.assignPlayer(): initializing iheartradio
    22:14:24 ExternalMusicSource.getNextItem(): queue empty
    22:14:24 MusicPlayer.gotExternalPlayers(): true

The catalogue comes from `http://music.chumby.com/music_sources/show/?hw=…
&sw=…&fw=…&id=<guid>&nocache=…<dcid>&config=<platform>` (F2:26677, 26710) —
a chumby.com host our navigator allowlist does not name (`fixture.rs` admits
only `shoutcast.chumby.com` and `bor.chumby.com` under `access_chumby_com`),
and it carries the GUID, unlike the SHOUTcast requests. Each entry is a
*SWF player* fetched from that catalogue which splices itself into
`MusicPlayer.musicSources` at its own declared `position` (F2:26652) from
`assignPlayer`, i.e. long after frame 2 defined the array.

Why it matters here: `music_sources::apply()` is one-shot at frame-2
definition time (`APPLIED`), and its doc comment claims `reorderSources` and
externalmusic.xml "only permute or insert, never restore a removed entry".
That is still true of *removal*, but the comment does not cover sources that
did not exist when the filter ran — `hidden_selectors` never sees pandora or
iheartradio, and neither does `ALWAYS_HIDDEN`.

EXPECTED but NOT VERIFIED: with `access_chumby_com=0` the panel never calls
`loadExternalPlayers` (the `MusicPlayer` ctor guards it on `hasNetwork()`,
F2:12792), and with `=1` the catalogue host is fixture-answered, so the load
should fail into `loadFailed()` and no external source should appear. Both
paths still end at `reorderSources`. Confirm before assuming the fork is
unaffected; if the two ever do load, they are unfiltered and account-bound.

Two further real-hardware facts from the same session, both about the code
`music_sources.rs` patches:

- **Source hiding has no on-device equivalent, confirmed on hardware.**
  `availableSources` (F2:12800) admits a source when its player's `exists()`
  is true, and for shoutcast, chumbcast, sleepcast, cbspodcasts, noaa,
  slimserver *and* directurl that method is literally
  `return this._musicPlayer.hasNetwork()` — one global flag, which also
  gates the dashboard thumbnail and the Info registration block. `ipod` is
  worse: its `exists()` probes `service_control chumbipodd status`, but
  `availableSources` carries an extra `selector == "ipod" && platform !=
  "insignia3.5"` clause, so on `ironforge` it is listed even with the daemon
  stopped. Already self-hiding without help: `mp3tunes` (`exists()` returns
  `false`), `fmradio` (daemon probe), `internode` (DCID part ≠ `0006`).
  So the VM splice is not reimplementing a knob the panel already has.
- **`/psp/music_order` works and is reorder-only** (`reorderSources`,
  F2:12847): listed selectors move to the front, the remainder is
  concatenated, nothing is dropped. Verified live — writing `directurl`
  first put My Streams at the top of Music on the real device. It is
  reachable *only* from `gotExternalMusicPlayers`, so it is applied once per
  panel start and never at all on a boot without network. Plain
  `_fileExists`/`getFile`, so it would work unmodified in the fork;
  `fixtures/rootfs/psp/` has no such file.
  Undone on the real device 2026-08-24 at Jan's request: the file written
  2026-08-21 (`directurl, shoutcast, chumbcast, sleepcast, mp3files`) was
  backed up to `/psp/music_order.bak-undone` and removed, so Music is back to
  the panel's native order there. Restoring it means moving the backup back
  and restarting the panel.

Method note for future device work: the panel's traces are recoverable from
real hardware, which is how the log above was taken. `start_control_panel`
inspects its own stdin and sends the player's output to `/dev/null` when it
looks detached (`FPREDIR`), so run it through a wrapper that redirects —
`printf '#!/bin/sh\nexec /usr/chumby/scripts/start_control_panel >/tmp/cp.log 2>&1\n'`
— started with `start-stop-daemon -S -b -x` (busybox there has no `nohup`
and no `setsid`).

---

Number: 5
Timestamp: 2026-08-21, 23:15
Title: The panel drops any stream that is not audible within ~2 s of starting.
Status: open — measured on real hardware; a constraint on every stream or
playlist we ship or document, not a bug we can fix from our side.
Description: Track supervision is a polled heuristic, not an event.
`MusicPlayer.onEnterFrame` → `sourcePlayer.step()` → `doStepTrack()`, so it
ticks once per movie frame, and `controlpanel.swf` is authored at **12 fps**
(SWF header). Every `doStartTrack` sets `_step = -18` and the "still
playing?" test only runs at `_step >= 6`: **first check 24 frames ≈ 2.0 s
after the track starts**, then every 6 frames (0.5 s).

`DirectURLPlayer.doStepTrack` (F2:15579) — and the same override in
`ShoutcastPlayer` (13669), `MP3FilesPlayer` (13900), `CBSPodcastsPlayer`
(15870), `NOAAPlayer` (16107) — treats "not playing" *before*
`THRESHOLD = 5000` ms as **dead**, not finished:

- more than one track → `killCurrentTrack()` = `_tracks.splice(_trackIndex, 1)`,
  then `nextTrack()` wraps: the offending entry is gone from the list for the
  rest of that playback
- `_asAlarm && _tracks.length <= 1` →
  `replaceCurrentTrack({path: Alarm.defaultBeep(), loops: 1000})`: the alarm
  degrades to the beep

A track that plays *longer* than 5 s and then stops is correctly treated as
finished (plain `nextTrack()`). And `playURL` (the `audio/mpeg` path,
F2:15529) does `setTracks([url])` — a single-track list — so a slow-starting
stream used as alarm audio always lands in the beep branch.

Consequence for the fork: a stream must be audible within ~2 s of
`BTPlayer.start`, or the panel discards it. Note the grace period is counted
in *frames*, so on the small Pis (well under 12 fps) we are more forgiving
than real hardware — a stream that works on the appliance can still be
dropped on a chumby.

Playlist parsing, same session:

- `M3U` (F2:15009) splits on newlines and pushes **every line not starting
  with `#`**, untrimmed. A file-final newline therefore yields an empty
  track, which dies at the first check. Serve playlists with no trailing
  newline.
- `playM3U`/`playPLS` arm `setInterval("alarmPlayFailed", 10000)` when
  `_asAlarm`: the playlist *fetch* has its own 10 s deadline, after which
  `forceAlarmBeep()`. The plain `audio/mpeg` path has no such fetch.

Verified on hardware, and relevant to our host boundary: the panel's own
`file:////` form works for **data** loads, not just movie loads —
`file:////psp/list.m3u` was fetched by `playM3U`'s `XML.load`/`onData`:

    DirectURLPlayer.playStream(): M3U
    DirectURLPlayer.gotM3U(): #EXTM3U
    - adding /psp/birds.mp3
    - adding http://liveradio.swr.de/sw282p3/swr3/play.mp3
    TrackedPlayer.setTracks(): got 2 tracks

`fixture.rs` answers `file://` today only for the licenses viewer's
hardcoded path; a user-supplied `file:////` playlist would need the same
treatment before it works under our player.

Alarm wiring needs no patch — it is all already in the panel: the alarm
audio list (F2:17975) offers every source with `canAlarm:true` whose
`exists()` is true, and `directurl` qualifies; `DirectURLAlarmPanel.doSelect`
(F2:24236) stores `alarm.param = stream.asXML().toString()`, a **snapshot**
of the entry, so editing a stream afterwards does not update alarms already
programmed against it; `DirectURLPanelEdit` (F2:23994) exposes name, URL and
four mimetype radios (mp3/ogg/m3u/pls), so a playlist entry can be created
from the device UI alone.

NOT OURS, recorded once because it cost an hour of the same session: on the
Classic, `/etc/asound.conf` pins the dmix slave to `rate 44100` while
btplayd outputs to `alsa:plug:dmixer`, and `chumbyflashplayer.x` runs at
~72 % CPU during playback at nice -20 — the same priority as btplayd. A
48 kHz mp3 therefore stutters (160/147 resampling on a 266 MHz ARM9 behind a
CPU-hungry player); the identical file at 22 050 Hz mono 64 kbps plays
clean, and dropping the player to nice 0 was not needed. Our appliance uses
mpv/pipewire and shares none of this path.

Confirmed end to end on hardware (Jan, 2026-08-21): a two-entry playlist —
30 s local intro from `/psp`, then `http://liveradio.swr.de/sw282p3/swr3/play.mp3`
(two CDN redirects, MP3 128, followed by btplayd's own HTTP client) — plays
the intro, advances cleanly at its end, and SWR3 takes over. So the 2 s gate
is not an automatic loss for a remote stream; it is a race that a healthy
CDN wins. What it punishes is a slow or unreachable one, and the punishment
is silent removal rather than an error.

---

Number: 6
Timestamp: 2026-08-24, 14:50
Title: _setSystemVolume reaches only mpv, not the player's own audio.
Status: open — Jan raised the expectation 2026-08-24, no fix designed yet
Description: The panel's volume slider calls `_setSystemVolume` (ASnative
5,181). `FixtureHost` intercepts it (`core/src/chumby/fixture.rs`), stores the
value in `native_state`, writes `/psp/volume`, and calls
`ChumbyAudio::set_volume` — which is the **mpv** volume and nothing else. So
the slider governs streams and alarm tones, while the player's own SWF audio
(UI clicks, widget sounds, the intro) keeps playing at the player's fixed
volume, and the PipeWire sink is untouched too.

On real hardware that native drove the system mixer, so it governed every
sound the device made. Jan expects the same ("it should apply to all audio
sources"), which makes the current behavior narrower than the original.

Candidate fix, undesigned: have the same touchpoint also set Ruffle's own
player volume (the `--volume` equivalent at runtime), so one slider scales
both audio paths. Open questions before coding — whether panel space (0-100)
should map linearly onto the player's 0.0-1.0, whether `_setSystemMute` needs
the same treatment (it currently mutes mpv only), and whether the appliance
wants the sink left alone deliberately, since the sink is where an owner sets
"how loud is loud" (chumby-pi claude-docs/development.md §6 records the 5"
DSI box's sink at 100 % after Jan found the output weak).

---

Number: 7
Timestamp: 2026-08-26, 17:00
Title: No network was reported as a fabricated Ethernet page.
Status: fixed on dev 2026-08-26 — device verification outstanding (Jan)
Description: With no default route every reader in `real_net.rs` returned
`None` and the call fell through to `fixtures/exec/`, which answered
`type="lan" ip="192.168.1.50" gateway="192.168.1.1"`, MAC
`00:11:22:33:44:55`, and `<wifi connected="1" linkquality="100"
signalstrength="100"/>`. Jan hit it after taking the 5" DSI box off LAN
before wifi came up: the Info screen showed an Ethernet at full signal with
a mock address, and no mention of WLAN. Same lie as the I3 hardcoding
(chumby-pi memory "status page: derive from live state"), reached through
the fallback instead of a constant, and in the one state nobody had tested.

The panel already had the protocol and the words for this, which is why the
fix needed no UI work:
- `gotNetworkStatus` (F2:294) reads `childrenOfType("error")` inside
  `<interface>`; any error child sets `Object._chumby.hasNetwork = false`
  and NO field is copied.
- `InfoPanel.loadInfo` (F2:27192) wraps type/ssid/ip/netmask/gateway/dns and
  the `signal_strength` call in `if (hasNetwork)`, with an `else` that prints
  the translated **"network: not connected"** (F2:27253).

Fix: the three network touchpoints now always answer from code and can no
longer reach a fixture — `NO_NETWORK`
(`<network><interface><error/></interface></network>`) when there is no
connected interface, `NO_WIFI` (`<wifi connected="0"/>`) with no route or a
wired one. `macgen.sh` no longer depends on a route at all (the Info screen
prints the MAC outside the has-network branch, so a fake would sit right
above "not connected"): default-route interface first, else the first
non-loopback interface with a non-zero address, else an empty line. Their
three fixture files and manifest lines are deleted, so no path can serve
them; `desktop/src/main.rs:194` is the only host construction site and
always wraps `FixtureHost`, so nothing else consumed them.

Why the fixture files could not be the fix on their own:
`chumby-player-run:135` seeds `$STATE/fixtures` only `if [ ! -d ]`, so an
upgrade never replaces an existing tree — an owner's box (Jan's, seeded
2026-08-24) would have kept the lying XML.

Tests: `network_xml` for wired and for wireless (ssid escaping included),
the no-network answer against the panel's error protocol, and `mac()`
asserted never to be the old fixture value. The no-network test fails
against the previous code, which was the point.

Not fixed here, same class, recorded for the audit: `chumby_version -s`
(1.7.2) and `-f` (1830) are canned values shown on the Info screen with no
real source; `_headphonesIn`, `_dcVolts`, `_powerSource` and
`_accelerometer` answer with literal constants (`fixture.rs:160-171`). The
levers `_getPlatform` "ironforge" and `chumby_version -h` "3.8" are
deliberate (`brightness.rs:5` selects the slider UI from the latter) and not
part of this class.

---

Number: 8
Timestamp: 2026-08-27, 09:00
Title: With no network the local widget channel does not load (since 0.9.4).
Status: open — needs proper investigation, nothing designed
Description: On 0.9.4 a box with no network at all shows only the built-in
clock: the local widget channel (`/psp/profile.xml` plus the local widget
SWFs) is absent. Connecting a network brings it back immediately and nothing
is lost on disk — Jan observed both halves on the 5" DSI box, 2026-08-26/27.

Introduced by issue 7: reporting "no network" honestly sets
`Object._chumby.hasNetwork` false, and that flag gates considerably more than
the Info screen's display. Its complete set of consumers in the decompiled
panel is F2:3569, 3704, 3863, 4047, 4149, 4272, 4854, 5335, 7772/7773, 7833 —
recorded here so the next attempt starts from the list rather than
rediscovering it on a device.

No remedy is proposed. The trade-off between honest network reporting and a
local channel that works offline needs investigation before any code, and the
options considered so far were either too broad or unverifiable without a
device. Deliberately left open (Jan, 2026-08-27).


---

Number: 9
Timestamp: 2026-08-27, 20:15
Title: tiny-skia painted every `EditText` border as a filled black box.
Status: closed — fixed and verified on the desktop, both renderers, 2026-08-27
Description: Jan, at the 5" DSI box: the custom-alarm wizard's "Name this
alarm" step shows a black box where the name field should be. Reproduced on
the desktop with the same build: `--renderer tiny-skia` draws a black band
across the full stage width, ~85 px tall, swallowing the field and the CLEAR
button beside it; `--renderer wgpu` draws the field correctly. The appliance
runs tiny-skia (`CHUMBY_RENDERER`), the desktop defaults to wgpu
(`cli.rs:158`), which is why it never showed here.

Cause: the field is `DefineEditText` chid 1102 (`textStr` in
`DefineSprite_1104` = `AlarmPanelSetName`), flags `05 29` — `Border=1`,
`UseOutlines=1`. So the core takes `EditText::draw_text_box` and emits
`draw_line_rect` for the border (`edit_text.rs:2956`) with
`Matrix::create_box(width, height, …)`, which carries the box *size* in the
matrix's linear part. The backend stroked a unit square with
`Stroke::default()` (width 1.0) under that matrix, and tiny-skia strokes in
path space and transforms afterwards (`painter.rs:433`: `path.stroke(…)` then
`fill_path(…, transform, …)`). The 1 px border was therefore multiplied by the
box dimensions: at 2x viewport scale the ~228x22 field yields ~456 px-thick
vertical edges and ~44 px-thick horizontal ones — full stage width, ~88 px
tall, matching the observed band.

Flash's rule is the opposite, and upstream states it at `edit_text.rs:2850`:
"line width of the border is always 1px regardless of zoom and transform."
wgpu obeys it with a hardware line primitive and, where the adapter has none,
falls back to `ruffle_render::lines::emulate_line_rect`
(`wgpu/src/surface/commands.rs:875`) — a backend-independent helper that
builds four 1 px rects from already-transformed corners. The fix routes our
`draw_line_rect` through the same helper; it lands on our `draw_rect`, which
was already correct. `render/tiny_skia/tests/render.rs`
(`line_rect_border_stays_one_pixel_thick`) pins a 1 px frame with a clear
interior, and fails against the old code.

Consumer list — every bordered `EditText` in the panel. All 398
`DefineEditText` tags were decoded; exactly five carry `Border=1`, and all
five have identical `hasFont=1 / useOutlines=1`, so all five take the same
`draw_text_box` → `draw_line_rect` path:

| chid | screen | verdict |
|---|---|---|
| 1102 | `AlarmPanelSetName` — "Name this alarm" | the report; verified fixed on screen, and live (typing updates the text inside a 1 px frame) |
| 549 | `ssidEntry` / WEP-key keypad in `DefineSprite_552` | verified fixed on screen ("Enter name of access point"); reachable only with the `settings-network` ui-policy rule lifted, which was a throwaway probe build, since reverted |
| 597 | `IPEntry` (`DefineSprite_598`) | verified fixed on screen via `SlimServerPanel` (chid 1233), reached with `enable_lyrion = 1`; its other host, the manual-IP frames of `DefineSprite_612`, needs a real association and stays unreachable |
| 1275, 1276 | `MP3tunesPanelLogin` account/password | same flags, same code path; the screen needs chumby.com and was not reached |

`draw_line` was examined and left alone. Its callers put the line's thickness
on an axis the matrix scales by 1.0 — `create_box(width, 1.0, …)` for the
horizontal edges, and `create_box_with_rotation(1.0, height, PI/2, …)` for the
vertical ones, where the rotation moves the thickness onto the `1.0` axis
(`edit_text.rs:2874-2905`, `render_underline` at 1367) — so it never produced
the artifact. It does differ from wgpu by the half-pixel offset wgpu adds
(`commands.rs:862`); switching it to `emulate_line` would settle that too, but
that is a separate call and nothing is known to be broken by it.

---

Number: 10
Timestamp: 2026-09-01, 11:00
Title: A silent alarm cancelled a sounding one — deliberate deviation from stock.
Status: fixed on dev 2026-09-01, A/B-verified on the desktop; device
verification outstanding (chumby-pi issue 13).
Description: `Alarm.ringAlarm` (F2:11178) opens with
`_alarmSet.stopAlarmsExcept(this)`, and `stopAlarmsExcept` (F2:12039) calls
`stopAlarm(true)` on every *other* ringing alarm. An alarm with `type="none"`
(`Alarm.TYPE_NONE`, F2:10170) makes no sound at all — its whole ringAlarm
branch is `doPreAction` → `stopAlarm(false)` (F2:11187-11191), the shape a
nightmode alarm has — yet it runs that cancel first. Two alarms a minute
apart and the silent one wins: on chumby-pi-3 a `"Daily at 8:00"` nightmode
alarm killed a stream alarm that had been ringing since 07:59 (measured on
the device 2026-09-01, full evidence in chumby-pi `claude/issues.md` #13).

The guard exists to keep two *sounding* alarms from overlapping. An alarm
that makes no sound has nothing to protect, so `alarm_guard.rs` wraps
`AlarmSet.prototype.stopAlarmsExcept` — one-shot with retry until frame 2
defines it, the original parked at `__chumby_stopAlarmsExcept`, surgery
precedent `empty_channel.rs`. When the argument's `_type` is `"none"` the
wrapper logs and returns; every other call delegates unchanged. Wired at
`avm.rs` beside the other one-shots. **No upstream file is touched, so
`patch-surface.md` is unchanged.**

Wrapped here rather than at the `ringAlarm` call site because the ringing
alarm passes *itself* as `anAlarm` — canceller and survivor are the same
object on the only reachable path — while wrapping `ringAlarm` would mean
either reimplementing its 50-line body or shadowing `stopAlarmsExcept` with
a no-op and restoring it, where a missed restore disables cancelling for
every alarm. Consumer list for `stopAlarmsExcept`, all four sites: F2:11182
`Alarm.ringAlarm(this)` — the path that bites; F2:12033 `AlarmSet.step`'s
periodic reload, F2:12238 and F2:12244 `AlarmSet.gotEvent` — all pass
`undefined` (so they fail the `_type` test anyway) and are unreachable on our
stack, the reload interval being `ONE_YEAR` without the `alarmReloadInterval`
FlashVar (F2:11784) and `ExtendedEvents.AlarmPlayer` (F2:12182) never driven.
Outside the decompile there is no consumer: the fork's Rust names `AlarmSet`
only in two `fixture.rs` comments about the backup-alarm exec protocol.

**This is a deliberate deviation from stock chumby behaviour**, chosen by Jan
over the configuration-only workaround (move the nightmode alarm out of the
wake alarm's ring window) and over leaving it alone. Nothing else in the
silent alarm's ring changes: night mode off, widget mode, the
`post_alarm_action` probes, its own `stopAlarm` and `saveAlarms` all still
run.

Verified on the desktop, same binary with the guard compiled out and in, two
`when="once"` alarms a minute apart — the earlier `type="beep"
auto_dismiss="0"`, the later `type="none" action="nightmode"
auto_dismiss="1"`:

    guard off  10:37:00.036  AlarmSet.stopAlarmsExcept(): cancelling … 10:36 am
               10:37:00.037  Alarm.stopAlarm() … 10:36 am isCancel:true
               10:37:00.037  Alarm.restoreSoundSettings(): volume:60
    guard on   10:41:00.043  silent alarm "Daily at 8:00" rang — not cancelling
               10:41:00.044  Alarm.doPrePostAction(): night mode off
               10:41:00.046  Alarm.restoreDisplay():  - alarm still playing,
                             continuing

With the guard the surviving alarm is never mentioned again after it rings,
and the panel's own `restoreDisplay` acknowledges it — stock code already
handles a survivor correctly.

Side effect examined and dismissed: without the cancel, `_alarmRefCount` ends
at 0 rather than −1. That counter is written at four sites (F2:11783 init,
12057 `++`, 12062 `--`, 12068 snooze) and **never read as a condition**
anywhere in the decompile; its only consumers are the three `trace()` strings
that print it. It is already asymmetric on stock — `Alarm.ringAlarm` calls
`_alarmSet.ringAlarm` only on the two sounding branches (F2:11203, 11225)
while `Alarm.stopAlarm` (F2:10994) decrements unconditionally — so a silent
alarm drives it negative on a real chumby too.

Also observed and not part of this fix: the cancel used to run
`Alarm.restoreSoundSettings()`, which is why a manual restart after a
cancelled alarm played at the pre-alarm volume (44 → 16 on the device). Moot
now for silent cancellers.

---

Number: 22
Timestamp: 2026-09-10, 19:57
Title: Sony Dash panel support — parked on the sony-dash branch.
Status: parked — the fork side of steps 1-4 done, step 5 not started
Description: The Dash player work is on branch `sony-dash`, tip `0ea50a70b`
(pushed to origin); dev was rewound to `75f3acf28`, and the appliance repo's
dev to `e7aa3ee`, so the Pi appliance work continues without the Dash tree in
the way. On that branch and nowhere else: `claude/dash-panel-survey.md`,
`fixtures-dash/`, `run-dash.sh`, the bounded-mask work in
`render/tiny_skia/src/lib.rs`, and issues 11-21 — so numbers 11-21 are
reserved here and must not be reused on dev. The plan and its checkpoints live
in the appliance repo (`claude/sony-dash-panel-plan.md`, also branch-only),
its issue 18 is the counterpart of this block, and its Dash appliance issues
are 14-17.
Open on that branch, both wanting the DSI box: issue 20 (brightness via
`ChumbyNative._setLCDBrightness`, native 5,22, never reaches the backlight on
platform `yume` — `display/ScreenManager.as:296-304`) and issue 21 (the issue-10
silent-alarm cancel is probably unguarded on the Dash, `alarm_guard.rs` binds
by the classic's signature).
Untracked and in git nowhere, left in this worktree by the step-4 session:
`fixtures-dash/rootfs/psp/guid` and `fixtures-dash/rootfs/psp/theme.swf` (the
Dash theme asset — do not commit an extracted SWF).

---

Number: 23
Timestamp: 2026-09-10, 20:40
Title: The backlight scale wastes its whole range above 25 %.
Status: fixed in the player — the value itself wants a look on the box
Description: On the 5" DSI panel (`10-0045`, `max_brightness` 255) Jan runs
day brightness at 7.2 and night/dim at 1.5 out of the panel's 0-100, i.e. duty
18 and 4 — the useful range is the bottom 2 % of the scale, and that is also
its coarsest part: one panel unit is 655.35 raw (F2:9119
`setRawBrightness` → `int(v × 655.35)`) = 2.55 duty steps, so a pixel of
slider travel jumps several steps where he needs single ones.

Fix: `brightness_cap`, a percent in `player.toml` (config.rs), shipped as an
active `25` in `fixtures/player.toml.example` — the file `build-debs.sh:48`
installs as the appliance conffile. `Backlight` stores `max_brightness × cap`
(brightness.rs:79-90) and `set_raw` scales the panel's 0-65535 linearly onto
it, keeping the ≥1 floor so no "on" position turns the screen off. Panel 100 %
now reaches duty 64 and the slider's travel spreads over it: four times the
resolution down low, which is where this panel lives.

Consumers checked: `fixture.rs:597-603` (the sole `set_raw` caller, the
`/proc/sys/sense1/brightness` interception) unchanged; `fixture.rs:78` passes
the cap at detection; `ui_policy.rs:242` `brightness_available` untouched — a
cap is not availability; `brightness_ctl`'s discrete 0/1/2 path is not scaled
and says so in the template. The Dash's own route (native 5,22, issue 20) is
unimplemented and must land on the same cap.

A cap so small that `max × cap` rounds below one step warns at detection and
leaves a switch, not a dimmer.

Still to do: deploy and let Jan judge 25 %. The panel's two settings files are
in panel space and untouched, so today's levels move — day 7.2 → duty 5,
night 1.5 → duty 1; reproducing today's brightness means roughly 29 and 6 on
the sliders.

---

Number: 24
Timestamp: 2026-09-10, 22:05
Title: The volume ceiling is too high, and the master is not the only fader.
Status: fixed by config — the 50 % value is Jan's to judge on the box
Description: Jan's rule for the appliance: every stage at unity, the panel's
slider the single master, capped at half. `volume_cap` (FR14, config.rs) is
already that scale — `audio.rs:69-74` `effective_volume` = panel × cap / 100,
linear, applied to mpv's `volume` at spawn (`--volume=`) and over IPC on every
change; panel space and `/psp/volume` keep an honest 0-100. Only its shipped
value changed, 100 → 50 in `fixtures/player.toml.example`.

Audited on chumby-pi-3 the same evening, everything else already at unity:
ALSA `PCM` 100 % on card 0 (Headphones) and card 1 (the USB dongle, the
default sink); both PipeWire sinks `vol: 1.00`; SWF audio unity (the desktop
CLI's `--volume` defaults to 1, `desktop/src/cli.rs:118`, and the launcher
passes none); the backup-alarm Klaxon 100 (`backup_alarm.rs:34`), which
ignores the cap by design. Nothing to change in any of them.

Panel-side on the box: `/psp/volume` 32 → 100 and `/psp/alarm_volume` 44 → 100
(the shipped seed is already 100), so the slider starts at the top of the new
scale and alarms are not quieter than the master.
