# Phase 5c — Queue-shaped Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the TUI display the queue it already writes into — cassettes in queue order, closed ones folded into one expandable row, `MAX_CASSETTES` counting the working set, and damaged cassettes as visible error rows.

**Architecture:** `Cassette` gains two scalar fields (`priority: i64`, `closed: bool`) carrying store values with no store import, following the precedent `locked_by` set. `App::sort_queue()` orders the stack by `(closed, priority, id)`, which makes the open set a prefix — so the fold, the cap and the scroll window are all arithmetic over `open_count()` rather than a second collection. `read_only: bool` + `busy_holder` collapse into one `ReadOnly` enum before "closed" lands as a second reason to be unwritable.

**Tech Stack:** Rust, ratatui + crossterm, no new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-23-queue-shaped-display-design.md`

## Global Constraints

- **`App` and `Cassette` stay pure.** No I/O, no ratatui types, no `store::` imports. Verified by `grep -nE '^use ' src/app.rs src/cassette.rs` — `app.rs` may import only `std::time::SystemTime` and `crate::cassette::{...}`; `cassette.rs` imports nothing.
- **The TUI never writes `priority` or `status`.** `flush_held` replaces only `topic`, `last_writer`, `updated_at`. The new fields flow store → `Cassette` and never back, so an autosave cannot undo a concurrent `queue move`/`close`/`lock`.
- **Rendering colours come from the active `Theme`.** Never hardcode a colour in `ui.rs`.
- **`MAX_CASSETTES` = 36**, and after this phase it counts `!closed` cassettes only.
- Every task ends green: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Any command that spawns the `cassette` binary sets `CASSETTE_DATA_DIR` to a temp path. Without it, tests read and write the user's real store. This has happened before.

## File Structure

| File | Responsibility this phase |
|---|---|
| `src/cassette.rs` | Two new scalar fields, defaulted in all three constructors |
| `src/app.rs` | `ReadOnly` enum, `sort_queue`, `open_count`, `stack_len`, fold state, open-only cap, `merge_external` without `insert_at`, `damaged` rows |
| `src/ui.rs` | Closed row, closed-cassette rendering, `-- CLOSED --`, help row's third case, damaged rows |
| `src/main.rs` | `z` binding, wiring store fields in, dropping the `insert_at` computation |
| `src/session_writer.rs` | `refresh_from_disk` and `create_cassette` adopt the new fields |
| `src/store/mod.rs` | `DamagedCassette`, `SessionScan::damaged`, `unreadable()` as a method |
| `src/stats.rs`, `src/find.rs` | Follow `unreadable` field → method |

---

### Task 1: `Cassette` carries priority and closed; `App` sorts by them

**Files:**
- Modify: `src/cassette.rs` (struct + `new`, `from_sides`, `from_sides_with_cursor`)
- Modify: `src/app.rs` (add `sort_queue`, `open_count`)

**Interfaces:**
- Produces: `Cassette.priority: i64`, `Cassette.closed: bool`, `App::sort_queue(&mut self)`, `App::open_count(&self) -> usize`
- Consumed by: every later task

- [ ] **Step 1: Write the failing tests**

In `src/app.rs`'s `mod tests`:

```rust
/// Queue order is the store's, not insertion order: open before closed,
/// then priority, then id as the tie-break. `store::priority::queue_order`
/// is the authority on this; `sort_queue` must not disagree with it.
#[test]
fn sort_queue_orders_open_before_closed_then_priority_then_id() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    let mut mk = |id: &str, priority: i64, closed: bool| {
        let mut c = Cassette::new();
        c.id = id.to_string();
        c.priority = priority;
        c.closed = closed;
        app.cassettes.push(c);
    };
    mk("z", 20, false);
    mk("b", 10, true);
    mk("a", 20, false);
    mk("m", 10, false);

    app.sort_queue();

    assert_eq!(
        app.cassettes.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["m", "a", "z", "b"],
        "open by priority then id, closed last"
    );
    assert_eq!(app.open_count(), 3);
}

/// Sorting moves cassettes under `focus_idx`, which is an index. Focus is
/// identity, not position — the same rule `merge_external` already follows
/// and the reason 5b bound the lock guard to an id.
#[test]
fn sort_queue_keeps_focus_on_the_same_cassette() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    for (id, priority) in [("z", 30), ("a", 10)] {
        let mut c = Cassette::new();
        c.id = id.to_string();
        c.priority = priority;
        app.cassettes.push(c);
    }
    app.focus_idx = 0; // "z"

    app.sort_queue();

    assert_eq!(app.cassettes[app.focus_idx].id, "z", "focus follows the cassette");
    assert_eq!(app.focus_idx, 1, "which is now at the tail");
}

/// A `Ctrl+N` cassette has no store priority yet. It must sort to the tail
/// rather than to the head, so a new cassette appears where the human
/// expects it until `create_cassette` mints the real value.
#[test]
fn an_unminted_cassette_sorts_to_the_tail() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    let mut stored = Cassette::new();
    stored.id = "a".to_string();
    stored.priority = 10;
    app.cassettes.push(stored);
    app.cassettes.push(Cassette::new()); // unminted: empty id, i64::MAX

    app.sort_queue();

    assert_eq!(app.cassettes[0].id, "a");
    assert!(app.cassettes[1].id.is_empty(), "the new one stays last");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --bin cassette sort_queue`
Expected: FAIL — no field `priority` on `Cassette`, no method `sort_queue`.

- [ ] **Step 3: Implement**

In `src/cassette.rs`, add to the struct (keep the doc-comment density of the neighbouring `locked_by`):

```rust
    /// This cassette's queue priority, mirroring `store::meta::CassetteMeta`'s.
    /// Carried as a plain `i64` so `Cassette` needs no store import — the same
    /// reason `locked_by` carries a resolved name rather than a writer id.
    /// `i64::MAX` means "no store counterpart yet": a `Ctrl+N` cassette holds
    /// it until `session_writer::create_cassette` mints the real tail-of-queue
    /// value. The TUI never writes this back — `flush_held` owns only `topic`,
    /// `last_writer` and `updated_at`.
    pub priority: i64,
    /// Whether this cassette is closed in the store (`queue close`). A `bool`
    /// rather than `store::meta::Status`, to keep this module import-free.
    /// Closed cassettes fold away in the TUI and open read-only; reopening is
    /// a CLI act, so the TUI never writes this back either.
    pub closed: bool,
```

Set `priority: i64::MAX, closed: false` in `Cassette::new()`, `from_sides` and `from_sides_with_cursor`.

In `src/app.rs`:

```rust
    /// Cassettes in queue order: open before closed, then priority, then id.
    /// Mirrors `store::priority::queue_order`, which stays the authority on
    /// what queue order means — this cannot import it (`App` is pure), so the
    /// two are kept in step by a test that pins the same ordering.
    ///
    /// Focus is preserved by **identity**, not position: sorting moves
    /// cassettes under `focus_idx`. Doing it here rather than in each caller
    /// is the same discipline `merge_external` follows.
    pub fn sort_queue(&mut self) {
        let focused_id = self.cassettes.get(self.focus_idx).map(|c| c.id.clone());
        self.cassettes
            .sort_by(|a, b| {
                a.closed
                    .cmp(&b.closed)
                    .then(a.priority.cmp(&b.priority))
                    .then_with(|| a.id.cmp(&b.id))
            });
        if let Some(id) = focused_id {
            if let Some(i) = self.cassettes.iter().position(|c| c.id == id) {
                self.focus_idx = i;
            }
        }
        self.ensure_focus_visible();
    }

    /// How many cassettes are open. Because `sort_queue` puts closed ones
    /// last, the open set is a prefix — so this is also the index of the
    /// first closed cassette, and no second collection is needed anywhere.
    pub fn open_count(&self) -> usize {
        self.cassettes.iter().take_while(|c| !c.closed).count()
    }
```

- [ ] **Step 4: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: Cassette carries queue priority and closed state"
```

---

### Task 2: `ReadOnly` replaces `read_only` + `busy_holder`

**Files:**
- Modify: `src/app.rs` (enum, field, `modify_focused`)
- Modify: `src/ui.rs` (`info_text`, help row, `render`'s dimming guard)
- Modify: `src/main.rs` (`try_acquire`, `retry_lock`, startup)
- Modify: `src/session_writer.rs` (its cross-process test's assertions)

**Interfaces:**
- Consumes: nothing
- Produces: `app::ReadOnly` (`No`, `Busy { holder: Option<String> }`, `Closed`), `App.read_only: ReadOnly`
- Consumed by: Task 6 (renders `Closed`), Task 5 (sets it on focus)

**This task touches code that landed hours ago in 5b.** Read `src/main.rs`'s `try_acquire` doc comment before changing it — it explains why the busy state is recorded in these fields and deliberately *not* in `status_msg`. That reasoning survives this change; only the shape of the fields changes.

- [ ] **Step 1: Write the failing test**

In `src/ui.rs`'s `mod tests`:

```rust
/// Busy, sticky and closed are three states with three different remedies:
/// wait, go unlock it, go reopen it. 5b established that the display must
/// tell busy and sticky apart; closed is the third and must not read like
/// either.
#[test]
fn a_closed_cassette_does_not_read_like_a_busy_one() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.read_only = crate::app::ReadOnly::Busy {
        holder: Some("refactor-agent".to_string()),
    };
    let busy = crate::ui::info_text(&app);

    app.read_only = crate::app::ReadOnly::Closed;
    let closed = crate::ui::info_text(&app);

    assert!(busy.contains("READ ONLY (open by refactor-agent)"), "{busy}");
    assert!(closed.contains("-- CLOSED --"), "{closed}");
    assert!(
        !closed.contains("READ ONLY"),
        "closed is not 'someone else has it': {closed}"
    );
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test --bin cassette a_closed_cassette` — FAIL, no `ReadOnly`.

In `src/app.rs`:

```rust
/// Why the focused cassette cannot be written, or `No` when it can.
///
/// One field rather than a `bool` plus a holder name: 5b shipped a display
/// bug caused by recording one fact in two places, and adding "closed" as a
/// second reason would have compounded it. Each variant carries exactly what
/// its banner needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ReadOnly {
    #[default]
    No,
    /// Another live writer holds the lock. Frees itself; `holder` is `None`
    /// only when the lock anchor yields no name (a garbled or crashed write).
    Busy { holder: Option<String> },
    /// Closed in the store (`queue close`). Only `queue reopen` clears it.
    Closed,
}

impl ReadOnly {
    pub fn is_read_only(&self) -> bool {
        !matches!(self, ReadOnly::No)
    }
}
```

Replace `pub read_only: bool` and `pub busy_holder: Option<String>` with `pub read_only: ReadOnly`. Update `modify_focused`'s gate to `if self.read_only.is_read_only() { return; }`.

In `src/ui.rs`'s `info_text`, replace the read-only arm:

```rust
        _ if app.read_only.is_read_only() => match &app.read_only {
            ReadOnly::Busy { holder: Some(h) } => format!("-- READ ONLY (open by {h}) --"),
            ReadOnly::Busy { holder: None } => "-- READ ONLY --".to_string(),
            ReadOnly::Closed => "-- CLOSED --".to_string(),
            ReadOnly::No => unreachable!("guarded above"),
        },
```

and the idle-nudge guard and `render`'s dimming guard both become `!app.read_only.is_read_only()`.

In `src/main.rs`, `try_acquire`'s arms become:

```rust
        Ok(()) => app.read_only = ReadOnly::No,
        Err(e) => {
            app.read_only = ReadOnly::Busy { holder: busy_holder_name(&e) };
            if !matches!(e, store::lock::LockError::Busy { .. }) {
                app.flash(format!("cannot take this cassette's lock: {e}"));
            }
        }
```

`retry_lock`'s early return becomes `if !app.read_only.is_read_only() { return; }`.

- [ ] **Step 3: Run the tests and commit**

The suite will have several mechanical failures from the field change — fix each by reading what the test meant, not by deleting the assertion.

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "refactor: one ReadOnly field with a reason, not a bool plus a name"
```

---

### Task 3: Wire the store fields in, and retire `insert_at`

**Files:**
- Modify: `src/main.rs` (`load_session_cassettes`, `sync_external_writes`)
- Modify: `src/session_writer.rs` (`refresh_from_disk`, `create_cassette`)
- Modify: `src/app.rs` (`merge_external` signature)

**Interfaces:**
- Consumes: Task 1's `Cassette.priority`/`closed`, `App::sort_queue`
- Produces: `App::merge_external(&mut self, id: &str, incoming: Cassette)` — **`insert_at` removed**

**Critical:** this edits the merge path. Two things must come through untouched, and a reviewer should treat a failure of either as Critical: the exclusion of any cassette whose lock this process holds, and `merge_external`'s `debug_assert!(!existing.dirty)`. The two seam tests from 5b (`sync_lands_an_external_write_on_screen`, `sync_shows_a_cassette_another_writer_created`) are the safety net — they must still pass, unchanged in intent.

- [ ] **Step 1: Write the failing test**

In `src/main.rs`'s `mod tests`:

```rust
/// A newcomer lands at its queue position because it carries its priority,
/// not because the caller computed an index. This is what retires
/// `merge_external`'s `insert_at` parameter.
#[test]
fn sync_places_a_newcomer_by_its_own_priority() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store::Store::new(dir.path().to_path_buf());
    let session = seeded_session(&store, None);
    let mine = store_cassette(&store, &session, 20, "## Side A\n\nmine\n");
    let mut app = App::new(None, None, None, session.clone());
    app.cassettes.clear();
    let mut c = cassette::Cassette::new();
    c.id = mine.clone();
    c.priority = 20;
    app.cassettes.push(c);

    let mut mtimes = HashMap::new();
    sync_external_writes(&mut app, &store, None, &mut mtimes);

    // Priority 30 sorts AFTER mine; priority 10 sorts before it.
    let later = store_cassette(&store, &session, 30, "## Side A\n\nlater\n");
    let earlier = store_cassette(&store, &session, 10, "## Side A\n\nearlier\n");
    sync_external_writes(&mut app, &store, None, &mut mtimes);

    assert_eq!(
        app.cassettes.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![earlier.as_str(), mine.as_str(), later.as_str()],
        "queue order comes from the cassettes' own priorities"
    );
    assert_eq!(app.cassettes[app.focus_idx].id, mine, "focus unmoved");
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test --bin cassette sync_places_a_newcomer` — FAIL (ordering wrong / arity mismatch).

Change `merge_external`'s signature to drop `insert_at`; its insert branch becomes `self.cassettes.push(merged);` followed by `self.sort_queue();`. Update its doc comment — the paragraph explaining why `insert_at` is the caller's job is now wrong and must be replaced, not left.

In `main.rs`'s `sync_external_writes`, delete the `insert_at` computation (the `take_while`/`filter`/`count` chain) and pass only id and incoming. Keep the `MAX_CASSETTES` cap check on newcomers — it now counts open cassettes (`app.open_count()`).

In `load_session_cassettes`, set `loaded.priority = c.meta.priority;` and `loaded.closed = c.meta.status == store::meta::Status::Closed;` beside the existing `loaded.id` / `loaded.locked_by` assignments. Call `app.sort_queue()` after `load_cassettes`.

In `refresh_from_disk`, adopt `stored.meta.priority` and the closed flag the same way it already adopts `topic`. In `create_cassette`, set `app.cassettes[idx].priority = meta.priority;` beside the existing `app.cassettes[idx].id = meta.id;`.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: place cassettes by their own priority, retiring insert_at"
```

---

### Task 4: The cap counts open cassettes only

**Files:**
- Modify: `src/app.rs` (`add_cassette`, `load_cassettes`)

**Interfaces:**
- Consumes: Task 1's `open_count`
- Produces: no new API

- [ ] **Step 1: Write the failing test**

```rust
/// Today's `truncate(MAX_CASSETTES)` runs over a list `queue_order` has
/// already put closed cassettes inside, so a resumed session with enough
/// history drops OPEN cassettes off the end. The cap is on working set.
#[test]
fn loading_keeps_every_open_cassette_when_closed_ones_fill_the_list() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    let mut loaded = Vec::new();
    for i in 0..MAX_CASSETTES {
        let mut c = Cassette::new();
        c.id = format!("open-{i:02}");
        c.priority = i as i64 * 10;
        loaded.push(c);
    }
    for i in 0..5 {
        let mut c = Cassette::new();
        c.id = format!("closed-{i}");
        c.closed = true;
        c.priority = i as i64;
        loaded.push(c);
    }

    app.load_cassettes(loaded);

    assert_eq!(app.open_count(), MAX_CASSETTES, "no open cassette is dropped");
    assert!(
        app.cassettes.iter().any(|c| c.id == "open-35"),
        "including the last one"
    );
}

/// `Ctrl+N` is capped on the working set too, so closed history never
/// blocks a new cassette.
#[test]
fn add_cassette_caps_on_open_not_total() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    for i in 0..5 {
        let mut c = Cassette::new();
        c.id = format!("closed-{i}");
        c.closed = true;
        app.cassettes.push(c);
    }
    app.sort_queue();
    for _ in 0..MAX_CASSETTES {
        app.add_cassette();
    }
    assert_eq!(app.open_count(), MAX_CASSETTES);
    assert_eq!(app.cassettes.len(), MAX_CASSETTES + 5, "closed ones are retained");
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

`add_cassette`'s guard becomes `if self.open_count() >= MAX_CASSETTES`. `load_cassettes` replaces `truncate(MAX_CASSETTES)` with a retain that keeps all closed cassettes and the first `MAX_CASSETTES` open ones (sort first, so "first" means "best priority"):

```rust
        self.cassettes = cassettes;
        self.sort_queue();
        let mut open_kept = 0usize;
        self.cassettes.retain(|c| {
            if c.closed {
                return true;
            }
            open_kept += 1;
            open_kept <= MAX_CASSETTES
        });
```

Note `focus_idx` is set after this; keep the existing `self.focus_idx = self.cassettes.len() - 1;` behaviour but clamp it to the open set (`self.open_count().saturating_sub(1)`) so a resume does not open focused on a closed cassette.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "fix: cap the working set, not the retained history"
```

---

### Task 5: The fold — state, scroll window, and `z`

**Files:**
- Modify: `src/app.rs` (`closed_expanded`, `stack_len`, three scroll helpers, `toggle_closed_fold`, focus movement)
- Modify: `src/main.rs` (`z` in `handle_normal_key`)

**Interfaces:**
- Consumes: Task 1's `open_count`, Task 2's `ReadOnly::Closed`
- Produces: `App.closed_expanded: bool`, `App::stack_len(&self) -> usize`, `App::toggle_closed_fold(&mut self)`
- Consumed by: Task 6 (renders the row)

- [ ] **Step 1: Write the failing tests**

```rust
/// Collapsed, Tab must not reach a closed cassette: expansion is what makes
/// them reachable, so focus movement needs no special case of its own.
#[test]
fn tab_skips_closed_cassettes_while_folded() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    for (id, closed) in [("a", false), ("b", false), ("c", true)] {
        let mut c = Cassette::new();
        c.id = id.to_string();
        c.closed = closed;
        app.cassettes.push(c);
    }
    app.sort_queue();
    assert!(!app.closed_expanded, "folded on open");

    app.focus_idx = 1; // "b", the last open one
    app.focus_next();
    assert_eq!(app.cassettes[app.focus_idx].id, "a", "wraps within the open set");

    app.toggle_closed_fold();
    app.focus_idx = 1;
    app.focus_next();
    assert_eq!(app.cassettes[app.focus_idx].id, "c", "expanded, it is reachable");
}

/// Collapsing while focused on a closed cassette would leave `focus_idx`
/// outside `stack_len()`. Focus moves to the last open cassette first.
#[test]
fn collapsing_moves_focus_off_a_closed_cassette() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    for (id, closed) in [("a", false), ("c", true)] {
        let mut c = Cassette::new();
        c.id = id.to_string();
        c.closed = closed;
        app.cassettes.push(c);
    }
    app.sort_queue();
    app.toggle_closed_fold();
    app.focus_idx = 1; // the closed one

    app.toggle_closed_fold(); // collapse

    assert!(!app.closed_expanded);
    assert_eq!(app.cassettes[app.focus_idx].id, "a");
    assert!(app.focus_idx < app.stack_len());
}

/// A session whose cassettes are all closed has nothing to focus when
/// folded, so the fold opens and refuses to close.
#[test]
fn a_fully_closed_session_stays_expanded() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    let mut c = Cassette::new();
    c.id = "c".to_string();
    c.closed = true;
    app.load_cassettes(vec![c]);

    assert!(app.closed_expanded, "nothing else could be shown");
    app.toggle_closed_fold();
    assert!(app.closed_expanded, "refuses to collapse to an empty screen");
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

Add `pub closed_expanded: bool` (default `false`) to `App`. Then:

```rust
    /// How many cassettes the scroll window covers. Closed cassettes are
    /// only in it while the fold is open, which is what keeps Tab out of
    /// them without a special case in focus movement.
    pub fn stack_len(&self) -> usize {
        if self.closed_expanded {
            self.cassettes.len()
        } else {
            self.open_count()
        }
    }

    /// Toggle the closed fold. Refuses to collapse when there are no open
    /// cassettes, since that would leave nothing on screen to focus, and
    /// moves focus off a closed cassette on the way down.
    pub fn toggle_closed_fold(&mut self) {
        if self.closed_expanded {
            if self.open_count() == 0 {
                return;
            }
            self.closed_expanded = false;
            if self.focus_idx >= self.open_count() {
                self.focus_idx = self.open_count() - 1;
            }
        } else {
            self.closed_expanded = true;
        }
        self.ensure_focus_visible();
    }
```

In `load_cassettes`, set `self.closed_expanded = self.open_count() == 0;` after the retain.

Change `visible_cassette_count`'s callers — `ensure_focus_visible` and `hidden_cassettes` — plus `focus_next`/`focus_prev` to use `self.stack_len()` in place of `self.cassettes.len()`.

In `main.rs`'s `handle_normal_key`, add `'z' => app.toggle_closed_fold(),`.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: fold closed cassettes out of the stack, toggled with z"
```

---

### Task 6: Render the closed row and closed cassettes

**Files:**
- Modify: `src/ui.rs` (`render`'s layout, a new `render_closed_row`, the help row)
- Modify: `src/main.rs` (set `ReadOnly::Closed` when focus lands on a closed cassette)

**Interfaces:**
- Consumes: Task 2's `ReadOnly::Closed`, Task 5's `closed_expanded`/`stack_len`
- Produces: no new public API

- [ ] **Step 1: Write the failing tests**

```rust
/// The fold's own row: an affordance saying how much history is there and
/// which way it opens.
#[test]
fn the_closed_row_counts_and_points() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes.clear();
    let mut open = Cassette::new();
    open.id = "a".to_string();
    app.cassettes.push(open);
    for i in 0..12 {
        let mut c = Cassette::new();
        c.id = format!("c{i}");
        c.closed = true;
        app.cassettes.push(c);
    }
    app.sort_queue();

    assert_eq!(crate::ui::closed_row_text(&app).as_deref(), Some("▸ 12 closed"));
    app.toggle_closed_fold();
    assert_eq!(crate::ui::closed_row_text(&app).as_deref(), Some("▾ 12 closed"));
}

/// No closed cassettes, no row — an empty affordance is noise.
#[test]
fn no_closed_row_without_closed_cassettes() {
    let app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    assert_eq!(crate::ui::closed_row_text(&app), None);
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

Add a pure `pub(crate) fn closed_row_text(app: &App) -> Option<String>` returning `None` when `app.cassettes.len() == app.open_count()`, else `Some(format!("{} {} closed", arrow, count))` with `▾`/`▸` per `closed_expanded` — following the `info_text`/`separator_text` precedent of a pure text function that `render` styles.

In `render`, the chunk loop iterates `first..first + n` where `n` is bounded by `stack_len()` rather than `cassettes.len()`; the closed row gets a `Constraint::Length(1)` chunk after the stack when `closed_row_text` is `Some`, styled with `theme.unfocused_fg`. Closed cassettes render through the existing `render_cassette_min` path, also in `unfocused_fg`.

The help row gains its third case:

```rust
        _ if matches!(app.read_only, ReadOnly::Closed) => {
            "this cassette is closed — `cassette queue reopen` to write in it again  z:fold  Tab:next  ^C:quit & save"
        }
        _ if app.read_only.is_read_only() => { /* existing busy text */ }
```

In `main.rs`, wherever focus settles (`follow_focus`), set `ReadOnly::Closed` when the newly focused cassette is closed, in preference to attempting a lock at all — a closed cassette is not contended, it is simply not writable.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: render the closed fold row and closed cassettes"
```

---

### Task 7: Damaged cassettes become visible rows

**Files:**
- Modify: `src/store/mod.rs` (`DamagedCassette`, `SessionScan.damaged`, `unreadable()` method)
- Modify: `src/stats.rs`, `src/find.rs`, `src/main.rs` (field → method)
- Modify: `src/app.rs` (`damaged: Vec<(String, String)>`), `src/ui.rs` (rows)

**Interfaces:**
- Consumes: nothing
- Produces: `store::DamagedCassette { path: PathBuf, id: Option<String>, reason: DamageReason }`, `SessionScan::unreadable(&self) -> usize`, `App.damaged`

- [ ] **Step 1: Write the failing test**

In `src/store/mod.rs`'s `mod tests`:

```rust
/// A damaged cassette must be nameable, not merely countable: an operator
/// told "1 unreadable" with no filename has nothing to act on. The count
/// stays exact by being derived from the list rather than kept beside it.
#[test]
fn scan_session_names_damaged_cassettes_and_says_why() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    // `session_meta()` is the fixture this module's tests already share.
    let session = store.create_session(&session_meta()).expect("session");
    std::fs::write(
        store.cassettes_dir(&session).join("01JBADFRONTMATTER0000000AA.md"),
        "no frontmatter here at all\n",
    )
    .expect("write");

    let scan = store.scan_session(&session).expect("scan");

    assert_eq!(scan.unreadable(), 1);
    assert_eq!(scan.damaged.len(), 1, "the count is derived from the list");
    assert_eq!(
        scan.damaged[0].id.as_deref(),
        Some("01JBADFRONTMATTER0000000AA"),
        "recovered from the filename stem"
    );
    assert!(matches!(scan.damaged[0].reason, DamageReason::BadFrontmatter));
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

```rust
/// Why a cassette file could not be turned into a `StoredCassette`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReason {
    /// The file could not be read at all (permissions, I/O, bad UTF-8).
    Unreadable,
    /// It was read, but has no parseable frontmatter block.
    BadFrontmatter,
}

impl std::fmt::Display for DamageReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DamageReason::Unreadable => write!(f, "could not be read"),
            DamageReason::BadFrontmatter => write!(f, "frontmatter is unparseable"),
        }
    }
}

/// One cassette file that exists but could not be loaded.
#[derive(Debug, Clone)]
pub struct DamagedCassette {
    pub path: PathBuf,
    /// The ULID from the filename stem, where it looks like one.
    pub id: Option<String>,
    pub reason: DamageReason,
}
```

Replace `SessionScan`'s `pub unreadable: usize` with `pub damaged: Vec<DamagedCassette>` and add `pub fn unreadable(&self) -> usize { self.damaged.len() }`. `scan_session`'s two `unreadable += 1` branches push a `DamagedCassette` with the matching reason. Update the three readers (`stats.rs`, `find.rs`, `main.rs`) from `scan.unreadable` to `scan.unreadable()`.

`App` gains `pub damaged: Vec<(String, String)>` — label and reason as plain strings, since these are not cassettes and `App` holds no store types. `load_session_cassettes` fills it. `ui.rs` renders one greyed row each, below the closed row, never focusable (they are not in `cassettes`, so focus movement cannot reach them by construction).

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: damaged cassettes render as named error rows"
```

---

### Task 8: Documentation and end-to-end verification

**Files:**
- Modify: `README.md`, `CLAUDE.md`

**This is the task most likely to be quietly wrong.** 5b's docs task documented an accident rather than the code, and shipped an unreachable banner. **Generate every piece of command output rather than typing it, and check every prose claim against the source.**

- [ ] **Step 1: Update the docs**

`README.md`: cassettes appear in queue order; closed ones fold into one row toggled with `z` and open read-only; the cap is on open cassettes; damaged files show as named rows.

`CLAUDE.md`: revise the `src/cassette.rs`, `src/app.rs`, `src/ui.rs` and `src/store/` entries in the density of their neighbours. Several sentences are still accurate — read each before rewriting. The `src/app.rs` entry's `merge_external` description mentions `insert_at`, which no longer exists.

- [ ] **Step 2: Drive the real TUI**

Use `.claude/skills/verify`'s pty driver, with `CASSETTE_DATA_DIR` set to a temp path (required, not optional) and `cargo build` run first (`cargo test` does not rebuild the binary). The driver must answer `ESC[6n`.

Drive: a session with two open and three closed cassettes plus one deliberately corrupted file.
1. Confirm the open cassettes appear in priority order and the closed ones do not.
2. Confirm the `▸ 3 closed` row and the damaged row are both present.
3. Press `z`; confirm the row opens and the closed cassettes appear.
4. Tab onto a closed one; confirm `-- CLOSED --` and the reopen help text.
5. Press `z` again; confirm focus returns to an open cassette.

Put the transcript in your report.

- [ ] **Step 3: Full verification**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
# App and Cassette stay pure
grep -nE '^use ' src/app.rs src/cassette.rs
# The TUI still never writes priority or status
awk '/fn flush_held/,/^    }/' src/session_writer.rs | grep -E 'priority|status' || echo "flush writes neither"
# No new cassette writes
git diff main...HEAD -- src/ | grep -E '^\+.*(guard\.write|atomic_write)' || echo "none"
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "docs: document queue order, the closed fold, and damaged rows"
```

---

## Plan Self-Review

**Spec coverage.** Data model → T1; `ReadOnly` → T2; ordering and retiring `insert_at` → T3; the open-only cap and its live bug → T4; the fold with both pinned edge cases → T5; the closed row, `-- CLOSED --` and the help row → T6; damaged rows and `unreadable` as a method → T7; docs, pty and the three invariants → T8.

**Ordering.** T1 first — every later task uses the fields. T2 is independent of T1 but precedes T6, which renders `Closed`. T3, T4 and T5 each consume T1 and are mutually independent; they are sequenced to keep `app.rs` edits serial. T6 consumes T2 and T5. T7 is independent of everything and could move, but sits late so its reviewer sees the display already working.

**Type consistency.** `ReadOnly` (T2) is used in T5 and T6. `open_count`/`sort_queue` (T1) are used in T3, T4, T5. `stack_len` (T5) is used in T6. `merge_external(&str, Cassette)` is defined in T3 and called nowhere else. `closed_row_text` is defined and used only in T6. `DamageReason`/`DamagedCassette`/`unreadable()` are defined in T7 and used only there.

**Two places this plan leaves a judgment call to the implementer**, each with an instruction to report which was taken: whether `load_cassettes` clamps focus to the open set or to the whole list (T4), and where exactly `main.rs` sets `ReadOnly::Closed` on focus change (T6) — `follow_focus` is the natural home, but the acquire path may read better.

**Known risk.** T3 is the one that can lose data. It edits the merge path, and if its reviewer finds any path that merges a cassette whose lock this process holds, or that drops `merge_external`'s `debug_assert!(!existing.dirty)`, that is Critical rather than a style point.
