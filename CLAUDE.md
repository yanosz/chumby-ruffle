# Project: chumby-ruffle — a Ruffle fork that runs the Chumby control panel

This repository is self-contained for player work: the fork, the fixtures
the panel is answered with, the decompiled panel, and the run harness are
all here. The appliance around it — Debian packaging, the Raspberry Pi
kiosk, the setup howto — lives in the **chumby-pi** repo and is not your
concern when working here.

Read `claude-docs/` before doing anything:

- `claude-docs/requirements.md` — what `controlpanel.swf` demands of a
  player, the non-negotiables, and §3 **Known gaps**: the open work. (The
  project-wide roadmap, spanning both repos, is chumby-pi's `ROADMAP.md`.
  It was compressed from a long plan and is a map, not evidence: when it
  and the code disagree, the code wins.)
- `claude-docs/design.md` — the host boundary, the interception points,
  and §8 the exact patch surface against upstream Ruffle.
- `claude-docs/development.md` — build, run, verify, merge upstream, and
  the traps that have already cost time.

Non-negotiable rules:

- **Never modify `controlpanel.swf` or any extracted SWF.** Control is
  exerted only from the Rust side: what natives return, what commands
  print, what files contain, what properties display objects carry.
- **`/home/jan/chumby_backup` is read-only ground truth. Never write there.**
- **The decompiled panel is the law.** `claude-docs/appendix/` holds the
  ffdec export (gitignored, ~36 MB). When the wiki, the backup and the
  export disagree, the export wins. Do not assert what a screen or a
  sprite does without reading it there — a doc saying a thing was done is
  not evidence that it was done.
- **Acceptance is "the movie runs", never "it compiles."** An upstream
  merge can compile clean and leave the ASnative hooks dead. See
  `claude-docs/development.md` §5.
- **Rebuild before concluding anything.** A stale binary has already
  produced a false "this feature doesn't work" twice.
- All chumby Rust stays in `core/src/chumby/`; upstream files get only
  registration hooks, each carrying the word `chumby` in a comment.
- Reimplement device touchpoints in Rust, not by shipping shell scripts or
  shelling out. A principle, not a hard constraint — deviate with a reason,
  and say what it is.
- Keep code comments brief. Comment only what the code cannot say: the
  why, a non-obvious constraint, a reference. Do not narrate the obvious.
- One feature branch per working session, squashed on merge.
- Ask before iterating. When scope is ambiguous, one clarifying question
  beats an exploratory detour. A question from the user is a question, not
  an instruction to start coding.
- Nothing copyrighted enters git: `controlpanel.swf`, widget SWFs, their
  thumbnails, the alarm tones, and the appendix are all gitignored.
