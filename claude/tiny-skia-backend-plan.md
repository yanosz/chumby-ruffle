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
