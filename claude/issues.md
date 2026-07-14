# Open issues

One block per issue. Deleted when closed — git remembers.

---

Number: 1
Timestamp: 2026-07-14, 23:00
Title: Rendering regression in upstream.
Status: open
Description: After rebasing to upstream (8328af42d / 2026-07-12), the keyboard
in the SHOUTcast search could no longer be used; the control panel feels
sluggish. Nothing in the 59 commits since 7f62f5dbf touches the renderer or
AVM1, so no single upstream commit is to blame — llvmpipe on one thread has no
headroom left and a slightly heavier binary tips it over. Not reportable
upstream. Workaround: upstream rebased to 7f62f5dbf / 2026-07-06, shipped as
0.9.2. Recheck the keyboard on every future rebase.
