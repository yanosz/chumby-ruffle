# Open issues

Known defects in the player, one section each. An issue leaves this file
when it is fixed or when it is shown not to exist. Closed issues are
deleted, not archived — git remembers.

---

## I1 — Text input too slow on newer upstream: we are out of render headroom

**Symptom.** In the SHOUTcast station search, typing on the on-screen keyboard
is unusable — the field does not keep up with the keypresses. Built on an
older upstream base, the same keyboard is responsive.

**Observed** 2026-07-14 on the development Pi by hot-swapping the player binary
under the installed 0.9.1 package. Same chumby patchset, same `dist` profile,
same device; only the upstream base differs.

| upstream base | date | keyboard |
| --- | --- | --- |
| `7f62f5dbf` (base of `chumby-old`) | 2026-07-06 | responsive |
| `8328af42d` (base of `chumby`) | 2026-07-12 | too slow to use |

**This is probably not an upstream regression, and must not be reported as
one.** We render with **llvmpipe** — software rasterisation through Vulkan —
pinned to a single thread (`LP_NUM_THREADS=1`, set in `chumby-player-run` to
halve CPU), onto a slow SPI TFT. That is a deliberately marginal configuration.
At the edge of the envelope, *any* slightly heavier binary tips over: worse
inlining under LTO, a bigger text section, more instruction-cache pressure.
Upstream owes us nothing here.

The commit archaeology supports that reading. Across the 59 commits in
`7f62f5dbf..8328af42d` **nothing touches the renderer or AVM1 rendering**:

- The sole `display_object/edit_text.rs` change (`e7010c9bb`) only drops an
  `#[allow(clippy::collapsible_else_if)]`. A no-op.
- The AVM1 changes (`f1a85e9ae`, `d9d0b5dbd`) move AMF serialisation out of
  `shared_object.rs`. Not rendering.
- Everything else is AVM2 (the panel is AVM1), tests, or CI.
- Dependency bumps: `bytemuck` (patch), `regex`, `crossbeam-channel`, `pest`,
  `humantime`, `gloo-net` (web only). **No `wgpu`, no `naga`.**

So there is no culprit commit to find. The likeliest story is simply *more
code*, costing a little more per frame in a configuration with no margin left.

**The A/B is also not yet clean.** The slow binary was the one `dpkg` had
installed for 0.9.1, whose build provenance was never verified; the fast one
was freshly built here. The comparison therefore varies two things — upstream
base *and* build — not one. Rebuild `chumby` (base `8328af42d`) with the
identical command and swap that in before trusting the table above.

**Mitigation in hand.** `chumby`'s two commits replay onto `7f62f5dbf` with no
conflicts and no source edits, giving a player that renders the keyboard
acceptably. Pinning there buys time; it does not buy headroom.

**The real fix is headroom, not archaeology.** Candidate levers, cheapest first:

1. `LP_NUM_THREADS` — currently 1 on a 4-core Pi. Raising it trades CPU for
   raster throughput. A config change, no rebuild.
2. Hardware rasterisation instead of llvmpipe (V3D/GLES via wgpu's GL backend)
   — potentially a large win, unexplored.

**Aside, not a cause.** Bases older than 2026-06-13 will not compile: upstream
`17e04296f` relaxed `Object::set` to take `impl Into<Value>` one day before the
`v0.3.0` tag was cut, and our code relies on it. Backporting past that point
costs six explicit `.into()`s.
