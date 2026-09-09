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

Number: 11
Timestamp: 2026-09-09, 19:10
Title: Dash: the Space Theme costs a full core on the DSI box.
Status: fixed on dev 2026-09-09 (bounded masks, `render/tiny_skia/src/lib.rs`); kiosk fps on the box not yet re-measured — needs a deploy
Description: `claude/dash-panel-survey.md` §4: on chumby-pi-3 (1024x600 DSI,
tiny-skia) the Dash panel itself holds 12 fps at 53 % of a core, but
`default_theme.swf` alone runs at 9.5 fps and 105 %; on the desktop the theme
takes 2–3x the CPU of either panel (12 % vs 6 % vs 4 %, same binary, same
1024x571 window). The theme is a vector-only SWF — 43 `DefineShape*`, 126
sprites, 30 `DefineEditText`, six fonts, no bitmaps, no filters (tag dump) —
whose `MainDate.onEnterFrame` rewrites the seconds field every second and
whose modules run one-shot `onEnterFrame` initialisers (`com/example/{Module,
Controls,Pushback,ListItem}.as`), `OtherTime` per minute, `Calendar` per day.
Not diagnosed: renderer versus script, and which shapes dominate. Since a
theme is the home screen most of the time, this number, not the panel's,
decides whether the Dash is viable on the Pi 3B+. Levers, in order of
preference: find the expensive construct and see whether a theme of our own
avoids it; the render-scale lever (rendering-defaults note, still open); a
frame cap (deferred by Jan). Size: M (measure with `top -H` and
`RUST_LOG=ruffle_render_tiny_skia=trace`-class instrumentation on the box, then
decide). Patch surface: none for the measurement; a render-scale knob would
touch `desktop/src/cli.rs` (already a hook file).

Diagnosis, 2026-09-09 (step 4, item 11). Method: `exporter/src/bin/tiny_skia_export.rs`
now also times `run_frame()` apart from `render()` (uncommitted); a temporary
`CHUMBY_TS_NOMASK` guard on the four mask entry points (reverted) switched
masks off; `CHUMBY_TS_STATS=1` printed the backend's per-frame counters;
thread view from `top -H` on the box.

1. **Single thread.** In the kiosk the player's main thread is at 99.9 %
   with the theme, 64–78 % with the classic; render, script and present are
   serial.
2. **Script is not it.** `run_frame()` is 4.7–4.9 ms per frame for the
   theme and the Dash panel, 7.2 ms for the classic, on the box.
3. **Masks are.** Per-frame draw counts at 1024x571: theme 205 shapes,
   110 rects, **56 masks**; Dash panel 108 / 16 / 8; classic 11 / 2 / 2.
   Ruffle clips every `EditText` to its bounds with `push_mask` +
   `draw_rect` (`core/src/display_object/edit_text.rs:2735-2763`), and the
   theme has 30 text fields plus list and panel masks. In the backend each
   mask is a full-frame `tiny_skia::Mask` (`render/tiny_skia/src/lib.rs:164-168`
   `push` → `take` → `Mask::clear()` on a 1024x571 spare), intersected with
   the enclosing clip by a full-frame byte loop (`intersect`, `:214-222`)
   whenever the theme's own panel mask is active. Fourteen of the 56 are
   degenerate (tiny-skia warns "empty paths … cannot be filled" ~14x per
   frame) and still pay the full price.
4. **Numbers** (`tiny_skia_export`, 30–60 frames, `render()` mean):

   | | masks on | masks off |
   |---|---|---|
   | theme, desktop x86, 1023x571 | 8.0 ms | 5.0 ms |
   | theme, box (aarch64 release, not dist) | **178.7 ms** | **59.6 ms** |
   | Dash panel, box | 12.8 ms | 11.0 ms |
   | classic, box, 800x600 | 25.3 ms | 18.8 ms |

   The July dist build measured 49.6 ms for the theme (§4 of the survey's
   method, same box) — `dist` (LTO) versus `release` and the mask commits
   since (`2cfa93297` "real mask clipping") both play in; the kiosk's 0.9.5
   sits between the two at ~100 ms per frame.
5. **The rest is present.** Kiosk CPU per frame minus render minus script
   leaves ~35–55 ms for every SWF on the 1024x600 DSI — the software
   present path (`desktop/src/gui/controller.rs`, softbuffer → cage/pixman).
   Not measured on its own yet; it caps the classic too and is a separate
   item.
6. **Renderer check** (Jan's rule): the theme's 24th frame from
   `tiny_skia_export` and from the wgpu `exporter` are identical except
   that tiny-skia draws a faint 1 px grey outline around the empty widget
   area (roughly x 160–1015, y 270–560) that wgpu does not — most likely
   the same "single outline" as issue 2.

Proposal (CHECKPOINT 4): **bounded masks** in `render/tiny_skia/src/lib.rs`
— our own crate, additions commit, no upstream surface. `MaskStack` keeps
the bounding box of the geometry filled into each `Building` mask
(`render_shape`, `draw_rect`, `render_bitmap` mask branches all know their
transformed bounds); `take()` zeroes only the previous box instead of
`Mask::clear()`; `intersect` loops only over the new box (outside it the
product is already zero); an empty box means the clip is empty and clipped
draws can be skipped. Expected: the theme's render on the box from ~179 ms
toward ~60 ms, the classic from 25 toward 19; the 14 degenerate masks
become free. Size S–M, ~80 lines plus tests next to the three existing
mask tests. Consumers of the touched code: the four mask entry points,
`intersect`, the three mask-geometry branches, `clip()` at the five fill
sites — all in this one file.

Implemented, 2026-09-09. `BoundedMask` wraps the full-frame `Mask` with the
`IntRect` of pixels it has touched; `fill` unions in the transformed path
bounds (+1 px for anti-aliasing, degenerate paths skipped), `take` zeroes
only that box, `intersect` multiplies only the inner box and shrinks it to
the overlap, and every draw under an active clip whose box is `None` is
skipped (`clip_is_empty`). `MaskStack::target()` became `fill()`, so the
three mask-geometry branches no longer touch the `Mask` directly. Three
unit tests added (reuse is clean, intersection stays inside the outer box,
degenerate geometry leaves an empty clip); 11 pass.

| `render()` mean per frame | before | after |
|---|---|---|
| theme, box (aarch64 release) | 178.7 ms | **62.7 ms** |
| Dash panel, box | 12.8 ms | 11.4 ms |
| classic, box, 800x600 | 25.3 ms | 25.1 ms |
| theme, desktop | 8.0 ms | 4.8 ms |
| classic, desktop | 1.82 ms | 1.80 ms |

The classic gains nothing because its two masks cover most of the frame,
so the box is the frame; what "masks off" saved there (18.8 ms) was the
per-pixel clip in the fills themselves, which stays. Fidelity: 24 frames
each of theme, Dash panel and classic rendered by the old and the new code
in the same session differ in **0 pixels**; a 40 s live desktop run of the
classic on tiny-skia (4 masks/frame) was clean, and the ~14 per-frame
"empty paths … cannot be filled" warnings are gone. `rustfmt`/`clippy` are
not installed on this toolchain and were not run. Patch surface unchanged
(own crate). Next for this item: a `dist` deploy to the box to read the
kiosk fps with the theme — expected to move from 9.5 toward the 12 fps
cap, with the present path (~35–55 ms/frame) as the remaining ceiling.

---

Number: 12
Timestamp: 2026-09-09, 19:10
Title: Dash: reach the home screen offline (the Dash's FR7).
Status: done on dev 2026-09-09 — home screen with fixtures only, then the XAPI channel with four fixtures and a wildcard lookup; the widget itself waits on issue 14
Description: The classic's offline route is `-Pbuiltin=1`; on the Dash
`builtin` makes the startup wizard *quit* (`startup/StartupPanel.as:130-139,
183-192, 214-223`, `fscommand("quit")` at `:262`). The route that reaches
`NORMAL_MODE_STATE` is CHECK_NETWORK → CHECK_AUTHORIZE → CHECK_DATE →
`doNetworkThings` + `CPMain` → `HomeScreenProxy` → `ThemeLoader`, which needs:
(a) `Chumby.hasNetwork` true — set from `network_status.sh` through
`StartupPanelCheckNetwork.as:28` (`!interfacex.hasErrors()`); the desktop run
got false from the classic fixture, so the Dash's `NetworkStatus.fromXML`
shape must be checked against `fixtures/exec` (survey §3); (b)
`needStartNetwork()` false, i.e. `/psp/securityQuestion` and
`/psp/securityAnswer` present and no `/psp/running_delink`
(`StartupPanel.as:264-272`); (c) `/xml/authorize?id=<guid>&hw=…` answered
(`structure/Authorization.as:52`; the classic fixture exists under
`fixtures/http/xml.chumby.com/`, response shape to compare); (d) a valid date.
After that `CPMain` loads device, profile and widgets through the **XAPI**
family — `chumbynetwork/{XAPI,XAPIRequest,Device,Profile,Profiles,
WidgetCatalog}.as`, OAuth-style `MD5-HEX`-signed requests under
`{base}/xapis/…` with an `/xapis/auth/create` handshake — which has no fixture
today and replaces the classic's `/xml/chumbies`, `/xml/profiles`. The
Dash equivalent of FR17 (an empty channel is a clock; `empty_channel.rs`)
needs its own answer, since the theme, not a widget, is the default view.
Size: L. Patch surface: fixtures and `core/src/chumby/fixture.rs`/`navigator.rs`
(additions commit only) unless the XAPI signature needs a Rust-side verifier
(it does not: fixture answers are keyed on path).

Result, 2026-09-09 (step 4). The gate was smaller than feared, and the
XAPI family is *not* on the home-screen path: `CPMain.initialize()` calls
`goHome()` before `fetchDevice()` (`controlpanel/CPMain.as:81-82`), so the
theme loads first and the device/profile/profiles requests only feed the
widget area. What the Dash needed on the desktop, all fixtures, no code:

- `/psp/securityQuestion` and `/psp/securityAnswer` present, so
  `needStartNetwork()` (`StartupPanel.as:264-272`) is false. This — not
  `hasNetwork` — is what sent the first run into the network wizard;
  `hasNetwork` ends true because `StartupPanelCheckNetwork.as:28` uses
  `!hasErrors()`, and the fork's `network_status.sh` answer has no
  `<error>`. (`NetworkStatus.lastStatusResult`, `NetworkStatus.as:29`,
  also wants `up="true"`, which the fork's XML lacks; its three consumers
  are status surfaces — WiredButton, NetworkSummaryPanel,
  DelinkResult — and the classic reads no `up`/`link` attribute, so adding
  both is safe; filed under issue 19.)
- the classic `fixtures/http/xml.chumby.com/xml/authorize` answer as is:
  `Authorization.fromXML` (`structure/Authorization.as:75-92`) wants
  `<chumby id><name>…</name></chumby>`, which it is.
- `CHECK_DATE` is `new Date().getFullYear() > 2007`
  (`StartupPanelCheckDate.as:21`).
- **no `/tmp/nightmode`**: the classic tree tracks that file
  (`fixtures/rootfs/tmp/nightmode`, from the first host commit), and the
  Dash's `ScreenManager` boots into night mode when it exists — the second
  run showed the big night clock with a "Power Save" button over the
  theme. This is the concrete reason the Dash gets its own tree, together
  with `/psp/alarms`, which the Dash rewrites in its own schema at first
  start.
- a theme at `/psp/theme.swf` (`swf-assets/dash/default_theme.swf`,
  symlinked in by `run-dash.sh`) and `/psp/theme_name.txt`.
- exec manifest entries for `killall bivlcored` (→ `0`), the chumbrowser
  stop, `ap_scan` and `network_adapter_list.sh` (→ empty).

With those the panel goes BLANK → CHECK_NETWORK → CHECK_AUTHORIZE (fixture
hit) → CHECK_DATE → NORMAL_MODE → `CPMain` → `HomeScreenProxy` →
`ThemeLoader`: `rootfs HIT file:////psp/theme.swf` — the navigator's
`file://` mapping (survey §1.7 "unverified") works for the theme — and the
theme answers with `_setDisplayRect(2, 134, 454, 232, 472)` /
`_setDisplayRectEventTranslate(2, -134, -232)`, its widget rectangle. The
screen is the Space Theme's LCARS layout with "LOADING…" in the widget and
pick slots. No panic in 45 s; the 2 000-odd stack-underflow warnings
(issue 18) are still there.

Still open on this item, now as its second half: the widget area. After
`goHome()` the panel POSTs `/xapis/auth/create` (fixture missing → clean
fail → `gotBadDeviceInfo` does nothing, `DEVICE_UPDATE_TIME` poll), so an
offline channel needs `xapis/auth/create` → `<oauth_session valid_for="…">key</oauth_session>`,
`xapis/device/index/<guid>` → `<chumby anonymous="false"><name/><user id=""/><profile id=""/></chumby>`
(`Device.as:110-121`), `xapis/profile/show/<id>` → `<profile id><name/><info master=""/><widget_instances><widget_instance…/></widget_instances></profile>`
(`Profile.as:98-137`, `WidgetInstance.as:114-126`, `Widget.as:122-144`),
`xapis/profile/list/<guid>` → `<profiles><profile id><name/><description/><widget_instances thumbnail="" count=""/></profile></profiles>`
(`ProfileSummary.as:42-47`). Two fork points: fixture paths carry the GUID,
which is per box on the appliance, so `FixtureHost::fetch` needs a wildcard
segment; and the GET requests carry `oauth_*` query parameters, which the
fixture host already strips. Whether the Dash's default view needs any
widget at all (FR17's "empty channel is a clock" was about a widget-only
screen; here the theme *is* the clock) is a scope question for Jan.

Also seen and not chased: `tzdump America/Los_Angeles` and `…/New_York`
(issue 13 — the Dash's clock locations default to those two), the
`chumbthumb` exec for the sample photo (issue 13/15), the six music icons
and `externalmusic/sources.xml` on `files.chumby.com` (issue 16), and the
`exec://` command arriving percent-encoded (`nice -n 10 chumbthumb%20…`),
which the manifest prefix match survives but a Rust reimplementation must
decode — noted for issue 13.

Second half, 2026-09-09: the XAPI channel. Four answers under
`fixtures-dash/http/xml.chumby.com/xapis/` — `auth/create`
(`<oauth_session valid_for="86400">local</oauth_session>`; any non-empty
key authenticates, `XAPI.as:93-129`), `device/index/_`, `profile/show/1`,
`profile/list/_` — shaped after the parsers cited in each file's comment.
The one code change: `FixtureHost::fetch` now falls back to a file named
`_` in the same directory when the exact path misses (`fixture.rs`, test
`test_http_fixture_wildcard_last_segment`), because the Dash ends two of
these paths in the device GUID, which is per box. The exact file still
wins; every other fixture path is unaffected (a miss without a `_` sibling
stays a miss). The POST from `XML.sendAndLoad` reaches the same intercept
as a GET. Log of the run: `auth/create` HIT → `device/index/<GUID>` HIT →
`profile/show/1` HIT → `profile/list/<GUID>` HIT →
`_startSlave("file:////usr/widgets/builtinclock.swf", "<object>")` with
`_setSlaveVar(_chumby_widget_stage_width/height, 320/240)` — the theme
then shows "CLOCK" in its widget slot and the picker lists
"CHUMBYPI-CHANNEL / Clock / the built-in clock". The widget is not drawn:
`_startSlave` is the fork's logging stub (issue 14).

Also new in that run: once the channel plays, `WidgetSequencer` reads the
slave player's memory through `util/VSZ.as:3-13` — `cat /proc/<pid>/stat |
cut -d " " -f 23` with the pid from `/var/run/chumbyflashplayer.pid`; the
file is absent, so the command is `cat /proc/undefined/stat …`, issued
every second (32 times in 30 s). Harmless (empty answer → `Number("") = 0`)
but noisy; a pid fixture or a manifest entry belongs to issue 13.

---

Number: 13
Timestamp: 2026-09-09, 19:10
Title: Dash: exec touchpoints the fork does not answer.
Status: the ones the panel actually issues are answered on dev 2026-09-09 (tzdump, list_mounts, the memory poll, chumbthumb as a stub); the theme-picker strings wait on issue 16, the rest on being seen
Description: Survey §2.4 lists every command; missing today (desktop run and
`fixtures/exec/manifest.txt`): `ap_scan`, `network_adapter_list.sh`, `killall
bivlcored; echo $?`, `list_mounts` (USB mount list, four callers — real value
on the Pi, `real_net.rs` precedent), `imgtool --fb=N --fill=0,0,0`, `metadb
--prune <mnt>`, `du -s <cache>`, `chumby_haptic`, `hide_gfx_layer0`,
`delink_request` and friends, the chumbrowser start/stop pair (no browser: a
clean failure), and two that need real work: **`tzdump <zone>`**
(`time/TimeZoneTransitions.as:30` — the Dash computes DST transitions from
its XML `<zone><time utc= …/></zone>` output; implement in Rust from the
system tzdata, size M, the clock is wrong without it) and `chumbthumb`
(`image/ImageResizer2.as:8`, photo resize to a path — M, photos only). The
theme-picker path needs `/psp/download_theme <url> <md5>` → `<download_theme
error="success"/>`, `md5sum /psp/theme.swf`, and the `cp … /psp/theme.swf;
rm …; sync; echo $?` / `rm …` strings interpreted in Rust the way
`parse_widget_curl` (`navigator.rs:130-190`) does for the widget cache — S
for the local copy, M with the download. Size: M overall. Patch surface:
`core/src/chumby/` only (NFR2: Rust, not shell).

Done, 2026-09-09 (step 4, after item 12 — driven by what the runs on
`fixtures-dash/` actually asked for):

- **Two exec-string fixes in front of every handler.** The Dash's
  `AsynchronousCommand` sends `escape(cmd)`, so `exec://` commands arrived
  percent-encoded (`nice -n 10 cat%20%2Fproc…`); `navigator.rs` now decodes
  `%XX` in the `exec://` branch before dispatch (`percent_decode`, tested).
  The classic never encodes and none of its commands carries a `%` (grep of
  `frame_2/DoAction.as`), and its widget-cache `curl` URL is parsed from
  the raw URL before this point. `FixtureHost::exec` then strips a leading
  `nice -n <n> ` (`strip_nice`), so handlers and manifest prefixes see the
  command itself — the classic manifest's `nice -n 10 curl` became `curl`
  (only consumer of that form). Backtick commands were never encoded and
  never wrapped; unchanged.
- **`tzdump <zone>`** — `core/src/chumby/tzdump.rs` (160 lines with tests)
  reads the system TZif file `/usr/share/zoneinfo/<zone>` (RFC 8536, the
  64-bit block) and emits `<zone name><time utc gmtoff isdst abbrev/>…</zone>`
  ascending from 1970, the shape `time/TimeZoneTransitions.fromXML` and
  `TimeZoneTransition.fromXML` parse; a zone without transitions (UTC)
  yields its one type at utc 0 so the panel has an entry. Debian's tzdata
  spells every transition out through 2037 (236 for New York, checked on
  this machine and on the box, tzdata 2026b), so the POSIX footer is not
  expanded — a 2038 item. Zone names cannot leave zoneinfo. An unknown zone
  answers empty, which the panel turns into a single UTC entry, with a
  warning logged. `jiff` sits in `Cargo.lock` but is not built for our
  targets, so no dependency was added. The appliance must depend on
  `tzdata` (Debian base has it; make it explicit — appliance issue 15).
- **`list_mounts`** — `FixtureHost::mounts_xml`: `<mounts><mount point=
  "/mnt/usbN" port="N"/></mounts>` for `usb`, `usb2`…`usb4` entries of the
  virtual rootfs that resolve to a directory, i.e. on the appliance the
  launcher's symlink onto the real mount, so an unplugged stick lists
  nothing (`USBMediaEvents.gotMountList`, `USBVolume.addGenericVolume`).
- **the slave-memory poll** — `cat /proc/<pid>/stat | cut -d " " -f 23`
  every second while a widget plays (`util/VSZ.as`, `WidgetSequencer.
  checkMemory`): answered empty by an explicit handler, because there is no
  slave player to measure; `Number("")` is 0, below both `MAX_VSZ`
  (100 MB, which would `fscommand("quit")`) and `MAX_VSZ_WIDGET`. Feeding
  it the fork's own VSZ (~1.5 GB) would quit the panel every 16 polls.
- **`chumbthumb`** — a manifest stub answering `1` (its failure status):
  the theme's photo module then shows no photo. Photos are out of scope
  until someone wants them (issue 15's optional half).

- **`platform`** — with `tzdump` answered, both world clocks still showed
  plain UTC: `time/InternationalDate.as:17` applies a location's transition
  only when `Chumby.platform == "yume"`, the Dash's hardware config name,
  and the fork answered `_getPlatform` (5,202) with the classic's
  `ironforge`. The name is now a fixture file, `<tree>/platform`
  (`fixtures/platform` = `ironforge`, `fixtures-dash/platform` = `yume`;
  absent → `ironforge` with a warning), read once by `FixtureHost::new` and
  answered by `_getPlatform` and `_getEnvironment("CONFIGNAME")`. Consumers
  of the value: the Dash's `InternationalDate` and a `PLATFORM_STORMWIND`
  compare in `display/ScreenManager.as:298`; the classic's 23 compares
  against `falconwing`, `insignia3.5`, `ironforge` — its tree keeps
  `ironforge`, so nothing moves there.

Verified: `cargo test` for the chumby modules (51 pass); the Dash on
`fixtures-dash/` — `tzdump` answered for both clock locations and, after a
click on the theme's TIME tile (a press held 0.3 s; a plain xdotool click
was too quick for the theme's button), the world clocks read San Diego
11:03 AM and New York 2:03 PM against the system's 11:03 / 14:03; no
MISSING exec left except the two `files.chumby.com` families of issue 16 —
and 35–40 s classic runs on `fixtures/` after each change (exit 0, no
MISSING, `_getPlatform` → `ironforge`). Not done, by design: `imgtool`, `metadb
--prune`, `du -s`, `chumby_haptic`, `hide_gfx_layer0`, `delink_*` — none
has been issued in a run yet; add them when they are. The theme-picker
`cp`/`rm`/`md5sum`/`download_theme` strings belong with the catalog
(issue 16).

---

Number: 14
Timestamp: 2026-09-09, 19:10
Title: Dash: widgets are composed inside a theme-chosen rectangle.
Status: done on dev 2026-09-09 — `dash_widget.rs` pins `WidgetSequencer._isChumby` to false, the panel's own proxy branch does the rest
Description: On a device `WidgetSequencer.playCurrentWidget` takes the
`_startSlave` branch (`widgetbrowser/WidgetSequencer.as:353-360`), which the
fork does not run (requirements FR2 M6); the classic gets its widgets through
the panel's own `widgetProxy`/localCache path, and the intro through a
prototype replacement (`intro.rs`). The Dash carries the same alternative
inline: its off-device branch (`:363-372`) `loadClip`s the widget into
`__widgetProxy` under `_lockroot`, and `setPositionAndSize` (`:600-611`)
places it in the rectangle the theme asked for via
`ThemeCallbacks.setWidgetPosition` (`dash/themes/ThemeCallbacks.as:109-113`);
on the slave branch the same rectangle goes to `_setDisplayRect(MAP_SLAVE,…)`
and `_setDisplayRectEventTranslate` plus `_chumby_widget_stage_width/height`
slave vars. Options: make `WidgetSequencer._isChumby` false while
`Chumby.isChumby` stays true (its origin is unread — verify first), or
answer `_startSlave` by performing the proxy load. Widgets stay 320x240
content (`:364`) scaled into the mask, so the classic widget set is reusable.
Size: M. Patch surface: additions (a prototype surgery next to `intro.rs`).

Done, 2026-09-09. `_isChumby` is an instance field assigned once in the
constructor from `Chumby.isChumby` (`WidgetSequencer.as:67`) and read at
`:70` (which loader to set up), `:351` (slave or proxy load) and `:572`
(how to place it); no other class reads it, and `CPMain.isChumby()` is a
different thing. `core/src/chumby/dash_widget.rs` adds a virtual
`_isChumby` to `com.chumby.controlpanel.widgetbrowser.WidgetSequencer.prototype`
through `addProperty` — getter false, setter a no-op that swallows the
constructor's assignment — one-shot with retry on the native-call cadence
like `intro.rs`; the classic has no such class, so the lookup never
succeeds there (`pinned=0` in its log). With that the Dash takes its own
off-device branch unchanged: `MovieClipLoader` into `__widgetProxy` under
`_lockroot`, `onLoadComplete` injects the `_chumby_*` parameters and calls
`setPositionAndSize`, which puts the 320x240 widget at the theme's
rectangle by position and scale (`:596-600`); `checkWidgetDoneProxy` polls
the clip. The device-only calls (`_startSlave`, `_setDisplayRect`,
`_setSlaveVar`, `_grantSlavePrivileges`, `prepareWidgetSettings`) are no
longer made; the one slave-var read left is `_getSlaveVar("_chumby_widget_state")`
from `WidgetStateKeeper`, answered `Undefined`, harmless.

Run: `rootfs HIT file:////usr/widgets/builtinclock.swf` (the classic's
built-in clock from the backup, linked in by `run-dash.sh` from
`swf-assets/dash/widgets/`), no `_startSlave`, and the clock draws inside
the Space Theme's widget area at 134,232 with the theme's PREV / PIN / NEXT
controls above it. Classic regression run clean. Not covered: the
`ExtendedEvents.WidgetLoadStatus` → `onEvent("loadstatus")` path (slave
only: `_getSWFDimensions`, aspect ratio into `WidgetMask`) — on the proxy
path the mask keeps the theme's rectangle, which is what the built-in
clock needs; a widget with an odd aspect ratio may want it later.

---

Number: 15
Timestamp: 2026-09-09, 19:10
Title: Dash: the `sys://` local-file scheme.
Status: open — step 3 item
Description: The theme loads resized photos as `sys://<path>`
(`default_theme` `com/example/PhotoHolder.as:25`), the panel builds
`sys:////<cache path>` (`util/CacheManager.as:132`) and `sys://<usb photo>`
(`settings/usbphotos/USBPhotoPanelItemInfo.as:22`), and
`ThemeCallbacks.deleteTemporaryFile` strips `sys:/` (`:196`).
`navigator.rs:59` already strips `file://` before the rootfs lookup; `sys://`
is the same one-liner. Size: S. Photos as a feature (USB scan, Photobucket,
`chumbthumb`) are a separate, optional M–L on top.

---

Number: 16
Timestamp: 2026-09-09, 19:10
Title: Dash: chumby.com surface to intercept (NFR6).
Status: done on dev 2026-09-09 — the self-updater and music manifest answered from fixtures, the theme catalog generated from the rootfs, the picker's file commands interpreted in Rust
Description: Survey §2.6. Beyond `xml.chumby.com` (issue 12) the Dash talks to
`files.chumby.com/dash/$RELEASE$controlpanel/controlpanel.xml` — a **panel
self-updater** 30 min after start and daily (`structure/UpdateMonitorCP.as:5-27`;
a fixture miss is a clean "no update", but it must never pass through),
`…/themes/themes.xml` (the catalog: a fixture listing the locally installed
themes makes the picker work offline — this is how a user-supplied theme
shows up in the UI without a stick), `…/externalmusic/sources.xml`,
`…/icons/*.png`, `files.chumby.com/yume/photoimages/photos.xml`,
`content.chumby.com` (SHOUTcast, NYT podcasts, NOAA), `music.chumby.com`
(chumbcast, sleepcast — classic passthrough precedent, `fixture.rs:391-399`),
plus `chumby.weather.com`, `images.weather.com`, `sonyyume.accu-weather.com`
(dead third parties: clean failure). Verify `is_chumby_host` covers
`files.chumby.com` and `content.chumby.com`. Size: S–M, fixtures under
`fixtures/http/`. Patch surface: none.

Done, 2026-09-09. Two static fixtures under
`fixtures-dash/http/files.chumby.com/dash/production/`:
`controlpanel/controlpanel.xml` with an empty `build` — `UpdateMonitorCP`
(`structure/UpdateMonitorCP.as:29-54`) compares it against the running
version and treats empty as "no update", so the **panel self-updater can
never fire**; and `externalmusic/sources.xml` as an empty `<MusicSources/>`
(`music/ExternalMusicSources.as:54`). The six `dash/icons/*.png` stay
missing on purpose: a missed icon load leaves the music row's art blank and
costs nothing, and inventing chumby's artwork would be worse than a gap.

**The theme catalog is generated, not canned** —
`core/src/chumby/dash_theme.rs` (240 lines with tests). `FixtureHost::fetch`
routes `files.chumby.com/dash/<release>/themes/themes.xml` to
`catalog_xml`, which lists every `*.swf` in `/psp/themes` of the virtual
rootfs as a `<theme>` with the md5 the panel will check, a `file:///` url,
and a `.jpg`/`.png` sibling as `thumbnailURL` if there is one; the name is
the filename with underscores as spaces (`Space_Theme.swf` → "Space
Theme", chumby's own label). So a theme dropped into the tree is offered by
the in-panel picker with no server at all — the appliance half of the
user-supplied-theme route (chumby-pi issue 16), and the reason the
`externalthemes.xml` stick is now optional rather than the only way.

**The picker's four command shapes are interpreted on the rootfs**, in Rust
rather than a shell (NFR2), in the same module: `download_theme <url> <md5>`
copies a `file://` source to `/tmp/theme.swf` when the md5 matches and
answers the script's own `<download_theme error="…"/>` (anything not
`file://` is "theme not found", so nothing is ever fetched from
chumby.com); `cp <src> <dst>`, `rm <paths>`, `sync` and `echo $?` are
applied step by step for the exact sequences the picker and the scheduler
updater issue, and a sequence with a step we do not know falls through
untouched rather than being half-executed.

Verified on the desktop, driving the real UI: bend → the popup bar's themes
button (`chumby_pick` names `_popupBar.themesButton.b`) → "Theme Selector"
lists **space theme** from the generated catalog → picking it runs
`/psp/download_theme file:////psp/themes/Space_Theme.swf <md5>` then
`cp /tmp/theme.swf /psp/theme.swf; rm …; sync; echo $?` and the panel
reloads the theme (`rootfs HIT file:////psp/theme.swf`). Unit tests cover
the install writing the new bytes, a checksum mismatch, a non-`file://` url
and the delete. 53 chumby tests pass.

Two things the runs taught, both recorded rather than fixed here:

- **`/psp/theme.swf` must never be a symlink to an asset.** The launcher
  used to link it at `swf-assets/dash/default_theme.swf`; installing a
  theme writes that path, so the panel wrote *through* the link into the
  source copy (same bytes this time, silent corruption with any other
  theme). `run-dash.sh` now seeds it with a copy. The appliance's own
  seeding (chumby-pi issue 14/16) must do the same.
- **Without an installed theme the Dash has no home screen**: `ThemeLoader`
  finds none of its five paths, the load never completes, and the screen
  shows only the widget at 0,0 with the popup bar unreachable — so the
  picker cannot be the first-boot route. chumby's own `install_chumby.sh:7`
  seeds `/psp/theme.swf` for exactly this reason.

---

Number: 17
Timestamp: 2026-09-09, 19:10
Title: Dash: six natives the fork does not bind.
Status: done on dev 2026-09-09 — all six named, the three the panel calls answered
Description: `com/blueocty/DashNative.as`: (5,390) `_getFlipState` and (5,391)
`_setFlipState` (the Dash's upside-down mode, `accelerometer/Flipper`),
(5,392)/(5,393) logo LED, (5,394) `_fadeBacklight` (map onto `brightness.rs`
or ignore); `ChumbyNative.as:287` (5,445) `_getWidgetNumber`, read at
`WidgetSequencer.as:238-240,681` behind an `!= undefined` guard, so
`Undefined` skips the branch. Bind in `avm.rs`'s name table, answer in
`fixture.rs`. Size: S. Patch surface: additions only.

Done, 2026-09-09. Six names in `avm::wrapper_name`, three defaults in
`fixture::default_for_getter`; the host's existing name-keyed store handles
each get/set pair without further code. Answers and why:

- `_getFlipState` (5,390) → 0, `DashNative.UNFLIPPED`. Read once in
  `accelerometer/Flipper.as:15` to seed `flipped`; the panel is the right
  way up and only a real accelerometer changes that (`Flipper.update` is
  polled solely when `P3D.hasAccelerometer()`, `:16-19`).
- `_setFlipState` (5,391) → stored. Called from `Flipper.update:47`, which
  we never reach; storing keeps a later `_getFlipState` honest.
- `_getLogoLEDState` (5,392) → 0, `_setLogoLEDState` (5,393) → stored.
  There is no logo LED on a Pi. Inherited oddity worth knowing:
  `LogoLED.turnOn()` (`controlpanel/gadgets/LogoLED.as:18-21`) sets
  `LOGO_LED_OFF`, exactly as `turnOff()` does — chumby's own bug, so on a
  real Dash the LED can only ever be switched off.
- `_fadeBacklight` (5,394) → Undefined. Bound by `DashNative` but called
  from nowhere in the export; the name only makes a future call legible in
  the log.
- `_getWidgetNumber` (5,445) → 0. `WidgetSequencer.playCurrentWidget:238`
  defers a widget switch while two or more widget players are alive and
  `onTimer:681` releases it again; we run none beside the panel itself, so
  nothing is deferred. This also lets the panel's own index wrap at `:243`
  run, which the previous `Undefined` skipped.

Run: `_getFlipState() -> 0`, `_setLogoLEDState(0)`, `_getWidgetNumber() -> 0`
all dispatched, no `_unknown` left, home screen and widget unchanged;
classic run clean and never calls any of them.

---

Number: 18
Timestamp: 2026-09-09, 19:10
Title: Dash: 2 267 AVM1 stack-underflow warnings in 45 s.
Status: open — step 3 item, investigation
Description: Survey §3: `Avm1::pop: Stack underflow` 2 267 times during the
panel's first 45 s and 64 times for the theme alone, almost all during
`DoInitAction` class initialisation (1 332 such tags, SWF version 8). No
panic and the panel runs, so the effect is unknown — a wrong value on an
empty pop is the kind of thing that later shows as a missing listener or a
silent branch. Diagnose with `RUST_LOG=ruffle_core::avm1=trace` on the theme
(64 warnings, small); if it is a Ruffle opcode-semantics gap it is an
upstream bug and, if fixed locally, lands in the edits commit. Size: S to
diagnose, unknown to fix.

---

Number: 19
Timestamp: 2026-09-09, 19:10
Title: Dash: subsystems written against the classic that need re-verifying.
Status: open — step 3 item
Description: Things the fork does by name or by classic layout: (a)
`alarm_guard.rs` and `backup_alarm.rs` hook `Alarm.ringAlarm` /
`stopAlarmsExcept` F2 prototypes — the Dash's `com.chumby.alarm.AlarmSet` is
a different class tree, so the hooks will not match and the issue-10 bug may
or may not exist there; the Dash also has scheduler events (`/psp/events`,
`com/chumby/event/*`). (b) `ui-policy.toml` (12 rules) names classic
screens; the Dash needs its own policy set, starting empty. (c) Audio: the
Dash calls the same BTPlayer natives the fixture host answers
(`_playAudio`, `_getAudioPlayerState`, …) plus `_sendMediaPlayerCommand`
(5,152, 3 sites) and the pipe family (5,191-195), both logging stubs today.
(d) Volume, mute, time zone and system time go through natives 5,176-185
(fixture-answered) rather than `chumby_set_*` scripts. (e) Display: the Dash
drives two framebuffers (`_setDisplay` 0/1, overlay visibility and chroma
blending, `_fillFrameBufferBytes`, `_enableSlaveUpdates`) with no
`/psp/nooverlay` switch; all are logging stubs and the panel still painted on
the desktop — confirm nothing hides behind an overlay assumption once theme
and widget are on screen. (f) `_getScreenWidth/Height` (5,200/201) feed
`Chumby.screenWidth/Height`; check what the fixture returns for an 860x480
stage. Size: M in total, mostly reading and running.
