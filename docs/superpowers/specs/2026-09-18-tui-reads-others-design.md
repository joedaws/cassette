# Phase 5b — The TUI reads what others write

**Parent spec:** `2026-09-13-session-store-design.md`, the binding authority.
**Predecessor:** 5a (`2026-09-17-tui-writes-the-store-design.md`), merged as `94df416`.

## Purpose

5a made the TUI a writer in the store protocol. It is still blind: it sees a session exactly
as it was when it launched, plus whatever it wrote itself. 5b makes it a reader of what
everyone else is doing — an agent's words appearing as they are written, a claimed cassette
saying who claimed it, and a busy one becoming editable the moment its holder lets go.

## What 5a already did, so this phase does not redo it

- **`refresh_from_disk` runs inside `acquire`**, the instant a cassette's lock is won. Focusing
  a cassette an agent wrote gives you its text, not your stale copy. This was a Critical found
  in 5a's final review; the mechanism exists and 5b builds on it rather than repeating it.
- **`follow_focus` retries acquisition after every keypress**, so a busy cassette already
  becomes editable when you type. 5b extends that to idle ticks.
- **`App.read_only` and the `-- READ ONLY --` info line** exist. 5b replaces the bare label
  with the spec's banner.

## Live sync

Each event-loop tick, `main.rs` stats the session's `cassettes/` directory. For any file whose
mtime has moved **and whose lock this process does not hold**, it re-reads the file, parses it,
and hands `App` the result:

```rust
app.merge_external(id: &str, cassette: Cassette)
```

The stat-and-read lives in `main.rs` because `App` performs no I/O — the crate's standing rule
(CLAUDE.md), and the same reason the `LockGuard` lives in the event loop rather than in `App`.

**The focused writable cassette is excluded by construction.** This process holds its lock, so
no other writer can have changed it. The two cassettes live sync actually updates are:

1. **Unfocused cassettes** an agent is writing — their minimized rows update.
2. **A read-only focused cassette**, whose holder is writing it — the "watch an agent work"
   view the parent spec argues for when it says refusing to focus a busy cassette "would be
   worse: you could not see what an agent is doing to your queue".

### Polling on the existing tick

The parent spec says "stat the cassettes dir … Polling ~10 files at 500ms is negligible". That
number is an illustration of the cost, not a requirement. 5b polls on the **existing
one-second event-loop tick** rather than introducing a second clock. One second is
imperceptible for watching prose arrive, and a single timer is one fewer thing to reason about
when the loop also drives the timer, the idle nudge, the status flash and the autosave.

### When the session directory vanishes

Another process can remove a session directory — 5a's `store_is_empty` refuses to do so while
any cassette holds a live anchor or while any cassette it did not create is present, but the
case is reachable. Live sync **degrades silently**: the TUI keeps its in-memory state and its
guard, and simply has nothing to merge. Erroring mid-sentence over a directory the user is
still typing into would be worse than showing slightly stale neighbours.

## The cursor rule

When `merge_external` replaces a cassette's text, the cursor is placed by one rule:

- **If the cursor was at the end of the text, it moves to the new end.**
- **Otherwise it stays at its character offset.**

So watching an agent write follows the newest words, and scrolling up to read an earlier
paragraph is not yanked away on the next update — the behaviour of every log viewer worth
using.

**This costs no new state.** The focused cassette's viewport has no stored scroll offset:
`ui.rs` derives `scroll_top` from `cursor_row` on every render (typewriter scroll, cursor
centred, clamped at the ends). "Was I at the bottom" is therefore exactly "was the cursor at
the end", which the zipper already knows — the `right` half is empty. Placing the cursor is
the whole of the fix; the viewport follows.

## New cassettes appear

An agent running `queue new` creates a file the TUI has never seen. Live sync adds it as a
**minimized row at its priority**, like any other cassette.

It **never steals focus** and **never shifts the cassette the user is typing in** —
`App::ensure_focus_visible` (private, `src/app.rs:210`, already called by every method that
mutates the cassette list) keeps the focused cassette in the window, and the stack already
renders "N more ↑/↓" hints for rows scrolled out of view. `merge_external` calls it the same
way its neighbours do, so an arrival is absorbed by machinery that exists.

Inserting a cassette shifts the **indices** of those after it, so `focus_idx` must be adjusted
to keep pointing at the same cassette — and `SessionWriter`'s guard is held against an index
too. Whatever `merge_external` does to the list, the guard and the focus must still name the
cassette the user was typing in.

The alternative — a list frozen at launch — was rejected: it lets an agent add ten cassettes
the human never sees, which undercuts the reason for sharing a session at all. A status-line
notice on each arrival was also rejected: freewriting is the activity where an interruption
costs most, and a steadily working agent would produce a stream of them.

## The banner, and retry on every tick

A busy focused cassette shows the parent spec's banner — `open by refactor-agent` — naming the
holder, rather than 5a's bare `-- READ ONLY --`. The holder comes from the lock anchor's
attribution, the same source `queue write`'s exit-3 message and the JSON contract's
`sticky_lock` already use, resolved to a writer name with the raw id as fallback.

Acquisition retries **each tick**, not only on each keypress as 5a left it. A human who walks
away from a cassette an agent holds should find it editable on their return without having to
type a character to discover that. When the retry succeeds, 5a's `refresh_from_disk` runs
inside `acquire` and the agent's final text is already there.

## Sticky-lock indicators

A cassette whose `locked_by` is set names its holder in the separator, beside the existing side
tag and topic label — the separator is already where per-cassette state is announced, so this
is one more field rather than a new surface.

The distinction the display must carry: a **busy** cassette is transiently held by a live
process and will free itself; a **sticky-locked** one is a durable claim that only a human can
clear. They are different states with different remedies (wait, versus go and unlock it), and
the parent spec assigns them different exit codes for exactly that reason.

## Testing

- **Unit** — `merge_external`'s cursor rule at both branches (cursor at end, cursor mid-text),
  a new cassette arriving at its priority without changing `focus_idx`, and an arrival while
  the stack is scrolled.
- **Integration** — a cassette written behind the TUI's back appears after a tick; a new
  cassette from `queue new` appears; a busy cassette becomes editable after its holder releases,
  without a keypress.
- **Cross-process** — the discipline of `tests/lock.rs` applies unchanged: no sleeps, no
  polling, a child's acquisition proven by output it writes after acquiring. Note 5a
  established that holding the lock in-process and *then* spawning is the stronger pattern
  where it applies, since it removes the race entirely.
- **Pty** — `.claude/skills/verify` drives the TUI. It now documents `CASSETTE_DATA_DIR`
  isolation as required; that is not optional, and a run without it wrote into the user's real
  store during 5a.

## Out of scope

- Priority ordering, the collapsed closed row, `MAX_CASSETTES` on open cassettes only, damaged
  cassettes as error rows, the `cassette sessions` picker — **5c**.
- Repointing `today`/`resume`/`export`, deleting the append/draft/conflict-rename machinery,
  the man page and completions — **Phase 6**.
- Merging *concurrent edits* to the same cassette. That cannot arise: a cassette this process
  can edit is one whose lock it holds, and a cassette it does not hold it cannot edit.
  `merge_external` replaces; it never reconciles.

## Open items carried in from 5a

Recorded for triage; none blocking:

- A cassette changed on disk silently resets cursor and undo and flips to side A, with no
  status message. **5b should settle this** — the cursor half is what this phase's cursor rule
  addresses, but undo history and the side flip are still unaddressed.
- `follow_focus` re-locks on every keypress while read-only; 5b's per-tick retry should
  subsume it rather than sit beside it.
- `find` rows match on alias but do not print it, and `NoteEntry.path` now holds an id — a
  stale field name.
- `src/output.rs` carries a module-wide `#![allow(dead_code)]`; Phase 6 removes the module's
  legacy half outright.
- A read-only session that wrote nothing still prints "saved to session X".
- `Ctrl+N` bypasses the `max_open` cap that `queue new` enforces.
- `load_cassettes` silently truncates a resumed session to 36 cassettes.
- An orphan session directory is left behind when writer resolution fails after the session was
  created.
