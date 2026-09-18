# Phase 5a — The TUI writes the store

**Parent spec:** `2026-09-13-session-store-design.md`, the binding authority.
**Predecessors:** 4b (`2026-09-15-session-queue-core-design.md`), 4c
(`2026-09-16-json-and-sticky-lock-design.md`), merged as `b1a624c` and `c05b457`.

## Purpose

Phase 4 gave agents a queue they can drive. The TUI still writes a single flat markdown
note per session and knows nothing about the store. 5a repoints it: the human's editor
becomes another writer in the same protocol the agents already obey.

This is the largest single change in the redesign. The TUI today holds `Vec<Cassette>` with
**one** global `dirty` flag and rewrites **one** whole file every thirty seconds. Afterwards
it holds per-cassette identity, per-cassette dirty state, and a lock on the cassette you are
typing in.

## Phase 5 is split

Phase 5 as the parent spec defines it is four subsystems plus four inherited items. It is
split the way Phase 4 was, each sub-phase leaving the tool working:

- **5a — The TUI writes the store** (this document). Identity, the focused-cassette lock,
  per-cassette autosave, flush-on-blur, minimal read-only for busy cassettes, `Store::holds`,
  `queue write --side/--append/--replace`, and repointing `stats`/`find`.
- **5b — The TUI reads what others write.** Live-sync polling, `merge_external`, the
  read-only banner with per-tick retry, sticky-lock indicators.
- **5c — Queue-shaped display.** Priority ordering, the collapsed closed row, `MAX_CASSETTES`
  applying to open cassettes only, damaged cassettes as error rows, the `cassette sessions`
  picker.

## Decisions

### 1. Focus means held, with no timeout

The TUI acquires the focused cassette's lock and holds it until focus moves or the process
exits. There is no idle release and no reacquisition.

The cost is real and accepted: a TUI left open holds one cassette against every agent —
`queue next` skips it and `queue write` returns exit 3. **The mitigation is a usage norm, not
a mechanism: close sessions rather than leaving them open overnight.** Agents drive the queue
entirely through the CLI and never need the TUI to be running, so a closed TUI costs an agent
nothing.

An idle timeout was considered and rejected. Releasing after N quiet seconds means
reacquisition can **fail** — you come back, type, and the cassette is now held by an agent
mid-write, so the keystroke has nowhere to go. That puts the failure on a human mid-thought,
which is the one place this design has consistently refused to put it. A held lock is also
visible: `queue list --json` reports `busy: true` with the holder's name.

### 2. A busy cassette is read-only, never silently unwritable

If the focused cassette's lock cannot be acquired, the TUI renders it read-only and says so
in the status line. 5a's version is minimal — no banner, no per-tick retry; those are 5b.

What 5a must not do is either of the alternatives. Refusing to start would mean a running
agent locks you out of your own session. Focusing it writably without the lock would let you
type into a cassette you do not hold and lose the text at the next flush. **Silent text loss
is the one outcome this phase must not have.**

### 3. Per-cassette dirty, per-cassette write

The single `App.dirty` flag becomes per-cassette state. The thirty-second autosave writes
only cassettes that changed, and only through the guard the TUI already holds. Moving focus
flushes the cassette being left before dropping its guard.

### 4. `Store::holds` lands here

The TUI is the reentrancy case the earlier phases deferred. Autosave runs while the TUI holds
a guard, and any code that re-acquires the same cassette's lock deadlocks against itself —
flock is per-open-file-description and does not recognise its own owner. 4b's `queue move`
avoided this by never nesting; the TUI cannot, because holding is the point.

`Store::holds(session, id) -> bool` answers "do I already hold this?", so a write path can
use the existing guard rather than taking a second one.

### 5. The store cassette body carries sides and nothing else

`output::build_body` currently writes `# Cassette N — topic` before each cassette's
`## Side A` / `## Side B`. In the store each cassette is its own file with its topic in
frontmatter, so that heading is redundant — and actively harmful: `json::split_sides` folds
any text before the first `## Side A` into `side_a` as preamble, so a stray `# Cassette 1`
line would surface inside the JSON contract's `side_a`.

A store cassette's body is therefore `## Side A\n\n<text>\n` plus `## Side B\n\n<text>\n` when
side B is non-empty, and nothing else. This is exactly the shape `json::split_sides` already
parses, so 4c's contract begins reporting real `side_b` content with no change to the
contract.

### 6. `queue write` gains `--side` and `--append`/`--replace`

Listed in the parent spec's CLI surface, implemented in neither 4a nor 4b, and recorded as an
inherited gap in 4c. Sides become real when the TUI writes them, so the flags land here.
`--side` defaults to `a`; `--append`/`--replace` default to `--replace`, preserving today's
behaviour for every existing caller.

### 7. `stats` and `find` read the store, and the old totals reset

Both repoint to the session store in this phase rather than in Phase 6, so there is no window
where the TUI writes one place and the reporting commands read another.

**The existing 45 notes (15,992 words, since 2026-05-31) are not migrated and not read.**
`stats` will report the store's totals, starting near zero. This was chosen deliberately with
the cost visible: one scanner and no merge rule, against a streak that restarts.

**Nothing is destroyed.** `~/.local/share/cassette/notes/` is left untouched and every note
remains readable in any editor, so a deliberate migration remains possible in a later phase.
If one is ever written it deserves its own phase with a backup step — not a corner of this
one.

`find`'s `NoteEntry.draft` field becomes meaningless: the store has no autosave draft marker,
because a cassette file is either written or it is not. Drop the field rather than always
reporting `false`.

### 8. Empty sessions clean up after themselves

Today an empty session writes no file and removes any autosaved draft. Preserved: on quit, a
session the TUI created in this run whose cassettes are all empty is removed, so a mistaken
launch does not litter `session list`. A session that was opened rather than created is never
removed, whatever its contents.

## Entry points map onto sessions

No new verbs. The existing surface acquires session meanings:

| Command | Session |
|---|---|
| `cassette` | a new session |
| `cassette new <NAME>` | a new session aliased `<NAME>` |
| `cassette today` | the session aliased with today's date, created if absent |
| `cassette resume [NAME]` | the most recent session, the one with that alias, or the one with that **id** |
| `cassette -T <template>` | a new session with one cassette per topic |

`resume` accepts a session id as well as an alias (aliases first). `find` prints ids, and an id
it printed that `resume` rejected would be a discovery loop closing on nothing. 4b's "sessions
are named by ULID only, an alias never resolves" governs `--session` on the queue commands,
where a machine would have to resolve an ambiguous name silently; `resume` is the human entry
point, and accepting the printed id adds an opening rather than an ambiguity.

`-o` persists nothing and so names no session: combined with `resume`, `new` or `today` it is a
usage error (exit 2), not a silently dropped subcommand. Same for `-T` with a `today` whose day
already has a session — `load_cassettes` would replace everything `apply_topics` built.

`Ctrl+N` calls `Store::add_cassette` at runtime, so a cassette created mid-session is a store
cassette like any other.

## Focus means held — and every acquire re-reads

Taking the lock is only half of the parent spec's invariant 1 ("you may only write a cassette
whose lock you hold; every cassette you do not hold, you re-read from disk"). The other half is
that `SessionWriter::acquire` re-reads the cassette the instant the lock is won
(`refresh_from_disk`), adopting the disk's body and topic when they differ from memory. While a
cassette is unfocused the TUI holds nothing and an agent may `queue write` it; without the
re-read, one keystroke after tabbing back republishes the stale in-memory copy over the agent's
words and warns nobody — the silent text loss decision 2 says this phase must not have.

Nothing is merged — that is 5b's `merge_external` — and nothing is lost: an unfocused cassette
can never be dirty (`modify_focused` only touches the focused one, and `acquire` flushes the
outgoing cassette through its own guard before the guard moves), so adopting the disk's version
discards no keystrokes. An unchanged file is skipped, so an ordinary Tab away and back keeps the
cursor, undo stack and active side.

## What changes in the code

| File | Change |
|---|---|
| `src/app.rs` | `App` gains `session: String`; `Cassette` gains `id: String`; `dirty` becomes per-cassette |
| `src/main.rs` | startup creates or opens the session; the event loop holds a `LockGuard` for the focused cassette; autosave and flush-on-blur write through it; `Sink` and the note path go |
| `src/store/mod.rs` | `Store::holds` |
| `src/queue/write.rs`, `src/cli.rs` | `--side`, `--append`/`--replace` |
| `src/stats.rs`, `src/find.rs` | scan the store instead of the notes directory |
| `src/output.rs` | per-cassette body building; the flat-note writer and its draft machinery stay until Phase 6 deletes them |

`App` and `Cassette` stay pure — no I/O, no ratatui types — as CLAUDE.md requires. The guard
lives in `main.rs`'s event loop, not in `App`.

## Testing

- **Unit** — per-cassette dirty tracking, the body builder's side-only output, empty-session
  detection, and `stats`/`find` rendering over store-shaped data.
- **CLI** — `queue write --side b` lands in side B and `--append` does not clobber side A;
  `stats` and `find` report a store session.
- **Cross-process** — the discipline of `tests/lock.rs` applies unchanged: no sleeps, no
  polling, and a child's lock acquisition proven by output it writes after acquiring, never
  by the parent's write to its stdin. Two cases: an agent's `queue write` on the cassette the
  TUI has focused returns exit 3, and it succeeds once focus moves.
- **Pty** — the TUI paths need the `.claude/skills/verify` pty driver. Note its requirements:
  answer `ESC[6n`, and `cargo test` does not rebuild the binary.

## Out of scope

- Live-sync polling, `merge_external`, the read-only **banner** with per-tick retry, sticky
  indicators — **5b**.
- Priority ordering, the collapsed closed row, `MAX_CASSETTES` on open only, error rows, the
  `sessions` picker — **5c**.
- Deleting the append/draft/conflict-rename machinery, the `0700` data dir, the cloud-sync
  warning, man page and completions — **Phase 6**.
- Migrating the 45 legacy notes — deliberately unowned; see decision 7.

## Open items carried in

From 4c, still open and not blocking:

- The exit-4 message names the sticky-lock holder by raw ULID while the JSON `sticky_lock`
  field resolves the same writer to `{name, kind}`. **5a should fix this** — it is one
  registry lookup, and 5a is already touching writer resolution.
- `src/store/mod.rs`'s `#![allow(dead_code)]` rationale comment is stale.
- `queue/view.rs`'s `open_candidates` re-sorts cassettes `scan_session` has already ordered.
- `show_view`'s frontmatter-parse-failure path is untested.
