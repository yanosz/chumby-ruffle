# Development

How to build, run, verify, and evolve this fork. Deployment of the finished
player onto hardware is not covered here — that belongs to
[chumby-pi](https://github.com/yanosz/chumby-pi), which pins this repo as a
submodule and owns the packaging, the kiosk, and the fixture data.

Concepts: [design.md](design.md). What the player owes the panel:
[requirements.md](requirements.md). The public overview and the
`ASnative(5,N)` reference: [`README.md`](../README.md).

---

## 1. Branch and commit policy

**One feature branch per working session, squashed on merge.** Branch from
the fork's default branch (`chumby`), do the session's work in as many
commits as are useful, and squash when merging back. Do not amend and
force-push a long-lived shared commit — that was the old discipline and it
is retired.

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
kept *outside* this repo so the fork stays upstream-clean:

```sh
cargo build --profile dist -p ruffle_desktop --target aarch64-unknown-linux-gnu
```

Upstream Ruffle needs a JVM at build time (it compiles its ActionScript
stdlib with Adobe's `asc.jar`), on the build host only.

## 4. Run

The player needs the panel SWF and a fixtures directory, neither of which
lives here (both are chumby-pi's). From a chumby-pi checkout,
`run-controlpanel.sh` wraps this:

```sh
ruffle_desktop \
    --load-behavior blocking \        # mandatory, see requirements.md FR7
    --filesystem-access-mode allow \
    --chumby-fixtures fixtures/ \
    --chumby-control /tmp/chumby-ctl \
    -PlocalCache=1 \
    controlpanel.swf
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

Drive the panel from a script through the FIFO:

```sh
echo bend            > /tmp/chumby-ctl   # summon/dismiss the button bar
echo "click 448 458" > /tmp/chumby-ctl
echo "drag 100 200 300 200" > /tmp/chumby-ctl
```

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

CI (`.github/workflows/chumby.yml`) runs build + movie-start on every push
and PR to `chumby`. It fetches `controlpanel.swf` and a fixtures tarball
from a private share — the SWF is copyrighted and is never committed,
cached, or uploaded anywhere. Inherited upstream workflows are left
untouched (they filter on `master`, or guard on the upstream repository
name) so that future upstream merges stay conflict-free.

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
