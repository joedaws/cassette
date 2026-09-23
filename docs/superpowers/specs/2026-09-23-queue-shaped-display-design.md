# Phase 5c — Queue-shaped display

## Purpose

The store has been a queue since Phase 4: cassettes carry a priority and an open/closed
status, and `store::priority::queue_order` sorts open before closed, then by priority, then by
id. The TUI has never known any of it. Its stack is a bare `Vec<Cassette>` whose order is
whatever insertion happened to produce, and `Cassette` carries neither field.

This phase makes the TUI show the queue it is already writing into: cassettes appear in queue
order, closed ones collapse into a single expandable row, `MAX_CASSETTES` counts the working
set rather than the retained history, and a cassette too damaged to parse becomes a visible
error row instead of a silent omission.

## What 5a and 5b already did, so this phase does not redo it

- **5a** gave `App` per-cassette store identity (`Cassette.id`) and dirty state, and made the
  TUI write through a held `LockGuard`.
- **5b** added live sync, `App::merge_external`, the read-only busy banner with per-tick
  retry, and sticky-lock indicators. Two of its decisions are load-bearing here:
  - Its Task 1 bound the lock guard to a cassette **id, not a list index**, so that an
    insertion elsewhere could not silently repoint it. That is exactly what lets this phase
    reorder `app.cassettes` freely: the held guard does not care where its cassette sits.
  - Its final review established that the phase's sync→merge seam is covered by two positive
    integration tests. This phase edits that seam (see "Ordering replaces insertion"), so
    those tests are the safety net for the change.

## The data model

`Cassette` gains two scalar fields:

```rust
pub priority: i64,
pub closed: bool,
```

Both are plain data with no store import, following the precedent `locked_by` already set: it
carries a **resolved display name** rather than a writer id specifically so that `Cassette`
needs no store knowledge. A `bool` rather than `store::meta::Status` is what keeps
`src/cassette.rs`'s zero-import purity — an invariant the last two phases checked explicitly
(`grep -nE '^use ' src/app.rs src/cassette.rs`) and this one keeps checking.

A cassette created with `Ctrl+N` starts at `priority: i64::MAX`, so it sorts to the tail until
`session_writer::create_cassette` mints its real tail-of-queue priority and writes it back.
`i64::MAX` is a sentinel for "not yet minted", not a magic number with meaning of its own.

**Both new fields are read-only to the TUI.** `flush_held` re-reads a cassette's frontmatter
under the lock and replaces only `topic`, `last_writer` and `updated_at`; `priority`, `status`
and `locked_by` belong to the queue commands, so an autosave cannot undo a concurrent
`queue move`, `queue close` or `queue lock`. Adding these fields must not change that: they
flow store → `Cassette` and never back. The `i64::MAX` sentinel therefore cannot reach disk,
because nothing in the TUI ever writes a priority at all.

Four existing places set these, all of which already do exactly this for `topic` and
`locked_by`: `load_session_cassettes` (from `meta.priority` / `meta.status`),
`merge_external`, `SessionWriter::refresh_from_disk`, and `SessionWriter::create_cassette`.

### Read-only becomes one field with a reason

`App` currently encodes one fact in two fields: `read_only: bool` and
`busy_holder: Option<String>`. Adding "closed" as a second reason to be unwritable would make
that worse. Both collapse into:

```rust
pub enum ReadOnly {
    No,
    Busy { holder: Option<String> },
    Closed,
}
```

This is the same one-fact-one-place principle that fixed 5b's banner — applied *before* the
second reason lands rather than after. `modify_focused` gates on `!matches!(self.read_only,
ReadOnly::No)`, so the existing read-only path is unchanged in behaviour.

The cost is real: this touches code that landed in 5b. The mitigation is already in place —
5b's cross-process contention test asserts the rendered banner string under a genuinely held
lock, so a wording regression fails a test rather than reaching a screen.

## Ordering

`App::sort_queue()` sorts `cassettes` by `(closed, priority, id)`, mirroring
`store::priority::queue_order`, which stays the authority on what queue order means. It takes
and restores `focus_idx` **by id** itself rather than leaving each caller to remember —
the discipline `merge_external` already follows, and the reason 5b's guard is id-bound.

### Ordering replaces insertion

`merge_external`'s `insert_at` parameter is **removed**. Today `main.rs` computes an insertion
position by walking `scan.cassettes` with a `take_while` and a `filter`; with priority on the
cassette itself, the caller pushes and re-sorts instead. 5b's plan flagged `insert_at` as an
open judgment call ("whether `Cassette` gains a priority field or `main.rs` passes an
insertion position"); this phase takes the other branch and retires the parameter.

This edits the merge path, which is the one with data-loss potential. Two things must come
through untouched, and a reviewer should treat a failure of either as Critical rather than
stylistic:

- the exclusion of any cassette whose lock this process holds, and
- `merge_external`'s `debug_assert!(!existing.dirty)`.

## The cap counts the working set

`MAX_CASSETTES` (36) counts only `!closed` cassettes, in both places that enforce it:

- `App::add_cassette` refuses at 36 **open**, rather than 36 total.
- `App::load_cassettes` keeps every open cassette up to the cap instead of blindly
  `truncate`ing the whole list.

The second is a live bug, not only a new feature: today a resumed session with enough closed
cassettes can push open ones off the screen entirely, because `truncate(MAX_CASSETTES)` runs
over a list that `queue_order` has already put the closed ones inside. It gets a test that
fails before the change.

**Closed cassettes are then retained without limit.** That is the intent — the cap is on
working set, not on history — and the accepted cost is that a long-lived daily session holds
every closed cassette's full text in memory to render one collapsed row. Lazy body loading is
deliberately not built here; if a real session ever makes this hurt, the fix is to skip
reading closed bodies until the row is expanded, which is a self-contained change.

## The display

The stack stays a single `Vec`. Because `sort_queue` guarantees open-before-closed, the open
set is a prefix, so no second collection and no index mapping are needed:

```rust
open_count() = cassettes.iter().take_while(|c| !c.closed).count()
stack_len()  = if closed_expanded { cassettes.len() } else { open_count() }
```

The three existing scroll helpers — `visible_cassette_count`, `ensure_focus_visible`,
`hidden_cassettes` — switch from `cassettes.len()` to `stack_len()`. That is the whole
scrolling change.

### The closed row

Rendered below the open stack as `▸ N closed`, greyed with the theme's `unfocused_fg`.
Never hardcode the colour; it comes from the active `Theme` like everything else in `ui.rs`.

`z` in normal mode toggles it — free (normal mode binds `$ 0 a A b d g G h i I j k l o O q t u
w x`) and vim-adjacent, where `z` is the fold prefix. Collapsed on session open. Expanded, the
row reads `▾ N closed` and the closed cassettes render minimized and greyed beneath it.

Tab and Shift+Tab cycle within `stack_len()`, so while collapsed they never land on a closed
cassette and expansion is what makes them reachable. Focus movement itself needs no
special case.

When `N` is 0 the row is not drawn at all.

Two edge cases the fold introduces, both pinned rather than left to the implementer:

- **Collapsing while focused on a closed cassette** would leave `focus_idx` outside
  `stack_len()`. `z` moves focus to the last open cassette before collapsing.
- **A session with no open cassettes at all** (every one closed by an agent, then resumed)
  would give `stack_len() == 0` and no valid focus. The row starts expanded whenever
  `open_count() == 0`, and `z` refuses to collapse it in that state — there would be nothing
  left on screen to focus.

### Focusing a closed cassette

Yields `ReadOnly::Closed`, so `modify_focused` drops the edit through the gate that already
exists — no new write path. The info line reads `-- CLOSED --`, deliberately not borrowing the
busy banner's wording, and the help row (which already branches on read-only) gains a third
case naming `queue reopen`. Busy, sticky and closed are three states with three different
remedies — wait, go unlock it, go reopen it — and 5b established that the display has to tell
them apart rather than blur them into one "you cannot type here".

Reopening stays a CLI act. The TUI grows no write path for queue state in this phase.

### Damaged cassettes

`Store::scan_session` currently counts unreadable files and discards everything a row would
need. The parent spec flags this against itself ("Phase 2 discards the information such a row
would need"). This phase closes it.

`SessionScan` gains:

```rust
pub damaged: Vec<DamagedCassette>,   // path, id (where the filename stem yields one), reason
```

and the `unreadable: usize` **field becomes a method** returning `damaged.len()`. Three
modules read it today (`stats.rs`, `find.rs`, `main.rs`); deriving it means the count and the
list cannot disagree — the same defect shape as 5b's banner, avoided the same way.

`reason` distinguishes the two failures `scan_session` already separates internally: the file
could not be read, and the frontmatter could not be parsed.

Damaged rows render greyed below the closed row, are skipped by Tab entirely, and do not count
toward the cap. They never enter `app.cassettes` — they are not cassettes — so `App` carries
them as plain `(label, reason)` strings.

### Bottom-up order

```
open stack  →  ▸ N closed  →  damaged rows  →  separator  →  footer
```

Both extra groups sit outside the working set, which is what they have in common.

## Testing

- **Invariant** — no new cassette write: the TUI must still never write `priority` or
  `status`. `git diff main...HEAD -- src/ | grep -E '^\+.*(priority|status)\s*='` inside
  `session_writer.rs`'s flush path should stay empty, alongside the two invariants the last
  two phases checked (`App`/`Cassette` purity, and no new `guard.write`/`atomic_write`).
- **Unit** — ordering at all three keys (open before closed, then priority, then id);
  `open_count`/`stack_len` in both fold states; the cap counting only open cassettes; Tab never
  reaching a closed cassette while collapsed; `ReadOnly::Closed` dropping an edit;
  `sort_queue` preserving focus identity across a reorder.
- **Store** — `scan_session` returning damaged entries with a distinguishable reason, and
  `unreadable()` equal to `damaged.len()` by construction.
- **Integration** — a session of 36 open plus several closed cassettes loading with **no open
  cassette dropped**; this must fail before the `load_cassettes` change.
- **pty** — the fold toggling, since it is a display change and assertions cannot see a
  screen. Under an isolated `CASSETTE_DATA_DIR`, which is not optional.

## Out of scope

- The `cassette sessions` interactive picker. Split to **5d**: it is a new full-screen UI with
  its own event loop, and phases 4 and 5 were both split when they carried this much. Its
  parent-spec clause also needs a correction first (below).
- Reopening or closing a cassette from the TUI.
- Lazy loading of closed cassette bodies.
- Removing the flat-note format — that is Phase 6.

## Corrections to the parent spec

Both are recorded here and should be fixed in
`docs/superpowers/specs/2026-09-13-session-store-design.md` as part of this phase, since the
spec files are the durable record:

1. **The picker "marks it active" (line ~361) is dead.** Phase 4b removed the active-session
   pointer and made `--session <id>` required on every queue command. There is no active
   session to mark. The clause should say the picker opens the session in the TUI, nothing
   more.
2. **The phase list (line ~643) should show the 5c/5d split**, with the picker under 5d.

## Open items carried in

Recorded for triage; none blocking. Inherited from 5b's final review:

- `assert!(after >= before)` in `tests/cli.rs` passes when the mtime did not move.
- A newcomer arriving above a scrolled viewport shifts the visible set by one row. Focus
  identity is preserved, so this is cosmetic — but this phase touches the same scroll helpers,
  so it is worth a look while there.
- After a side-B merge, `cursor_for_a` is hardcoded to 0 while the mirror case keeps side B's
  stored cursor.
- `queue::write::write_permitted` keeps an inline id→name lookup beside the shared
  `store::writers::display_name`.
