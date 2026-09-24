# Reader Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the human keep a cassette focused and full-height while releasing its lock, so other writers can write it and the human watches it change. Then optionally release automatically after idle time in normal mode.

**Architecture:** A fourth `ReadOnly` variant, `Reading { id }`, entered with `r` in normal mode. `SessionWriter::release` flushes, then drops the guard. The existing live sync already merges any unheld cassette, focused or not. Cursor motions move off the read-only gate onto a new `App::view_focused` that never sets `dirty`, so a reader can scroll without breaking the re-read-on-acquire invariant. Entering insert mode from Reading takes the lock back and replays the key. Phase 2 (Task 4) adds `release_idle_secs` to config.

**Tech Stack:** Rust, ratatui/crossterm; unit tests in-module; the `verify` skill for the end-to-end screen check.

**Spec:** `docs/superpowers/specs/2026-09-24-reader-mode-design.md`

## Global Constraints

- `App` stays pure: key handlers only set flags (`release_requested`, `replay_after_acquire`). All store work happens in `main.rs`'s `follow_focus` / tick.
- An unheld cassette must never be `dirty`. Nothing reachable in Reading may call `modify_focused` successfully.
- Flush before drop, always (`release` = `flush_held` then `guard = None`).
- Banner text, verbatim: `-- READING (open to writers) --`.
- Help text, verbatim: `reading: others may write here  i/a/o:write  r:hold again  hjkl:scroll  Tab:next  ^C:quit`.
- Idle release never fires in insert mode or record mode, and is off unless `release_idle_secs` is set.
- Clippy `-D warnings` green after every task. Each task below ships a producer together with its consumer.

## Review Focus

- **`r` pressed with unsaved keystrokes.** They must reach disk before the lock drops. Task 2 test: type, `r`, read the file.
- **External write lands while Reading with the cursor mid-text.** The cursor stays put and the text updates. Task 2 test via `merge_external`, the existing rule.
- **`i` while Reading, lock busy.** No keystroke is swallowed, the banner goes to `READ ONLY (open by …)`, the mode stays Normal, and `retry_lock` then retries it. That retry is a Busy retry, which is fine: the human asked to write. Task 3 test.
- **Ctrl+B flip while Reading.** Allowed as a view op, never dirty. Task 1 test.
- **Tab away from Reading while the next cassette is busy.** Reading ends and the new cassette is Busy. No guard should be left dangling. Task 2 test.

---

### Task 1: Motion is not an edit (`App::view_focused`)

**Files:**
- Modify: `src/app.rs` (add `view_focused` beside `modify_focused` ~line 613; tests)
- Modify: `src/main.rs` (`handle_normal_key` motions ~1590–1621, arrow keys; Ctrl+B flip ~1394)

**Interfaces:**
- Produces: `pub fn view_focused<F: FnOnce(&mut Cassette)>(&mut self, f: F)`.

- [ ] **Step 1: Failing tests** (`src/app.rs` tests)

```rust
    #[test]
    fn view_focused_moves_the_cursor_on_a_read_only_cassette_without_dirtying_it() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.modify_focused(|c| c.insert_str("hello"));
        app.clear_dirty(0);
        app.read_only = ReadOnly::Busy { holder: None };

        app.view_focused(|c| c.move_left());

        assert_eq!(app.cassettes[0].cursor_pos(), 4, "motion works while read-only");
        assert!(!app.cassettes[0].dirty, "a view never marks the cassette dirty");
    }

    #[test]
    fn view_focused_flip_is_allowed_while_closed() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = ReadOnly::Closed;
        app.view_focused(|c| c.flip());
        assert_eq!(app.cassettes[0].side, Side::B);
        assert!(!app.cassettes[0].dirty);
    }
```

- [ ] **Step 2: Run** `cargo test --bin cassette view_focused`. Expected: compile error.

- [ ] **Step 3: Implement** (`src/app.rs`, after `modify_focused`)

```rust
    /// Apply a cursor-only operation (motion, side flip) to the focused
    /// cassette. Unlike `modify_focused` this ignores `read_only` and never
    /// sets `dirty`. A motion writes nothing, so there is no reason to
    /// forbid scrolling a cassette you cannot write. And a dirty flag on a
    /// cassette whose lock is not held would make `refresh_from_disk` keep
    /// the stale copy on the next acquire. Callers must only pass closures
    /// that leave the text unchanged.
    pub fn view_focused<F: FnOnce(&mut Cassette)>(&mut self, f: F) {
        if let Some(c) = self.cassettes.get_mut(self.focus_idx) {
            f(c);
        }
    }
```

In `src/main.rs`, switch `modify_focused` to `view_focused` for: `'h' 'l' 'j' 'k' '0' '$' 'w' 'b' 'G'`, `('g','g')`, `KeyCode::Left/Right/Up/Down` in `handle_normal_key`, and the Ctrl+B/Shift+Enter flip at ~line 1394. Leave every `snapshot()`-bearing arm and every text edit on `modify_focused`. **Leave insert-mode arrow keys on `modify_focused`**: in insert mode the cassette is held, and changing them buys nothing.

Flip previously marked the cassette dirty, which caused a (harmless) flush of unchanged text. Check that `session_writer`'s flush tests don't depend on that: `grep -n "flip" src/session_writer.rs`.

- [ ] **Step 4: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. Existing tests that asserted motions are *ignored* while read-only (`grep -n "read_only" src/main.rs | grep -i test`) now fail. Flip each one to assert the motion happens and `dirty` stays false. Say so in the commit.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: cursor motion and side flip work on read-only cassettes and never dirty them"
```

---

### Task 2: `r` releases to Reading; Reading syncs; Tab ends it

**Files:**
- Modify: `src/app.rs` (`ReadOnly::Reading { id }` ~line 41; `pub release_requested: bool` beside `suspend`)
- Modify: `src/session_writer.rs` (`pub fn release`)
- Modify: `src/main.rs` (`handle_normal_key` `'r'`; `follow_focus` ~1092)
- Modify: `src/ui.rs` (`info_text` ~313, `help_text` ~138)
- Test: `src/session_writer.rs`, `src/main.rs`, `src/ui.rs` tests

**Interfaces:**
- Consumes: `App::view_focused` (Task 1).
- Produces: `ReadOnly::Reading { id: String }`; `App.release_requested: bool`; `SessionWriter::release(&mut self, app: &mut App) -> Result<(), LockError>`.

- [ ] **Step 1: Failing tests**

`src/session_writer.rs` (uses existing `fixture`, `bin_path`):
```rust
    #[test]
    fn release_flushes_then_frees_the_lock_for_another_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire");
        app.modify_focused(|c| c.insert_str("unsaved words"));
        let id = app.cassettes[0].id.clone();

        w.release(&mut app).expect("release");

        assert!(w.held_id().is_none());
        assert!(!store.holds(&session, &id));
        let scan = store.scan_session(&session).expect("scan");
        assert!(scan.cassettes[0].body.contains("unsaved words"), "flushed before the drop");

        let mut agent = std::process::Command::new(bin_path())
            .args(["queue", "write", &id, "--session", &session])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "agent")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("spawn");
        agent.stdin.take().expect("stdin").write_all(b"agent\n").expect("write");
        assert!(agent.wait().expect("wait").success(), "the released cassette is writable");
    }
```

`src/main.rs` tests (the existing `follow_focus` tests build a real store and writer; follow their setup):
```rust
    #[test]
    fn r_releases_the_focused_cassette_and_follow_focus_does_not_retake_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let (mut app, session) = /* same fixture the other follow_focus tests use */;
        let mut w = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        follow_focus(&mut app, Some(&mut w));
        assert!(w.held_id().is_some());

        app.mode = Mode::Normal;
        handle_key(&mut app, key(KeyCode::Char('r'), KeyModifiers::NONE));
        follow_focus(&mut app, Some(&mut w));
        assert!(w.held_id().is_none());
        assert!(matches!(app.read_only, app::ReadOnly::Reading { .. }));

        // Any later key (here, a motion) must not quietly retake the lock.
        handle_key(&mut app, key(KeyCode::Char('j'), KeyModifiers::NONE));
        follow_focus(&mut app, Some(&mut w));
        assert!(w.held_id().is_none(), "reading is not undone by scrolling");

        // `r` again toggles back to holding.
        handle_key(&mut app, key(KeyCode::Char('r'), KeyModifiers::NONE));
        follow_focus(&mut app, Some(&mut w));
        assert!(w.held_id().is_some());
        assert_eq!(app.read_only, app::ReadOnly::No);
    }

    #[test]
    fn leaving_a_reading_cassette_ends_reading() {
        // two cassettes; Reading on 0; Tab; follow_focus holds 1 and read_only is No.
    }

    #[test]
    fn a_reading_cassette_is_merged_by_sync_like_any_unheld_one() {
        // Reading on 0; queue write it from `bin()`; touch_forward; sync_external_writes;
        // assert app.cassettes[0].side_a_text() contains the agent's words.
    }
```
Write the last two out in full, following the neighbouring `follow_focus`/`sync_external_writes` tests (~lines 2100–2260: `touch_forward`, `HashMap::new()` stamps). The skeletons above name the exact assertions.

`src/ui.rs` tests:
```rust
    #[test]
    fn reading_has_its_own_banner_and_help() {
        let mut app = /* the fixture other info_text tests use */;
        app.read_only = ReadOnly::Reading { id: "x".into() };
        assert!(info_text(&app).contains("-- READING (open to writers) --"));
        assert!(help_text(&app).starts_with("reading: others may write here"));
    }
```

- [ ] **Step 2: Run.** Expected: compile errors (`Reading`, `release`, `release_requested`).

- [ ] **Step 3: Implement**

`src/app.rs`:
```rust
    /// The human released this cassette's lock on purpose (`r`) to watch
    /// another writer work in it. Voluntary and undone by a key, unlike
    /// `Busy` (wait) or `Closed` (reopen). `id` pins it to the cassette it
    /// was entered on, so `follow_focus` can tell focus moved without a held
    /// guard to compare against.
    Reading { id: String },
```
Add `#[derive(PartialEq, Eq)]` to `ReadOnly` if it isn't there already. Add `pub release_requested: bool` (default `false`) with a doc comment: "One-shot `r` request, consumed by `main.rs`'s `follow_focus`, like `suspend`."

`src/main.rs` `handle_normal_key`, in the `match c` block:
```rust
                'r' => app.release_requested = true,
```

`src/session_writer.rs`:
```rust
    /// Stop holding the focused cassette's lock while it stays focused
    /// (reader mode). Flush through the guard first, drop it second. That
    /// is the same order `acquire` uses when focus moves, for the same
    /// reason.
    pub fn release(&mut self, app: &mut App) -> Result<(), LockError> {
        self.flush_held(app)?;
        self.guard = None;
        Ok(())
    }
```

`src/main.rs` `follow_focus`, inserted after `create_missing_cassettes` and **before** the closed check:
```rust
    let focused_id = app.cassettes.get(app.focus_idx).map(|c| c.id.clone());
    if std::mem::take(&mut app.release_requested) {
        match (&app.read_only, focused_id) {
            (app::ReadOnly::Reading { .. }, _) => {
                // `r` again: hold it again.
                try_acquire(app, w, app.focus_idx);
                return;
            }
            (app::ReadOnly::No, Some(id)) if w.held_id() == Some(id.as_str()) => {
                if let Err(e) = w.release(app) {
                    app.flash(format!("cannot release: {e}"));
                    return;
                }
                app.read_only = app::ReadOnly::Reading { id };
                return;
            }
            // Busy or Closed: there is nothing of ours to release.
            _ => {}
        }
    }
    if let app::ReadOnly::Reading { id } = &app.read_only {
        if app.cassettes.get(app.focus_idx).is_some_and(|c| &c.id == id) {
            return;
        }
        // Focus moved: reading described that cassette, not the session.
        app.read_only = app::ReadOnly::No;
    }
```
Check `App::flash`'s signature (`grep -n "pub fn flash" src/app.rs`) and adapt the call.

`src/ui.rs`: add `ReadOnly::Reading { .. } => "-- READING (open to writers) --".to_string(),` to `info_text`'s read-only match. In `help_text`, add an arm **before** the generic `is_read_only()` arm:
```rust
        _ if matches!(app.read_only, ReadOnly::Reading { .. }) => {
            "reading: others may write here  i/a/o:write  r:hold again  hjkl:scroll  Tab:next  ^C:quit"
        }
```
`idle_nudge` is already suppressed while read-only, so it stays quiet here. Watching isn't idling.

- [ ] **Step 4: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. Expect green.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/session_writer.rs src/main.rs src/ui.rs
git commit -m "feat: r releases the focused cassette's lock to read it while others write"
```

---

### Task 3: Intent to write (`i a I A o O` from Reading)

**Files:**
- Modify: `src/app.rs` (`pub replay_after_acquire: Option<crossterm::event::KeyEvent>`). If `App` must stay free of crossterm types, store `Option<char>` instead, since all six intent keys are plain chars. **Use `Option<char>`.**
- Modify: `src/main.rs` (`handle_normal_key` intent arms; `follow_focus`)

**Interfaces:**
- Consumes: `ReadOnly::Reading`, `follow_focus`'s Reading branch (Task 2).
- Produces: `App.replay_after_acquire: Option<char>`.

- [ ] **Step 1: Failing tests** (`src/main.rs` tests, same fixture as Task 2)

```rust
    #[test]
    fn a_from_reading_takes_the_lock_and_replays_as_a() {
        // hold 0 with text "hello", cursor at 2 (via view_focused moves); r; follow_focus;
        // press 'a'; follow_focus;
        // assert held, mode == Insert, cursor_pos() == 3, read_only == No.
    }

    #[test]
    fn i_from_reading_when_the_lock_is_busy_goes_busy_and_stays_normal() {
        // Reading on 0; hold 0's lock from a second Store handle
        // (`store.lock(&session, &id, &Attribution::for_now("x","other"))`) — but note flock is
        // per open file description, so a second `lock` in the same process on a fresh open
        // does contend. Follow `contend_for_lock` in session_writer tests if that proves flaky.
        // press 'i'; follow_focus;
        // assert read_only is Busy, mode == Normal, replay_after_acquire == None.
    }
```
Write both in full from the comments. Each line names its assertion.

- [ ] **Step 2: Run** and expect failures.

- [ ] **Step 3: Implement.** At the top of `handle_normal_key`'s `match c`, before the existing arms:
```rust
                'i' | 'a' | 'I' | 'A' | 'o' | 'O'
                    if matches!(app.read_only, ReadOnly::Reading { .. }) =>
                {
                    // Intent to write. The lock is taken in `follow_focus`,
                    // which replays this key once it is ours, so `a` still
                    // moves right and `o` still opens a line, against the
                    // re-read text.
                    app.replay_after_acquire = Some(c);
                }
```
In `follow_focus`, inside the Reading branch **before** the "focused id matches → return" check:
```rust
    if let Some(c) = app.replay_after_acquire.take() {
        if matches!(app.read_only, app::ReadOnly::Reading { .. }) {
            try_acquire(app, w, app.focus_idx);
            if app.read_only == app::ReadOnly::No {
                handle_normal_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            }
            return;
        }
    }
```
`try_acquire` moves Reading to `No` / `Busy` / `Closed` itself.

- [ ] **Step 4: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. Expect green.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: entering insert mode from reading takes the lock back and replays the key"
```

---

### Task 4: Phase 2, `release_idle_secs`

**Files:**
- Modify: `src/config.rs` (`pub release_idle_secs: Option<u32>` with doc)
- Modify: `src/app.rs` (`pub release_idle_secs: Option<u32>`, set by `main.rs` after `App::new`)
- Modify: `src/main.rs` (set it from `cfg` ~line 458; tick block in `run` ~line 985, **after** the autosave flush)

- [ ] **Step 1: Failing tests** (`src/app.rs`)

```rust
    #[test]
    fn idle_release_is_due_only_in_normal_mode_after_the_threshold() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.release_idle_secs = Some(2);
        app.mode = Mode::Normal;
        app.idle_secs = 1;
        assert!(!app.idle_release_due());
        app.idle_secs = 2;
        assert!(app.idle_release_due());
        app.mode = Mode::Insert;
        assert!(!app.idle_release_due(), "never mid-sentence");
        app.mode = Mode::Normal;
        app.read_only = ReadOnly::Reading { id: "x".into() };
        assert!(!app.idle_release_due(), "nothing held to release");
        app.read_only = ReadOnly::No;
        app.release_idle_secs = None;
        assert!(!app.idle_release_due(), "off unless configured");
    }
```

- [ ] **Step 2: Run** and expect a compile error.

- [ ] **Step 3: Implement**

`src/app.rs`:
```rust
    /// Phase 2 of reader mode: whether this tick should release the focused
    /// cassette the way `r` does. Normal mode only (a pause mid-sentence in
    /// insert mode must not hand the cassette away), never in record mode,
    /// and only when there is something of ours to release.
    pub fn idle_release_due(&self) -> bool {
        self.release_idle_secs.is_some_and(|n| self.idle_secs >= n)
            && self.mode == Mode::Normal
            && !self.record
            && self.read_only == ReadOnly::No
    }
```
`src/config.rs`:
```rust
    /// Seconds of normal-mode idleness after which the TUI releases the
    /// focused cassette's lock to reader mode, as if `r` had been pressed.
    /// Unset: never. See the reader-mode spec.
    pub release_idle_secs: Option<u32>,
```
`src/main.rs`: after `App::new(..)` add `app.release_idle_secs = cfg.release_idle_secs;`. In `run`, directly after the autosave block:
```rust
        if app.idle_release_due() {
            app.release_requested = true;
            follow_focus(app, writer.as_deref_mut());
        }
```
`follow_focus`'s Task 2 branch does the flush-and-drop. `idle_secs` is reset by the next keypress (`main.rs` ~line 1001), so the release fires once.

- [ ] **Step 4: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. Expect green.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/app.rs src/main.rs
git commit -m "feat: release_idle_secs releases the focused cassette after normal-mode idleness"
```

---

### Task 5: End-to-end check and docs

**Files:** `CLAUDE.md`, `README.md` (if it lists keys), `docs/follow-up.md`

- [ ] **Step 1: Drive it.** Use the `verify` skill: launch the TUI on a scratch store, type a line, `Esc`, `r`. Capture the screen (expect the READING banner). From another shell, run `queue write` on the focused cassette and capture again (expect the new text full-height). Then `A`, type `!`, capture (expect INSERT, `!` at the end). Keep the captures for the review.

- [ ] **Step 2: CLAUDE.md.** In the `session_writer.rs` paragraph, change "**Focus means held**" to "**Focus means held, unless released**" and add a sentence: "`release` (reader mode, `r`) flushes then drops the guard while focus stays. The cassette then syncs like any unheld one, and `ReadOnly::Reading { id }` keeps `follow_focus` from re-taking it until `r` again, an insert-intent key (replayed once the lock is won), or focus moving." In `app.rs`'s paragraph, add `Reading` to the `ReadOnly` list and document `view_focused`. Under Key bindings → Normal, add "`r` releases the lock to read while others write (again, or `i`/`a`/`o`, to take it back)". Under Conventions, add `release_idle_secs` beside `visible_lines`.

- [ ] **Step 3:** Remove the "Reader mode" section from `docs/follow-up.md`. Add one line under "What the Stage 2 trial found" recording the open default question for `release_idle_secs`.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md README.md docs/follow-up.md
git commit -m "docs: reader mode"
```
