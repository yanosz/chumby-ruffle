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

`--repo` is not optional: this checkout's `origin` is a local clone
(`/home/jan/chumby-ruffle`), not GitHub, so a bare `git push origin` or
`gh pr create` targets the wrong place. Push to the GitHub URL explicitly.

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
| `fixture.rs` | `FixtureHost`: natives, exec manifest, HTTP fixtures, virtual rootfs |
| `real_net.rs` | `RealNetHost`: live network state, wraps `FixtureHost` |
| `avm.rs` | the `ASnative(5,N)` dispatch table |
| `navigator.rs` | `exec://`, chumby HTTP, and `file://` interception |
| `ui_policy.rs` | declarative disabling/tinting of panel controls |
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
| `chumby-ctl` | writes bend/click/drag to the player's control FIFO |
| `verify-screens.sh` | drives the panel to named screens and screenshots them |
| `chumby-widget-channel` | regenerates the widget-channel profile fixture |
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

`-Pbuiltin=1` additionally takes the offline boot path (no authorize round
trip). Useful environment:

```sh
RUST_LOG=warn,chumby_host=info          # every host call, args and result
RUST_LOG=warn,avm_trace=trace           # the panel's own trace() output
RUST_LOG=warn,chumby_pick=debug         # what a click actually hit
```

`chumby_pick=debug` is the tool for UI-policy work: a missed click and an
inert control look identical without it.

Drive the panel from a script through the FIFO (`./chumby-ctl bend` is the
same thing). The FIFO must be a real FIFO — `echo >` to a missing path
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
  instead, because we own it. Its `defaultUpdateTime=9999` /
  `defaultProfileTime=9999` profile parameters are the panel's own documented
  way to quiet its polling loops.
- `/home/jan/chumby_backup` — a real Chumby Classic's rootfs (firmware 1.7.2,
  `ironforge`), read-only ground truth for what the environment looked like.

When the wiki, the backup and the decompiled SWF disagree, **the SWF wins.**
The wiki lists the time family under category 103; the SWF binds it at
5,176–178. The wiki numbers a `PlayAudio` at 5,151; the panel calls
`_playAudio` at 5,144.

## 4b. Regenerating the widget channel

`fixtures/http/xml.chumby.com/xml/profiles` is generated, not hand-written.
Each widget carries a `*.widget.xml` sidecar next to its SWF in
`fixtures/widgets/`; `./chumby-widget-channel` enumerates them and emits the
profile plus the two `/tmp/currentProfile*` files. It skips the rewrite when
the sidecar set is unchanged (`--force` overrides). A committed profile
ships in the tree, so a debug run never needs to regenerate first.

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

CI (`.github/workflows/chumby.yml`) builds on every push and PR to `chumby`,
and additionally runs the movie-start check on push and manual dispatch —
never on a PR, because that step needs the SWF (§1). Fixtures are in-repo;
only `controlpanel.swf` is fetched, by rclone from a private share configured
entirely through `RCLONE_CONFIG_RSHARE_*` secrets. The SWF is never
committed, cached, or uploaded anywhere. The tracked fixture tree lacks the
gitignored widget SWFs; the panel boots without them (the widget load fails
with a non-fatal `FetchError`), which is what makes this work.

Inherited upstream workflows are left untouched (they filter on `master`, or
guard on the upstream repository name) so that future upstream merges stay
conflict-free.

## 6. Merging upstream

1. Merge or rebase onto current upstream `master`.
2. For each file in the patch-surface table ([design.md](design.md) §8),
   `grep -n chumby` it and confirm the hook survived.
3. Do **not** re-introduce the `chumby` cargo feature, however tempting the
   conflict resolution looks. Its removal was a decision, not drift.
4. Build, then run the movie-start check. Compiling is not passing.
5. Update [design.md](design.md) §8 if the surface moved.

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

## 8. Documentation

Three documents, this one included; keep it that way. Requirements state
what must be true, design states how and why, development states how to work
on it. A finding that fits none of the three is either obsolete or belongs
in the appliance repo — decide which rather than starting a fourth file.

The `ASnative(5,N)` reference lives in [`README.md`](../README.md), because
it is the thing an outside reader most needs. Keep it honest about
ignorance: indices whose purpose is genuinely unknown (`_csccd`/`_gsccd` at
5,70/71, the nameless 5,72/74) say so.
