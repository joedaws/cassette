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
  **Recommendation (2026-09-24, not applied):** this is skill guidance, not tool behaviour —
  `queue new` already lands at the tail. Change `cassette-session`'s "surface a reply" bullet
  to: leave replies in place by default; move one to the top only when the human asked a
  question and is waiting on it; never reorder a morning-pages or topic-structured session.
  A per-session `mode = "conversation"` in `session.toml` could make it explicit later.
  Awaiting the user's call.

---

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
- **`close_permitted`'s sticky refusal prints the raw `locked_by` id**, where
  `write_permitted`'s names the holder via `store::writers::display_name`. (Found 2026-09-24.)
- **`queue topic` rejects `\n` but lets `\r` through**, which `meta::one_line` then flattens
  silently — the silent rewrite the newline rule exists to prevent. (Found 2026-09-24.)
- **`stats` output still counts "notes"** (`5 notes · 2183 words`) while its `--help` says
  sessions. (Found 2026-09-24.)
- **Other commands may panic writing to a closed pipe with large output** (`print!` on EPIPE,
  Phase 6's `export | head` lesson). `completions`/`man` are safe via `tolerate_broken_pipe`;
  the rest were not audited. Not reproduced.

## Owed verification (2026-09-24 build)

- **The release workflow's new packaging step has never run.** Publish a release (or
  pre-release tag) and check the tarball listing against `docs/distribution.md`'s "What ships".
- **`cassette-writeup` has had no independent pressure test** — only its author running it on
  `fixture.sh`. `superpowers:writing-skills` calls for a baseline run by a fresh agent without
  the skill, then one with it.
- **The non-unix build of `ChangeStamp` has never compiled** (only the linux target is
  installed here).
- **The whole-branch review of the 2026-09-24 build was a self-review**, not a fresh reviewer.
