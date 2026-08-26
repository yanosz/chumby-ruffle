# The patch surface against upstream

Live record of every upstream file the fork touches. **Supersedes
`claude-docs/design.md` §8**, which is frozen and now out of date: it
predates the tiny-skia renderer and never listed the Cargo files,
`controller.rs`, `.gitignore`, `README.md`, `Cargo.lock` or the workflow.

Why it matters: `chumby` is two commits on upstream master — additions
first, edits second — so this list *is* the conflict surface of every
rebase. The first commit cannot conflict; this one is the whole cost.

Measured 2026-08-07 against `upstream/master` (`8328af42d`): **18
modified, 1 deleted.**

## Registration hooks — the intended shape

Each carries the word `chumby` in a comment, which is what a rebase greps
for. Count in parentheses.

| File | Change | markers |
|---|---|---:|
| `core/src/lib.rs` | `pub mod chumby;` | 1 |
| `core/src/avm1.rs` | `pub use function::FunctionObject;` | 1 |
| `core/src/avm1/globals/asnative.rs` | `5 => chumby::avm::method` match arm | 3 |
| `core/src/avm1/fscommand.rs` | `swallow_fscommand_quit` guard | 2 |
| `core/src/player.rs` | click-target diagnostic in `run_mouse_pick` | 3 |
| `core/Cargo.toml` | `toml`, target-gated `libc` | 3 |
| `desktop/src/player.rs` | `ChumbyNavigator` wrap | 3 |
| `desktop/src/cli.rs` | `--chumby-fixtures`, `--chumby-control`, `--renderer` | 6 |
| `desktop/src/main.rs` | host init, `input::spawn` | 6 |
| `desktop/src/app.rs` | Home→bend, touch arm, control-FIFO drain | 18 |

## What tiny-skia added, and where it breaks the rule

Four paths entered the surface with the CPU renderer. Three are benign;
one is not.

| File | Change | markers |
|---|---|---:|
| `Cargo.toml` | workspace member `render/tiny_skia` | 0 |
| `desktop/Cargo.toml` | `ruffle_render_tiny_skia`, `softbuffer`, `tiny-skia` | 0 |
| `exporter/Cargo.toml` | `ruffle_render_tiny_skia` | 0 |
| `desktop/src/gui/controller.rs` | **+328 / −158** — the software present path | 1 |

**`controller.rs` is a genuine violation** and should be treated as
technical debt, not as the new normal. The rule says upstream files get
*only registration hooks*; this file instead carries the renderer's host
integration — a `Present` enum, `render_software()`, `GuiController::new`
branching on `RendererChoice`, a downcast to `TinySkiaRenderBackend`. Its
one `chumby` mention is prose about the hardware, not a hook marker, so
the grep that is supposed to make a rebase tractable finds nothing. It is
the largest code change in the surface and the most expensive file in any
future upstream merge.

The three Cargo files carry no marker either. Minor — a dependency line is
self-evident to a human — but it is invisible to the same grep.

`render/tiny_skia/` itself is a new top-level crate, so not all chumby
Rust lives in `core/src/chumby/` any more. This breaks the letter of the
rule and serves its purpose: the path is absent from upstream, so it lands
in the additions commit and can never conflict. A justified deviation.

## Non-code edits

| File | Change |
|---|---|
| `README.md` | fork readme |
| `.gitignore` | copyrighted assets, appendix, fixtures |
| `Cargo.lock` | consequence of the dependency additions |
| `.github/workflows/test_extension_dockerfile.yml` | repository guard (a scheduled job ran on the default branch and failed; cron ignores branch filters) |
| `.github/dependabot.yml` | **deleted** — keep it deleted on every rebase |
