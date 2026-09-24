# Phase 5d — Sessions Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cassette sessions` opens an interactive list of recent sessions; Enter opens the highlighted one in the TUI.

**Architecture:** `find::scan_store` is shared rather than copied, so there is one answer to "what sessions exist". `Picker` (new `src/picker.rs`) is a pure state machine over the scanned entries — cursor, recent/all toggle, filter text — with no `Store`, no ratatui and no `std::fs`, testable without a terminal exactly as `App` is. `ui.rs` renders it; `main.rs` runs the event loop and hands the chosen id to the ordinary TUI path.

**Tech Stack:** Rust, ratatui + crossterm, clap v4 derive. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-23-sessions-picker-design.md`

## Global Constraints

- **`Picker` stays pure.** No I/O, no ratatui types, no `Store`. Verified by `grep -nE '^use ' src/picker.rs` — `crate::find::NoteEntry` and std only.
- **Rendering colours come from the active `Theme`.** Never hardcode a colour in `ui.rs`.
- **Sequence tasks by producer-and-consumer, not by file** (CLAUDE.md). Clippy runs `-D warnings`, so a task that adds an API with no caller fails its own "ends green" gate on dead code. Each task below carries its own consumer.
- Every task ends green: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Any command that spawns the `cassette` binary sets `CASSETTE_DATA_DIR` to a temp path. Without it, tests read and write the user's real store. This has happened before.

## File Structure

| File | Responsibility |
|---|---|
| `src/find.rs` | `NoteEntry.path` → `id`; gains `alias`; `scan_store`/`build_haystack` become `pub(crate)`; `render` prints the alias |
| `src/picker.rs` | **new** — `Picker`: pure cursor/filter/toggle state over `Vec<NoteEntry>` |
| `src/ui.rs` | `render_picker` |
| `src/cli.rs` | the `Sessions` subcommand |
| `src/main.rs` | `run_picker` event loop and the handoff to the TUI |

---

### Task 1: Share the scanner, and print the alias it already matches

**Files:**
- Modify: `src/find.rs`, `src/main.rs` (the `find` caller)

**Interfaces:**
- Produces: `pub(crate) find::scan_store`, `pub(crate) find::build_haystack`, `NoteEntry { id, alias, date, words, topics, preview }`
- Consumed by: Tasks 2 and 4

`NoteEntry.path` has held a session id since 5a. `find` already matches on alias via `build_haystack` but never prints it, so a row can match a query for a word the user cannot see — both are on the carried triage list and this is the phase whose work touches them.

- [ ] **Step 1: Write the failing test**

In `src/find.rs`'s `mod tests`:

```rust
/// `find` has always MATCHED on alias through `build_haystack`; not
/// printing it meant a row could match a query for a word nowhere on
/// screen, which reads as a bug in the filter.
#[test]
fn a_row_prints_the_alias_it_can_be_found_by() {
    let e = NoteEntry {
        id: "01M3TEST0000000000000000AA".to_string(),
        alias: Some("morning".to_string()),
        date: NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap(),
        words: 120,
        topics: vec!["gratitude".to_string()],
        preview: "today I".to_string(),
    };
    let out = render(&[e], Some("morning"), 0);
    assert!(out.contains("morning"), "the matched alias must be visible: {out}");
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test --bin cassette a_row_prints_the_alias` — FAIL (no field `id`/`alias`).

Rename the field `path` → `id` throughout `find.rs` and at its `main.rs` call site. Add `pub alias: Option<String>` to `NoteEntry`; `scan_store` already reads `session.toml` for the date, so the alias is in hand there. Print it in `render` — alias where set, id otherwise, matching what the picker will show. Make `scan_store` and `build_haystack` `pub(crate)`.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "refactor: share find's scanner, and print the alias it matches on"
```

---

### Task 2: `Picker`, the pure state machine

**Files:**
- Create: `src/picker.rs`
- Modify: `src/main.rs` (`mod picker;`)

**Interfaces:**
- Consumes: Task 1's `NoteEntry`
- Produces: `Picker::new(entries: Vec<NoteEntry>)`, `visible(&self) -> Vec<&NoteEntry>`, `move_down/move_up`, `toggle_all`, `start_filter/push_filter/pop_filter/end_filter`, `selected(&self) -> Option<&NoteEntry>`, `filtering: bool`, `query: String`, `show_all: bool`

**This task carries its own consumer** (its tests). It is the only task whose API has no production caller until Task 4, and unit tests are what keep clippy quiet about it — `Picker`'s methods are `pub(crate)` and exercised by `mod tests` in the same file.

- [ ] **Step 1: Write the failing tests**

```rust
fn entry(id: &str, alias: Option<&str>, topic: &str) -> NoteEntry {
    NoteEntry {
        id: id.to_string(),
        alias: alias.map(|a| a.to_string()),
        date: NaiveDate::from_ymd_opt(2026, 9, 23).unwrap().and_hms_opt(8, 0, 0).unwrap(),
        words: 10,
        topics: vec![topic.to_string()],
        preview: String::new(),
    }
}

/// A list is not a carousel: wrapping past the end of 200 sessions is
/// disorienting, so movement clamps at both ends.
#[test]
fn movement_clamps_rather_than_wrapping() {
    let mut p = Picker::new(vec![entry("a", None, "x"), entry("b", None, "y")]);
    p.move_up();
    assert_eq!(p.cursor, 0, "already at the top");
    p.move_down();
    p.move_down();
    p.move_down();
    assert_eq!(p.cursor, 1, "stops at the last row");
}

/// The `a` toggle changes how many rows are shown; the session the human
/// was looking at must still be the one highlighted, by id — the same rule
/// `App::sort_queue` follows when the list moves under the cursor.
#[test]
fn toggling_all_keeps_the_same_session_highlighted() {
    let entries: Vec<NoteEntry> = (0..20)
        .map(|i| entry(&format!("id{i:02}"), None, "t"))
        .collect();
    let mut p = Picker::new(entries);
    assert_eq!(p.visible().len(), DEFAULT_LIST_LIMIT, "recent by default");
    p.move_down();
    p.move_down();
    let before = p.selected().expect("a row").id.clone();

    p.toggle_all();

    assert_eq!(p.visible().len(), 20, "all of them now");
    assert_eq!(p.selected().expect("a row").id, before, "same session");
}

/// Filtering narrows the list live, and a filter matching nothing must
/// leave the cursor valid rather than pointing past the end.
#[test]
fn filtering_narrows_and_leaves_the_cursor_valid() {
    let mut p = Picker::new(vec![
        entry("a", Some("morning"), "gratitude"),
        entry("b", Some("evening"), "review"),
    ]);
    p.start_filter();
    for c in "morn".chars() {
        p.push_filter(c);
    }
    assert_eq!(p.visible().len(), 1);
    assert_eq!(p.selected().expect("a row").alias.as_deref(), Some("morning"));

    for c in "zzz".chars() {
        p.push_filter(c);
    }
    assert!(p.visible().is_empty(), "nothing matches");
    assert!(p.selected().is_none(), "and nothing is selected, rather than panicking");
}

/// An empty store is a normal state, not an error and not a panic.
#[test]
fn an_empty_picker_selects_nothing() {
    let p = Picker::new(Vec::new());
    assert!(p.visible().is_empty());
    assert!(p.selected().is_none());
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

`Picker` holds `entries`, `cursor`, `query`, `filtering`, `show_all`. `visible()` derives the row list each call — filter first, then the `show_all`/`DEFAULT_LIST_LIMIT` cap — rather than keeping a second `Vec` in sync. Clamp `cursor` in the methods that can invalidate it (`push_filter`, `pop_filter`, `toggle_all`). `toggle_all` records the selected id before and restores it after, falling back to clamping when it is no longer visible.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: add Picker, a pure state machine for the sessions list"
```

---

### Task 3: Render it

**Files:**
- Modify: `src/ui.rs`

**Interfaces:**
- Consumes: Task 2's `Picker`
- Produces: `pub(crate) fn render_picker(frame: &mut Frame, picker: &Picker, theme: &Theme)`

- [ ] **Step 1: Write the failing test**

```rust
/// 5c shipped two display bugs a green suite could not see. The fixture
/// must make rows distinguishable ON SCREEN — a row whose alias and topic
/// are absent proves nothing by being absent.
#[test]
fn the_picker_draws_rows_and_highlights_the_cursor() {
    let mut p = Picker::new(vec![
        entry("01M3AAA0000000000000000AAA", Some("morning"), "gratitude"),
        entry("01M3BBB0000000000000000BBB", Some("evening"), "review"),
    ]);
    p.move_down();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render_picker(f, &p, &Theme::default())).unwrap();
    let buf = terminal.backend().buffer();
    let rows: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect())
        .collect();
    let screen = rows.join("\n");

    assert!(screen.contains("morning"), "{screen}");
    assert!(screen.contains("evening"), "{screen}");
    let cursor_row = rows.iter().position(|r| r.contains("evening")).expect("row");
    assert!(
        rows[cursor_row].starts_with('>') || buf[(0, cursor_row as u16)].symbol() == ">",
        "the highlighted row is marked: {:?}",
        rows[cursor_row]
    );
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

A title line, one line per visible row (`> ` marker on the cursor row; date, alias-or-id, words, topics), a filter line when `filtering`, an `N unreadable` footer when non-zero, and a key hint row. Empty list renders "no sessions yet". All colours from `Theme`.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: render the sessions picker"
```

---

### Task 4: Wire it up — subcommand, event loop, handoff

**Files:**
- Modify: `src/cli.rs`, `src/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: Tasks 1–3
- Produces: `cassette sessions`

- [ ] **Step 1: Write the failing test**

In `tests/cli.rs`:

```rust
#[test]
fn sessions_help_parses() {
    let out = run(&["sessions", "--help"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("pick a session"), "{text}");
}

#[test]
fn sessions_rejects_an_unexpected_argument() {
    let out = run(&["sessions", "nope"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

Add `Sessions` to `cli::Command` (doc comment: "pick a session to open from a list") and an `Args` field. In `main.rs`, when it is set: scan the store, build the `Picker`, run `run_picker` — terminal setup, alternate screen, the crossterm loop dispatching keys to `Picker`, teardown through the existing panic-hook-and-restore path — and on `Enter` fall through to the ordinary TUI on the returned id. On quit, exit 0 having done nothing.

**Modal filter:** while `picker.filtering`, character keys go to `push_filter` and only `Esc`/`Enter` leave the mode — so `q` types a `q`. This mirrors `Mode::Topic` owning the keyboard in the main TUI.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: cassette sessions opens a session picker"
```

---

### Task 5: Documentation and end-to-end verification

**Files:**
- Modify: `README.md`, `CLAUDE.md`

**Generate every piece of command output rather than typing it, and check every prose claim against the source.** 5b's docs task documented an accident; 5c's pty drive caught two live bugs an hour before merge.

- [ ] **Step 1: Update the docs**

`README.md`: `cassette sessions` and its keys, near `find`/`resume`. `CLAUDE.md`: a `src/picker.rs` entry in the density of its neighbours, plus the `find.rs` entry's `path` → `id` and alias change, and the new subcommand in `src/cli.rs`'s entry.

- [ ] **Step 2: Drive the real TUI**

`.claude/skills/verify`'s pty driver, `CASSETTE_DATA_DIR` set to a temp path, `cargo build` first, and answer `ESC[6n`. **Force a full repaint (resize) before asserting** — ratatui diff-renders, and 5c nearly lost a real bug to a stale diff capture.

1. Several sessions in the store, some aliased. Launch `cassette sessions`.
2. Confirm rows show aliases and dates, newest first.
3. `j` twice; confirm the highlight moves.
4. `/`, type a filter; confirm the list narrows and that `q` types rather than quits.
5. `Esc`, then `Enter`; confirm the TUI opens on the highlighted session with its cassettes.

Put the transcript in your report.

- [ ] **Step 3: Full verification**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
grep -nE '^use ' src/picker.rs        # Picker stays pure
git diff main...HEAD -- src/ | grep -E '^\+.*(guard\.write|atomic_write)' || echo "none"
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "docs: document the sessions picker"
```

---

## Plan Self-Review

**Spec coverage.** Sharing the scanner and the two triage fixes → T1; the pure state machine, clamping, the by-id toggle and the empty case → T2; rendering → T3; the subcommand, the modal filter and the handoff → T4; docs and pty → T5.

**Ordering.** T1 first — T2 and T4 both consume `NoteEntry`'s new shape. T2 before T3 (which renders it) and T4 (which drives it). T3 before T4 so the event loop has something to draw.

**Type consistency.** `NoteEntry { id, alias, … }` is defined in T1 and used in T2, T3, T4. `Picker`'s methods are defined in T2 and called in T3 and T4. `render_picker` is defined in T3 and called in T4.

**Producer-and-consumer sequencing.** T2 is the one task whose API has no *production* caller until T4; its unit tests are the consumer that keeps `-D warnings` quiet. If clippy still objects to an unused method, fold that method into T4 rather than reaching for `#[allow(dead_code)]` — the rule CLAUDE.md now carries.

**Known risk.** T4 owns terminal setup and teardown. A picker that panics or takes an early return without restoring the terminal leaves the user's shell in raw mode — the existing panic hook and `catch_unwind` must cover this path too, not just the TUI's.
