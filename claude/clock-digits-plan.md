# Issue 3: the built-in clock draws no digits

Plan and running record for `claude/issues.md` #3 — the FR17 empty-channel
clock (`bi_clock`) showing no digits on the 5" DSI box.

## Step 1 — what the clock is made of, and a desktop A/B (done)

**How `bi_clock` is built** (character 381, read off the SWF tag stream;
`ffdec` script export of frame 2 for the ActionScript):

| depth | instance | character | what it is |
|---|---|---|---|
| 1–4 | — | shapes 350–353 | octopus background, solid fills |
| 5, 7 | `day_high`, `day_low` | sprite 364 | date digit strips, **white** |
| 9 | `monthName` | edit text 366 | embedded font `Garamond Premr Pro It` |
| 10 | `colon` | sprite 368 → shape 367 | single frame, solid `#023168` |
| 12–18 | `minutes_high/low`, `hours_high/low` | sprite 379 | time digit strips, **solid `#023168`** |
| 20 | `ampm` | edit text 380 | initial text `a.m.` |

Both digit strips are 11-frame sprites: **frame 1 is empty**, frames 2–11
carry one `DefineShape` per digit 0–9. `BuiltinClockDigit.prototype.setDigit
= function(d) { this.gotoAndStop(d + 2) }`, so `setDigit(-1)` parks the strip
on the blank frame — that is the panel's leading-zero suppression.
`BuiltinClock.prototype.update` (F2:9868) runs from `onEnterFrame`, sets the
month name first and only then calls `setDigit` on all six strips. The
classes are bound by `MCUtil.LinkWithClass` in each sprite's frame-1
`DoAction`, which assigns `__proto__` and applies the constructor.

**This rules the renderer out on structure alone.** Every digit is a plain
`DefineShape` with one solid fill — the same tag kind and the same colour as
the colon beside them. A backend that draws the colon cannot fail to draw
the digits. Whatever is wrong, it is not a glyph, font or fill gap, so
probe (1) in the issue (`CHUMBY_RENDERER=wgpu`) is not the discriminating
test it looked like.

**Desktop A/B under the device's exact settings.** `ruffle_desktop`
`--renderer tiny-skia --quality low --width 1024 --height 600` against the
repo fixtures (`fixtures/widgets/` is empty, so the run enters clock mode on
its own): the clock renders complete — "August 20", `10:09`, "p.m.".
Renderer, quality and the 1024x600 geometry are therefore all cleared on
x86 with fixtures; the cause lives in something the device has and this run
does not.

## Step 2 — CHECKPOINT: which symptom, exactly

Blank digits and a missing `update()` look identical from three metres away,
but they separate cleanly on what *else* is on the screen, because the
non-digit parts of the clock come from different sources:

| what is on screen | what it means |
|---|---|
| month name, colon and "p.m." present, all six digits blank | `update()` runs; the `setDigit` calls or the strips are the problem |
| colon present, month name **empty**, "p.m." showing | `update()` never runs — `ampm` would be showing its *initial* text, not a computed one |
| digits visibly cycling or flickering | `BuiltinClockDigit` never linked — the strips have no `stop()` of their own and would free-run |

Needed from the device before going further: which of those it is (a photo
settles it), and SSH access to the box so the panel's own trace can be read
— `RUST_LOG=…,avm_trace=info` shows whether `BuiltinClock` is constructed
and what `setDigit` is called with.

## Step 3 — narrow on the device (not started)

## Step 4 — fix and verify (not started)
