# tiny-skia render backend — forward plan (real backend)

Successor to `tiny-skia-render-backend-plan.md`, which is the **frozen spike
record** (do not extend it). The spike concluded 2026-07-26 with a strong GO:
`ruffle_render_tiny_skia` rasterises `controlpanel.swf` offscreen on a Pi 3B+ at
**~3.95 ms/frame single-threaded vs wgpu/lavapipe's ~92 ms on two threads** at
320×240 — ~20× cheaper on one core instead of two.

This document tracks turning that crate into a shipped render path.

## Scope decisions (Jan, 2026-07-26)

1. **Order: live cage integration first.** Get tiny-skia driving the real panel
   on the device with the spike's known fidelity gaps (no masks → the faint
   octopus watermark is missing; bitmap *fills* flat grey; focal gradients drawn
   as plain radial). Fidelity work follows once the live number is confirmed.
2. **Selection: both backends stay, tiny-skia is the appliance default.**
   Selectable at runtime so we can A/B on the device and fall back if fidelity
   regresses. Desktop development keeps wgpu (it carries the egui GUI).
3. **Branch: continue on `renderer/tiny-skia-spike`** (off `dev`), on top of
   `487620ec7`. The name no longer describes the work; history stays linear and
   the chumby-pi `dev` gitlink keeps tracking one branch.

## The integration problem

Swapping the `RenderBackend` is *not* sufficient, because the present path is
wgpu all the way to the screen:

- `desktop/src/player.rs:274` builds `WgpuRenderBackend::new(descriptors,
  movie_view)` — the movie renders into a `MovieView`, a wgpu texture.
- `desktop/src/gui/controller.rs:406` downcasts the live renderer to
  `WgpuRenderBackend<MovieView>` (`expect("Renderer must be correct type")`) and
  composites that texture inside the **egui** render pass onto the wgpu surface.

`TinySkiaRenderBackend` instead owns a CPU `Pixmap`, exposed as `frame()`.
Nothing in the frontend can present that today. Two candidate paths:

- **(A) softbuffer** — present the pixmap straight to the winit window through
  `wl_shm`. No Vulkan initialised at all in tiny-skia mode, no egui overlay in
  that mode. This is where the spike's 20× actually lands.
- **(B) wgpu blit** — keep the surface and egui, upload the pixmap as a texture
  each frame. Much smaller change, GUI survives, but lavapipe stays loaded and
  every frame still crosses a software-Vulkan upload + blit + egui pass, eating
  an unknown slice of the win.

Recommendation: **(A)** for the appliance; (B) only as a fallback if a CPU
present path under cage misbehaves. Step 1 decides this with a measurement
rather than an argument.

Second lever, new to the live path: **display resolution, and there is no single
target display.** Two panels exist on the two test boxes (chumby-pi
`claude-docs/development.md` §1, `design.md`):

- ILI9486 **480×320** SPI TFT on the 3B+ (192.168.42.51), pinned by its
  `by-path` DRM name in the launcher;
- Waveshare 3.5" HDMI LCD (E), EDID `WS-35-640`, **640×480@75** on the 3A+
  (192.168.210.159).

Both run through cage, so the present path is the same; the pixel counts and the
SPI upload cost are not. `set_viewport_dimensions` reallocates the pixmap to the
viewport, so by default the panel rasterises at the display's resolution rather
than the movie's 320×240 — and the spike's 3.95 ms was measured at 320×240. The
lavapipe resolution curve in §6 (640×480 193 ms, 480×320 123, 320×240 92) says
that costs roughly 2×, not the naive 4×, because fixed per-frame costs don't
shrink. Rendering at the movie's native 320×240 and letting the present path or
the compositor upscale is the "render at stage size" open lever from that same
section, now cheap to try. Steps 3–4 measure both modes, on both panels.

## Steps

Each step ends with this document updated. STOP at every CHECKPOINT.

### Step 1 — Prove the present path

Standalone probe (example binary or a throwaway on the branch, not the player):
winit window under cage on the Pi, blitting a pixmap at stage size (320×240) and
at panel size via softbuffer. Measure CPU% and achieved commits/s the way §6
counted fps (`DRM_IOCTL_MODE_ATOMIC` commits). Do this on both boxes — the SPI
TFT (480×320, transfer-bound over SPI) and the HDMI panel (640×480) can differ
here even though the mechanism is identical. Compare against the cost of option
(B)'s upload+blit if (A) disappoints.

Deliverable: a number for "what does presenting cost, before any rendering",
and a decision between A and B.
**CHECKPOINT 1** — present path confirmed with Jan.

#### Step 1 results (2026-07-26, DONE)

Probe: `render/tiny_skia/examples/present_probe.rs` — winit + softbuffer, no
Vulkan anywhere. Modes `direct` (paint at window size, copy out), `upscale`
(paint at the 320×240 stage, scale up via `draw_pixmap`), `zerocopy` (paint
straight into the `wl_shm` buffer; R/B swapped, so a cost probe only, not a
usable path). Paced by an explicit sleep, not `ControlFlow::WaitUntil` —
buffer-completion events cancel the wait and the loop runs flat out.

Box: 3B+ `192.168.42.51`, **480×320 SPI TFT** (`platform-3f204000.spi-cs-0-card`),
cage with `WLR_RENDERER=pixman` — the compositor composites in software here, so
its share is CPU work. The 640×480 HDMI box was offline; that half is deferred,
not dropped. Idle CPU floor measured first: **0.3 % of a core**, so the
"elsewhere" column is clean. `dist` cross-build, `aarch64-unknown-linux-gnu`.

Because handing over a `wl_shm` buffer is asynchronous, per-frame `present()`
timing sees almost nothing (~0.07 ms); the probe therefore accounts CPU from
`/proc` — its own utime+stime versus the whole box's busy time, the difference
being cage plus kernel.

At **12 fps** (the panel's ceiling), 480×320, clock pinned to 1.4 GHz:

| mode | own CPU ms/frame | whole box ms/frame | elsewhere (cage+kernel) |
|---|---|---|---|
| **direct** | 1.17 | **5.75–6.17** | 4.6–5.0 |
| zerocopy | 0.58 | 5.58 | 5.0 |
| upscale, nearest † | 11.50 | 17.58 | 6.1 |
| upscale, bilinear † | 22.75 | 27.50 | 4.8 |

† measured under `ondemand`, which parks at the 600 MHz floor at these loads —
pessimistic by ~2×, and still an order of magnitude worse than `direct`.

`direct` phase split at pinned clock: paint 0.364 ms, RGBA→X8R8G8B8 copy
0.567 ms, present 0.074 ms.

**Verdict: path (A) softbuffer, confirmed.** Presenting a full frame costs the
whole box ~6 ms CPU per frame at 12 fps — **~7 % of one core**, against the
~166 % (2 lavapipe threads @ 83 %) the shipped wgpu path burns for the same rate.
Presenting is not where the budget goes, and no Vulkan is initialised at all.
Option (B) needs no measurement to be rejected.

Three further findings:

- **The upscale lever is dead in this form.** tiny-skia's `draw_pixmap`
  scaling costs 11 ms/frame (nearest) to 22 ms (bilinear) — ~30× the 0.36 ms of
  simply painting at panel size. "Render at stage size and upscale" helped
  lavapipe because *rasterisation* scaled with pixels; here the upscale blit
  itself dominates. If scene rasterisation at panel size turns out expensive in
  step 3, the variant worth measuring is a hand-rolled nearest-neighbour
  replication folded into the copy loop, not `draw_pixmap`.
- **The copy is affordable.** Removing it entirely (zerocopy) saves 0.58 ms/frame
  — 0.7 % of a core. Not worth contorting the pixel format for; keep the honest
  conversion.
- **Presents are dropped, not queued.** Unthrottled, the probe pushed 957
  presents/s while the SPI panel can physically show ~12; softbuffer does not
  block. A frame cap in the player therefore remains a real lever (§6 open
  levers), since nothing upstream throttles the loop.

Two device facts worth keeping (both now in chumby-pi `claude-docs/development.md`
§7): **cage cannot start from a plain ssh session** — libseat finds no VT
(`Could not open target tty`), the DRM backend times out, and the hung cage keeps
ssh's stdout open so the ssh call itself never returns; run it from a transient
systemd unit that copies the service's `PAMName=login` + `TTYPath=/dev/tty1`.
And the SPI panel shows **row-banded motion** (Jan, observing the probe): tinydrm
shifts a frame out progressively, so during a transfer the top rows already carry
the new frame. Not a renderer artifact — the wgpu path uses the same transfer.

**CHECKPOINT 1 answered (Jan, 2026-07-26):** path **(A) softbuffer** confirmed —
no option (B), so tiny-skia mode has no egui overlay and wgpu stays selectable
for desktop work. Proceed to step 2 now; the 640×480 HDMI figure is folded into
step 4's on-device A/B instead of blocking integration.

### Step 2 — Renderer selection seam

- `--renderer wgpu|tiny-skia` on the player CLI. No silent fallback: an
  unparseable or unavailable value is an error, not a quiet downgrade.
- Split the frontend run path so tiny-skia mode does not require the
  egui/wgpu compositor (removes the `controller.rs:406` downcast in favour of
  an explicit renderer enum).
- The wgpu path must stay behaviour-identical for desktop development.

Verification: existing test suite plus `verify-screens.sh` on the wgpu path,
proving no regression; `--renderer tiny-skia` reaching the new present path.
**CHECKPOINT 2**.

#### Step 2 results (2026-07-26, DONE)

`--renderer wgpu|tiny-skia` (`cli.rs`, `RendererChoice`); an unknown value is a
clap error, never a fallback. Default stays `wgpu`, so upstream behaviour is
untouched and the appliance opts in from its conffile (step 4).

The seam sits inside `GuiController`, which now owns a `Present` enum instead of
wgpu fields: `Present::Wgpu` (descriptors, egui, surface, movie-view renderer,
theme controller — the desktop path, unchanged) and `Present::Software`
(a softbuffer surface). `render()` dispatches to `render_wgpu` (the old body) or
`render_software`, which downcasts the player's renderer to
`TinySkiaRenderBackend`, converts its pixmap RGBA→X8R8G8B8 into the shm buffer
and presents. `GuiController::descriptors()` became `Option` — nothing builds a
wgpu device in software mode. `player.rs` gained `RenderTarget::{Wgpu(MovieView),
Software{width,height}}` and builds the backend from it via
`with_boxed_renderer`. Choosing the seam here rather than a second frontend keeps
one event path and one frame path; `app.rs` needed only two lines.

One real bug found while verifying: `needs_render()` compared against
`repaint_after`, which only egui ever sets, so in software mode it was always
true and the player redrew every loop iteration — 455 fps at 12 fps of content.
With no GUI, nothing but the movie's own cadence may request a redraw.

**Verified:** all 13 tiny-skia tests pass; the wgpu path renders the panel
exactly as before (menu bar, clock, artwork); `--renderer tiny-skia` reaches the
screen and rasterises real content at the right geometry and scale (viewport
640×504, letterboxed stage, correct gradients).

**But the panel is not usable in tiny-skia mode yet, and the cause is masks.**
Instrumenting the backend (temporary `SpikeStats`, `CHUMBY_TS_STATS=1`) shows what
the clock screen submits per frame: **28 shapes, 6 rects, 4 masks, and no
bitmaps, blends or `render_offscreen`** — so the offscreen/`cacheAsBitmap`
question from step 3 is answered: the panel does not need it.

With the four mask calls as no-ops, ruffle's mask geometry — which it brackets
around the maskee, and which wgpu turns into stencil build/clear — was being
*drawn as content*, painting a flat grey (204,204,204) over the whole panel.
Fixed halfway: a `mask_depth` counter now drops draws submitted as mask geometry
instead of painting them, which is strictly closer to correct and made the screen
appear. What remains is the other half of the same gap: the octopus watermark is
a *maskee*, so unclipped it covers the entire stage and paints over the clock
digits and the date. Screenshot comparison: wgpu shows "July 26 / 5:46 / p.m."
over the artwork; tiny-skia shows the watermark alone.

**Consequence for the plan's order.** "Integration first" rested on the spike's
fidelity gaps being cosmetic. For masks that premise is false on the real panel:
either mask geometry overpaints the screen or the maskee does. Step 5a (real
clipping via tiny-skia's `Mask`) is therefore a **prerequisite for steps 3–4**,
not later polish — an on-device A/B against wgpu has nothing meaningful to
compare until the panel draws correctly. Bitmap fills (5b) remain genuinely
cosmetic here: the clock screen submits no bitmaps at all.

### Step 5a — Masks, promoted ahead of steps 3–4 (2026-07-26, DONE)

**Jan, 2026-07-26:** masks first, then the device — an A/B on a knowingly wrong
picture cannot be judged.

Real clipping now: a `MaskStack` of levels, each `Building` (geometry submitted
now *defines* this mask) or `Active` (it clips every draw), rasterising into
`tiny_skia::Mask` and passed as the clip argument of every `fill_path`,
`stroke_path` and `draw_pixmap`. Nested masks intersect with the enclosing one
(tiny-skia can intersect a mask with a *path* but not with another mask, so that
multiply is ours). Retired masks are pooled — a mask is `w*h` bytes and the panel
builds four per frame.

Two bugs, one of them older than this step:

- **A mask can be pushed and popped without ever being activated.** The first
  cut tracked a single depth counter and popped an *active* clip on every
  `pop_mask`, so an unactivated mask tore down the enclosing clip. Hence
  per-level state rather than a counter.
- **`draw_rect`/`draw_line`/`draw_line_rect` were 20× too small — a spike bug,
  not a mask bug.** They scale a *unit* square, and `Matrix::create_box` (core's
  `create_box_from_rectangle`) supplies the size in the linear part **in pixels**
  while the translation stays in twips. The spike scaled all six components by
  `TWIPS_TO_PIXELS`, as twips-space shape paths require. Invisible until masks
  worked: a text field with a `scrollRect` is masked exactly this way, so the
  masks for "July" and "p.m." collapsed to 9×2 px and 30×6 px slivers and clipped
  the text away entirely. Fixed with `sk_transform_unit`, which converts only the
  translation.

Finding them took measurement, not reading: dumping the player's own pixmap
proved the frontend copy was faithful, per-mask coverage logging showed masks of
18, 210 and 0 covered pixels, and dumping the masks as PNGs put the "July" mask
at (24,0)–(32,1) — 20× short of the text field it should have covered.

**Verified on the desktop against wgpu, same window geometry, same minute:** the
clock screen now matches — identical bounding boxes for the white text
(24,0)–(604,466) and the navy digits (158,132)–(536,372), and **99.7 % of pixels
within 16 levels**. 16 tests pass, three of them new: the unit-geometry scale,
the pop-without-activate case, and the state machine.

Still open by design: bitmap fills (5b) and focal gradients (5c) — the clock
screen submits neither; `render_alpha_mask` still draws its maskee unclipped
(the panel emits none); filters, PixelBender, Context3D and `render_offscreen`
stay unimplemented, which step 2's instrumentation showed the panel never needs.

### Step 3 — Live panel on the desktop, then the resolution lever

Run the real `controlpanel.swf` fullscreen with `--renderer tiny-skia`: touch
and input unchanged (`desktop/src/app.rs` handles winit events, independent of
the renderer), audio unchanged. Confirm the known gaps are exactly the spike's
list and nothing new appeared. Then compare rasterise-at-viewport against
rasterise-at-stage (320×240 + upscale on present) — CPU and fidelity both.

Open verification item: whether the panel ever needs `render_offscreen`
(`cacheAsBitmap` / `BitmapCacheEntry`), which the spike stubs to `None`.
**CHECKPOINT 3**.

### Step 4 — On device: A/B against wgpu

Deploy with chumby-pi `pkg/deploy-pi.sh`; expose the choice as
`CHUMBY_RENDERER` in `/etc/default/chumby-player`, shipped as an **active line**
set to tiny-skia (same pattern as `CHUMBY_QUALITY` / `LP_NUM_THREADS`, NFR4 —
the appliance default lives in the conffile, never as a hidden constant in the
launcher or the player). Measure live fps, CPU per thread and RSS for both
renderers on **both** boxes — the 3B+ with the 480×320 SPI TFT and the 3A+ with
the 640×480 HDMI (E) — against the shipped ~11–12 fps / 2 lavapipe threads @ 83%
baseline (which was taken on the 3A+/HDMI box).

This is the step that either confirms or refutes the spike's promise in the real
cage. **CHECKPOINT 4** — including whether tiny-skia becomes the shipped
default.

### Step 5 — Fidelity

Only after the live number holds, in this order:

- **5a masks** — real clipping via tiny-skia's `Mask`; replaces the four no-op
  `push_mask`/`activate_mask`/`deactivate_mask`/`pop_mask` and the
  `render_alpha_mask` passthrough. Brings back the octopus watermark (bisected
  during the spike as a maskee).
- **5b bitmap fills** — currently flat grey; proper bitmap-shader fills.
- **5c focal gradients** — drawn as plain radial today.

Filters, PixelBender, Context3D and `render_offscreen` stay unimplemented
unless step 3 shows the panel needs them. **CHECKPOINT 5** after each substep.

### Step 6 — Records and packaging

Fork side: this document plus the fork's `claude-docs`/`claude` record for the
new renderer flag and the measured numbers. Appliance side (chumby-pi
`claude-docs/`): the `CHUMBY_RENDERER` conffile knob, the device measurements,
and what the win means for the `LP_NUM_THREADS` / `CHUMBY_QUALITY` defaults —
if lavapipe is no longer in the frame on the appliance, both may become dead
knobs there.

## Risks

- **CPU present under cage** — wlroots supports `wl_shm`, but this is the one
  genuinely unproven mechanism; step 1 exists for it.
- **Losing the egui GUI in tiny-skia mode** — acceptable in kiosk, which is why
  wgpu stays selectable for desktop work.
- **Fidelity regressions users see before step 5** — the missing watermark is
  cosmetic and already accepted for the spike; anything beyond the known list
  found in step 3 gates the default flip.
- **Blend modes** — `blend()` draws through without applying the mode; unchanged
  from the spike, unquantified for the panel.
