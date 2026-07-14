# Development

How to build, run, verify, and evolve this fork. Everything player work
needs is in this repository. Deployment onto hardware is not covered here —
Debian packaging, the Raspberry Pi kiosk and the setup howto belong to
[chumby-pi](https://github.com/yanosz/chumby-pi), which pins this repo as a
submodule.

Concepts: [design.md](design.md). What the player owes the panel:
[requirements.md](requirements.md). The public overview and the
`ASnative(5,N)` reference: [`README.md`](../README.md).

---

## 1. Branch and commit policy

**One feature branch per working session, squashed on merge.** Branch from
the fork's default branch (`chumby`) and do the session's work in as many
commits as are useful. Do not amend and force-push a long-lived shared
commit — that was the old discipline and it is retired.

**Finishing a session means opening the pull request.** Push the branch and
create the PR yourself; Jan reviews and merges it with GitHub's *Squash and
merge*. Leaving a pushed branch with no PR is an unfinished session.

```sh
git push -u origin <branch>
gh pr create --repo yanosz/chumby-ruffle --base chumby --head <branch> \
    --title "…" --body "…"
```

`origin` is the GitHub fork (`yanosz/chumby-ruffle`) and `upstream` is
`ruffle-rs/ruffle`, so a plain `git push -u origin <branch>` is right.
Keep `--repo` on `gh pr create` anyway: with two GitHub remotes, `gh`
may otherwise resolve against `upstream` and open the PR on
ruffle-rs/ruffle. (An earlier claim here that `origin` was a local
clone was stale — verified against `git remote -v`, 2026-07-10.)

`chumby.yml` runs on the PR, but **on a pull request it only builds.**
Starting the movie needs `controlpanel.swf`, which is copyrighted and lives
on a private share, so the movie-start check runs on push to `chumby` — that
is, after the squash-merge — and on manual dispatch. A PR proves it compiles;
`chumby` proves it runs.

The consequence is that CI cannot catch a dead ASnative hook before a merge.
Run the movie-start check locally before opening the PR (§5); that is now the
only pre-merge gate there is.

The fork's default branch tracks upstream Ruffle with the chumby work
applied on top. When the pin in chumby-pi moves, the submodule gitlink in
that repository must be bumped in the same change that depends on it.

## 2. Layout

Everything the fork adds is in `core/src/chumby/`:

| file | role |
|------|------|
| `mod.rs` | module wiring |
| `host.rs` | the `ChumbyHost` trait + the process-global registry |
| `config.rs` | owner knobs from `<fixtures>/player.toml`, read once at start (FR14) |
| `fixture.rs` | `FixtureHost`: natives, exec manifest, HTTP fixtures, virtual rootfs |
| `real_net.rs` | `RealNetHost`: live network state, wraps `FixtureHost` |
| `avm.rs` | the `ASnative(5,N)` dispatch table |
| `music_sources.rs` | VM-level hiding of unsupported music sources (FR15) |
| `navigator.rs` | `exec://`, chumby HTTP, and `file://` interception |
| `ui_policy.rs` | declarative disabling of panel controls |
| `ui-policy.toml` | the rules themselves, compiled in with `include_str!` |
| `audio.rs` | mpv backend over JSON IPC |
| `input.rs` | control FIFO |

Upstream files carry registration hooks only. Every hook site has a comment
containing the word `chumby`, so `grep -rn chumby <file>` finds what to
re-apply after a rebase. The full list is in [design.md](design.md) §8.

Around the fork, at the repo root:

| path | role |
|------|------|
| `fixtures/` | what the panel is answered with — `rootfs/`, `exec/`, `http/`, `widgets/` |
| `swf-assets/` | `controlpanel.swf` goes here; self-ignoring, you supply it |
| `run-controlpanel.sh` | the desktop run |
| `verify-screens.sh` | drives the panel to named screens and screenshots them |
| `claude-docs/appendix/` | the ffdec export of the panel — gitignored, ~36 MB, **the law** |
| `claude-docs/images/` | panel screenshots — gitignored (chumby artwork) |

## 3. Build

```sh
cargo build -p ruffle_desktop
```

There is no `chumby` cargo feature — the chumby code is always compiled. If
you find `--features chumby` anywhere, it is stale.

Profiles: `debug` for iteration; `release` for a quick optimized check
(~20 s warm); **`dist`** (release + fat LTO + `codegen-units=1`) for anything
that ships. `dist` measurably reduces CPU on the Pi and shrinks the binary
from ~37 MB to ~29 MB, at the cost of a multi-minute link even on an
incremental rebuild.

Cross-compiling for the Pi (aarch64) is driven from the chumby-pi tree,
whose `.cargo/config.toml` sets the linker and `PKG_CONFIG_*` — deliberately
kept *outside* this repo so the fork stays upstream-clean. You do not need
it for player work.

Upstream Ruffle needs a JVM at build time (it compiles its ActionScript
stdlib with Adobe's `asc.jar`), on the build host only.

## 4. Run

`controlpanel.swf` is copyrighted chumby firmware and is not in the repo.
Obtain it from your own chumby (or its backup) and drop it in
`swf-assets/`, then:

```sh
cargo build -p ruffle_desktop
./run-controlpanel.sh                 # extra args pass through to ruffle
```

which is a wrapper around:

```sh
ruffle_desktop \
    --load-behavior blocking \        # mandatory, see requirements.md FR7
    --filesystem-access-mode allow \
    --chumby-fixtures fixtures/ \
    --chumby-control /tmp/chumby-ctl \
    -PlocalCache=1 \
    swf-assets/controlpanel.swf
```

An optional `fixtures/player.toml` (gitignored; absent = defaults;
template: `fixtures/player.toml.example`) carries the owner knobs —
`volume_cap` (percent, panel 100 % maps to it), `access_chumby_com` and
`enable_lyrion` (both 0/1, default 0), `brightness_ctl` (path to a 0/1/2
brightness executable, FR16) — read once at start (requirements
FR14/FR15).

`-Pbuiltin=1` additionally takes the offline boot path (no authorize round
trip). Useful environment:

```sh
RUST_LOG=warn,chumby_host=info          # every host call, args and result
RUST_LOG=warn,avm_trace=trace           # the panel's own trace() output
RUST_LOG=warn,chumby_pick=debug         # what a click actually hit
```

`chumby_pick=debug` is the tool for UI-policy work: a missed click and an
inert control look identical without it.

Drive the panel from a script through the FIFO (the appliance's
`chumby-ctl bend` — chumby-pi `pkg/chumby-player/` — is the same thing). The FIFO must be a real FIFO — `echo >` to a missing path
creates a regular file and the player disables the channel:

```sh
echo bend            > /tmp/chumby-ctl   # summon/dismiss the button bar
echo "click 448 458" > /tmp/chumby-ctl
echo "drag 100 200 300 200" > /tmp/chumby-ctl
```

`./verify-screens.sh` walks the panel to the alarms, My Streams and volume
screens and screenshots each into `claude-docs/images/`.

## 4a. The decompiled panel

`claude-docs/appendix/` holds the ffdec export of `controlpanel.swf`
2.8.87b3 — scripts, frame labels, sprite renders, the tag dump. It is
gitignored (copyrighted, ~36 MB) and is the ground truth for every claim
about what a screen contains, which is why `F2:<line>` references in
requirements.md and design.md point into
`appendix/controlpanel-2.8.87b3/scripts/frame_2/DoAction.as`.

Regenerate it with:

```sh
ffdec -export script,frame,image,shape claude-docs/appendix/controlpanel-2.8.87b3 \
      swf-assets/controlpanel.swf
```

Instance names and depths — what UI-policy selectors are built from — come
from the XML dump (`ffdec -swf2xml`), not from the script export.

### Where the knowledge came from

The method, if any of it ever has to be redone: export with ffdec, scan the
ActionScript statically for every external touchpoint (`ASnative(`,
`fscommand(`, `getURL`, `XML.load`, `loadMovie`, `SharedObject`), then run the
panel under *stock* Ruffle and triage what breaks. The static scan gives the
prior; the run tells you what actually fires.

- [ChumbyNative](https://wiki.chumby.com/index.php?title=ChumbyNative) — the
  primary reference for the vendor-call table, still online. So is
  [Controlling BTplay](https://wiki.chumby.com/index.php?title=Controlling_BTplay)
  (the audio family). `Developing_Widgets_for_Chumby:_Sensor_Access` is a 404
  as of 2026-07.
- [Scott Janousek's Flash Lite deck](https://speakerdeck.com/scottjanousek/developing-flash-lite-widgets-for-the-chumby-platform)
  — a community list of `ASnative(5,n)` indices.
- forum.chumby.com thread id=9663 ("Success!", a Sony dash running the panel)
  — the evidence that a replacement control panel needs only the ChumbyNative
  call set, which is the premise this whole fork rests on.
- [zurk's offline firmware](https://github.com/francistheodorecatte/zurks-offline-firmware-classic)
  — the prior art. It impersonates chumby.com with DNS capture plus a local
  lighttpd serving ~10 static XML stubs and ~10 CGI scripts, keeps real
  internet radio flowing while mocking everything account-related, and serves
  widget SWFs over `file://` hrefs. We do the same thing inside the player
  instead, because we own it.
- `/home/jan/chumby_backup` — a real Chumby Classic's rootfs (firmware 1.7.2,
  `ironforge`), read-only ground truth for what the environment looked like.

When the wiki, the backup and the decompiled SWF disagree, **the SWF wins.**
The wiki lists the time family under category 103; the SWF binds it at
5,176–178. The wiki numbers a `PlayAudio` at 5,151; the panel calls
`_playAudio` at 5,144.

## 4b. Widgets on a dev box

`fixtures/http/xml.chumby.com/xml/profiles` is a static fixture with an
empty instance list — the panel boots and idles fine without widgets, so
CI and most dev runs need nothing more. To play widgets on the desktop,
hand-write a `profile.xml` into `fixtures/rootfs/psp/` (absolute
`file://` movie hrefs — `_getFile` does **not** expand `{FIXTURES}`); the
panel merges it at channel load (design §3). `fixtures/widgets/` remains
the gitignored drop zone for the SWFs themselves. On the appliance the
same merge is fed by `chumby-local-widgets` (chumby-pi design §4).

## 5. Verify

Three levels, in ascending cost.

**Unit tests** — `cargo test -p ruffle_core chumby`. The ones that matter
cover things that silently rot: the ui-policy parser and selector
segmentation, that the *shipped* `ui-policy.toml` parses with no rule
skipped (a typo there would otherwise surface as a control that stays live
on the device), the `_setTimeZone` → `_getTimeZone` round trip, and the
audio state machine.

**The movie-start check** — the real acceptance gate (NFR8), because a
build can compile clean and still have dead ASnative hooks. Run the player
headless under a timeout with `chumby_host=info`; success is *all three* of:
exit code **124** (still alive when the timeout fired), `_getPlatform`
present in the log (the panel executed a chumby native), and no `panicked`
line. Audio-device failure on a headless machine is expected and non-fatal.

**A real run**, watching the screens change. Nothing else catches a control
that renders disabled but still fires, or a widget that loads but never
paints.

**Empty channel → clock** (FR17, desktop, verified 2026-07-14): with no
`psp/profile.xml` in the fixtures the panel entered clock mode (`bi_clock`
rendering date + local time) instead of the former black widget area; with a
one-widget local profile the wrapper took the original path (widget played,
no clock-mode line in the log); from clock mode the bend menu, Music (My
Streams / My Music Files), Settings and Alarms all opened normally. The
boot-time "screen present but control missing" ui-policy warnings are a
startup transient (buttons attach after the screen sprite) — the rules apply
correctly once the Settings screen shows, confirmed by pick-trace-inert
clicks; the same transient also fires on the 2.8.75 panel from the 1.7.3
`update.zip`, which otherwise passes the movie-start check but misses the
`content.chumby.com/music_sources` fixture (why 2.8.87b3 stays the shipped
panel; appliance downloader).

**USB music** (desktop, verified 2026-07-11): Music → My Music Files opens
the `/mnt/usb` browser over the fixture tones (`fixtures/README.md`).
Exercised end-to-end: browse, descend into `album/`, back, breadcrumb
normalization, per-track play, Play All (recursive `FileFinderPOSIX` scan
found all 4 including the subfolder), next/prev, shuffle (Random order
visible in the track sequence), stop, now-playing row highlight, the
source-list "Last:" resume banner, and the "No files available" message on
an emptied `/mnt/usb` (no hang; the browser re-lists on every screen
entry, so hotplug needs no restart). Alarm-from-USB both ways: an
`arg="mp3files"` alarm with a valid `param` path rang from that file at
the set minute, and one with a vanished path logged "path not found,
searching for files", re-scanned `/mnt/usb` and rang from what it found.
To arm one without clicking through the wizard, write `/psp/alarms` with
`type="audio" arg="mp3files"
param="&lt;mp3files path=&quot;/mnt/usb/…&quot; /&gt;"` and restart — the
panel reads the file only at boot. One xdotool trap from this pass: a
`pkill -f <pattern>` whose pattern appears in the invoking shell's own
command line kills the wrapper first (`pkill -x ruffle_desktop` instead).

**Music sources** (desktop, verified 2026-07-11): with no player.toml the
Music screen lists exactly My Streams / My Music Files (boot log:
`music sources hidden: […]`). `access_chumby_com = 1` brings back
SHOUTcast / blue octy radio / Sleep Sounds — directory fetches log
`music host passthrough`, and SHOUTcast played audibly end-to-end (select
station → PLAY → tune-in redirect → mpv on the real stream URL);
`enable_lyrion = 1` brings back Squeezebox Server. Navigation: bend →
Music icon (570,335); rows start at (150,160), PLAY at (57,458).

**Brightness** (desktop, verified 2026-07-13, both modes): with no
`/sys/class/backlight` and no `brightness_ctl`, Settings renders the
BRIGHTNESS icon dimmed like NETWORK/TOUCHSCREEN (the
`only_without_brightness` rule holding). With
`brightness_ctl = "<script>"` in `fixtures/player.toml`: boot logs
`chumby_version -h -> "3.7"` and the script runs once with `0`
(`restoreDimFromFile`); the icon is live; it opens the **radio view**
("Set screen brightness", Full/Low); selecting Low runs the script with
`1`, Full with `0` (log: `_setLCDMute(1)` / `(0)`); DONE writes
`/psp/dimlevel` — that write lands in `fixtures/rootfs/psp/` like all
panel persistence, so clean it up after a walkthrough. The slider→sysfs
path is unit-tested (`test_brightness_knob_write_drives_backlight`,
scaling in `brightness.rs`); exercising it live needs a machine with a
real backlight device. Navigation (window coords): bend → SETTINGS
(448,458) → BRIGHTNESS (527,163); radio Full (163,188), Low (163,280);
DONE (557,463).

**Intro** (desktop, verified 2026-07-11): needs `intro.swf` from the backup
at `fixtures/rootfs/usr/widgets/intro.swf` (gitignored), and on a box with
no sound card the ALSA null-device workaround from §7 — without it the
tour freezes on its first frame. Navigation: bend → SETTINGS (448,458) →
CHUMBY INFO (121,164) → INTRO (342,462). Success: the Hamby/factory scene
animates, chapters change (~7–25 s each), and tapping `exit` (557,75)
advances to the next channel widget (log: `playIntro: staging`, then
`intro completed — advancing`). Two faithful oddities, not bugs: mid-tour
the exit button is dead (the SWF re-places `exit_btn` without rewiring
`onRelease`; only frames 1 and 15 wire it), and the tour's second half
*renders a mock control panel* — screenshots of it look confusingly like
the real UI. Exercised in this pass: INTRO click, chapter progression
through the accelerometer ball page (ball centered = the 5,60 fixture is
right), exit-at-frame-1 → channel resume, and the movie-start check with
the surgery in (exit 124, `_getPlatform`, no panic).

The two remaining flows, verified 2026-07-12. **Standalone ending**
(`intro.swf` run directly, `--chumby-fixtures` so the backticks reach the
host): `exit` (557,75) → the three-button control screen. RESUME TOUR
resumes from the bookmark (mid-tour, not frame 1); NEVER SHOW TOUR AGAIN
ran the `disable_intro` backtick (flag file appeared in the fixture
rootfs) and `fscommand("quit")` really exited the process, exit 0 — the
in-panel swallow correctly stands down when `Object._chumby` is absent;
SHOW TOUR AT NEXT STARTUP removed the flag and quit the same way.
**Same-session replay**: INTRO → exit → clock widget → INTRO again played
from the first scene (`playIntro: staging` ×2, `intro completed` ×2, no
panic) — each play attaches a fresh widgetProxy, so no stale done-flag.

To exercise the backup alarm (FR13) without waiting for a real alarm: start
the player, let it boot (~15 s — the panel rewrites `/psp/ifalarm` at boot,
so arming earlier gets overwritten), then
`date +%s > fixtures/rootfs/psp/ifalarm`. Within the 2 s poll the log shows
"primary alarm unanswered — sounding", the Klaxon plays for
`/psp/backup_alarm_duration` seconds, and the file is deleted. Write the
duration/volume knobs first to keep a desktop test short and quiet.
Verified this way 2026-07-10, including the panel-side arm (boot wrote
`ifalarm` for an enabled `backup="1"` alarm) and dismissal (the panel's
boot cleanup `rm` really deleted the file).

CI is `.github/workflows/chumby.yml`; what it runs where is §1. Fixtures are
in-repo, so only `controlpanel.swf` is fetched, by rclone from a private
share configured entirely through `RCLONE_CONFIG_RSHARE_*` secrets. The SWF
is never committed, cached, or uploaded anywhere. The tracked fixture tree lacks the
gitignored widget SWFs; the panel boots without them (the widget load fails
with a non-fatal `FetchError`), which is what makes this work.

Inherited upstream workflows are kept, not deleted, so that future upstream
merges stay conflict-free. Most disable themselves on a fork already: the
test/lint ones filter on `master`, which this fork never has, and the Crowdin
and release ones guard their entry job on `github.repository ==
'ruffle-rs/ruffle'`.

`test_extension_dockerfile.yml` was the exception, and it bit us on
2026-07-10. Its guard covered only the Discord-notify step, so the job itself
ran — and scheduled runs use the default branch, which here is `chumby`. It
builds the Firefox extension for `wasm32-unknown-unknown`, and **this fork
does not compile for wasm32**: since the `chumby` cargo feature was removed
(§3) `core/src/chumby` is always in the build, and `audio.rs` imports
`std::os::unix::net::UnixStream` for the mpv IPC socket. Weekly red CI on a
job about Docker, whose real cause was neither Docker nor the extension. The
fix was to give its job the same repository guard the others carry. Any
inherited workflow that runs on a schedule needs that guard: cron ignores
branch filters.

## 6. Merging upstream

The history was rebuilt for exactly this on 2026-07-14 (roadmap item 6):
`chumby` is **two commits on upstream master** (pinned `8328af42d`) — first
the pure additions (`core/src/chumby/`, `fixtures/`, docs, harness), then
the small edits to upstream files — so a rebase concentrates its conflicts
in the second commit. The pre-rebuild history (18 commits on `7f62f5dbf`)
is preserved as branch `chumby-old`. Keep the shape: rebase the pair, don't
pile merges on top.

1. Merge or rebase onto current upstream `master`.
2. For each file in the patch-surface table ([design.md](design.md) §8),
   `grep -n chumby` it and confirm the hook survived.
3. Do **not** re-introduce the `chumby` cargo feature, however tempting the
   conflict resolution looks. Its removal was a decision, not drift.
4. Do **not** restore `.github/dependabot.yml`. Upstream ships one; this fork
   deleted it on 2026-07-10. Dependencies here move when upstream is merged,
   not when a bot opens a pull request, so its only effect was noise — ten
   open PRs, five of them failing CI. A merge will present the deletion as a
   conflict; keep the deletion. Dependabot reads that file only from the
   default branch, and security updates and vulnerability alerts are both off,
   so its absence stops every Dependabot PR.
5. Build, then run the movie-start check. Compiling is not passing.
6. Update [design.md](design.md) §8 if the surface moved.

## 7. Traps

Each of these cost real time.

- **Stale binary.** Rebuild before you conclude anything. A UI-policy change
  that "did nothing" was a `target/debug/ruffle_desktop` predating the
  feature; it never loaded the policy file at all. The same applies twice
  over when deploying to the Pi.
- **Stale `target/` after a tree rename.** Build-script artifacts bake in
  `CARGO_MANIFEST_DIR` as an absolute path, and cargo fingerprints do not
  notice a rename, so `asc.jar` gets looked up where it no longer is
  (`Could not find or load main class …ScriptCompiler`). `cargo clean -p`
  does not help — build-script outputs survive it. `rm -rf target/<profile>`
  does.
- **Clicks that silently miss.** Window automation against the desktop
  player needs the window raised first (`xdotool windowraise` before
  `mousemove --window`), or the pointer lands on whatever overlaps it.
- **`pkill -f ruffle_desktop`** also matches the SSH session's own command
  line and kills the session. Use `pkill -x`.
- **Orphaned mpv.** SIGTERM on the player skips destructors, so its mpv
  child survives and keeps playing. Kill mpv too.
- **mpv stalls silently on network loss.** Measured 2026-07-10 against a
  local server that streamed 5 s of MP3 then held the socket open sending
  nothing (a WLAN drop, as TCP sees it): audio stops when the ~5 s demuxer
  cache drains, the process stays alive and *reports playing* for its 60 s
  default `--network-timeout`, then exits. `poll_state` sees nothing until
  the exit. Live stall signals, if ever needed: `paused-for-cache` /
  `core-idle` over the IPC socket. This is the failure mode the backup
  alarm (FR13) exists for.
- **No audio device freezes stream-synced SWFs.** With no usable sound
  card (this dev box: ALSA "cannot find card '0'", so cpal falls back to
  `NullAudioBackend`), any clip carrying a `SoundStreamHead` timeline
  stalls: the null backend reports stream position 0 forever, and the
  player's `audio_skew_time` sync throttles the timeline toward the audio
  clock that never advances. `intro.swf` (narration on every chapter) is
  the case that bit — it froze on its first frame and looked exactly like
  a broken loadMovie (2026-07-11). The panel itself has no stream sounds
  and is unaffected; on hardware with sound it does not happen. Dev-box
  workaround: point ALSA at a real-clock null *device* —
  `ALSA_CONFIG_PATH=<file>` with
  `</usr/share/alsa/alsa.conf>` + `pcm.!default { type null }` +
  `ctl.!default { type null }`.
- **A fixture response with an unparseable URL silently eats loadMovie
  query params.** `SwfMovie::append_parameters_from_url` runs `Url::parse`
  on the *response* URL and drops the whole query on failure — no panic,
  no warn at default log level. The widget cache's scheme-less
  `/tmp/widgetcache/<id>?_chumby_…` requests hit this: widgets loaded and
  rendered, but arrived with no parameters, so 24h mode "didn't persist"
  (2026-07-12, device only — desktop fixture widgets use `file://` hrefs).
  `ChumbyNavigator::fetch` now rewrites scheme-less response URLs to
  `file://…`. If a loaded movie ever ignores its parameters again, check
  the response URL shape first.
- **`AlarmSet.repair()` (F2:11811) force-resets alarm[0]** after every
  parse: `_backup=true`, `_backupDelay=5`, `_duration=default`,
  `_autoDismiss=false` — whatever `/psp/alarms` says. The first alarm
  always carries a 5-minute backup; per-alarm backup settings only hold
  from alarm[1] on. Cost an investigation round on 2026-07-10 (a
  hand-edited `backupDelay="1"` that kept computing as 5 looked exactly
  like an XML-attribute bug in our interpreter; insert/lookup tracing
  proved the parse correct before the decompile gave up `repair()`).

## 8. Documentation

Three documents, this one included; keep it that way. Requirements state
what must be true, design states how and why, development states how to work
on it. A finding that fits none of the three is either obsolete or belongs
in the appliance repo — decide which rather than starting a fourth file.

The `ASnative(5,N)` reference lives in [`README.md`](../README.md), because
it is the thing an outside reader most needs. Keep it honest about
ignorance: indices whose purpose is genuinely unknown (`_csccd`/`_gsccd` at
5,70/71, the nameless 5,72/74) say so.
