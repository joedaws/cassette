# Phase 5a — The TUI Writes the Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the TUI a writer in the same store protocol the agents already obey.

**Architecture:** `App` and `Cassette` gain store identity and per-cassette dirty state while staying pure — no I/O, no ratatui types. `main.rs`'s `Sink` (one file, one global dirty flag, whole-session rewrite) is replaced by a store-backed writer that owns the session id and a `LockGuard` for the focused cassette; autosave and flush-on-blur write single cassettes through that guard. `stats` and `find` swap what they scan.

**Tech Stack:** Rust 2021, ratatui/crossterm, `fs4` advisory locks, serde/serde_json, `tempfile` in tests, a Python `pty.fork()` driver for TUI verification.

**Spec:** `docs/superpowers/specs/2026-09-17-tui-writes-the-store-design.md`, which argues from `docs/superpowers/specs/2026-09-13-session-store-design.md`.

## Global Constraints

- **The brief may be wrong.** Across Phases 2–4c the plans were wrong roughly twenty times — a nonexistent function parameter, a refactor that would have deadlocked the test suite, a test asserting an exit code the command does not return. Verify every claim here against the code and the spec before acting. Deviating is allowed and expected; say so in your report.
- **`LockGuard::write` is the only code in the crate that writes a cassette file.** The TUI is no exception: its autosave writes through the guard it already holds.
- **Focus means held.** The TUI acquires the focused cassette's lock and holds it until focus moves or the process exits. No idle timeout, no reacquisition. The accepted mitigation is a usage norm — close sessions — not a mechanism.
- **A busy cassette is read-only, never silently unwritable.** Never let a keystroke reach a cassette whose lock this process does not hold.
- **`flock` is per-open-file-description.** A second acquire of a lock this process already holds does **not** succeed — it reports Busy and blames a phantom writer. That is what `Store::holds` exists to prevent.
- **A store cassette's body is side headings and nothing else**: `## Side A\n\n<text>\n`, plus `## Side B\n\n<text>\n` when side B is non-empty. No `# Cassette N` wrapper — `json::split_sides` folds pre-heading text into `side_a`, so a wrapper would surface inside the 4c JSON contract.
- **`App` and `Cassette` stay pure** — no I/O, no ratatui types (CLAUDE.md). The `LockGuard` lives in `main.rs`'s event loop.
- Sessions are named by ULID only. `src/cli.rs` is the only place the command line is read.
- **No sleeps and no polling in tests.** In `tests/lock.rs`, a child's lock acquisition is proven by output it writes AFTER acquiring — never by the parent's write to its stdin.
- **This crate is binary-only — there is no `[lib]` target.** `cargo test --lib <name>` does not work. Use `cargo test <name>`.
- `cargo fmt` clean and `cargo clippy --all-targets -- -D warnings` clean at every commit.

## File Structure

| File | Responsibility |
|---|---|
| `src/store/mod.rs` (modify) | `Store::holds` |
| `src/queue/write.rs`, `src/cli.rs` (modify) | `--side`, `--append`/`--replace` |
| `src/cassette.rs` (modify) | `Cassette` gains `id` and `dirty` |
| `src/app.rs` (modify) | `App` gains `session`; `modify_focused` marks the cassette dirty, not the app |
| `src/session_writer.rs` (create) | Owns the session id and the focused cassette's `LockGuard`; the only place the TUI writes the store |
| `src/main.rs` (modify) | Startup creates/opens the session; the event loop drives the writer; `Sink` and the note path go |
| `src/stats.rs`, `src/find.rs` (modify) | Scan the store instead of the notes directory |
| `src/output.rs` (modify) | Per-cassette body building; the flat-note writer stays until Phase 6 |

---

### Task 1: `Store::holds`, `queue write --side/--append/--replace`, and the ULID message

**Files:**
- Modify: `src/store/mod.rs`, `src/queue/write.rs`, `src/cli.rs`
- Test: `src/store/mod.rs`, `src/queue/write.rs`, `tests/cli.rs`

**Interfaces:**
- Produces: `Store::holds(&self, session: &str, id: &str) -> bool`
- Produces: `queue::Side { A, B }`, `queue::WriteMode { Append, Replace }`
- Produces: `queue::write::write(store, id, session, side: Side, mode: WriteMode, who_name, source)`

**Why first:** groundwork the TUI needs, all of it independent of the TUI. This task touches no TUI code and leaves the CLI strictly more capable.

**On `Store::holds`:** this process holds a cassette's lock when it is holding a live `LockGuard` for it. `flock` gives no way to ask the kernel "is this *my* lock?" — a second `try_lock` from the same process fails exactly as it would for another process. So `holds` must be answered from **in-process bookkeeping**, not by probing: a registry of ids this process currently guards. Design it so a `LockGuard`'s creation records the id and its `Drop` removes it. Do not implement `holds` by probing the anchor — that would answer "is it locked" (always true when we hold it) rather than "do *we* hold it", which is the question.

- [ ] **Step 1: Write the failing tests**

In `src/store/mod.rs`'s test module:

```rust
#[test]
fn holds_is_true_only_while_this_process_guards_the_cassette() {
    let (_d, s) = store();
    let sid = s.create_session(&session_meta()).expect("session");
    const ID: &str = "aaa00000000000000000000000";
    s.add_cassette(&sid, &cassette_meta(ID, 10), "").expect("add");

    assert!(!s.holds(&sid, ID), "nothing held yet");
    {
        let who = lock::Attribution::for_now("writer-1", "joseph");
        let _guard = s.lock(&sid, ID, &who).expect("acquire");
        assert!(s.holds(&sid, ID), "we are holding it now");
        assert!(!s.holds(&sid, "bbb00000000000000000000000"), "a different id");
    }
    assert!(!s.holds(&sid, ID), "the guard dropped, so we no longer hold it");
}
```

In `src/queue/write.rs`'s test module (it exists as of 4c):

```rust
#[test]
fn writing_side_b_leaves_side_a_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = store
        .create_session(&crate::store::session::SessionMeta {
            alias: None,
            created: crate::store::meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        })
        .expect("create session");
    let id = "aaa00000000000000000000000";
    let m = CassetteMeta {
        id: id.to_string(),
        topic: Some("sides".to_string()),
        priority: 10,
        status: Status::Open,
        locked_by: None,
        created_by: "w".to_string(),
        last_writer: "w".to_string(),
        updated_at: "2026-09-17T09:00:00Z".to_string(),
    };
    store.add_cassette(&sid, &m, "## Side A\n\nfront\n").expect("add");
    store.ensure_writer("joseph", Kind::Human).expect("human");

    write_body(&store, &sid, &id, "back\n", Side::B, WriteMode::Replace,
               "joseph", WriterSource::Flag).expect("write side b");

    let body = &store.scan_session(&sid).expect("scan").cassettes[0].body;
    assert!(body.contains("front"), "side A must survive: {body}");
    assert!(body.contains("## Side B"), "side B heading written: {body}");
    assert!(body.contains("back"), "{body}");
}

#[test]
fn append_adds_to_a_side_rather_than_replacing_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = store
        .create_session(&crate::store::session::SessionMeta {
            alias: None,
            created: crate::store::meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        })
        .expect("create session");
    let id = "aaa00000000000000000000000";
    let m = CassetteMeta {
        id: id.to_string(),
        topic: Some("sides".to_string()),
        priority: 10,
        status: Status::Open,
        locked_by: None,
        created_by: "w".to_string(),
        last_writer: "w".to_string(),
        updated_at: "2026-09-17T09:00:00Z".to_string(),
    };
    store.add_cassette(&sid, &m, "## Side A\n\nfirst\n").expect("add");
    store.ensure_writer("joseph", Kind::Human).expect("human");

    write_body(&store, &sid, &id, "second\n", Side::A, WriteMode::Append,
               "joseph", WriterSource::Flag).expect("append");

    let body = &store.scan_session(&sid).expect("scan").cassettes[0].body;
    assert!(body.contains("first"), "the original text survives: {body}");
    assert!(body.contains("second"), "{body}");
}
```

Build the two fixtures with this module's existing helper shape — `store.add_cassette(&sid, &meta, "## Side A\n\nfront\n")` after `create_session`. Read the file's existing `sticky_cassette` helper and follow it.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test holds` and `cargo test queue::write`
Expected: FAIL — `holds`, `Side` and `WriteMode` do not exist.

- [ ] **Step 3: Implement `Store::holds`**

Keep the bookkeeping inside `store` — a set of `"<session>/<id>"` keys behind a `Mutex`, written when `lock`/`lock_many` produce a guard and cleared in `LockGuard::drop`. Document why it is bookkeeping and not a probe, in the terms above.

- [ ] **Step 4: Implement `--side` and `--append`/`--replace`**

Reuse `json::split_sides` to read the existing sides, replace or append to the named one, and rebuild the body in the canonical form — `## Side A` always, `## Side B` only when non-empty. Defaults: `--side a`, `--replace`. Those defaults preserve today's behaviour for every existing caller, which matters because `tests/lock.rs` drives `queue write` in its cross-process cases.

Clap enums live in `cli.rs` with a `From` into the domain type, following `StatusArg`.

- [ ] **Step 5: Fix the exit-4 ULID message**

`queue write`'s sticky refusal names the holder by raw ULID (`cassette is locked by '01M2…'`). The JSON `sticky_lock` field resolves the same writer to `{name, kind}` one field away. Resolve it to a name here too, falling back to the raw id when the registry does not know it — the same stance `build_view` takes.

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add Store::holds and queue write --side/--append

holds is answered from in-process bookkeeping, not by probing the anchor:
flock cannot distinguish our own lock from another process's, so a probe
would answer 'is it locked' rather than 'do we hold it'."
```

---

### Task 2: Per-cassette identity and dirty state

**Files:**
- Modify: `src/cassette.rs`, `src/app.rs`
- Test: `src/cassette.rs`, `src/app.rs`

**Interfaces:**
- Produces: `Cassette.id: String`, `Cassette.dirty: bool`
- Produces: `App.session: String`, set by a new fourth parameter on `App::new`, whose signature becomes `App::new(timer_secs: Option<u32>, word_goal: Option<usize>, visible_lines: Option<usize>, session: String) -> Self`
- Produces: `App::dirty_indices(&self) -> Vec<usize>` and `App::clear_dirty(&mut self, idx: usize)`
- Removes: `App.dirty`

**Fixture ids in pure tests** use `"01JTESTSESSN00000000000000"` — 26 characters and Crockford-safe. Do not use a readable string containing `I`, `L`, `O` or `U`: those are excluded from the alphabet `store::ids::is_valid_id` enforces, so such an id would pass a pure test and be rejected the moment it reached the store.

**This task is pure data.** No I/O, no store calls, no ratatui — `App` and `Cassette` must stay that way (CLAUDE.md). It exists separately so the next task's diff is about locking rather than about threading a field through every constructor.

**Every construction site must be found.** `Cassette::new`, `Cassette::from_sides`, `App::new`, `App::apply_topics`, `App::load_cassettes`, and every test fixture in `app.rs`, `cassette.rs`, `ui.rs` and `output.rs`. Compile errors will find them; the risk is a test fixture given a placeholder id that later looks real. Use an obviously-fake id in fixtures (`"test-cassette-1"`), never a ULID-shaped string.

- [ ] **Step 1: Write the failing test**

In `src/app.rs`'s test module:

```rust
#[test]
fn editing_marks_only_the_focused_cassette_dirty() {
    // The whole point of per-cassette dirty: an autosave must write the one
    // cassette that changed, not rewrite every cassette in the session.
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.new_cassette();
    assert_eq!(app.dirty_indices(), Vec::<usize>::new(), "clean to start");

    app.focus_idx = 1;
    app.modify_focused(|c| c.insert('x'));
    assert_eq!(app.dirty_indices(), vec![1], "only the focused one");

    app.clear_dirty(1);
    assert_eq!(app.dirty_indices(), Vec::<usize>::new(), "cleared");
}
```

Read `app.rs`'s existing tests for the real `App::new` argument list rather than guessing it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test editing_marks_only`
Expected: FAIL — `dirty_indices` does not exist.

- [ ] **Step 3: Implement**

Add the fields, delete `App.dirty`, and change `modify_focused` to set the focused cassette's flag. `App.session` is a plain `String` set at construction.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "refactor: per-cassette identity and dirty state

App.dirty becomes Cassette.dirty so an autosave can write the one cassette
that changed rather than rewriting the session."
```

---

### Task 3: The store-backed session writer — startup and quit

**Files:**
- Create: `src/session_writer.rs`
- Modify: `src/main.rs`, `src/output.rs`
- Test: `src/session_writer.rs`, `src/output.rs`

**Interfaces:**
- Consumes: `App.session`, `Cassette.id`, `Cassette.dirty` (Task 2); `Store::holds` (Task 1).
- Produces:
  - `session_writer::SessionWriter` with `open(store, session, created_here: bool) -> Self`
  - `SessionWriter::acquire(&mut self, app, idx) -> Result<(), LockError>` — takes the guard for cassette `idx`
  - `SessionWriter::flush_focused(&mut self, app) -> io::Result<()>` — writes the focused cassette through the held guard when dirty
  - `SessionWriter::finish(&mut self, app) -> io::Result<()>` — final flush, drop the guard, remove an empty session created in this run
  - `output::cassette_body(c: &Cassette) -> String` — the canonical side-only body

**After this task the TUI persists to the store on quit.** Autosave and flush-on-blur are Task 4; this task is deliberately the smaller half so the locking is reviewable on its own.

**`output::cassette_body` is the piece to get exactly right.** It emits `## Side A\n\n<side A text>\n` and, when side B is non-empty, `## Side B\n\n<side B text>\n` — and **no `# Cassette N` heading**. The existing `build_body` writes that heading because the old format put every cassette in one file; per-file it is redundant, and `json::split_sides` folds any text before the first `## Side A` into `side_a`, so a stray heading would appear inside the 4c JSON contract an agent reads. Leave `build_body` itself alone — Phase 6 deletes it.

**On `finish` and empty sessions:** today an empty session writes no file and cleans up its autosaved draft. Preserve that: when the session was created in this run (`created_here`) and every cassette is empty, remove the session directory. A session that was *opened* rather than created is never removed, whatever it contains — that would delete a human's earlier work.

- [ ] **Step 1: Write the failing tests**

In `src/output.rs`:

```rust
#[test]
fn a_store_cassette_body_carries_sides_and_no_cassette_heading() {
    // The `# Cassette N` wrapper is redundant per-file AND harmful:
    // json::split_sides folds pre-heading text into side_a, so a wrapper
    // would surface inside the JSON contract agents read.
    let c = Cassette::from_sides("front words\n".to_string(), String::new(), Some("topic".into()));
    let body = cassette_body(&c);
    assert!(body.starts_with("## Side A"), "no preamble before the heading: {body:?}");
    assert!(!body.contains("# Cassette"), "no per-cassette wrapper: {body:?}");
    assert!(!body.contains("## Side B"), "side B is empty, so no heading: {body:?}");

    // And it round-trips through the contract's own splitter.
    let (a, b) = crate::queue::json::split_sides(&body);
    assert_eq!(a.trim(), "front words");
    assert_eq!(b, "");
}

#[test]
fn a_store_cassette_body_includes_side_b_when_it_has_text() {
    let c = Cassette::from_sides("front\n".to_string(), "back\n".to_string(), None);
    let body = cassette_body(&c);
    let (a, b) = crate::queue::json::split_sides(&body);
    assert_eq!(a.trim(), "front");
    assert_eq!(b.trim(), "back");
}
```

In `src/session_writer.rs`, with this shared helper both tests use:

```rust
    /// One store session holding `n` empty cassettes, plus an `App` whose
    /// `session` and per-cassette `id`s point at them. The ids are the store's
    /// real ULIDs — an `App` carrying invented ids would pass this test and
    /// fail against a real store.
    fn fixture(store: &Store, n: usize) -> (App, String) {
        let session = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        for i in 0..n {
            let id = crate::store::ids::new_id();
            let m = CassetteMeta {
                id: id.clone(),
                topic: None,
                priority: (i as i64 + 1) * 10,
                status: Status::Open,
                locked_by: None,
                created_by: "w".to_string(),
                last_writer: "w".to_string(),
                updated_at: crate::store::meta::now_utc(),
            };
            store.add_cassette(&session, &m, "").expect("add");
            let mut c = Cassette::new();
            c.id = id;
            app.cassettes.push(c);
        }
        app.focus_idx = 0;
        (app, session)
    }

#[test]
    fn finish_removes_a_session_this_run_created_when_nothing_was_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let (mut app, session) = fixture(&store, 1);
    let mut w = SessionWriter::open(&store, &session, true);
    w.finish(&mut app).expect("finish");
    assert!(
        store.list_sessions().expect("list").is_empty(),
        "an empty session created in this run must not litter `session list`"
    );
}

#[test]
fn finish_never_removes_a_session_it_only_opened() {
    // Deleting a session the human already had would destroy earlier work.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let (mut app, session) = fixture(&store, 1);
    let mut w = SessionWriter::open(&store, &session, false);
    w.finish(&mut app).expect("finish");
    assert_eq!(store.list_sessions().expect("list").len(), 1);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test cassette_body` and `cargo test session_writer`
Expected: FAIL — neither `cassette_body` nor `SessionWriter` exists.

- [ ] **Step 3: Implement `output::cassette_body` and `SessionWriter`**

`SessionWriter` owns `session: String`, `created_here: bool`, and `guard: Option<(usize, LockGuard)>` — the cassette index the guard belongs to, alongside the guard. Register `mod session_writer;` in `main.rs`.

- [ ] **Step 4: Wire startup and quit in `main.rs`**

Startup, before the terminal is touched, so a failure dies cleanly:
1. Resolve the session per the spec's entry-point table — bare `cassette` creates one; `new <NAME>` creates one aliased; `today` finds-or-creates the date-aliased one; `resume [NAME]` opens the most recent or the named one.
2. Create a store cassette per initial cassette (`-T` gives several), recording each id on its `Cassette`.
3. Build the `SessionWriter` and acquire the focused cassette's guard.

`Ctrl+N` must call `Store::add_cassette` so a runtime cassette is a store cassette like any other — find where `new_cassette` is handled in the event loop and add it there.

On quit, call `finish`. Delete `Sink`, `save_note`, and the note-path resolution; leave `output`'s flat-note writer in place for Phase 6.

- [ ] **Step 5: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: the TUI writes its session to the store

A store cassette's body carries side headings and nothing else: the old
`# Cassette N` wrapper is redundant per-file, and json::split_sides would
fold it into side_a, surfacing it inside the JSON contract."
```

---

### Task 4: Per-cassette autosave and flush-on-blur

**Files:**
- Modify: `src/main.rs`, `src/session_writer.rs`
- Test: `src/session_writer.rs`, `tests/lock.rs`

**Interfaces:**
- Consumes: everything from Task 3.
- Produces: `SessionWriter::focus(&mut self, app, idx) -> Result<(), LockError>` — flush the cassette being left, drop its guard, acquire `idx`'s.

**The ordering is the correctness.** `focus` must flush **before** dropping the old guard: the write goes through that guard, and dropping first would either lose the text or force a re-acquire that `flock` may refuse. Write the flush, the drop and the acquire in that order and say so in a comment.

**The autosave writes through the guard the TUI already holds.** It must not call `Store::lock` — that is a second acquire of a lock this process holds, which `flock` refuses while blaming a phantom writer. `Store::holds` (Task 1) is the assertion to reach for if you want a debug check.

- [ ] **Step 1: Write the failing test**

In `src/session_writer.rs`:

```rust
#[test]
fn moving_focus_flushes_the_cassette_being_left() {
    // Text typed into cassette 0 must reach disk when focus moves to 1,
    // not wait for the next 30-second autosave.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let (mut app, session) = fixture(&store, 2);
    let mut w = SessionWriter::open(&store, &session, true);
    w.acquire(&mut app, 0).expect("acquire 0");

    app.focus_idx = 0;
    app.modify_focused(|c| c.insert_str("typed into zero"));

    w.focus(&mut app, 1).expect("move focus to 1");

    let scan = store.scan_session(&session).expect("scan");
    let zero = scan.cassettes.iter().find(|c| c.meta.id == app.cassettes[0].id).expect("found");
    assert!(
        zero.body.contains("typed into zero"),
        "blur must flush before the guard is dropped: {}",
        zero.body
    );
    assert!(!app.cassettes[0].dirty, "and the cassette is clean afterwards");
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test moving_focus_flushes` — FAIL, `focus` does not exist.

Implement `focus`, then wire it into the event loop wherever focus changes: Tab, Shift+Tab, and `Ctrl+N` (a new cassette takes focus). Add the autosave branch — replace the existing `if app.dirty && !app.is_empty() && elapsed >= AUTOSAVE_SECS` block with one that flushes the focused cassette when it is dirty.

**Only the focused cassette is written.** A non-focused cassette cannot be dirty: nothing edits it, and `focus` flushed it on the way out. If you find a path that dirties a non-focused cassette, stop and report it — that is a design hole, not something to paper over with a loop.

- [ ] **Step 3: Add the cross-process test**

In `tests/lock.rs`, assert that an agent's `queue write` on the cassette the TUI has focused returns exit 3, and succeeds once focus moves.

**Follow the file's discipline exactly** — read its module doc and the comment on `contend` first. No sleeps, no polling; a child's acquisition is proven by output it writes *after* acquiring, never by the parent's write to its stdin. Driving the real TUI needs a pty (see `.claude/skills/verify`); if a pty harness makes this test unreliable, hold the lock with a `SessionWriter` in-process instead and say in your report which you chose and why.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: per-cassette autosave and flush-on-blur

focus() flushes before dropping the guard: the write goes through that
guard, and dropping first would lose the text or force a re-acquire flock
may refuse."
```

---

### Task 5: A busy focused cassette is read-only

**Files:**
- Modify: `src/app.rs`, `src/main.rs`, `src/ui.rs`
- Test: `src/app.rs`, `src/ui.rs`

**Interfaces:**
- Produces: `App.read_only: bool`, set when the focused cassette's lock could not be acquired.

**The rule:** if `acquire` returns `Busy`, the cassette is shown but not editable, and the status line says so. 5b replaces this with a proper banner and per-tick retry — **do not build those here**, and do not add a retry loop.

**What must not happen** is either alternative: refusing to start would let a running agent lock the human out of their own session, and accepting keystrokes without the lock would lose the text at the next flush. In read-only mode every text-mutating key is ignored; navigation, `Tab`, and quit still work, because you must be able to leave.

- [ ] **Step 1: Write the failing test**

In `src/app.rs`:

```rust
#[test]
fn read_only_ignores_text_keys_but_allows_leaving() {
    let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
    app.read_only = true;
    let before = app.cassettes[app.focus_idx].side_a_text();

    app.modify_focused(|c| c.insert('x'));
    assert_eq!(
        app.cassettes[app.focus_idx].side_a_text(),
        before,
        "a keystroke must never reach a cassette this process does not hold"
    );
    assert!(!app.cassettes[app.focus_idx].dirty, "and it must not be marked dirty");
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test read_only_ignores` — FAIL.

Gate `modify_focused` on `read_only`. In `ui.rs`, show the state in the info line, matching the way `-- RECORD --` is rendered — read that first and follow it rather than inventing a second style.

- [ ] **Step 3: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: a busy focused cassette renders read-only

Refusing to start would let an agent lock the human out of their own
session; accepting keystrokes without the lock would lose the text at the
next flush."
```

---

### Task 6: `stats` and `find` read the store

**Files:**
- Modify: `src/stats.rs`, `src/find.rs`, `src/main.rs`
- Test: `src/stats.rs`, `src/find.rs`, `tests/cli.rs`

**Interfaces:**
- Produces: `stats::scan_store(store: &Store) -> Vec<NoteMeta>`, `find::scan_store(store: &Store) -> Vec<NoteEntry>`
- Removes: `NoteEntry.draft`

**Both modules already separate scanning from rendering** — `scan_notes_dir` plus a pure `render`. Keep `render` and its tests untouched; this task swaps what fills the structs. One session becomes one `NoteMeta`/`NoteEntry`: its date from `session.toml`'s `created`, its words summed across the session's cassettes, its topics the cassettes' topics, its preview the first body line of the highest-priority cassette.

**`NoteEntry.draft` goes.** The store has no autosave draft marker — a cassette file either exists or it does not — so the field would always be `false`. Delete it rather than report a constant, and update `render` accordingly.

**The legacy notes directory is not read and not migrated.** That is the spec's decision 7, taken with the cost visible: the user's 45 existing notes stop appearing in `stats`. Do not add a fallback that reads both, and do not migrate anything — if you think the reset is wrong, say so in your report rather than softening it in code.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_session_becomes_one_stats_entry_summing_its_cassettes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = store
        .create_session(&crate::store::session::SessionMeta {
            alias: None,
            created: crate::store::meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        })
        .expect("create session");
    for (i, body) in ["## Side A\n\none two\n", "## Side A\n\nthree\n"].iter().enumerate() {
        let m = CassetteMeta {
            id: crate::store::ids::new_id(),
            topic: Some(format!("topic {i}")),
            priority: (i as i64 + 1) * 10,
            status: Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store.add_cassette(&sid, &m, body).expect("add");
    }

    let metas = scan_store(&store);
    assert_eq!(metas.len(), 1, "one session is one entry, not one per cassette");
    assert_eq!(metas[0].words, 3, "words sum across the session's cassettes");
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test a_session_becomes_one` — FAIL, `scan_store` does not exist.

In `main.rs`, the `stats` and `find` arms build a `Store` from `store_root()` instead of resolving `notes_dir`.

- [ ] **Step 3: Add a CLI test**

In `tests/cli.rs`: create a session with `session new`, add a cassette with `queue new`, write words into it with `queue write`, then assert `cassette stats` reports them and `cassette find` lists the session.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: stats and find read the session store

The legacy notes directory is deliberately not read: the totals reset, the
files stay on disk untouched, and a migration remains possible as its own
phase."
```

---

### Task 7: Documentation and end-to-end verification

**Files:**
- Modify: `README.md`, `CLAUDE.md`

**This is the task most likely to be quietly wrong.** Phase 4b shipped a README `--help` transcript that had silently dropped a clause, and the clause it dropped was the security-relevant one. **Generate every piece of command output rather than typing it**, and diff it against the binary before committing.

- [ ] **Step 1: Update `README.md`**

Document that the TUI now writes to the session store; that the focused cassette is locked while focused and the norm is to close sessions rather than leave them open, since agents drive the queue through the CLI and never need the TUI running; that a cassette held by someone else opens read-only; `queue write --side/--append/--replace`; and — stated plainly — that `stats` and `find` now read the store, so totals start from the store and existing notes in `~/.local/share/cassette/notes/` are no longer counted but remain on disk and readable.

- [ ] **Step 2: Update `CLAUDE.md`**

Add `src/session_writer.rs` to the source layout in the style and density of the neighbouring entries, and revise the `src/main.rs`, `src/app.rs`, `src/output.rs`, `src/stats.rs` and `src/find.rs` entries, which describe the old note-file model throughout. Read each entry before rewriting it — several sentences are still accurate and should survive.

- [ ] **Step 3: Drive the real TUI**

Use `.claude/skills/verify`'s pty driver. Its requirements, which have cost previous phases time: answer `ESC[6n` (a real terminal does; a driver that does not makes `terminal.clear()` fail and the app exit 1), and `cargo test` does **not** rebuild the binary — run `cargo build` first or you will verify stale code.

Drive at least: launch, type into cassette 1, `Ctrl+N`, type into cassette 2, `Tab` back, quit. Then assert against the **store** — two cassette files, each with the right text under `## Side A`, and the session listed by `session list`. Put the transcript in your report.

- [ ] **Step 4: Full verification**

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Then confirm the invariants:

```bash
# Only LockGuard::write, session.toml and writers.toml write via atomic_write
grep -rn 'atomic_write(' src/ | grep -v test
# The TUI's autosave must not re-acquire a lock it already holds
grep -n 'store.lock(' src/session_writer.rs src/main.rs
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs: document the TUI's store integration"
```

---

## Plan Self-Review

**Spec coverage.** Decision 1 (focus means held, no timeout) → Tasks 3 and 4; decision 2 (busy is read-only) → Task 5; decision 3 (per-cassette dirty and write) → Tasks 2 and 4; decision 4 (`Store::holds`) → Task 1; decision 5 (side-only body) → Task 3; decision 6 (`--side`/`--append`) → Task 1; decision 7 (`stats`/`find` repointed, totals reset, nothing migrated) → Task 6; decision 8 (empty sessions clean up) → Task 3. The entry-point table → Task 3, step 4. The carried ULID-message item → Task 1, step 5. Testing → every task, cross-process in Task 4, pty in Task 7.

**Ordering.** Task 1 is independent of the TUI and comes first so the rest can assume `Store::holds`. Task 2 is pure data and precedes every task that touches `App`. Task 3 precedes Task 4 (which extends `SessionWriter`) and Task 5 (which reacts to `acquire` failing). Task 6 is independent of Tasks 3–5 and could move, but sits after them so its reviewer sees a TUI that already writes the store.

**Type consistency.** `SessionWriter`, `acquire`, `flush_focused`, `finish` and `focus` are defined in Tasks 3–4 and used under those names throughout. `output::cassette_body` is defined in Task 3 and referenced in Tasks 3 and 7. `App::new` gains its fourth parameter in Task 2, and Tasks 2 and 5's tests call the four-argument form. `NoteEntry.draft` is removed in Task 6 and referenced nowhere later.

**Three errors found and fixed during this review**, recorded because the pattern has held across every phase: `cassette_meta` was called with one argument when it takes `(id, priority)`; eight test fixtures were `/* … */` prose rather than code; and the placeholder session id `01TESTSESSION…` contains `I` and `O`, which `is_crockford_byte` excludes — it would have passed a pure test and been rejected on first contact with the store.

**Known risk.** Task 3 is the largest and replaces the entire persistence path. If its reviewer finds the TUI writing anywhere other than through a `LockGuard`, that is the invariant this whole redesign rests on — treat it as Critical, not as a style point.

**One judgment call left to the implementer**, with an instruction to report which was taken: Task 4's cross-process test may either drive the real TUI through a pty or hold the lock with an in-process `SessionWriter`. Both are defensible; a pty harness is closer to reality but historically flaky in this repo.
