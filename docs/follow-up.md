# Follow-up work

The session-store redesign closed with Phase 6 (merged `1f8e8a8`, 2026-09-23). Its six phase
specs each carried an "Open items" list forward to the next phase; with no next phase, they
collect here instead.

There is no external issue tracker. This file is the durable record, the way the spec files are
for designs — if something here gets built, it gets a spec and this entry goes away.

Ordered roughly by how much they would change the tool, not by effort.

---

## Reader mode — focus without holding the lock

**Raised 2026-09-23, during the Stage 2 multi-writer trial, by the user.**

Today **focus means held**: the TUI takes a cassette's lock the moment you focus it and keeps
it until you move away. So there is no way to *watch* a cassette being written. To let an agent
write one you must Tab off it, and then it collapses to its last line — you can read it
properly or let the agent write it, never both.

That assumption dates from Phase 5a and was reasonable when a cassette had one author. It is
wrong under the distillation model the trial arrived at (below), where the whole point is
watching a shared nugget get refined.

Sketch, which is less work than it sounds because the machinery exists:

- A fourth `ReadOnly` variant, `Reading`, beside `No`/`Busy`/`Closed`. `modify_focused` already
  drops edits for any non-`No` variant, so keystrokes stop for free.
- A binding that releases the held lock while keeping focus. The release path is what `Tab`
  already does, minus the focus move.
- `sync_external_writes` already merges every cassette this process does **not** hold, so a
  released-but-focused cassette starts syncing full-height with no new sync code.
- 5b's follow-if-at-end cursor rule then does the right thing unmodified: a cursor at the end
  follows incoming text, one parked mid-paragraph stays put.
- Banner `-- READING (open to writers) --`, distinct from `BUSY` (someone took it) and `CLOSED`
  (needs a reopen) — this one is voluntary, which is what needs conveying.

**The larger question inside it:** whether this is a mode you toggle, or whether focus should
stop implying the lock at all — take it on the first keystroke, release it on idle. That would
make reading the default and writing the claim, which matches how the user actually worked
through the entire trial.

---

## What the Stage 2 trial found

Recorded because they are design findings, not defects.

- **A cassette is a distillation, not a transcript.** The user's model, arrived at mid-trial:
  each writer rewrites the whole cassette into the current shared understanding rather than
  appending a turn. This makes `queue write`'s replace-the-body default correct and `--append`
  the exception, recasts `last_writer` as "whose turn ended last" (a routing hint, which is
  exactly what `waiting_on` derives from it), and makes an export a *document* rather than a
  log. It also settles the inline-attribution question in the negative: a rewritten cassette
  has no history to attribute. What is lost is the path — if that ever matters it wants git on
  the store, not speaker prefixes in the prose.
- **Sharing a cassette is the natural move, not a collision.** The user replied *inside* the
  agent's cassette twice within ten minutes, unprompted. The design had assumed one writer per
  cassette. The lock still behaved correctly — the agent's write was refused with exit 3 — but
  the two writers were queued behind each other for no reason.
- **Top-priority replies turn the queue into a feed.** Moving each answer to position 1 means
  the newest is always where you land, but it orders the session by recency of reply rather
  than by topic. Plausibly right for a conversational session and wrong for morning pages;
  worth a deliberate choice rather than a default.

---

## Document export — two modes, one of them unbuilt

**Raised 2026-09-23, after the Stage 2 trial.**

Getting a session out should have two shapes, because they answer different questions:

1. **Raw markdown — `cassette export <id>`. Exists.** Every cassette verbatim in queue order,
   closed and damaged ones included and marked. Faithful, lossless, no interpretation. This is
   the archive, and the right thing when you want *what was written*.

2. **A coherent document, written by the agent. Unbuilt.** The session's cassettes are points
   raised, refined and sometimes abandoned across a working session; nobody wants to read them
   as a numbered list a month later. The agent reads the session and writes the document the
   session was *converging on* — the argument rather than its scaffolding.

The second is not a format conversion, which is why it is not simply a flag on the first. A
session's cassettes under the distillation model are already each a nugget; what is missing is
the connective tissue and the ordering that makes them one argument rather than six notes.
That is a judgement call, so it belongs to an agent, not to `export`'s renderer.

Open questions worth settling before building it:

- **Where does the output go?** A new cassette in the session (it is a thought about the
  session, and stays with it), or a file outside the store (it is a deliverable, and lives
  where deliverables live)? Leaning file — putting it in the session invites a later pass to
  distil the summary into the thing it summarises.
- **What does it do with disagreement?** A session where a question was raised and answered
  three different ways has no single coherent line. Flattening that is lossy in a way the raw
  export is not.
- **Is it a CLI command at all?** It needs a model, so `cassette` cannot do it alone. Most
  likely it is a skill the agent runs against `export`'s output, not a subcommand — which
  would keep the binary model-free, as it is today.

## Tune the agent's cassette length

Set at **~15 lines** on 2026-09-23 after the user observed that 30-line responses are a wall to
land on mid-flow. Lives in `.claude/skills/cassette-session`. A starting point, not a finding —
revisit once there is enough use to say what actually reads well.

## Carried triage

Small, none blocking, each verified to still exist as of Phase 6.

- **`cassette sessions` scans every cassette of every session before its first frame.**
  Pre-existing — `find` pays the same cost — but it is now on an interactive path. **Measured
  2026-09-24** (release build, warm page cache, 5 cassettes × one line each per session, median
  of 5 `cassette find` runs, the same `scan_store` the picker calls): 100 sessions 8 ms, 1 000
  sessions 55 ms, 5 000 sessions 285 ms — linear, ~56 µs a session. A year of daily use
  (~400 sessions) is ~22 ms, far under the ~150 ms first-frame threshold, which is not reached
  until ~2 700 sessions. **No action now.** A cold cache and longer cassettes will cost more;
  re-measure on a real store before acting, and the likely fix then is a cached per-session
  summary, which is a design of its own.
- **A newcomer arriving above a scrolled viewport shifts the visible set by one row.** Focus
  identity is preserved, so this is cosmetic. Fix when built: capture the id at `cassette_scroll` before
  `merge_external`'s re-sort, restore the index by that id after, then `ensure_focus_visible`
  — the by-id rule focus already follows.
