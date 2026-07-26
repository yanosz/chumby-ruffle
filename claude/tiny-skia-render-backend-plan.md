# Plan: tiny-skia render backend spike

Branch: `renderer/tiny-skia-spike` (off `dev`).

## Context

CPU load on the Pi is render-bound: `wgpu` has no GPU path on any VideoCore IV
Pi (Zero–3) — no Vulkan driver, and Mesa's `vc4` GLES 2.0 sits below wgpu's
GLES 3.0 floor — so every frame goes through `lavapipe` (software Vulkan).
Current packaged defaults (`CHUMBY_QUALITY=low`, `LP_NUM_THREADS=2`) reach
~11-12 fps on a Pi 3A+ at 2 lavapipe worker threads ×83% CPU each; stock
settings were 2.5 fps at 96% on one thread (full numbers and method:
`chumby-pi/claude-docs/development.md` §6). Hardware acceleration is a closed
door on this hardware, not an open lever.

The working hypothesis: lavapipe's cost isn't "software rendering is slow",
it's "translating every draw call through the full Vulkan API in software is
slow". A direct CPU 2D rasterizer (`tiny-skia`) skips that translation layer
entirely. This spike tests that hypothesis on real content, on real hardware,
before committing to a full backend + live-player integration.

## Non-goals for this spike

Masks / blend-mode compositing, filters (blur/glow/drop-shadow/PixelBender),
offscreen bitmap caching, live windowed (cage) integration. All stubbed as
unsupported, `render/src/backend/null.rs`-style. Revisit only if the spike's
numbers justify building the real thing.

## Steps

### 1. `render/tiny_skia` crate

Modeled on `render/canvas` (the wasm Canvas2D backend), not `render/wgpu`:
convert `DistilledShape` to `tiny_skia::Path` + fill directly, skipping the
lyon tessellator. `register_bitmap` → `tiny_skia::Pixmap`. `submit_frame`
clears then walks the `CommandList` via a `CommandHandler` impl painting into
a `Pixmap`. `Context3D`, filters, PixelBender, `render_offscreen` all stubbed
unsupported.

**Checkpoint**: crate compiles standalone. — **DONE** (see log below).

### 2. Correctness: unit + integration tests

Lock the backend's behaviour down with deterministic, headless tests before any
visual/acceptance pass. Unit tests cover the conversion helpers; integration
tests drive the public API and assert on rendered pixels.

**Checkpoint**: `cargo test -p ruffle_render_tiny_skia` green. — **DONE** (log
below).

### 2b. Visual harness — DEFERRED (acceptance phase)

Fork `exporter` (only its renderer-setup lines change, per the trace done during
planning) with a backend switch, so it produces PNGs from
`opening.swf`/`controlpanel.swf` the same way it does today with wgpu; eyeball
against the wgpu reference for shape/bitmap/gradient correctness; note anything
that visibly needed masks or blends we skipped.

Deferred by Jan (2026-07-26), then built the same day when he said to proceed —
the harness *is* the "something on the screen". Automated tests carry
correctness; this harness also unblocks the measurement in steps 3–4 (real SWF
frames through the backend). — **BUILT; awaiting Jan's review** (log below).

### 3. Desktop timing + CPU sanity gate

Rough pass only — not the real answer, just catches pathological slowness
before spending a cross-build/deploy cycle. Capture both:
- ms/frame over the same `opening.swf` fixture (132 frames) the existing
  offscreen measurements used.
- CPU utilization while rendering (`top -H` per-thread, matching the
  existing methodology), since a backend that's fast but still pegs a core
  isn't the win we're after.

**Checkpoint — report back, decide go/no-go before touching hardware.**

### 4. Cross-build + deploy to the test Pi

Same 3A+ WaveShare box, same methodology as `claude-docs/development.md` §6
so numbers are directly comparable to the existing baseline:

| | fps / ms-per-frame | CPU |
|---|---|---|
| Stock (`high`, 1 thread) | 2.5 fps | 1 thread @ 96% |
| `low` + `LP_NUM_THREADS=2` (shipped default) | ~11 fps | 2 threads @ 83% each |
| Offscreen exporter, 640×480 / 480×320 / 320×240 | 193 / 123 / 92 ms/frame | (not captured at the time) |
| **tiny-skia spike** | *(measure)* | *(measure, `top -H`)* |

Capture fps/ms-per-frame **and** per-thread CPU% for the tiny-skia backend
under the same conditions, so the comparison is apples-to-apples on both
axes — a backend that hits 12 fps at 40% of one core is a very different
result than one that hits 12 fps at 100%.

**Checkpoint — report Pi numbers vs. baseline (both fps and CPU%); decide
whether to invest in the real `submit_frame`/live-player integration or stop
here.**

## Checkpoint log

### Step 1 — crate compiles standalone (DONE)

`render/tiny_skia` (`ruffle_render_tiny_skia`) builds warning-clean; added to
the workspace members. `tiny-skia = "0.11.4"` (already transitive in the lock).
`TinySkiaRenderBackend { frame: Pixmap, dimensions }` with a `frame()`
read-back accessor (the trait has no present step; the harness reads pixels out
in Step 2).

Implemented for real:
- `register_shape`: `DrawPath` → `tiny_skia::Path` via `PathBuilder`, no lyon.
  Solid + linear + radial gradient fills (native tiny-skia shaders). Strokes
  reuse the line's `fill_style` shader.
- `register_bitmap`/`update_texture`/`create_empty_texture` → `Pixmap`
  (`RefCell` so `update_texture` can mutate a shared handle).
- `submit_frame`: clear + `CommandList::execute(self)`.
- `render_bitmap`, `draw_rect`/`draw_line`/`draw_line_rect`.

Coordinate convention mirrors `canvas`: paths/matrices kept in twips, a 1/20
scale folded into the paint transform (`TWIPS_TO_PIXELS`). tiny-skia
premultiplied storage matches `BitmapFormat::Rgba` (premultiplied), fed
directly.

Deliberate gaps to eyeball in Step 2 (all defensible for a spike, flagged so
the PNG diff isn't a surprise):
- **Bitmap *fills*** (`FillStyle::Bitmap`) render flat grey — photographic
  content still goes through `render_bitmap`, which is real.
- **Focal gradients** rendered as plain radial (focal offset dropped).
- **Masks / blend modes** are no-ops: masked/blended content draws unclipped
  and un-blended (over-draw), not skipped, so cost is still counted.
- **Color transforms** ignored (no tint/alpha fade on shapes/bitmaps yet).
- Offscreen, filters, PixelBender, Context3D → `Unimplemented`/`None`.

Open risk for the harness (2b, not the tests): handles use non-`Send`/`Sync`
`Arc`/`RefCell`; fine for the crate and a single-threaded harness, may need
revisiting if the exporter path demands `Send`.

### Step 2 — unit + integration tests (DONE)

`cargo test -p ruffle_render_tiny_skia`: 5 unit + 7 integration, all green.
Colours are opaque so premultiplied == straight and raw `data()` bytes compare
directly.

- Unit (`src/lib.rs`): channel mapping (`sk_color`), fill-rule and spread
  mapping, the twips+scale fold in `sk_transform`, `build_path` some/none.
- Integration (`tests/render.rs`): clear fills the frame; a solid fill covers
  its region and nothing outside; a translate repositions the shape; a linear
  gradient runs cyan→magenta across its bar (exercises the gradient
  local-matrix ∘ fill-transform composition); a 2×2 bitmap renders by quadrant
  with nearest sampling; `update_texture` replaces pixels; `name`/viewport
  realloc/`create_context3d`-errors metadata.

The `examples/demo.rs` smoke test stays as a quick eyeball; the real visual
pass is 2b.

### Step 2b — exporter harness + first eyeball (BUILT, awaiting review)

`exporter/src/bin/tiny_skia_export.rs`: a standalone binary (not the
wgpu-bound `Exporter` — no GPU adapter needed) that builds a `Player` with
`TinySkiaRenderBackend`, advances frames, and reads pixels straight from the
backend's `frame()` Pixmap to PNG. `with_renderer` has no `Send` bound, so the
`RefCell` handles integrate fine — the Send risk above did not bite.
Fixtures copied read-only from `/home/jan/chumby_backup/usr/widgets/`.

First eyeball vs. the wgpu (lavapipe) reference, both 320×240:
- **opening.swf** (~frame 30): faithful — octopus, "chumby" wordmark, AA all
  match. The tiny ™ superscript is the only near-invisible miss.
- **controlpanel.swf** (frame 7, boot "Initializing…"): header, octopus glyph
  and segmented spinner all correct, **but** the large faint octopus watermark
  that wgpu shows behind the spinner is **missing** — instead the whole ground
  reads neutral grey.

Root cause — **bisected**, not guessed (initial color-transform hypothesis was
wrong):
- Implemented colour-transform support (fills: full `ctx`; bitmaps:
  `a_multiply` → opacity) and re-rendered — octopus **still missing**. So it is
  *not* colour transform.
- Probe: bitmap-fills tinted magenta → nothing appears. Not a bitmap fill.
- Probe: mask *shapes* tinted green → nothing appears. Not the masker.
- Numeric diff (my frame vs wgpu ref): octopus region is flat background grey
  (204,204,204) here vs faint blue (206,221,239) in wgpu; mean abs diff ~60.
- The panel emits **2 masks/frame**. Conclusion: the octopus is the **maskee**
  (a light-blue fill clipped to an octopus-shaped mask). Our no-op masks draw
  the maskee **unclipped**, and later opaque draws cover it → it vanishes.

Takeaway: the CPU backend renders most panel content correctly; the visible gap
is **mask clipping** (a spike non-goal), *not* colour transform. Real masks
(tiny-skia has `Mask`) are the fix — bigger than the colour-transform change.
The colour-transform work is still correct and useful (tints/alpha fades on
shapes and bitmaps) and stays. Harness output lives in the session scratchpad
(not committed).

**Decision (Jan, 2026-07-26): skip masks for now.** Accept the cosmetic octopus
gap and proceed to Step 3 (measurement) — the spike is about CPU load, and
fidelity is already enough to measure representative work. Masks (and bitmap
*fills*) are the top fidelity items **only if** the numbers justify turning this
into a real backend; not worth building on a spike that might not proceed.

### Step 3 — desktop timing + CPU sanity gate (DONE)

`tiny_skia_export` now times `render()` per frame (`out_dir` of `-` = time only,
no PNGs). Release build, desktop **i3-1315U**, single-threaded:

| Content | Size | mean ms/frame | ≈ fps | CPU |
|---|---|---|---|---|
| opening.swf (132 fr) | 320×240 | 0.15–0.17 | ~6500 | 97% of 1 core |
| controlpanel.swf (100 fr) | 320×240 | ~0.40 | ~2500 | 98% of 1 core |
| opening.swf (132 fr) | 640×480 | ~0.29 | ~3400 | — |

The panel's real load (37 shapes + 2 masks/frame) rasterises in ~0.4 ms on
desktop — no pathological slowness; the sanity gate passes decisively. tiny-skia
is single-threaded, so it pegs one core while busy (expected).

Caveat: desktop x86 ≫ Pi VideoCore-era ARM — these absolute numbers do **not**
transfer. Their only job is the go/no-go gate. **Verdict: GO to Step 4** — the
real apples-to-apples comparison is on the 3A+ vs the lavapipe baseline
(~11–12 fps live; 92 ms/frame offscreen at 320×240). Awaiting Jan's ok to touch
hardware.

### Step 4 — on-device measurement (DONE)

Box: **192.168.42.51** (Pi **3B+**, quad A53, aarch64 Debian 13) — not the 3A+
the baseline used, but the same A53 SoC/clock, so comparable. `dist` cross-build
(`aarch64-unknown-linux-gnu`, reusing the project's existing exporter
cross-build). `chumby-player` stopped for the run, restarted after.

tiny-skia `render()`, offscreen, **single-threaded**:

| Content | Size | ms/frame | ≈ fps |
|---|---|---|---|
| opening.swf | 320×240 | ~1.5 | ~680 |
| **controlpanel.swf** (37 shapes + 2 masks) | 320×240 | **~3.95** | ~250 |

500-frame panel run: user 2.79 s / wall 2.87 s ⇒ **~1 core** (0.97), confirming
single-threaded.

**vs. baseline** (wgpu/lavapipe, `development.md` §6): offscreen exporter
92 ms/frame at 320×240; live shipped ~11–12 fps on **2** lavapipe threads @ 83%.

⇒ tiny-skia renders the panel offscreen at **~4 ms/frame vs lavapipe's ~92** —
roughly **20× cheaper — on one core instead of two.**

Honest caveats: (a) tiny-skia skips masks/proper bitmap-fills, so it does
slightly *less* work — but those are cheap clips, nowhere near a 20× gap; (b)
3B+ vs 3A+ (same SoC); (c) `render()`-only timing isolates rasterisation, which
was the bottleneck; (d) offscreen, not the live cage — live-player integration
is exactly what the decision below is about.

**Verdict: strong GO.** The spike's hypothesis holds decisively — bypassing the
Vulkan-through-software translation is ~20× cheaper on this hardware and frees a
core. Building the real `submit_frame` + live-player (cage) integration — and
then masks/bitmap-fills/color-transform for fidelity — is well justified.
**Awaiting Jan's decision to proceed past the spike.**
