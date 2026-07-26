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
Status: open — observed at the screen; both leading hypotheses measured and
refuted; cause not yet identified
Description: Jan compared the two backends on the 3B+ / ILI9486 SPI TFT and
reports three distinct looks. This is a fidelity gap the spike's own metric
missed: it scored 93-98.8 % of pixels within 16 levels and called that good,
but a one-pixel fringe around every glyph is a tiny fraction of pixels and
extremely visible.

Observed (Jan, at the screen):
- **tiny-skia** — "super-high contrast, with a small bright border around every
  symbol", plus "some kind of frame / outline on the right hand side of the
  spaceclock". His words: "a bit like anti-aliasing in the wrong color".
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
