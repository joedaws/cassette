# Phase 5b — The TUI Reads What Others Write Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the TUI show what other writers are doing to the session it has open.

**Architecture:** `main.rs` stats the session's `cassettes/` directory on each event-loop tick and hands `App` any cassette whose file changed and whose lock this process does not hold; `App::merge_external` replaces or inserts it, placing the cursor by one rule that makes the viewport follow new text only when it was already at the end. The busy-cassette indicator becomes the spec's banner naming the holder, retried every tick rather than only on a keypress.

**Tech Stack:** Rust 2021, ratatui/crossterm, `fs4` advisory locks, `tempfile` in tests, a Python `pty.fork()` driver for TUI verification.

**Spec:** `docs/superpowers/specs/2026-09-18-tui-reads-others-design.md`, which argues from `docs/superpowers/specs/2026-09-13-session-store-design.md`.

## Global Constraints

- **The brief may be wrong.** Across Phases 2–5a the plans were wrong more than twenty times, and implementers caught nearly all of it — a nonexistent method, a refactor that would have deadlocked the suite, a signature that contradicted its own prose. Verify every claim here against the code and the spec before acting. Deviating is allowed and expected; say so in your report.
- **`App` and `Cassette` stay pure: no I/O, no `Store`, no ratatui types** (CLAUDE.md). All stat-and-read lives in `main.rs`.
- **`LockGuard::write` is the only code in the crate that writes a cassette file.** Nothing in this phase writes a cassette at all — 5b only reads.
- **Never merge a cassette whose lock this process holds.** We hold it precisely so nobody else can have changed it; re-reading it would be at best wasted work and at worst a way to lose the human's unsaved keystrokes.
- **Never hardcode a colour in `ui.rs`** — every colour flows through the active `Theme`.
- **`merge_external` never changes focus and never reorders under the cursor.** After it returns, the focused cassette must still be the one the user was typing in, and any held guard must still name that same cassette.
- **No sleeps and no polling in tests.** In `tests/lock.rs`, a child's lock acquisition is proven by output it writes AFTER acquiring, never by the parent's write to its stdin.
- **Any command that touches a cassette store must set `CASSETTE_DATA_DIR` to a temp path.** A subagent on 5a omitted it and wrote a real session into the user's store.
- **This crate is binary-only — there is no `[lib]` target.** `cargo test --lib <name>` does not work. Use `cargo test <name>`.
- `cargo fmt` clean and `cargo clippy --all-targets -- -D warnings` clean at every commit.

## File Structure

| File | Responsibility |
|---|---|
| `src/session_writer.rs` (modify) | Guard bound to a cassette **id** rather than an index |
| `src/cassette.rs` (modify) | `from_sides_with_cursor` — rebuild a cassette with the cursor at a chosen offset |
| `src/app.rs` (modify) | `merge_external`: replace or insert, apply the cursor rule, keep focus identity |
| `src/main.rs` (modify) | Live sync on the tick; per-tick lock retry |
| `src/ui.rs` (modify) | The busy banner; sticky-lock holder in the separator |
| `README.md`, `CLAUDE.md` (modify) | Documentation |

---

### Task 1: Bind the guard to a cassette id, not an index

**Files:**
- Modify: `src/session_writer.rs`, `src/main.rs`
- Test: `src/session_writer.rs`

**Interfaces:**
- Produces: `SessionWriter.guard: Option<LockGuard>` (the `usize` is gone)
- Produces: `SessionWriter::held_id(&self) -> Option<&str>`
- Removes: `SessionWriter::held_idx`

**Why this is first, and why it is not optional.** `SessionWriter.guard` is `Option<(usize, LockGuard)>` — the guard is bound to a **position in `app.cassettes`**. The next task makes that list grow at arbitrary positions when an agent creates a cassette. An insertion before the held index would silently repoint the guard at a different cassette, and the very next flush would write one cassette's text into another's file. Removing the index removes the whole class of bug before the code that could trigger it exists.

`LockGuard` already knows its own id — `LockGuard::id(&self) -> &str` exists in `src/store/lock.rs`. So the `usize` is redundant: where an index is needed, look it up by id.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_guard_survives_a_cassette_being_inserted_before_it() {
    // The guard must name a cassette, not a position. An agent creating a
    // cassette shifts every index after it; a guard bound to an index would
    // then write the held cassette's text into a different file.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let (mut app, session) = fixture(&store, 2);
    let mut w = SessionWriter::open(&store, &session, true, "w", "w");

    w.acquire(&mut app, 1).expect("hold cassette 1");
    let held = app.cassettes[1].id.clone();
    assert_eq!(w.held_id(), Some(held.as_str()));

    // Something inserts at the front; index 1 is now a different cassette.
    let mut newcomer = Cassette::new();
    newcomer.id = "aaa00000000000000000000000".to_string();
    app.cassettes.insert(0, newcomer);

    assert_eq!(
        w.held_id(),
        Some(held.as_str()),
        "the guard must still name the cassette it locked, not whatever now sits at index 1"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test the_guard_survives`
Expected: FAIL — `held_id` does not exist.

- [ ] **Step 3: Implement**

Change the field to `Option<LockGuard>` and replace `held_idx` with `held_id`. Every place that needed the index now resolves it: `app.cassettes.iter().position(|c| c.id == id)`. Where a lookup can fail — the cassette is gone from the list — treat it as "nothing to flush" rather than panicking; a cassette cannot actually be removed today, but a `panic!` in the event loop would take the user's session with it.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "refactor: bind the guard to a cassette id, not an index

Live sync makes the cassette list grow at arbitrary positions. A guard bound
to an index would silently repoint at a different cassette, and the next
flush would write one cassette's text into another's file."
```

---

### Task 2: `from_sides_with_cursor`

**Files:**
- Modify: `src/cassette.rs`
- Test: `src/cassette.rs`

**Interfaces:**
- Produces: `Cassette::from_sides_with_cursor(side_a: String, side_b: String, topic: Option<String>, cursor: usize) -> Cassette`

**Why it is separate:** the existing `from_sides` always leaves the cursor at the end, because it puts all of side A into the zipper's `left` half. The next task needs a cassette rebuilt with the cursor at a *given* offset, and that is a pure-data concern belonging beside the zipper rather than inside merge logic.

`cursor` is a **character** offset into side A, not a byte offset — `Cassette::cursor_pos` is `left.chars().count()`, and the crate counts characters everywhere. An offset past the end clamps to the end.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn from_sides_with_cursor_splits_side_a_at_a_character_offset() {
    let c = Cassette::from_sides_with_cursor("hello world".to_string(), String::new(), None, 5);
    assert_eq!(c.cursor_pos(), 5);
    assert_eq!(c.side_a_text(), "hello world");
}

#[test]
fn from_sides_with_cursor_clamps_past_the_end() {
    let c = Cassette::from_sides_with_cursor("short".to_string(), String::new(), None, 999);
    assert_eq!(c.cursor_pos(), 5, "an offset past the end lands at the end");
}

#[test]
fn from_sides_with_cursor_counts_characters_not_bytes() {
    // Multi-byte text is the case a byte offset gets wrong, and the crate
    // counts characters everywhere else (`cursor_pos` is chars().count()).
    let c = Cassette::from_sides_with_cursor("héllo".to_string(), String::new(), None, 2);
    assert_eq!(c.cursor_pos(), 2);
    assert_eq!(c.side_a_text(), "héllo");
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

Run: `cargo test from_sides_with_cursor` — FAIL, the function does not exist.

Split side A at the character offset into the zipper's `left` and `right`. Leave `from_sides` as it is; it has callers that want the end.

- [ ] **Step 3: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add Cassette::from_sides_with_cursor"
```

---

### Task 3: `App::merge_external`

**Files:**
- Modify: `src/app.rs`
- Test: `src/app.rs`

**Interfaces:**
- Consumes: `Cassette::from_sides_with_cursor` (Task 2)
- Produces: `App::merge_external(&mut self, id: &str, incoming: Cassette)`

**This task is pure data.** No I/O, no `Store`, no ratatui — `main.rs` does the reading and hands the result in. That is the crate's rule and the reason this is testable without a store.

**The cursor rule, which is the whole point:**
- If the cursor was at the **end** of the text, it moves to the new end — so watching an agent write follows the newest words.
- Otherwise it stays at its **character offset**, clamped — so scrolling up to read an earlier paragraph is not yanked away on the next update.

This costs no scroll bookkeeping: `ui.rs` derives `scroll_top` from `cursor_row` on every render, so placing the cursor is the whole of it and the viewport follows.

**Three invariants it must not break:**
1. **Focus identity.** After merging, `focus_idx` must point at the same cassette it did before, even when an insertion shifted indices.
2. **Never merge over unsaved edits.** A cassette this process holds cannot have been changed by anyone else, and `main.rs` will not offer one — but assert it rather than trusting the caller, the way `refresh_from_disk` does.
3. **Insertion position.** A cassette the list has never seen is inserted at its place in priority order, not appended.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn merging_follows_the_new_text_when_the_cursor_was_at_the_end() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes[0].id = "aaa00000000000000000000000".to_string();
    app.modify_focused(|c| c.insert_str("first"));
    app.clear_dirty(0);
    assert_eq!(app.cassettes[0].cursor_pos(), 5, "cursor at the end to start");

    let incoming = Cassette::from_sides("first and more".to_string(), String::new(), None);
    app.merge_external("aaa00000000000000000000000", incoming);

    assert_eq!(app.cassettes[0].side_a_text(), "first and more");
    assert_eq!(
        app.cassettes[0].cursor_pos(),
        14,
        "the cursor was at the end, so it follows the new end"
    );
}

#[test]
fn merging_leaves_a_scrolled_back_cursor_where_it_was() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes[0].id = "aaa00000000000000000000000".to_string();
    app.modify_focused(|c| c.insert_str("first"));
    app.modify_focused(|c| c.move_text_start());
    app.clear_dirty(0);
    assert_eq!(app.cassettes[0].cursor_pos(), 0);

    let incoming = Cassette::from_sides("first and more".to_string(), String::new(), None);
    app.merge_external("aaa00000000000000000000000", incoming);

    assert_eq!(
        app.cassettes[0].cursor_pos(),
        0,
        "a reader who scrolled up must not be yanked to the bottom"
    );
}

#[test]
fn an_unknown_cassette_is_inserted_without_moving_focus() {
    // An agent's `queue new` arrives. The human is typing in what is
    // currently index 0; after the insert they must still be typing in it.
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.cassettes[0].id = "bbb00000000000000000000000".to_string();
    app.focus_idx = 0;
    let focused_id = app.cassettes[0].id.clone();

    let mut newcomer = Cassette::new();
    newcomer.id = "aaa00000000000000000000000".to_string();
    app.merge_external("aaa00000000000000000000000", newcomer);

    assert_eq!(app.cassettes.len(), 2);
    assert_eq!(
        app.cassettes[app.focus_idx].id, focused_id,
        "focus must still name the cassette the user was typing in"
    );
}
```

`merge_external` takes the incoming cassette's **priority** into account when inserting. `Cassette` does not carry a priority today — decide whether to add one or to have `main.rs` pass the insertion position, and **say which you chose and why in your report**. Adding a field to a pure type is cheap; passing a position keeps ordering knowledge in the layer that reads the store. Either is defensible; an unprincipled append is not.

- [ ] **Step 2: Run to verify they fail, then implement**

Run: `cargo test merge_external` — FAIL, the method does not exist.

- [ ] **Step 3: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add App::merge_external with the follow-if-at-end cursor rule

The viewport has no stored scroll offset — ui.rs derives scroll_top from
cursor_row — so placing the cursor is the whole of 'follow new text only if
I was already at the bottom'."
```

---

### Task 4: Live sync on the tick

**Files:**
- Modify: `src/main.rs`
- Test: `src/main.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: `App::merge_external` (Task 3), `SessionWriter::held_id` (Task 1)
- Produces: a `sync` step in the event loop, running once per tick

**What it does, per the spec:** stat the session's `cassettes/` directory; for any file whose **mtime has moved** and **whose lock this process does not hold**, re-read it, parse it, and call `app.merge_external(id, cassette)`.

**The exclusion is not an optimisation — it is the correctness rule.** This process holds the focused cassette's lock precisely so nobody else can have changed it. Re-reading it could only overwrite the human's unsaved keystrokes with what was last flushed. Skip it via `SessionWriter::held_id`, and skip it before reading, not after.

**Cadence:** poll on the **existing one-second tick**. The spec's "~500ms" is an illustration of how cheap stat-ing ten files is, not a requirement, and a second clock beside the timer, idle nudge, status flash and autosave is a cost with no benefit at this scale.

**When the session directory has vanished** (another process cleaned it up), sync **degrades silently** — keep the in-memory state and the guard, merge nothing. Erroring mid-sentence over a directory the user is still typing into is worse than showing stale neighbours. The same applies to a single unreadable or unparseable file: skip that file, keep the rest.

**Track mtimes per cassette id**, not per index. A `HashMap<String, SystemTime>` in the event loop is enough; the first sight of a file counts as changed.

- [ ] **Step 1: Write the failing test**

In `tests/cli.rs` — an integration test, because the behaviour is "the TUI notices what another process did", and that needs two processes:

```rust
#[test]
fn a_cassette_written_behind_the_tuis_back_is_visible_to_the_next_reader() {
    // Not a TUI test: this pins the store-side contract live sync depends on
    // — that an external write is observable by re-reading, with the mtime
    // moving. The TUI-side merge is unit-tested in app.rs, and the two
    // together are what Task 6 drives through a pty.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin()).args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root).output().expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin()).args(["queue", "new", "shared", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root).env("USER", "joseph").output().expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let path = std::fs::read_dir(root.join("sessions").join(&sid).join("cassettes"))
        .expect("read cassettes dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains(&cid))
        .expect("the cassette file");
    let before = std::fs::metadata(&path).expect("stat").modified().expect("mtime");

    let mut child = Command::new(bin())
        .args(["queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root).env("USER", "joseph")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
        .spawn().expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"agent words\n").expect("write");
    assert!(child.wait().expect("wait").success());

    let after = std::fs::metadata(&path).expect("stat").modified().expect("mtime");
    assert!(after >= before, "the write must move the mtime live sync watches");
    let body = std::fs::read_to_string(&path).expect("read");
    assert!(body.contains("agent words"), "{body}");
}
```

- [ ] **Step 2: Run to verify it fails, then implement the sync step**

Run: `cargo test a_cassette_written_behind` — this one may **pass immediately**, since it pins store behaviour rather than new code. If it does, say so in your report and do not manufacture a failure; its job is to keep that contract from regressing under you.

Write the sync step in the event loop, after the tick block that already drives `tick_timer`/`tick_status`/`tick_idle`.

- [ ] **Step 3: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: live-sync cassettes other writers change

The held cassette is skipped before reading, not after: this process holds
its lock so nobody else can have changed it, and re-reading could only
overwrite the human's unsaved keystrokes."
```

---

### Task 5: The busy banner, per-tick retry, and sticky indicators

**Files:**
- Modify: `src/main.rs`, `src/ui.rs`, `src/app.rs`
- Test: `src/ui.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `SessionWriter::held_id` (Task 1)
- Produces: `App.busy_holder: Option<String>` — the name to show while read-only

**Three changes, all display or timing, none touching what is written:**

1. **The banner.** 5a shows a bare `-- READ ONLY --`. The spec asks for the holder: `open by refactor-agent`. The name comes from the lock anchor's attribution — the same source `queue write`'s exit-3 message uses — resolved to a writer name with the raw id as fallback, exactly as `queue::view::build_view` does for `sticky_lock`. Follow that, do not invent a second resolution path.
2. **Retry every tick.** 5a retries acquisition on each keypress via `follow_focus`. Add the tick, so a human who walks away from a cassette an agent holds finds it editable on return without typing a character to discover it. When the retry succeeds, 5a's `refresh_from_disk` already runs inside `acquire`, so the agent's final text is there.
3. **Sticky-lock indicators.** A cassette whose `locked_by` is set names its holder in the separator, beside the existing side tag and topic.

**The display must distinguish two states that look similar and mean opposite things.** A **busy** cassette is transiently held by a live process and will free itself — wait. A **sticky-locked** one is a durable claim only a human can clear — go and unlock it. The parent spec gives them different exit codes (3 and 4) for exactly this reason, and a display that blurs them sends the user to wait forever.

**Never hardcode a colour** — everything flows through the active `Theme`. Follow how `-- RECORD --` and 5a's `-- READ ONLY --` are rendered in `src/ui.rs` rather than inventing a style.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_busy_indicator_names_the_holder() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.read_only = true;
    app.busy_holder = Some("refactor-agent".to_string());
    let line = crate::ui::info_text(&app);
    assert!(line.contains("refactor-agent"), "the user must know WHO holds it: {line}");
}

#[test]
fn a_busy_cassette_and_a_sticky_one_do_not_read_the_same() {
    // Busy means wait; sticky means go and unlock it. A display that blurs
    // them sends the user to wait for something that will never free itself.
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.read_only = true;
    app.busy_holder = Some("refactor-agent".to_string());
    let busy = crate::ui::info_text(&app);

    app.read_only = false;
    app.busy_holder = None;
    app.cassettes[0].locked_by = Some("joseph".to_string());
    let sticky = crate::ui::separator_text(&app, 0);

    assert_ne!(busy, sticky);
    assert!(sticky.contains("joseph"), "{sticky}");
}
```

`ui.rs` has no `info_text`/`separator_text` helpers today — the strings are built inline inside `render`. **Extract them as pure functions returning `String`**, so a test can assert on their content without a terminal.

The precedent to follow for *purity* is `help_line` (`src/ui.rs:142`, private, pure, tested at `:622`) — but note it returns a ratatui `Line` because it applies per-span styling. Do **not** copy that return type here: a `Line` makes `assert!(line.contains(…))` awkward, and the tests above are written against a `String`. Return the text; keep the styling in `render`, where it already lives.

Also note `Cassette` has no `locked_by` field; adding one is a pure-data change. If you prefer to carry it on `App` instead, say which you chose and why.

- [ ] **Step 2: Run to verify they fail, then implement**

Run: `cargo test busy_indicator` — FAIL, neither the helpers nor the fields exist.

- [ ] **Step 3: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: name the holder, retry each tick, show sticky claims

Busy and sticky look similar and mean opposite things: one frees itself,
the other needs a human. The display has to tell them apart."
```

---

### Task 6: Documentation and end-to-end verification

**Files:**
- Modify: `README.md`, `CLAUDE.md`

**This is the task most likely to be quietly wrong.** Phase 4b shipped a README `--help` transcript that had silently dropped a clause, and 5a's docs task caught itself asserting a cap the TUI does not enforce. **Generate every piece of command output rather than typing it**, and check every prose claim against the code.

- [ ] **Step 1: Update the docs**

`README.md`: the TUI now shows other writers' changes as they happen; a cassette an agent holds opens read-only and names the holder; it becomes editable by itself when released; cassettes an agent creates appear in the stack; a sticky-locked cassette names its claimant in the separator.

`CLAUDE.md`: revise the `src/app.rs`, `src/main.rs` and `src/ui.rs` entries for live sync, `merge_external` and the banner, in the style and density of their neighbours. Read each entry first — several sentences are still accurate.

- [ ] **Step 2: Drive the real TUI**

Use `.claude/skills/verify`'s pty driver. **Set `CASSETTE_DATA_DIR` to a temp path** — the skill now documents this as required, after a run without it wrote into the user's real store during 5a. Two other hazards: the driver must answer `ESC[6n`, and **`cargo test` does not rebuild the binary**, so run `cargo build` first.

Drive this, which is the phase in one scenario:
1. Launch the TUI on a session with two cassettes; type in the first.
2. From a second process, `queue write` the **second** cassette.
3. Confirm the TUI's minimized row for it updates within a tick, **without** the first cassette's text or cursor moving.
4. From a second process, `queue new` a third cassette; confirm it appears and focus has not moved.
5. Have the second process hold the second cassette's lock; focus it in the TUI; confirm it is read-only and names the holder.
6. Release the lock; confirm the TUI becomes editable **without a keypress**, and shows the agent's text.

Put the transcript in your report.

- [ ] **Step 3: Full verification**

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Then confirm the invariants:

```bash
# Nothing in this phase writes a cassette
git diff main...HEAD -- src/ | grep -E '^\+.*(guard\.write|atomic_write)' || echo "no new cassette writes"
# App stays pure
grep -nE '^use ' src/app.rs src/cassette.rs
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "docs: document live sync and the busy banner"
```

---

## Plan Self-Review

**Spec coverage.** Live sync and its cadence → Task 4; the cursor rule → Tasks 2 and 3; new cassettes appearing → Task 3; the banner and per-tick retry → Task 5; sticky indicators → Task 5; the vanished-directory rule → Task 4; docs and pty → Task 6. The guard-by-index hazard the spec names → Task 1.

**Ordering.** Task 1 precedes everything, because Task 3 makes the list grow and a guard bound to an index would silently repoint. Task 2 precedes Task 3, which consumes it. Task 4 consumes Tasks 1 and 3. Task 5 is independent of 3 and 4 but sits after them so its reviewer sees the merge already working.

**Type consistency.** `held_id` replaces `held_idx` in Task 1 and is used in Tasks 4 and 5. `from_sides_with_cursor` is defined in Task 2 and used in Task 3. `merge_external(&str, Cassette)` is defined in Task 3 and called in Task 4. `busy_holder` is introduced in Task 5 and used only there.

**Three places this plan leaves a judgment call to the implementer**, each with an instruction to report which was taken: whether `Cassette` gains a priority field or `main.rs` passes an insertion position (Task 3); whether `locked_by` lives on `Cassette` or `App` (Task 5); and what to do when Task 4's integration test passes on arrival, since it pins existing store behaviour rather than new code.

**Known risk.** Task 4 is the one that can lose data if it gets the exclusion wrong. If its reviewer finds any path that merges a cassette whose lock this process holds, that is Critical, not a style point — it would overwrite unsaved keystrokes with the last flush.
