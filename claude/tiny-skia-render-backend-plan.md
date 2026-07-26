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

### 2. Prototype harness

Fork `exporter` (only its renderer-setup lines change, per the trace done
during planning) with a backend switch, so it produces PNGs from
`opening.swf`/`controlpanel.swf` the same way it does today with wgpu.

**Checkpoint**: desktop run, eyeball PNGs against the wgpu reference for
shape/bitmap/gradient correctness; note anything that visibly needed masks or
blends we skipped.

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

Open risk for Step 2 integration (not Step 1): handles use non-`Send`/`Sync`
`Arc`/`RefCell`; fine for the crate and a single-threaded harness, may need
revisiting if the exporter path demands `Send`.
