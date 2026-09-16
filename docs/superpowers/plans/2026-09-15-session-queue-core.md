# Phase 4b — Session and Queue Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three `session` commands and seven `queue` commands on top of the finished store, and remove the active-session pointer.

**Architecture:** A CLI layer over `src/store/`, which needs no new primitives. `src/queue.rs` becomes `src/queue/` split on the lock boundary — `view.rs` reads, `edit.rs` and `write.rs` hold a `LockGuard` while writing — so invariant 1 ("`LockGuard::write` is the only code that writes a cassette") is checkable by reading one module. `src/session.rs` is new and follows `writer.rs`: pure `render_*` functions plus thin I/O.

**Tech Stack:** Rust 2021, clap v4 derive, `fs4` advisory locks, `tempfile` in tests.

**Spec:** `docs/superpowers/specs/2026-09-15-session-queue-core-design.md` (authoritative for this sub-phase), which argues from `docs/superpowers/specs/2026-09-13-session-store-design.md`.

## Global Constraints

- **The brief may be wrong.** Across Phases 2–4a the plans were wrong about a dozen times — boundary expectations simply incorrect, defect lists missing an instance, a spec clause quoted half-way. Verify every claim here against the code and the spec before acting. Deviating is allowed and expected; say so in your report when you do.
- **`LockGuard::write` is the only code in the crate that writes a cassette file.** No new code path may write a `.md` under `cassettes/`. `src/queue/view.rs` must contain no writes at all.
- **Never block on a lock a human can hold.** Cassette locks are non-blocking (`Store::lock`, `Store::lock_many`); only the private registry lock blocks.
- **`--session <id>` is required on every `queue` command.** There is no active session and no fallback.
- **Sessions are named by ULID only.** An alias never resolves; id prefixes are not accepted.
- **Writer `kind` is fixed at registration.** Only `writer register` declares a kind. An unknown `--writer` (or `$CASSETTE_WRITER`) is exit 2; only `$USER` auto-creates, as human.
- **Exit codes:** 1 I/O, 2 usage, 3 busy, 4 sticky-locked, 5 nothing available, 6 queue full.
- **No sleeps or polling in tests.** Cross-process synchronisation is a child's stdin pipe, and a child's lock acquisition is proven by output it writes *after* acquiring — never by the parent's write to its pipe.
- `cargo fmt` clean and `cargo clippy --all-targets -- -D warnings` clean at every commit.
- Priorities are positive; `queue new --priority N` rejects `N <= 0` with exit 2.

## File Structure

| File | Responsibility |
|---|---|
| `src/cli.rs` (modify) | clap tree; gains `SessionAction`, grows `QueueAction`; lowers both into `Args` |
| `src/session.rs` (create) | `session new` / `list` / `alias`; pure `render_*` plus thin I/O |
| `src/queue/mod.rs` (create) | `QueueError`, `WriterSource`, shared helpers, re-exports |
| `src/queue/view.rs` (create) | `list`, `next`, `show` — **no writes** |
| `src/queue/edit.rs` (create) | `new`, `close`, `reopen`, `move_cassette`, `renumber_all` |
| `src/queue/write.rs` (create) | the existing `write`, moved |
| `src/store/writers.rs` (modify) | `WriterError` splits into `EnsureError` / `ResolveError` / `RequireError` |
| `src/store/mod.rs` (modify) | delete the active-pointer methods; add `MAX_OPEN` |
| `src/store/session.rs` (modify) | delete `read_active` / `write_active` / `ACTIVE_FILE` |
| `src/config.rs` (modify) | add `max_open` |
| `src/main.rs` (modify) | dispatch and exit-code mapping; `$CASSETTE_WRITER` |
| `tests/cli.rs`, `tests/lock.rs` (modify) | updated call sites plus new coverage |

**Naming note:** `src/session.rs` (commands) sits alongside `src/store/session.rs` (storage), exactly as `src/writer.rs` does alongside `src/store/writers.rs`. This is the established convention in this crate, not an accident — do not rename either.

---

### Task 1: Remove the active session; require `--session`

**Files:**
- Modify: `src/cli.rs` (`Args`, `QueueAction`, `into_args`)
- Modify: `src/queue.rs` (`write` signature and its session resolution)
- Modify: `src/store/mod.rs` (delete two methods and three tests)
- Modify: `src/store/session.rs` (delete `read_active`, `write_active`, `ACTIVE_FILE`, `active_path`, and their tests)
- Modify: `tests/lock.rs`, `tests/cli.rs`

**Interfaces:**
- Produces: `queue::write(store: &Store, id: &str, session: &str, who_name: &str, source: WriterSource) -> Result<(), QueueError>` — note `session: &str`, no longer `Option<&str>`.
- Produces: `cli::QueueCmd` enum, which Tasks 3–8 extend.

**Why this is first:** every later task consumes the required-`--session` shape. Doing it later would mean rewriting seven call sites.

- [ ] **Step 1: Update the existing tests to the new invocation, and watch them fail**

These call sites rely on the active pointer today and must carry `--session`. In `tests/lock.rs`, delete the fixture's pointer write and add the flag to all four spawn sites:

```rust
// tests/lock.rs — in fixture(), DELETE this line entirely:
//   std::fs::write(root.join("active"), format!("{SESSION}\n")).expect("active");

// tests/lock.rs:55  (spawn_write)
        .args(["queue", "write", id, "--session", SESSION])
// tests/lock.rs:73  (try_write)
        .args(["queue", "write", ID, "--session", SESSION])
// tests/lock.rs:237 and tests/lock.rs:290
        .args(["queue", "write", ID, "--session", SESSION])
```

In `tests/cli.rs` line 241, delete the `active` write the same way, and update both invocations:

```rust
// tests/cli.rs:151 — this test asserts stderr names --writer. Without
// --session, clap now rejects the missing flag FIRST and the assertion
// would match the wrong error while still exiting 2.
        .args(["queue", "write", "01K5GR7T2M9WPD0000000000AB",
               "--session", "01K5GQ2R8V3XQZ0000000000AB"])
// tests/cli.rs:253
        .args(["--writer", "nosuchwriter", "queue", "write", ID,
               "--session", SESSION])
```

Run: `cargo test` — expect failures: the new `--session` flag does not exist yet, so clap rejects it (exit 2) and these tests fail on their exit-code assertions.

- [ ] **Step 2: Add a required `--session` and the `QueueCmd` enum to the CLI**

In `src/cli.rs`, replace the `queue_write` field on `Args`:

```rust
    /// `queue …`, if that's what was invoked.
    pub queue_cmd: Option<QueueCmd>,
```

Add the enum beside `WriterCmd` (Tasks 3–8 add variants):

```rust
/// `queue …` as `main()` consumes it. Every variant carries `session`:
/// there is no active session to fall back to.
#[derive(Debug, PartialEq)]
pub enum QueueCmd {
    Write { id: String, session: String },
}
```

In the clap tree, make `session` required by making it a plain `String` — clap treats a non-`Option` argument as required:

```rust
#[derive(Subcommand, Debug)]
enum QueueAction {
    /// replace a cassette's body, read from stdin
    Write {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
    },
}
```

And in `into_args`:

```rust
            Some(Command::Queue { action }) => {
                args.queue_cmd = Some(match action {
                    QueueAction::Write { id, session } => QueueCmd::Write { id, session },
                });
            }
```

- [ ] **Step 3: Drop the resolution from `queue::write`**

In `src/queue.rs`, change the signature's `session: Option<&str>` to `session: &str` and delete the whole `let session = match session ... };` block at the top of the body (roughly lines 56–68). Nothing replaces it — the parameter is already the answer.

- [ ] **Step 4: Update the `main.rs` dispatch**

Replace the `if let Some((id, session)) = &args.queue_write { ... }` block:

```rust
    if let Some(cmd) = &args.queue_cmd {
        let store = store::Store::new(store_root());
        let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
            Ok(w) => w,
            Err(msg) => die_with(2, &msg),
        };
        let result = match cmd {
            cli::QueueCmd::Write { id, session } => {
                queue::write(&store, id, session, &who_name, writer_source)
            }
        };
        match result {
            Ok(()) => std::process::exit(0),
            Err(queue::QueueError::Usage(m)) => die_with(2, &m),
            Err(queue::QueueError::Busy(m)) => die_with(3, &m),
            Err(queue::QueueError::Io(m)) => die_with(1, &m),
        }
    }
```

- [ ] **Step 5: Delete the active-pointer store API**

From `src/store/mod.rs` delete the methods `active_session` and `set_active_session`, and the tests `the_active_session_round_trips_through_the_store`, `an_unreadable_active_pointer_errors_rather_than_reading_as_absent`, and `setting_the_active_session_creates_a_private_root`.

Deleting the last of those three loses no coverage: `registering_a_writer_creates_a_private_root` in the same module already asserts the root is created `0700`. Verify that is still true before deleting, and say so in your report.

From `src/store/session.rs` delete `read_active`, `write_active`, `active_path`, the `ACTIVE_FILE` constant, and the four tests covering them.

- [ ] **Step 6: Run the full suite**

Run: `cargo test`
Expected: PASS. Then `cargo fmt` and `cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: remove the active session; --session is required

A single `active` file is shared mutable state between a human and a running
agent: `session use B` would silently retarget an agent mid-run. Removing the
concept also collapses queue.rs's three-branch resolution into a field clap
guarantees, so a missing session is clap's own exit 2."
```

---

### Task 2: Split `WriterError` per call-site capability

**Files:**
- Modify: `src/store/writers.rs` (replace `WriterError` with three types)
- Modify: `src/store/mod.rs` (`ensure_writer`, `resolve_writer`, `require_writer` return types)
- Modify: `src/writer.rs`, `src/queue.rs`, `src/main.rs` (call sites)

**Interfaces:**
- Produces:
  - `writers::ensure(root, name, kind) -> Result<String, EnsureError>`
  - `writers::resolve(root, name) -> Result<(String, Kind), ResolveError>`
  - `writers::require_registered(root, name) -> Result<(String, Kind), RequireError>`
  - `Store::ensure_writer -> Result<String, EnsureError>`, `Store::resolve_writer -> Result<(String, Kind), ResolveError>`, `Store::require_writer -> Result<(String, Kind), RequireError>`

**Why now:** seven commands land in Tasks 3–8. Today every call site matches all four `WriterError` variants and `main.rs` carries an arm its own comment calls unreachable. Splitting after those commands exist means editing seven matches instead of three.

- [ ] **Step 1: Write the failing test**

Add to `src/store/writers.rs`'s test module:

```rust
#[test]
fn resolve_cannot_report_a_kind_mismatch() {
    // `resolve` declares no kind, so a mismatch is not one of its outcomes.
    // This is a type-level claim: it compiles only while ResolveError has
    // no KindMismatch variant, which is the whole point of the split.
    let dir = tempfile::tempdir().expect("tempdir");
    ensure(dir.path(), "bot", Kind::Agent).expect("ensure");
    let (id, kind) = resolve(dir.path(), "bot").expect("resolve");
    assert!(!id.is_empty());
    assert_eq!(kind, Kind::Agent, "resolve defers to the registered kind");

    match resolve(dir.path(), "   ") {
        Err(ResolveError::EmptyName) => {}
        other => panic!("a blank name is EmptyName, got {other:?}"),
    }
}

#[test]
fn require_registered_reports_an_unknown_name_as_its_own_variant() {
    let dir = tempfile::tempdir().expect("tempdir");
    match require_registered(dir.path(), "ghost") {
        Err(RequireError::Unregistered(n)) => assert_eq!(n, "ghost"),
        other => panic!("expected Unregistered, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib writers`
Expected: FAIL — `ResolveError` and `RequireError` do not exist.

- [ ] **Step 3: Define the three error types**

Replace `WriterError` in `src/store/writers.rs`. Each type lists exactly the failures its function can produce — no dead arms:

```rust
/// Failures of `ensure`, which declares a kind.
#[derive(Debug)]
pub enum EnsureError {
    /// This name is registered with a different `kind`.
    KindMismatch {
        name: String,
        registered: Kind,
        requested: Kind,
    },
    /// The name has no characters left after trimming.
    EmptyName,
    Io(io::Error),
}

/// Failures of `resolve`, which declares no kind and may auto-create.
/// It cannot report a mismatch (it requests no kind) and cannot report an
/// unknown name (it creates one).
#[derive(Debug)]
pub enum ResolveError {
    EmptyName,
    Io(io::Error),
}

/// Failures of `require_registered`, which never inserts.
#[derive(Debug)]
pub enum RequireError {
    EmptyName,
    /// `name` is not in the registry and this caller may not create it.
    Unregistered(String),
    Io(io::Error),
}
```

Give each a `Display` impl carrying the same wording the current `WriterError` produces, so no user-visible string changes in this task. Keep `impl std::error::Error` where the current type has it.

- [ ] **Step 4: Update the three store methods and their call sites**

`Store::ensure_writer` returns `EnsureError`, `resolve_writer` returns `ResolveError`, `require_writer` returns `RequireError`.

In `src/queue.rs`, `writer_error_to_queue_error` splits into two total functions with no unreachable arms:

```rust
fn resolve_error_to_queue_error(e: writers::ResolveError) -> QueueError {
    match e {
        writers::ResolveError::EmptyName => QueueError::Usage(e.to_string()),
        writers::ResolveError::Io(io_e) => {
            QueueError::Io(format!("cannot resolve writer: {io_e}"))
        }
    }
}

fn require_error_to_queue_error(e: writers::RequireError) -> QueueError {
    match e {
        writers::RequireError::EmptyName => QueueError::Usage(e.to_string()),
        writers::RequireError::Unregistered(_) => QueueError::Usage(e.to_string()),
        writers::RequireError::Io(io_e) => {
            QueueError::Io(format!("cannot resolve writer: {io_e}"))
        }
    }
}
```

and the `match source` block maps each arm through its own function.

In `src/main.rs`, `run_writer_cmd`'s `register` match loses the arm its comment calls unreachable; it now matches `EnsureError`'s three real variants.

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS, with no `WriterError` left in the crate. Verify with `grep -rn 'WriterError' src/` — expect no matches.

- [ ] **Step 6: Decide `Store::write_writers`**

`Store::write_writers` is `pub`, unlocked, unvalidated, and has no callers. Check whether anything in this plan needs it (nothing in Tasks 3–8 does; all writer mutation goes through `ensure_writer`, which holds the registry lock). If it is still uncalled, delete it and its test. Record the decision in your report either way.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: split WriterError per call-site capability

Every caller matched all four variants and main.rs carried an arm its own
comment called unreachable. Seven commands land next and would have copied
that. EnsureError, ResolveError and RequireError each list exactly what their
function can produce."
```

---

### Task 3: `session new` / `list` / `alias`

**Files:**
- Create: `src/session.rs`
- Modify: `src/store/mod.rs` (add `list_sessions`, `set_session_alias`)
- Modify: `src/cli.rs` (`SessionCmd`, `SessionAction`)
- Modify: `src/main.rs` (dispatch)
- Modify: `tests/cli.rs` (end-to-end)

**Interfaces:**
- Consumes: `Store::create_session(&SessionMeta) -> io::Result<String>`, `Store::session_meta(&str) -> io::Result<SessionMeta>`, `store::session::{read, write}` (both `pub(crate)`).
- Produces:
  - `Store::list_sessions(&self) -> io::Result<Vec<(String, SessionMeta)>>` — newest first by `created`
  - `Store::set_session_alias(&self, session: &str, alias: &str) -> io::Result<()>`
  - `session::new_session(store, alias: Option<&str>) -> Result<String, String>`
  - `session::render_list(rows: &[(String, SessionMeta)], limit: Option<usize>) -> String`
  - `session::list(store, all: bool) -> Result<String, String>`
  - `session::set_alias(store, id: &str, alias: &str) -> Result<String, String>`

`Result<_, String>` matches `writer.rs`'s `list`/`whoami`: these commands have exactly two outcomes, success or an I/O failure that exits 1, plus a usage failure that exits 2 — the caller distinguishes them by which function returned. Where a command needs a code the string cannot express, it returns `Result<_, QueueError>` instead; `set_alias` does, because an unknown session id is exit 2.

- [ ] **Step 1: Write the failing tests**

In `src/session.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::session::SessionMeta;

    fn meta(created: &str, alias: Option<&str>) -> SessionMeta {
        SessionMeta {
            alias: alias.map(str::to_string),
            created: created.to_string(),
            timer_secs: None,
            word_goal: None,
        }
    }

    #[test]
    fn list_is_newest_first_and_shows_the_alias() {
        let rows = vec![
            ("01AAA".to_string(), meta("2026-09-14T09:00:00Z", None)),
            ("01BBB".to_string(), meta("2026-09-15T09:00:00Z", Some("today"))),
        ];
        let out = render_list(&rows, None);
        let bbb = out.find("01BBB").expect("bbb");
        let aaa = out.find("01AAA").expect("aaa");
        assert!(bbb < aaa, "newest first: {out}");
        assert!(out.contains("today"), "alias must show: {out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&[], None), "no sessions");
    }

    #[test]
    fn the_default_listing_is_capped_and_says_how_many_it_hid() {
        let rows: Vec<_> = (0..20)
            .map(|i| (format!("01{i:03}"), meta("2026-09-15T09:00:00Z", None)))
            .collect();
        let out = render_list(&rows, Some(15));
        assert_eq!(out.lines().filter(|l| l.starts_with("01")).count(), 15);
        assert!(out.contains("5 more"), "must not hide rows silently: {out}");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib session`
Expected: FAIL — `src/session.rs` does not exist.

- [ ] **Step 3: Implement `src/session.rs`**

`render_list` sorts by `created` descending, tie-broken by id descending so the order is total and deterministic. Each row is `<id>  <created>  <alias or "">`. With a `limit` that hides rows, the last line is `… N more (--all)`.

Register the module in `src/main.rs` with `mod session;`.

- [ ] **Step 4: Add the two store methods**

```rust
    /// Every session, newest first. A session directory whose `session.toml`
    /// is missing or unparseable is skipped: `session list` is a listing, not
    /// a repair tool, and one damaged session must not hide the rest.
    pub fn list_sessions(&self) -> io::Result<Vec<(String, session::SessionMeta)>> { ... }

    /// Set a session's display alias. The alias never resolves — it is shown
    /// in `session list` and nowhere else — so no uniqueness check applies.
    pub fn set_session_alias(&self, session: &str, alias: &str) -> io::Result<()> { ... }
```

`list_sessions` reads `sessions_dir()`, returning an empty vec when it does not exist (mirror `scan_session`'s `NotFound` arm). `set_session_alias` reads `session.toml`, sets `alias`, and writes it back through `session::write`, which uses `atomic_write`.

- [ ] **Step 5: Wire the CLI**

In `src/cli.rs` add to `Args`: `pub session_cmd: Option<SessionCmd>,` and

```rust
/// `session …` as `main()` consumes it.
#[derive(Debug, PartialEq)]
pub enum SessionCmd {
    New { alias: Option<String> },
    List { all: bool },
    Alias { id: String, alias: String },
}
```

with the clap subcommand:

```rust
#[derive(Subcommand, Debug)]
enum SessionAction {
    /// create a session and print its id
    New {
        /// display label shown in `session list`; it never resolves
        #[arg(long, value_name = "NAME")]
        alias: Option<String>,
    },
    /// list sessions, newest first
    List {
        /// show every session instead of the 15 most recent
        #[arg(long)]
        all: bool,
    },
    /// set a session's display label
    Alias {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(value_name = "ALIAS")]
        alias: String,
    },
}
```

Add `Session { #[command(subcommand)] action: SessionAction }` to `Command`, and lower it in `into_args`.

- [ ] **Step 6: Dispatch in `main.rs`**

Place the block beside `run_writer_cmd`'s, before the terminal is touched. `new` and `alias` exit 2 on usage failure (unknown id) and 1 on I/O; `list` exits 1 on I/O.

- [ ] **Step 7: Add an end-to-end CLI test**

In `tests/cli.rs`:

```rust
#[test]
fn session_new_then_list_then_alias() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");

    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(new.status.code(), Some(0), "{}", stderr(&new));
    let id = String::from_utf8_lossy(&new.stdout).trim().to_string();
    assert_eq!(id.len(), 26, "a ULID is printed bare for scripting: {id:?}");

    let aliased = Command::new(bin())
        .args(["session", "alias", &id, "monday"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(aliased.status.code(), Some(0), "{}", stderr(&aliased));

    let list = Command::new(bin())
        .args(["session", "list"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&list.stdout).to_string();
    assert!(text.contains(&id), "{text}");
    assert!(text.contains("monday"), "{text}");
}

#[test]
fn session_alias_on_an_unknown_id_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["session", "alias", "01K5GQ2R8V3XQZ0000000000AB", "x"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}
```

- [ ] **Step 8: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add session new, list and alias

An alias is a display label only: it never resolves, so no uniqueness rule
applies and `--session` always takes a ULID."
```

---

### Task 4: Split `queue` into modules; add `list` and `show`

**Files:**
- Delete: `src/queue.rs` (contents move; use `git mv` so history follows)
- Create: `src/queue/mod.rs`, `src/queue/view.rs`, `src/queue/write.rs`
- Modify: `src/store/mod.rs` (`scan_session` returns a count of what it skipped)
- Modify: `src/cli.rs`, `src/main.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `QueueCmd` from Task 1, `resolve_error_to_queue_error` / `require_error_to_queue_error` from Task 2.
- Produces:
  - `store::SessionScan { pub cassettes: Vec<StoredCassette>, pub unreadable: usize }`
  - `Store::scan_session(&self, session: &str) -> io::Result<SessionScan>` (changed return type)
  - `queue::view::list(store, session, status: StatusFilter, since: Option<&str>) -> Result<String, QueueError>`
  - `queue::view::show(store, session, id) -> Result<String, QueueError>`
  - `queue::view::render_list(cassettes: &[StoredCassette], unreadable: usize) -> String`
  - `queue::StatusFilter { Open, Closed, All }`

**Module rule:** `src/queue/view.rs` must contain no writes. A reviewer checks this by reading the module; keep it true.

- [ ] **Step 1: Move the existing code with history**

```bash
mkdir src/queue
git mv src/queue.rs src/queue/write.rs
```

Create `src/queue/mod.rs` holding `QueueError`, `WriterSource`, the two error-mapping functions, `StatusFilter`, and `pub mod view; pub mod write; pub use write::write;`. Move only those items out of `write.rs`; leave `write` itself in place, unchanged.

- [ ] **Step 2: Write the failing tests**

In `src/queue/view.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};

    fn meta(id: &str, priority: i64, status: Status) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: Some(format!("topic-{id}")),
            priority,
            status,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-15T09:00:00Z".to_string(),
        }
    }

    fn stored(m: CassetteMeta) -> StoredCassette {
        StoredCassette {
            path: std::path::PathBuf::from(format!("{}.md", m.id)),
            meta: m,
            body: "## Side A\n\nwords here\n".to_string(),
        }
    }

    #[test]
    fn list_orders_open_by_priority_then_closed_last() {
        let rows = vec![
            stored(meta("c", 5, Status::Closed)),
            stored(meta("b", 20, Status::Open)),
            stored(meta("a", 10, Status::Open)),
        ];
        let out = render_list(&rows, 0);
        let a = out.find("topic-a").expect("a");
        let b = out.find("topic-b").expect("b");
        let c = out.find("topic-c").expect("c");
        assert!(a < b, "priority 10 before 20: {out}");
        assert!(b < c, "closed last despite priority 5: {out}");
    }

    #[test]
    fn list_reports_unreadable_cassettes_instead_of_hiding_them() {
        // A cassette the store could not parse must never be silently
        // absent: `queue list` is the operator's only view of the store.
        let out = render_list(&[stored(meta("a", 10, Status::Open))], 2);
        assert!(out.contains("2 unreadable"), "{out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&[], 0), "no cassettes");
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --lib queue`
Expected: FAIL — `render_list` does not exist.

- [ ] **Step 4: Change `scan_session` to report what it skipped**

In `src/store/mod.rs`:

```rust
/// The result of reading a session's cassettes, including how many files
/// could not be read or parsed. The count is carried rather than logged: a
/// damaged cassette that vanishes from `queue list` is invisible work, and
/// the operator has no other view of the store.
#[derive(Debug, Default)]
pub struct SessionScan {
    pub cassettes: Vec<StoredCassette>,
    pub unreadable: usize,
}
```

Change `scan_session` to return `io::Result<SessionScan>`, incrementing `unreadable` in each of the two `continue` arms (the unreadable-file arm and the unparseable-frontmatter arm). Update the six test call sites in the same module — they use `.len()`, `.is_empty()` and indexing, so they become `.cassettes.len()` and so on.

- [ ] **Step 5: Implement `list` and `show`**

`render_list` takes the cassettes already ordered — call `store::priority::queue_order` on the metas before rendering — and emits one line per cassette: `<id>  p<priority>  <status>  <topic>`. When `unreadable > 0`, append a final line `N unreadable`. `list` applies `StatusFilter` and the `--since` RFC3339 filter (a cassette is included when `updated_at >= since`; an unparseable `--since` is `QueueError::Usage`).

`show` finds the cassette by id via `Store::cassette_path`, reads it, and prints its frontmatter followed by its body. An unknown id is `QueueError::Usage`. It takes no lock: a torn read here shows stale text, which is what a viewer of a live session should expect, and taking a lock would make viewing fail while someone writes.

- [ ] **Step 6: Wire the CLI and dispatch**

Add to `QueueCmd`: `List { session: String, status: StatusFilter, since: Option<String> }` and `Show { session: String, id: String }`, with the matching clap actions. `--status` is a `ValueEnum` over `open|closed|all`, defaulting to `open`.

- [ ] **Step 7: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: split queue into view/edit/write; add queue list and show

The split is on the lock boundary so invariant 1 is checkable by reading one
module. scan_session now reports how many cassettes it could not parse —
queue list prints the count rather than letting damaged work vanish."
```

---

### Task 5: `queue next` — probe and skip busy cassettes

**Files:**
- Modify: `src/store/lock.rs` (add `probe`)
- Modify: `src/store/mod.rs` (add `Store::is_free`)
- Modify: `src/queue/mod.rs` (`QueueError::Empty`), `src/queue/view.rs` (`next`)
- Modify: `src/cli.rs`, `src/main.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Produces:
  - `store::lock::probe(anchor_path: &Path) -> io::Result<bool>` — `true` when the lock is free
  - `Store::is_free(&self, session: &str, id: &str) -> io::Result<bool>`
  - `queue::view::next(store, session) -> Result<String, QueueError>`
  - `QueueError::Empty` → exit 5

**The hazard this task exists to avoid:** do **not** probe by calling `Store::lock` and dropping the guard. `lock::acquire` stamps the holder's attribution into the anchor file after acquiring (`src/store/lock.rs`, the "Stamp the holder AFTER acquiring" block). Probing that way would make `queue next` write to every cassette it looks at, create anchor files that never existed, fail on a read-only store, and break this plan's rule that `view.rs` performs no writes. `probe` must acquire and release **without stamping**.

- [ ] **Step 1: Write the failing test for `probe`**

In `src/store/lock.rs`'s test module:

```rust
#[test]
fn probe_reports_free_without_creating_or_stamping_the_anchor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let anchor = dir.path().join("01K5GR7T2M9WPD0000000000AB");

    // A cassette nobody has ever locked has no anchor file. That is free,
    // and probing must not bring the file into existence.
    assert!(probe(&anchor).expect("probe"), "an absent anchor is free");
    assert!(!anchor.exists(), "probe must not create the anchor: it is a read");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib lock`
Expected: FAIL — `probe` does not exist.

- [ ] **Step 3: Implement `probe`**

```rust
/// Is this lock free right now? Opens the anchor without creating it and
/// tries the lock, releasing immediately when it succeeds.
///
/// Deliberately NOT `acquire` with the guard dropped: `acquire` stamps the
/// holder into the anchor after locking, so using it to ask a question would
/// write to every cassette the caller merely looked at. The answer is a
/// snapshot — the lock may be taken the instant after this returns — which is
/// why the only caller, `queue next`, reports rather than claims.
pub(crate) fn probe(anchor_path: &Path) -> io::Result<bool> {
    let anchor = match std::fs::OpenOptions::new().read(true).write(true).open(anchor_path) {
        Ok(f) => f,
        // No anchor means nobody has ever locked this cassette.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    match FileExt::try_lock(&anchor) {
        Ok(()) => Ok(true), // released when `anchor` drops
        Err(fs4::TryLockError::WouldBlock) => Ok(false),
        Err(fs4::TryLockError::Error(e)) => Err(e),
    }
}
```

Use UFCS (`FileExt::try_lock`) for the reason the file's existing comment gives: this toolchain's `std::fs::File` has inherent `lock`/`try_lock` that would otherwise shadow fs4's.

Add the `Store` wrapper:

```rust
    /// Whether `id`'s lock is free right now. A snapshot, not a claim.
    pub fn is_free(&self, session: &str, id: &str) -> io::Result<bool> {
        lock::probe(&self.locks_dir(session).join(id))
    }
```

- [ ] **Step 4: Implement `next`**

Walk the session's cassettes in `queue_order`, skipping closed ones, and return the first id for which `is_free` is true. Distinguish the two empty-handed outcomes — this is the spec's decision 4:

```rust
/// The next cassette a writer should take, or an error saying why there is
/// none. A one-shot process cannot hold a lock for its caller — flock dies
/// with the process — so this reports an id and the caller races for it with
/// `queue write`. The window between is real and is closed by `queue write`
/// returning exit 3, not by anything here.
pub fn next(store: &Store, session: &str) -> Result<String, QueueError> { ... }
```

- open cassettes exist, at least one free → `Ok(id)`
- open cassettes exist, **every one busy** → `QueueError::Busy("every open cassette is being written — try again shortly")` (exit 3)
- no open cassettes at all → `QueueError::Empty` (exit 5)

Add `Empty` to `QueueError` with a doc comment naming exit 5, and map it in `main.rs` to `die_with(5, ...)`.

- [ ] **Step 5: Add the CLI test**

```rust
#[test]
fn queue_next_on_an_empty_session_exits_five() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let id = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "next", "--session", &id])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(5), "{}", stderr(&out));
}
```

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add queue next, skipping cassettes another writer holds

Probing uses a non-stamping lock::probe rather than acquire-and-drop:
acquire writes the holder's attribution into the anchor, which would make a
read-only command write to every cassette it looked at.

'Every open cassette is busy' is exit 3, not 5 — 3 says wait, 5 says enqueue."
```

---

### Task 6: `queue new` — placement, the open cap, and renumbering

**Files:**
- Create: `src/queue/edit.rs`
- Modify: `src/queue/mod.rs` (`QueueError::Full`), `src/config.rs` (`max_open`), `src/store/mod.rs` (`MAX_OPEN`)
- Modify: `src/cli.rs`, `src/main.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `Store::add_cassette`, `Store::lock_many`, `priority::{first, last, renumber, queue_order}`, `SessionScan` from Task 4.
- Produces:
  - `store::MAX_OPEN: usize = 36`
  - `queue::edit::new(store, session, topic, placement: Placement, who_name, source, max_open: usize) -> Result<String, QueueError>`
  - `queue::edit::renumber_all(store, session, who: &Attribution) -> Result<(), QueueError>`
  - `queue::Placement { First, Last, Explicit(i64) }`
  - `QueueError::Full(String)` → exit 6

- [ ] **Step 1: Write the failing tests**

In `src/queue/edit.rs`. These are unit tests over the cap arithmetic and the placement choice, which are the parts with real edge cases:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_positive_explicit_priority_is_rejected() {
        // priority::first treats non-positive results as "no room left"
        // (`below > 0`, `halved > 0`), so a stored 0 would be a value the
        // placement functions cannot reason about. Reject at the boundary.
        for bad in [0i64, -1, i64::MIN] {
            assert!(
                validate_priority(bad).is_err(),
                "{bad} must be rejected at the CLI boundary"
            );
        }
        assert!(validate_priority(1).is_ok());
    }

    #[test]
    fn the_cap_counts_open_cassettes_only() {
        // A session full of closed cassettes is not a full queue.
        assert!(!is_full(&[Status::Closed, Status::Closed], 2));
        assert!(is_full(&[Status::Open, Status::Open], 2));
        assert!(!is_full(&[Status::Open, Status::Closed], 2));
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib queue`
Expected: FAIL — `validate_priority` and `is_full` do not exist.

- [ ] **Step 3: Add the cap constant and config key**

In `src/store/mod.rs`:

```rust
/// The most open cassettes one session may hold, overridable by the
/// `max_open` config key.
///
/// Defined here and NOT taken from `app::MAX_CASSETTES`, which happens to be
/// the same number: that one is a TUI display concern (how many cassettes the
/// stack can show and select), and binding the store's cap to it would assert
/// a relationship the code does not have.
pub const MAX_OPEN: usize = 36;
```

In `src/config.rs` add to `Config`:

```rust
    /// Most open cassettes one session may hold; defaults to
    /// `store::MAX_OPEN` (36).
    pub max_open: Option<usize>,
```

`main.rs` resolves `cfg.max_open.unwrap_or(store::MAX_OPEN)` and passes it in, so the queue module takes the cap as a parameter and stays testable without a config file.

- [ ] **Step 4: Implement `new`**

Order of operations, each step deliberate:

1. Resolve the writer (`resolve_writer`/`require_writer` by `WriterSource`) — **before** anything else, matching `write`'s existing rationale.
2. Scan the session. If the count of `Status::Open` is `>= max_open`, return `QueueError::Full` naming the cap. Do this before computing a priority so a full queue is never charged a renumber.
3. Compute the priority from `Placement`: `Last` → `priority::last(&open_priorities)`, `First` → `priority::first(&open_priorities)`, `Explicit(n)` → `validate_priority(n)?`. `Last` is the default: an agent adding work cannot jump the human's line.
4. If `first`/`last` returned `None`, call `renumber_all`, re-scan, and retry the placement **once**. A second `None` is `QueueError::Io` — it means the run is genuinely unrepresentable, which is only reachable through hand-edited frontmatter.
5. Build the `CassetteMeta` (`created_by` and `last_writer` both the resolved writer id, `status: Open`, `locked_by: None`, `updated_at: now_utc()`) and call `Store::add_cassette`, which takes and releases the new cassette's lock itself.
6. Print the new id.

- [ ] **Step 5: Implement `renumber_all`**

```rust
/// Give every cassette in the session fresh sparse priorities, in queue
/// order. The one consumer of `lock_many`.
///
/// **Call this holding no locks.** flock is per-open-file-description: a
/// second open+lock of a file this process already holds through another
/// descriptor does not "already own" it, so renumbering while holding a
/// cassette's guard would deadlock against itself. Every caller therefore
/// computes its placement first, releases, renumbers, and retries.
pub fn renumber_all(store: &Store, session: &str, who: &Attribution) -> Result<(), QueueError> { ... }
```

Scan, `queue_order` the metas, take every lock with `lock_many`, then write each cassette through its own guard with the priority from `priority::renumber(count)`, matching guards to metas by id rather than by position — `lock_many` sorts its input into ascending id order, so the guards it returns are **not** in the order the ids were passed.

- [ ] **Step 6: Wire the CLI**

`queue new <TOPIC> --session <id> [--first|--last|--priority N]`. Make the three placement flags mutually exclusive with clap's `conflicts_with_all`, defaulting to `--last`.

- [ ] **Step 7: Add CLI tests**

```rust
#[test]
fn queue_new_rejects_a_non_positive_priority() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "new", "a topic", "--session", &sid, "--priority", "0"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_new_then_next_returns_the_new_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let made = Command::new(bin())
        .args(["queue", "new", "gratitude", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(made.status.code(), Some(0), "{}", stderr(&made));
    let cid = String::from_utf8_lossy(&made.stdout).trim().to_string();

    let next = Command::new(bin())
        .args(["queue", "next", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(next.status.code(), Some(0), "{}", stderr(&next));
    assert_eq!(String::from_utf8_lossy(&next.stdout).trim(), cid);
}
```

- [ ] **Step 8: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add queue new with the open cap and priority validation

The cap is defined store-side rather than borrowed from app::MAX_CASSETTES,
which is a TUI display concern that shares the number by coincidence.
--priority rejects non-positive values: priority::first treats them as
'no room left', so storing one would be a value placement cannot reason about."
```

---

### Task 7: `queue close` and `queue reopen`

**Files:**
- Modify: `src/queue/edit.rs`, `src/queue/mod.rs` (`QueueError::Sticky`)
- Modify: `src/cli.rs`, `src/main.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Produces:
  - `queue::edit::close(store, session, id, message: Option<&str>, who_name, source) -> Result<(), QueueError>`
  - `queue::edit::reopen(store, session, id, who_name, source, max_open: usize) -> Result<(), QueueError>`
  - `QueueError::Sticky(String)` → exit 4

**Ruling on `-m`:** the close-out sentence is appended to the body as a final blockquote line (`\n> <message>\n`). It is a note *about* the cassette rather than cassette prose, and a blockquote survives the markdown round-trip through `meta::split` without colliding with the `## Side A` / `## Side B` headings `output::parse_markdown` looks for. Reject a message containing a newline as `QueueError::Usage`: it would inject a second body line that is not a quote.

- [ ] **Step 1: Write the failing test**

In `src/queue/edit.rs`:

```rust
#[test]
fn an_agent_may_not_close_a_sticky_locked_cassette_but_a_human_may() {
    // The permission boundary this phase's writer `kind` exists for.
    // Nothing in 4b SETS locked_by — `queue lock` is 4c — so this is the
    // rule being in place before the command that makes it reachable.
    assert!(matches!(
        close_permitted(Kind::Agent, Some("01WRITER")),
        Err(QueueError::Sticky(_))
    ));
    assert!(close_permitted(Kind::Human, Some("01WRITER")).is_ok());
    assert!(close_permitted(Kind::Agent, None).is_ok());
}

#[test]
fn a_close_message_may_not_contain_a_newline() {
    assert!(close_message_line("done for now").is_ok());
    assert!(
        close_message_line("done\n## Side B").is_err(),
        "a newline would inject a body line that is not a quote"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib queue`
Expected: FAIL — `close_permitted` and `close_message_line` do not exist.

- [ ] **Step 3: Implement `close`**

1. Resolve the writer, keeping the `Kind` this time — `queue/write.rs` binds it as `_kind` today with a comment saying this is where it starts to matter.
2. Acquire the lock. `Busy` → `QueueError::Busy` (exit 3): a cassette someone is writing cannot be closed by anyone, human or agent.
3. Read through the guard, then apply `close_permitted(kind, meta.locked_by.as_deref())`. An agent facing a set `locked_by` gets `QueueError::Sticky` (exit 4).
4. Set `status = Status::Closed`, `last_writer`, `updated_at`; append the blockquote when `-m` was given; write through the guard.

- [ ] **Step 4: Implement `reopen`**

The same lock-and-write shape, setting `status = Status::Open`. It raises the open count, so it takes the cap check from Task 6 and returns `QueueError::Full` (exit 6) when the session is already at `max_open`. Reopening is not gated on `locked_by`: the sticky lock guards *closing* work someone claimed, and nothing is claimed by reopening.

- [ ] **Step 5: Add a CLI test**

```rust
#[test]
fn queue_close_then_list_shows_it_closed_and_reopen_restores_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let out = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let cid = {
        let out = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let closed = Command::new(bin())
        .args(["queue", "close", &cid, "--session", &sid, "-m", "done for now"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(closed.status.code(), Some(0), "{}", stderr(&closed));

    // Closed cassettes are hidden by the default --status open filter.
    let listed = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(
        !String::from_utf8_lossy(&listed.stdout).contains(&cid),
        "a closed cassette must not show under --status open"
    );

    let reopened = Command::new(bin())
        .args(["queue", "reopen", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(reopened.status.code(), Some(0), "{}", stderr(&reopened));

    let again = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&again.stdout).contains(&cid));
}
```

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add queue close and reopen with the sticky-lock boundary

An agent refuses to close a cassette whose locked_by is set (exit 4); a human
may. Nothing in 4b sets that field — queue lock is 4c — so the rule is in
place before the command that makes it reachable."
```

---

### Task 8: `queue move`

**Files:**
- Modify: `src/queue/edit.rs`
- Modify: `src/cli.rs`, `src/main.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Produces:
  - `queue::MoveAnchor { Before(String), After(String) }` — the CLI form, carrying the anchor cassette's id
  - `queue::MoveSide { Before, After }` — the same choice without the id, so the placement arithmetic is testable without a store. `MoveAnchor::side(&self) -> MoveSide` converts.
  - `queue::edit::target_priority(sorted_open: &[i64], anchor_priority: i64, side: MoveSide) -> Option<i64>` — pure; `None` means "no gap, renumber"
  - `queue::edit::move_cassette(store, session, id, anchor: MoveAnchor, who_name, source) -> Result<(), QueueError>`

Named `move_cassette`, not `move`: `move` is a Rust keyword.

**The ordering hazard — read this before writing code.** The obvious implementation locks the cassette being moved, then discovers there is no priority gap, then calls `renumber_all` — which tries to lock every cassette *including the one already held*. flock is per-open-file-description, so a second open of a file this process already locked through another descriptor does **not** succeed; it reports `Busy`, and the command deadlocks against itself with a message blaming a phantom other writer.

Do it in this order instead, holding at most one lock at a time:

1. Scan the session **holding no locks**. Compute the target priority with `priority::between` against the anchor and its neighbour in `queue_order`.
2. If that is `None`, call `renumber_all` (which takes and releases every lock), re-scan, and recompute **once**. A second `None` is `QueueError::Io`.
3. Lock only the cassette being moved, re-read it through the guard, set the new priority, `last_writer` and `updated_at`, and write.

Reading priorities without a lock in step 1 is a deliberate, bounded race: another writer may change a priority between the read and the write, which lands the moved cassette in a slightly wrong position. That is a display-order inaccuracy, not corruption, and it self-corrects on the next move. Taking every lock to make step 1 atomic would mean a single busy cassette blocks all reordering — the opposite of this spec's "never block on a lock a human can hold".

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn moving_before_the_head_takes_a_priority_above_zero() {
    // `first`-style placement: there is no lower neighbour, so the new
    // priority is derived from the head alone and must stay positive.
    let existing = [10i64, 20, 30];
    let p = target_priority(&existing, /* anchor */ 10, MoveSide::Before)
        .expect("a gap exists below 10");
    assert!(p > 0, "priorities are positive: got {p}");
    assert!(p < 10, "before the head means below it: got {p}");
}

#[test]
fn adjacent_neighbours_report_no_gap_so_the_caller_renumbers() {
    // 10 and 11 have no integer between them. `None` is the signal to
    // renumber, not an error.
    assert!(target_priority(&[10, 11], 11, MoveSide::Before).is_none());
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test --lib queue` — FAIL, `target_priority` does not exist. Implement it as a pure function over the sorted priorities so both cases above are unit-testable without a store.

- [ ] **Step 3: Wire the CLI**

`queue move <ID> --session <id> --before <ID> | --after <ID>`, the two anchors mutually exclusive and exactly one required (`ArgGroup` with `required(true)`). Moving a cassette relative to itself is `QueueError::Usage`.

- [ ] **Step 4: Add a CLI test**

Create three cassettes, move the third before the first, and assert `queue list` prints them in the new order. Assert on relative positions with `find`, never on exact formatting.

- [ ] **Step 5: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add queue move

Placement is computed before any lock is taken, so exhausting the gap can
renumber without deadlocking against a guard this process already holds —
flock is per-open-file-description and does not recognise its own owner."
```

---

### Task 9: `$CASSETTE_WRITER`, cross-process tests, and docs

**Files:**
- Modify: `src/main.rs` (`resolve_writer_name`)
- Modify: `tests/lock.rs` (two new cases)
- Modify: `README.md`, `CLAUDE.md`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8.

- [ ] **Step 1: Implement `$CASSETTE_WRITER`**

Precedence in `resolve_writer_name`: `--writer` > `$CASSETTE_WRITER` > `$USER`. Both `--writer` and `$CASSETTE_WRITER` yield `WriterSource::Flag` — naming a writer explicitly is a claim about identity, so a typo must fail loudly (exit 2) rather than auto-create a second identity as `human`, the privileged kind. Only `$USER` is `WriterSource::Env` and may bootstrap.

The spec declined the equivalent for sessions on purpose: writer identity belongs to an agent process for its lifetime, while a session id is a per-invocation argument.

Test:

```rust
#[test]
fn cassette_writer_env_names_a_writer_but_must_already_be_registered() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let out = Command::new(bin())
        .args(["writer", "whoami"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("CASSETTE_WRITER", "ghost")
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}
```

- [ ] **Step 2: Add the two cross-process tests**

In `tests/lock.rs`, reusing its existing discipline — **no sleeps**, and a child's acquisition proven by output it writes after acquiring, never by the parent's write to its pipe.

1. **`queue next` skips a cassette a live holder is in.** Build a session with two open cassettes. Spawn `queue write` on the first with an open stdin pipe (it holds the lock). Then run `queue next` and assert it returns the *second* id. Close the pipe.
2. **`queue close` on a held cassette exits 3.** With the same holder live, run `queue close` on the held cassette and assert exit 3 and that stderr names the holder.

Reuse `fixture()` — extend it to write a second cassette — and `spawn_write`. Follow the existing note on `spawn_write`: a `BrokenPipe` from the parent's write is not a failure, it only means that child exited first.

- [ ] **Step 3: Update the docs**

In `README.md`, extend "The session store (in progress)" with the command surface, and state plainly that there is no active session: every `queue` command takes `--session <id>`, and `session list` is how an id is recovered. Note that an alias is a display label that never resolves.

In `CLAUDE.md`, add `src/session.rs` and `src/queue/` to the source layout with one line each, following the existing entries' style, and correct the Commands block to show the subcommands this phase adds.

- [ ] **Step 4: Full verification**

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Then confirm invariant 1 still holds on the finished branch — the only non-test callers of `atomic_write` must be `session.rs`, `writers.rs`, and `lock.rs`'s `LockGuard::write`, and nothing under `cassettes/` may be written outside a guard:

```bash
grep -rn 'atomic_write' src/ | grep -v '^src/store/mod.rs' | grep -v test
grep -rn 'fn write' src/queue/view.rs || echo "view.rs is write-free, as required"
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add \$CASSETTE_WRITER, concurrency tests, and docs

An env var naming a writer is explicit like the flag, so an unknown name is
exit 2; only \$USER bootstraps. The two cross-process tests cover the pair of
behaviours two nearby writers actually exercise: next skips a held cassette,
and close on one exits 3."
```

---

## Plan Self-Review

**Spec coverage.** Every section of the 4b spec maps to a task: decision 1 → Task 1; decision 2 → Task 3; decisions 3 and 4 → Task 5; command surface → Tasks 3–8; module layout → Task 4; the open cap → Task 6 (enforced again in Task 7's `reopen`); positive priorities → Task 6; close permission and exit 4 → Task 7; exit codes → introduced by the task that first raises each; unreadable cassettes → Task 4; carried gaps → Task 2 (`WriterError`, `write_writers`) and Task 9 (`CASSETTE_WRITER`); removals → Task 1; testing → Tasks 3–9, cross-process in Task 9.

**Known deviation from the spec, recorded here rather than silently:** the spec's module table lists `queue/edit.rs` as created alongside `view.rs` in the split. This plan creates `edit.rs` in Task 6 instead, because Task 4 has no mutating command to put in it and an empty module is not reviewable. The end state matches the spec.

**Ordering checks.** Task 1 must precede all others (every later task uses the required-`--session` shape). Task 2 must precede Tasks 3–8 (they consume the split error types). Task 6 must precede Tasks 7 and 8 (both reuse `is_full` and `renumber_all`). Tasks 4 and 5 both touch `view.rs`; 4 creates it, so 4 precedes 5.

**Type consistency.** `SessionScan` (Task 4) is consumed by Tasks 5–8 as `.cassettes` / `.unreadable`. `Placement`, `MoveAnchor` and `MoveSide` live in `queue/mod.rs` beside `StatusFilter`; `MoveAnchor` is what the CLI produces and `MoveSide` is what the pure `target_priority` takes, which is why both exist. `QueueError` grows `Empty` (Task 5), `Full` (Task 6) and `Sticky` (Task 7); each task adds the matching `main.rs` arm in the same commit, so the match stays exhaustive at every commit.

**Cap enforcement appears twice on purpose:** `queue new` and `queue reopen` are the only two commands that raise the open count.
