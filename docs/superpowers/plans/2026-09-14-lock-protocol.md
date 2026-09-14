# Lock Protocol Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `flock` on a `.locks/<id>` sidecar the only way to write a cassette, so two
writers — a human in the TUI and an agent on the CLI — can work the same session without
clobbering each other.

**Architecture:** A new `src/store/lock.rs` holds `Attribution`, `LockError` and
`LockGuard`. `LockGuard` owns the write: `Store::write_cassette` is removed and
`guard.write()` takes its place, so "you may only write a cassette whose lock you hold"
becomes a signature rather than a rule. Cassette locks are non-blocking; the writer
registry gets a blocking lock behind a private method. One CLI subcommand,
`cassette queue write`, is pulled forward from Phase 4 to give the concurrency tests a
second process that genuinely holds a lock.

**Tech Stack:** Rust 2021, `fs4` 1.1 (pure-Rust `flock`), `tempfile` 3 (dev, already
present), `clap` 4 derive (already present).

**Spec:** `docs/superpowers/specs/2026-09-14-lock-protocol-design.md`
(parent: `docs/superpowers/specs/2026-09-13-session-store-design.md`)

## Global Constraints

Copied from the spec. Every task's requirements implicitly include this section.

- **Invariant 1:** "You may only write a cassette whose lock you hold. Every cassette you
  do not hold, you re-read from disk."
- **Invariant 2:** "Acquire-and-write is one operation." Advisory listings may
  test-acquire to display busy state, but nothing in the write path may check-then-act.
- **Invariant 4:** all writes are write-temp-then-`rename()` within the same directory.
- **Never block on a lock a human can hold.** Cassette locks use `try_lock`
  (non-blocking); the registry lock uses `lock` (blocking) and is private to
  `ensure_writer`.
- **Locks live on the sidecar, never the `.md`.** `flock` attaches to an inode and
  `rename()` replaces the inode, so locking the cassette file is silently broken.
- **The anchor's existence carries no meaning.** Anchors are never deleted. Lockedness is
  kernel state, tested by attempting acquisition — never by the file existing.
- **Anchor contents are display-only.** Stale bytes after a crash are harmless.
- **Acquisition order is ascending id, and is NOT queue order.** It exists only so two
  overlapping multi-lock operations cannot livelock. Queue order remains `priority` first,
  closed last, id as tiebreak. Never name the acquisition constant "sort order".
- Exit codes: 0 ok, 1 I/O failure, 2 usage, 3 busy. 4/5/6 reserved for Phase 4.
- Data dir and every directory under it is created `0700` via `ensure_private_dir`.
- All dependencies are **pure Rust**; no C toolchain.
- `flock` itself is not tested — that is the kernel's job.

### Facts verified against the current tree (2026-09-14)

- `fs4` resolves to **1.1.0**. Its trait is `fs4::fs_std::FileExt` with methods
  **`try_lock()`** (non-blocking exclusive), **`lock()`** (blocking exclusive) and
  `unlock()`. These are NOT the `try_lock_exclusive` / `lock_exclusive` names used by
  older fs2-derived releases — do not use those.
- `fs4::TryLockError` is `{ Error(io::Error), WouldBlock }`.
- The lock is released when the `File` is dropped, so a guard holding a `File` needs no
  explicit `Drop` impl for correctness.
- `Store` today exposes `new, default_root, sessions_dir, session_dir, cassettes_dir,
  locks_dir, create_session, add_cassette, write_cassette, session_meta, writers,
  write_writers, ensure_writer, active_session, set_active_session, scan_session`.
- `src/store/mod.rs` carries a module-wide `#![allow(dead_code)]`; no per-item allows.
- Suite is **218 tests** (204 unit + 14 CLI) and must stay green.

## File Structure

| File | Responsibility |
|---|---|
| `src/store/lock.rs` (create) | `Attribution`, `LockError`, `LockGuard`, anchor acquisition |
| `src/store/mod.rs` (modify) | `Store::{lock, lock_many, lock_registry, cassette_path, root_locks_dir}`; `write_cassette` removed; `add_cassette` routed through a guard |
| `src/store/writers.rs` (modify) | `ensure` takes the registry lock around its read-modify-write |
| `src/cli.rs` (modify) | `queue write` subcommand |
| `src/main.rs` (modify) | dispatch `queue write`; map `LockError` to exit codes |
| `tests/lock.rs` (create) | cross-process concurrency tests, including `SIGKILL` |
| `Cargo.toml` (modify) | add `fs4 = "1"` |

---

### Task 1: `Attribution`

**Files:**
- Modify: `Cargo.toml`
- Create: `src/store/lock.rs`
- Modify: `src/store/mod.rs` (add `pub mod lock;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `store::lock::Attribution { writer: String, name: String, pid: u32, since: String }`
  - `Attribution::render(&self) -> String`
  - `Attribution::parse(&str) -> Option<Attribution>`
  - `Attribution::for_now(writer: &str, name: &str) -> Attribution`

- [ ] **Step 1: Add the dependency**

```bash
cd /home/jozzef/Atelier/software/cassette
cargo add fs4@1
```

Confirm `Cargo.toml` gains `fs4 = "1"` and that the default `sync` feature is on (it is;
the async features are off). Pure Rust, no C toolchain.

- [ ] **Step 2: Write the failing tests**

Create `src/store/lock.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn attribution() -> Attribution {
        Attribution {
            writer: "01K5H2WRITERID000000000000".to_string(),
            name: "joseph".to_string(),
            pid: 4242,
            since: "2026-09-14T14:02:11Z".to_string(),
        }
    }

    #[test]
    fn attribution_round_trips() {
        let a = attribution();
        assert_eq!(Attribution::parse(&a.render()).expect("parses"), a);
    }

    #[test]
    fn rendered_form_matches_the_spec() {
        assert_eq!(
            attribution().render(),
            "writer=01K5H2WRITERID000000000000 name=joseph pid=4242 since=2026-09-14T14:02:11Z"
        );
    }

    #[test]
    fn a_name_containing_spaces_survives() {
        // Names are free text and the fields are space-separated, so parsing
        // must key off the field markers rather than splitting on whitespace.
        let mut a = attribution();
        a.name = "joseph the writer".to_string();
        let parsed = Attribution::parse(&a.render()).expect("parses");
        assert_eq!(parsed.name, "joseph the writer");
        assert_eq!(parsed.pid, 4242, "fields after the name must still parse");
    }

    #[test]
    fn a_trailing_newline_is_tolerated() {
        let a = attribution();
        let parsed = Attribution::parse(&format!("{}\n", a.render())).expect("parses");
        assert_eq!(parsed, a);
    }

    #[test]
    fn garbage_does_not_parse() {
        // A crashed holder can leave anything here. Contents are display-only,
        // so unparseable bytes must be None rather than a panic or a partial.
        assert!(Attribution::parse("").is_none());
        assert!(Attribution::parse("not an attribution").is_none());
        assert!(Attribution::parse("writer=x name=y pid=notanumber since=z").is_none());
        assert!(Attribution::parse("writer=x name=y").is_none());
    }

    #[test]
    fn for_now_stamps_this_process() {
        let a = Attribution::for_now("writer-1", "joseph");
        assert_eq!(a.writer, "writer-1");
        assert_eq!(a.name, "joseph");
        assert_eq!(a.pid, std::process::id());
        assert!(a.since.ends_with('Z'), "{}", a.since);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test store::lock`
Expected: FAIL to compile — `cannot find type 'Attribution' in this scope`.

- [ ] **Step 4: Implement**

Put this above the test module in `src/store/lock.rs`:

```rust
//! Cassette locking: `flock` on a `.locks/<id>` sidecar.
//!
//! The lock is on the sidecar and never on the `.md`, because `flock` attaches
//! to an inode and an atomic write replaces the inode via `rename()` — locking
//! the cassette file would silently hand two writers the same "lock". The
//! anchor is never renamed and never deleted, so its inode is stable, and its
//! *existence* carries no meaning: lockedness is kernel state, tested by
//! attempting acquisition.

use crate::store::meta;

/// Who holds a lock. Written into the anchor after acquiring — an ordering the
/// lock itself serializes — and read by a blocked writer for its message.
///
/// Display-only. Stale contents after a crash are harmless, because lockedness
/// is decided by kernel `flock` state and never by these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub writer: String,
    pub name: String,
    pub pid: u32,
    /// RFC3339 UTC.
    pub since: String,
}

impl Attribution {
    /// This process, holding a lock as of now.
    pub fn for_now(writer: &str, name: &str) -> Attribution {
        Attribution {
            writer: writer.to_string(),
            name: name.to_string(),
            pid: std::process::id(),
            since: meta::now_utc(),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "writer={} name={} pid={} since={}",
            self.writer, self.name, self.pid, self.since
        )
    }

    /// Parse the rendered form. `None` for anything else — a crashed holder can
    /// leave arbitrary bytes here and the caller degrades to a generic message.
    ///
    /// Fields are located by their markers rather than by splitting on
    /// whitespace, because `name` is free text and may contain spaces.
    pub fn parse(line: &str) -> Option<Attribution> {
        let line = line.trim();
        let rest = line.strip_prefix("writer=")?;
        let (writer, rest) = rest.split_once(" name=")?;
        let (name, rest) = rest.split_once(" pid=")?;
        let (pid, since) = rest.split_once(" since=")?;
        Some(Attribution {
            writer: writer.to_string(),
            name: name.to_string(),
            pid: pid.parse().ok()?,
            since: since.to_string(),
        })
    }
}
```

Add `pub mod lock;` to `src/store/mod.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test store::lock`
Expected: PASS, 6 tests.

Then: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green; 224 total (210 unit + 14 CLI).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/store/
git commit -m "feat: lock attribution, written into the anchor after acquiring"
```

---

### Task 2: `LockGuard` and `Store::lock`

**Files:**
- Modify: `src/store/lock.rs`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `Attribution`, `Store::{locks_dir, cassettes_dir}`, `store::atomic_write`,
  `store::ensure_private_dir`, `meta::{build_frontmatter, split}`, `ids::id_from_file_name`.
- Produces:
  - `store::lock::LockError { Busy(Option<Attribution>), Io(io::Error) }`
  - `store::lock::LockGuard` with `read()`, `write(&CassetteMeta, &str)`, `id()`, `path()`
  - `Store::cassette_path(&self, session: &str, id: &str) -> io::Result<Option<PathBuf>>`
  - `Store::lock(&self, session: &str, id: &str, as_writer: &Attribution) -> Result<LockGuard, LockError>`

`Busy` carries `Option<Attribution>` rather than `Attribution`: a holder that crashed
before writing its line, or one whose bytes are garbage, still blocks us, and we must
report that without inventing a holder.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `src/store/lock.rs`:

```rust
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::session::SessionMeta;
    use crate::store::Store;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn session_meta() -> SessionMeta {
        SessionMeta {
            alias: None,
            created: meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        }
    }

    fn cassette_meta(id: &str) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: "writer-1".to_string(),
            last_writer: "writer-1".to_string(),
            updated_at: meta::now_utc(),
        }
    }

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";

    #[test]
    fn a_lock_can_be_taken_and_written_through() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        let m = cassette_meta(ID);
        s.add_cassette(&sid, &m, "## Side A\n\nold\n").expect("add");

        let who = Attribution::for_now("writer-1", "joseph");
        let guard = s.lock(&sid, ID, &who).expect("acquire");
        assert_eq!(guard.read().expect("read").body, "## Side A\n\nold\n");
        guard.write(&m, "## Side A\n\nnew\n").expect("write");
        assert_eq!(guard.read().expect("read").body, "## Side A\n\nnew\n");
    }

    #[test]
    fn a_second_acquisition_is_busy_and_names_the_holder() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");

        let who = Attribution::for_now("writer-1", "joseph");
        let _held = s.lock(&sid, ID, &who).expect("first acquire");
        match s.lock(&sid, ID, &Attribution::for_now("writer-2", "agent")) {
            Err(LockError::Busy(Some(a))) => {
                assert_eq!(a.name, "joseph", "the blocked writer must learn who holds it");
                assert_eq!(a.pid, std::process::id());
            }
            other => panic!("expected Busy with attribution, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_guard_releases_the_lock() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        {
            let _g = s.lock(&sid, ID, &who).expect("acquire");
        }
        s.lock(&sid, ID, &who).expect("must be free after drop");
    }

    #[test]
    fn two_threads_genuinely_contend() {
        // flock is per open file description, so two threads in one process
        // really do contend. Under fcntl locks they would silently share the
        // lock and this would pass for the wrong reason.
        use std::sync::mpsc;
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        let held = s.lock(&sid, ID, &who).expect("acquire");

        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let r = s.lock(&sid, ID, &Attribution::for_now("writer-2", "agent"));
                tx.send(matches!(r, Err(LockError::Busy(_)))).expect("send");
            });
        });
        assert!(rx.recv().expect("recv"), "the other thread must be blocked");
        drop(held);
    }

    #[test]
    fn locking_creates_the_anchor_when_it_is_missing() {
        // A cassette written by hand, or predating this phase, must still be
        // lockable — the anchor is created on demand.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        std::fs::remove_file(s.locks_dir(&sid).join(ID)).expect("remove anchor");
        let who = Attribution::for_now("writer-1", "joseph");
        s.lock(&sid, ID, &who).expect("must create the anchor on demand");
        assert!(s.locks_dir(&sid).join(ID).is_file());
    }

    #[test]
    fn a_garbled_anchor_still_blocks_without_inventing_a_holder() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        let _held = s.lock(&sid, ID, &who).expect("acquire");
        std::fs::write(s.locks_dir(&sid).join(ID), "garbage").expect("clobber");
        match s.lock(&sid, ID, &who) {
            Err(LockError::Busy(None)) => {}
            other => panic!("expected Busy(None), got {other:?}"),
        }
    }

    #[test]
    fn locking_an_unknown_cassette_is_an_error_not_a_lock() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        let who = Attribution::for_now("writer-1", "joseph");
        assert!(
            s.lock(&sid, "nosuchcassette0000000000AB", &who).is_err(),
            "locking a cassette that does not exist must fail"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::lock`
Expected: FAIL to compile — `no method named 'lock' found for struct 'Store'`.

- [ ] **Step 3: Implement the guard**

Add to `src/store/lock.rs`:

```rust
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

use crate::store::meta::CassetteMeta;
use crate::store::StoredCassette;

/// Why a lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another live writer holds it. `None` when the holder left no readable
    /// attribution — a crash before writing its line, or garbled bytes. It
    /// still blocks us; we just cannot name it.
    Busy(Option<Attribution>),
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy(Some(a)) => {
                write!(f, "held by {} (since {})", a.name, a.since)
            }
            LockError::Busy(None) => write!(f, "held by another writer"),
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for LockError {
    fn from(e: io::Error) -> LockError {
        LockError::Io(e)
    }
}

/// An held cassette lock. The only way to write an existing cassette.
///
/// The lock is released when this value is dropped, and by the kernel if the
/// process dies for any reason including `SIGKILL` — which is why there is no
/// reaper, no pid file and no `--force-unlock`.
pub struct LockGuard {
    id: String,
    /// The cassette file. Resolved once at acquisition: the slug is frozen at
    /// creation, so this path cannot be derived from a (possibly retopicked)
    /// `CassetteMeta`.
    path: PathBuf,
    /// Holding the `File` IS holding the lock: dropping it closes the fd and
    /// the kernel releases. No `Drop` impl needed, and none should be added —
    /// an explicit `unlock()` before close would be redundant and would give a
    /// future reader the impression that release is our responsibility.
    _anchor: File,
}

impl LockGuard {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cassette's current state, read under the lock.
    pub fn read(&self) -> io::Result<StoredCassette> {
        let content = std::fs::read_to_string(&self.path)?;
        let (meta, body) = crate::store::meta::split(&content);
        let meta = meta.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("'{}' has no cassette frontmatter", self.path.display()),
            )
        })?;
        Ok(StoredCassette {
            path: self.path.clone(),
            meta,
            body: body.to_string(),
        })
    }

    /// Replace the cassette. The file keeps the name it was minted with: a
    /// rename would move the inode out from under another writer's resolved
    /// path, which is the same hazard that put the lock on a sidecar.
    pub fn write(&self, m: &CassetteMeta, body: &str) -> io::Result<()> {
        crate::store::atomic_write(
            &self.path,
            &format!("{}\n{}", crate::store::meta::build_frontmatter(m), body),
        )
    }
}
```

- [ ] **Step 4: Implement `Store::lock` and `Store::cassette_path`**

Add to `impl Store` in `src/store/mod.rs`:

```rust
    /// The file backing a cassette id, found by its `-<id>.md` suffix. The
    /// slug half of the name is frozen at creation, so the path cannot be
    /// derived from a `CassetteMeta` whose topic may since have changed.
    pub fn cassette_path(&self, session: &str, id: &str) -> io::Result<Option<PathBuf>> {
        let dir = self.cassettes_dir(session);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if ids::id_from_file_name(&name) == Some(id) {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    /// Acquire a cassette's lock. **Never blocks** — a human may hold this one
    /// for as long as they keep the cassette focused, so a blocked caller gets
    /// `Busy` and writes a different cassette instead.
    ///
    /// `as_writer` is stamped into the anchor after acquiring, which the lock
    /// itself serializes.
    pub fn lock(
        &self,
        session: &str,
        id: &str,
        as_writer: &lock::Attribution,
    ) -> Result<lock::LockGuard, lock::LockError> {
        let path = self
            .cassette_path(session, id)?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no cassette '{id}' in session '{session}'"),
                )
            })?;
        let anchor_path = self.locks_dir(session).join(id);
        lock::acquire(id, path, &anchor_path, as_writer, lock::Blocking::No)
    }
```

Add `pub mod lock;` if Task 1 did not, and `use crate::store::lock;` is unnecessary —
`lock` is a sibling module, reachable as `lock::`.

- [ ] **Step 5: Implement the shared acquisition routine**

Add to `src/store/lock.rs`:

```rust
/// Whether an acquisition waits. See the spec's "never block on a lock a human
/// can hold": cassette locks are `No`, the writer registry is `Yes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blocking {
    No,
    Yes,
}

/// Open (creating if needed) the anchor, take the flock, and stamp the holder.
///
/// The anchor is created on demand so a cassette written by hand — or one
/// predating this phase — is still lockable. Two processes creating it race
/// benignly: `create(true)` on the same path yields the same inode, because no
/// `rename` is involved.
pub(crate) fn acquire(
    id: &str,
    path: PathBuf,
    anchor_path: &Path,
    as_writer: &Attribution,
    blocking: Blocking,
) -> Result<LockGuard, LockError> {
    if let Some(parent) = anchor_path.parent() {
        crate::store::ensure_private_dir(parent)?;
    }
    let anchor = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(anchor_path)?;

    match blocking {
        Blocking::Yes => anchor.lock().map_err(LockError::Io)?,
        Blocking::No => {
            if let Err(e) = anchor.try_lock() {
                return Err(match e {
                    fs4::TryLockError::WouldBlock => {
                        // Read the holder's line without the lock: it is
                        // display-only, so a torn read costs us a message, not
                        // correctness.
                        let held = std::fs::read_to_string(anchor_path)
                            .ok()
                            .and_then(|s| Attribution::parse(&s));
                        LockError::Busy(held)
                    }
                    fs4::TryLockError::Error(e) => LockError::Io(e),
                });
            }
        }
    }

    // Stamp the holder AFTER acquiring — the lock serializes this write, so
    // two holders can never interleave their lines.
    use std::io::{Seek, Write};
    let mut anchor = anchor;
    anchor.set_len(0)?;
    anchor.rewind()?;
    writeln!(anchor, "{}", as_writer.render())?;
    anchor.flush()?;

    Ok(LockGuard {
        id: id.to_string(),
        path,
        _anchor: anchor,
    })
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test store::lock`
Expected: PASS, 13 tests.

Then: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`

- [ ] **Step 7: Commit**

```bash
git add src/store/
git commit -m "feat: LockGuard over a .locks/<id> flock anchor"
```

---

### Task 3: Move writes onto the guard

**Files:**
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: Task 2's `LockGuard`.
- Produces: `Store::write_cassette` **removed**; `Store::add_cassette` unchanged in
  signature but now creates the anchor and writes through a guard.

This is the task that makes invariant 1 structural. After it, there is no way to write an
existing cassette without holding its lock.

- [ ] **Step 1: Write the failing test**

Add to the test module in `src/store/mod.rs`:

```rust
    #[test]
    fn adding_a_cassette_creates_its_lock_anchor() {
        // Phase 3 onward, every cassette has an anchor from birth — the
        // on-demand path in `lock` is a fallback for hand-written files, not
        // the normal route.
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        s.add_cassette(&sid, &m, "body\n").expect("add");
        assert!(
            s.locks_dir(&sid).join(&m.id).is_file(),
            "add_cassette must create .locks/<id>"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test store::tests::adding_a_cassette_creates_its_lock_anchor`
Expected: FAIL — `add_cassette must create .locks/<id>`.

- [ ] **Step 3: Replace `add_cassette` and delete `write_cassette`**

In `src/store/mod.rs`, replace both methods with:

```rust
    /// Create a new cassette, named `<slug>-<id>.md` from the topic at
    /// creation, and its lock anchor. Returns the path, which callers keep:
    /// the name is never recomputed, even when the topic changes.
    ///
    /// Takes and releases the cassette's own lock like every other write. A
    /// freshly minted ULID cannot be contended, so this cannot fail on `Busy` —
    /// routing it through the guard means there is exactly one way a cassette
    /// file is ever written.
    pub fn add_cassette(
        &self,
        session: &str,
        m: &CassetteMeta,
        body: &str,
    ) -> io::Result<PathBuf> {
        let path = self
            .cassettes_dir(session)
            .join(ids::file_name(m.topic.as_deref(), &m.id));
        if let Some(parent) = path.parent() {
            ensure_private_dir(parent)?;
        }
        // The file must exist before it can be locked, since `lock` resolves a
        // cassette id to its path. Create it empty, then write through the
        // guard like everything else.
        atomic_write(&path, "")?;
        let anchor = self.locks_dir(session).join(&m.id);
        let guard = lock::acquire(
            &m.id,
            path.clone(),
            &anchor,
            &lock::Attribution::for_now(&m.created_by, &m.created_by),
            lock::Blocking::No,
        )
        .map_err(io::Error::from)?;
        guard.write(m, body)?;
        Ok(path)
    }
```

Delete `pub fn write_cassette` entirely.

Add the `LockError` → `io::Error` conversion in `src/store/lock.rs`:

```rust
impl From<LockError> for io::Error {
    fn from(e: LockError) -> io::Error {
        match e {
            LockError::Io(e) => e,
            busy => io::Error::new(io::ErrorKind::WouldBlock, busy.to_string()),
        }
    }
}
```

- [ ] **Step 4: Fix the tests that called `write_cassette`**

`write_cassette_updates_in_place_without_renaming` in `src/store/mod.rs` must now go
through a guard. Replace its body with:

```rust
    #[test]
    fn write_cassette_updates_in_place_without_renaming() {
        // The slug is frozen at creation: retopicking must not move the file,
        // because an flock is held on the inode another writer resolved.
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let path = s.add_cassette(&sid, &m, "old\n").expect("add");

        m.topic = Some("completely different".to_string());
        let who = lock::Attribution::for_now("writer-1", "joseph");
        let guard = s.lock(&sid, &m.id, &who).expect("acquire");
        guard.write(&m, "new\n").expect("write");
        drop(guard);

        assert!(path.is_file(), "the file must not have been renamed");
        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].meta.topic.as_deref(), Some("completely different"));
        assert_eq!(found[0].body, "new\n");
        assert_eq!(
            found[0].path.file_name().unwrap().to_string_lossy(),
            "gratitude-01K5GR7T2M9WPD0000000000AB.md",
            "the slug stays as minted"
        );
    }
```

- [ ] **Step 5: Run the whole suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green. Any other test that called `write_cassette` must be updated the same
way — through a guard, never by reinstating the method.

- [ ] **Step 6: Verify invariant 1 structurally**

Run: `grep -rn 'fn write_cassette' src/`
Expected: exactly one hit — `LockGuard::write`'s neighbour in `src/store/lock.rs` is named
`write`, so this should return **nothing**. If `Store::write_cassette` still exists, the
task is not done.

- [ ] **Step 7: Commit**

```bash
git add src/store/
git commit -m "feat!: writing a cassette requires holding its lock"
```

---

### Task 4: `lock_many` and acquisition order

**Files:**
- Modify: `src/store/mod.rs`
- Modify: `src/store/lock.rs`

**Interfaces:**
- Produces: `Store::lock_many(&self, session: &str, ids: &[&str], as_writer: &Attribution) -> Result<Vec<LockGuard>, LockError>`

- [ ] **Step 1: Write the failing tests**

Add to `src/store/lock.rs`'s test module:

```rust
    #[test]
    fn lock_many_takes_them_all() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        for id in ["aaa00000000000000000000000", "bbb00000000000000000000000"] {
            s.add_cassette(&sid, &cassette_meta(id), "").expect("add");
        }
        let who = Attribution::for_now("writer-1", "joseph");
        let guards = s
            .lock_many(
                &sid,
                &["bbb00000000000000000000000", "aaa00000000000000000000000"],
                &who,
            )
            .expect("acquire");
        assert_eq!(guards.len(), 2);
    }

    #[test]
    fn lock_many_acquires_in_ascending_id_order() {
        // Acquisition order is a liveness device and nothing else: it stops two
        // overlapping multi-lock operations from livelocking. It is NOT queue
        // order, which is priority-first with the id only as a tiebreak.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        for id in ["aaa00000000000000000000000", "bbb00000000000000000000000"] {
            s.add_cassette(&sid, &cassette_meta(id), "").expect("add");
        }
        let who = Attribution::for_now("writer-1", "joseph");
        let guards = s
            .lock_many(
                &sid,
                &["bbb00000000000000000000000", "aaa00000000000000000000000"],
                &who,
            )
            .expect("acquire");
        assert_eq!(
            guards.iter().map(|g| g.id()).collect::<Vec<_>>(),
            vec!["aaa00000000000000000000000", "bbb00000000000000000000000"],
            "requested b,a — must be taken a,b"
        );
    }

    #[test]
    fn lock_many_releases_everything_it_took_when_one_is_busy() {
        // All or nothing: a partial hold would leave the loser wedging locks
        // the winner needs, which is the livelock the ordering rule prevents.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        for id in ["aaa00000000000000000000000", "bbb00000000000000000000000"] {
            s.add_cassette(&sid, &cassette_meta(id), "").expect("add");
        }
        let who = Attribution::for_now("writer-1", "joseph");
        let held = s
            .lock(&sid, "bbb00000000000000000000000", &who)
            .expect("hold b");

        let other = Attribution::for_now("writer-2", "agent");
        let r = s.lock_many(
            &sid,
            &["aaa00000000000000000000000", "bbb00000000000000000000000"],
            &other,
        );
        assert!(matches!(r, Err(LockError::Busy(_))), "must fail on b");
        // a must be free again — if lock_many kept it, this would be Busy.
        s.lock(&sid, "aaa00000000000000000000000", &other)
            .expect("a must have been released");
        drop(held);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test store::lock::tests::lock_many`
Expected: FAIL to compile — `no method named 'lock_many'`.

- [ ] **Step 3: Implement**

Add to `impl Store` in `src/store/mod.rs`:

```rust
    /// Acquire several cassette locks at once, all or nothing.
    ///
    /// Locks are always taken in **ascending id order**, regardless of the
    /// order requested. This is a liveness device and nothing more: a total
    /// order makes deadlock between two overlapping multi-lock operations
    /// impossible. It is emphatically **not** queue order — that is `priority`
    /// first, closed last, with the id only as a tiebreak.
    ///
    /// On contention every guard already taken is dropped, so a loser never
    /// wedges locks the winner needs.
    pub fn lock_many(
        &self,
        session: &str,
        ids: &[&str],
        as_writer: &lock::Attribution,
    ) -> Result<Vec<lock::LockGuard>, lock::LockError> {
        let mut ordered: Vec<&str> = ids.to_vec();
        ordered.sort_unstable();
        ordered.dedup();
        let mut guards = Vec::with_capacity(ordered.len());
        for id in ordered {
            // `?` drops `guards` on the way out, releasing everything taken.
            guards.push(self.lock(session, id, as_writer)?);
        }
        Ok(guards)
    }
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test store::lock`
Expected: PASS, 16 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "feat: lock_many acquires in ascending id order for liveness"
```

---

### Task 5: The registry lock

**Files:**
- Modify: `src/store/mod.rs`
- Modify: `src/store/writers.rs`

**Interfaces:**
- Produces: `Store::root_locks_dir(&self) -> PathBuf`; a **private**
  `Store::lock_registry(&self) -> io::Result<LockGuard>`; `writers::ensure` takes it.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `src/store/mod.rs`:

```rust
    #[test]
    fn the_registry_anchor_lives_at_the_root() {
        let (dir, s) = store();
        s.ensure_writer("joseph", writers::Kind::Human)
            .expect("ensure");
        assert!(
            dir.path().join(".locks").join("writers").is_file(),
            "the registry anchor belongs at the store root, not under a session"
        );
    }

    #[test]
    fn concurrent_registration_keeps_both_writers() {
        // Guards the lost update: ensure_writer reads the whole registry,
        // inserts, and writes it back. Without a lock the second write clobbers
        // the first and a writer id vanishes, orphaning every cassette
        // attributed to it.
        let (_dir, s) = store();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                s.ensure_writer("joseph", writers::Kind::Human).expect("a");
            });
            scope.spawn(|| {
                s.ensure_writer("agent", writers::Kind::Agent).expect("b");
            });
        });
        let all = s.writers().expect("read");
        assert_eq!(all.writers.len(), 2, "both registrations must survive");
        assert!(all.find_by_name("joseph").is_some());
        assert!(all.find_by_name("agent").is_some());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test store::tests::the_registry_anchor_lives_at_the_root`
Expected: FAIL — the anchor does not exist.

(`concurrent_registration_keeps_both_writers` may pass intermittently before the fix —
that is the nature of a lost-update race. Note it in your report; the anchor test is the
deterministic one.)

- [ ] **Step 3: Implement**

Add to `impl Store` in `src/store/mod.rs`:

```rust
    /// Anchors for store-wide locks, as opposed to a session's per-cassette
    /// ones. Currently just the writer registry.
    pub fn root_locks_dir(&self) -> PathBuf {
        self.root.join(LOCKS_DIR)
    }

    /// Acquire the writer registry's lock. **Blocks**, unlike every cassette
    /// lock — see the spec's "never block on a lock a human can hold". No
    /// human can hold this one: its critical section is a single
    /// read-modify-write over a small file, so nobody can wedge it, and a
    /// caller that cannot register a writer has no fallback to fall back to.
    ///
    /// Deliberately private. `ensure_writer` is the only caller, so blocking
    /// acquisition cannot leak into a code path where a human could hold it.
    fn lock_registry(&self) -> io::Result<lock::LockGuard> {
        ensure_private_dir(&self.root)?;
        let anchor = self.root_locks_dir().join("writers");
        let path = self.root.join(writers::WRITERS_FILE);
        lock::acquire(
            "writers",
            path,
            &anchor,
            &lock::Attribution::for_now("registry", "registry"),
            lock::Blocking::Yes,
        )
        .map_err(io::Error::from)
    }
```

Change `Store::ensure_writer` to hold it for the whole read-modify-write:

```rust
    /// The id for `name`, registering it on first sight. Idempotent: the same
    /// name never mints a second id.
    ///
    /// Holds the registry lock across the read-modify-write. Without it two
    /// writers registering at once both read a registry lacking the other,
    /// both insert, and the second write clobbers the first — which is exactly
    /// the first-run case, where the TUI registers from `$USER` while an agent
    /// registers itself.
    pub fn ensure_writer(&self, name: &str, kind: writers::Kind) -> io::Result<String> {
        let _registry = self.lock_registry()?;
        writers::ensure(&self.root, name, kind)
    }
```

`writers::ensure` itself is unchanged — it stays `pub(crate)` and now has exactly one
caller, which holds the lock.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test store::`
Expected: PASS, including both new tests.

Then the whole suite plus clippy and fmt.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "fix: guard the writer registry's read-modify-write"
```

---

### Task 6: `cassette queue write`

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `cassette queue write <ID> [--session <ID>]`, body on stdin; exit 0/1/2/3.
- Produces: `store_root()` in `main.rs`, honouring `$CASSETTE_DATA_DIR`. Task 7's tests
  depend on this override — without it they would write to the real store.

- [ ] **Step 1: Write the failing tests**

Add to `tests/cli.rs`:

```rust
#[test]
fn queue_write_without_an_id_exits_two() {
    let out = run(&["queue", "write"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_write_appears_in_help() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("queue"), "{text}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test cli queue_write`
Expected: FAIL — `unrecognized subcommand 'queue'` gives exit 2 for the first test but the
help test fails.

- [ ] **Step 3: Add the subcommand**

In `src/cli.rs`, add to `enum Command`:

```rust
    /// write a cassette, holding its lock for the duration
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
```

and a new enum beside it:

```rust
#[derive(Subcommand, Debug)]
enum QueueAction {
    /// replace a cassette's body, read from stdin
    Write {
        #[arg(value_name = "ID")]
        id: String,
        /// session to write in (default: the active session)
        #[arg(long, value_name = "ID")]
        session: Option<String>,
    },
}
```

Extend `Args` with:

```rust
    /// `queue write`: the cassette id and the session it lives in.
    pub queue_write: Option<(String, Option<String>)>,
```

and the `into_args` match arm:

```rust
            Some(Command::Queue { action }) => match action {
                QueueAction::Write { id, session } => args.queue_write = Some((id, session)),
            },
```

- [ ] **Step 4: Implement the command in `main.rs`**

Add a handler, dispatched before the terminal is touched (like `stats` and `find`):

```rust
/// `cassette queue write <ID>`: acquire the cassette's lock, THEN read the body
/// from stdin, write, and release.
///
/// The ordering is deliberate and is what makes the concurrency tests
/// deterministic: a child spawned with an open stdin pipe is provably holding
/// the lock, with no sleeps and no polling, and closing the pipe releases it.
fn queue_write(store: &store::Store, id: &str, session: Option<&str>) -> ! {
    let session = match session.map(str::to_string).map_or_else(
        || store.active_session(),
        |s| Ok(Some(s)),
    ) {
        Ok(Some(s)) => s,
        Ok(None) => die_with(2, "no active session; pass --session"),
        Err(e) => die_with(1, &format!("cannot read the active session: {e}")),
    };

    let writer = match store.ensure_writer(&whoami(), store::writers::Kind::Human) {
        Ok(w) => w,
        Err(e) => die_with(1, &format!("cannot register a writer: {e}")),
    };
    let who = store::lock::Attribution::for_now(&writer, &whoami());

    let guard = match store.lock(&session, id, &who) {
        Ok(g) => g,
        Err(store::lock::LockError::Busy(held)) => {
            let who = held
                .map(|a| format!("{} (since {})", a.name, a.since))
                .unwrap_or_else(|| "another writer".to_string());
            die_with(3, &format!("'{id}' is open by {who} — try again later"))
        }
        Err(store::lock::LockError::Io(e)) => die_with(1, &format!("cannot lock '{id}': {e}")),
    };

    // Lock first, stdin second. Reversing these would make the tests racy.
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
        die_with(1, &format!("cannot read stdin: {e}"));
    }

    let current = match guard.read() {
        Ok(c) => c,
        Err(e) => die_with(1, &format!("cannot read '{id}': {e}")),
    };
    let mut m = current.meta;
    m.last_writer = writer;
    m.updated_at = store::meta::now_utc();
    if let Err(e) = guard.write(&m, &body) {
        die_with(1, &format!("cannot write '{id}': {e}"));
    }
    std::process::exit(0)
}

/// The human's name for attribution: `$USER`, falling back to `unknown`.
fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

fn die_with(code: i32, msg: &str) -> ! {
    eprintln!("cassette: {msg}");
    std::process::exit(code)
}
```

Add the store-root resolver. The config key is still `notes_dir` and does not point at a
store, so do **not** read it here — Phase 6 renames it:

```rust
/// The store root: `$CASSETTE_DATA_DIR` when set, else the XDG default.
///
/// The environment override exists so tests never touch the real store at
/// `~/.local/share/cassette`. Phase 6 adds a `data_dir` config key beside it;
/// the existing `notes_dir` key points at the old flat notes folder and is
/// deliberately NOT consulted here.
fn store_root() -> PathBuf {
    std::env::var_os("CASSETTE_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(store::Store::default_root)
        .unwrap_or_else(|| die("cannot determine a data dir"))
}
```

Wire the command into `main` beside the other pre-terminal actions (`stats`, `find`,
`themes`), before the terminal is touched:

```rust
    if let Some((id, session)) = &args.queue_write {
        queue_write(&store::Store::new(store_root()), id, session.as_deref());
    }
```

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test --test cli`
Expected: PASS, 16 tests.

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: cassette queue write, holding the lock across stdin"
```

---

### Task 7: Cross-process concurrency tests

**Files:**
- Create: `tests/lock.rs`

**Interfaces:**
- Consumes: everything above, plus `env!("CARGO_BIN_EXE_cassette")` and the
  `CASSETTE_DATA_DIR` override added in Task 6.

These are the tests the phase exists for. Every one is deterministic — no sleeps, no
polling, no timing assumptions.

**Why the fixture is hand-built rather than driven through the CLI:** `tests/lock.rs` is
an integration test, so it sees only the binary's command line — and Phase 3 ships just
`queue write`. There is no `session new` or `queue new` until Phase 4. Writing the layout
with `std::fs` is ten lines, needs no new CLI surface, and keeps the fixture honest: if
the on-disk format drifts, these tests break, which is the point.

- [ ] **Step 1: Write the tests**

Create `tests/lock.rs`:

```rust
//! Cross-process lock behaviour. Deterministic by construction: the lock is
//! either held before the child starts, or the child holds it before the parent
//! looks, with an open stdin pipe as the synchronisation primitive. Nothing
//! here sleeps.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cassette")
}

const SESSION: &str = "01K5GQ2R8V3XQZ0000000000AB";
const ID: &str = "01K5GR7T2M9WPD0000000000AB";

/// Build the store layout directly. Deliberately not through the CLI: the
/// commands that would do it arrive in Phase 4, and a hand-built fixture keeps
/// these tests honest about the on-disk format.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let cassettes = root.join("sessions").join(SESSION).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::create_dir_all(root.join("sessions").join(SESSION).join(".locks"))
        .expect("mkdir");
    std::fs::write(
        root.join("sessions").join(SESSION).join("session.toml"),
        "created = \"2026-09-14T09:25:57Z\"\n",
    )
    .expect("session.toml");
    std::fs::write(root.join("active"), format!("{SESSION}\n")).expect("active");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\nlocked_by:\n\
             created_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
             ## Side A\n\noriginal\n"
        ),
    )
    .expect("cassette");
    (dir, root)
}

/// Spawn `queue write` with an open stdin pipe. It acquires the lock and then
/// blocks reading stdin, so once this returns the child provably holds the
/// lock — no sleep required.
fn spawn_holder(root: &std::path::Path) -> Child {
    let mut child = Command::new(bin())
        .args(["queue", "write", ID])
        .env("CASSETTE_DATA_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    // Writing a byte proves the child reached its stdin read, which is after
    // acquisition.
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"held\n")
        .expect("write");
    child
}

fn try_write(root: &std::path::Path, body: &str) -> std::process::Output {
    let mut child = Command::new(bin())
        .args(["queue", "write", ID])
        .env("CASSETTE_DATA_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(body.as_bytes())
        .expect("write");
    child.wait_with_output().expect("wait")
}

#[test]
fn an_uncontended_write_succeeds() {
    let (_d, root) = fixture();
    let out = try_write(&root, "rewritten\n");
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let text = std::fs::read_to_string(
        root.join("sessions")
            .join(SESSION)
            .join("cassettes")
            .join(format!("gratitude-{ID}.md")),
    )
    .expect("read");
    assert!(text.contains("rewritten"), "{text}");
}

#[test]
fn a_second_writer_gets_exit_three_and_is_told_who_holds_it() {
    let (_d, root) = fixture();
    let mut holder = spawn_holder(&root);

    let out = try_write(&root, "should not land\n");
    assert_eq!(out.status.code(), Some(3), "{:?}", out);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("try again later"), "{err}");

    // Releasing lets the next writer through.
    drop(holder.stdin.take());
    let done = holder.wait().expect("wait");
    assert!(done.success());
    assert_eq!(try_write(&root, "now ok\n").status.code(), Some(0));
}

#[cfg(unix)]
#[test]
fn killing_the_holder_releases_the_lock() {
    // The property the no-reaper decision rests on: the kernel releases an
    // flock on process death for any reason, including SIGKILL. If a future
    // refactor swapped in a create-to-lock/delete-to-unlock scheme, this test
    // is what would catch it.
    let (_d, root) = fixture();
    let mut holder = spawn_holder(&root);
    assert_eq!(
        try_write(&root, "blocked\n").status.code(),
        Some(3),
        "the holder must actually hold it"
    );

    holder.kill().expect("kill");
    holder.wait().expect("reap");

    assert_eq!(
        try_write(&root, "after the kill\n").status.code(),
        Some(0),
        "a SIGKILLed holder must not leave the lock stuck"
    );
}

#[test]
fn registering_writers_concurrently_keeps_both() {
    // The registry lock blocks rather than failing: two processes registering
    // at once must both succeed, because a caller that cannot register has no
    // fallback. Exit 3 here would be a regression.
    let (_d, root) = fixture();
    let spawn = |user: &str| {
        Command::new(bin())
            .args(["queue", "write", ID])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
    };
    let mut a = spawn("joseph");
    let mut b = spawn("agent");
    a.stdin.take().expect("stdin").write_all(b"a\n").expect("w");
    b.stdin.take().expect("stdin").write_all(b"b\n").expect("w");
    let ao = a.wait_with_output().expect("wait");
    let bo = b.wait_with_output().expect("wait");

    // One of them may lose the cassette lock (exit 3) — that is expected and
    // is not what this test is about. Neither may fail to register.
    for out in [&ao, &bo] {
        let code = out.status.code();
        assert!(
            code == Some(0) || code == Some(3),
            "registration must not fail: {:?}",
            out
        );
    }
    let registry = std::fs::read_to_string(root.join("writers.toml")).expect("read");
    assert!(registry.contains("joseph"), "{registry}");
    assert!(registry.contains("agent"), "{registry}");
}
```

- [ ] **Step 2: Run them**

Run: `cargo test --test lock`
Expected: PASS, 4 tests.

Then the whole suite plus clippy and fmt.

- [ ] **Step 3: Commit**

```bash
git add tests/lock.rs
git commit -m "test: cross-process lock contention and crash release"
```

---

## Verification

- [ ] `cargo test` — all green; ~240 tests.
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --check` — clean.
- [ ] `grep -rn 'fn write_cassette' src/` returns nothing — invariant 1 is structural.
- [ ] The production-write scan still returns exactly one line:
      `for f in src/store/*.rs; do awk '/#\[cfg\(test\)\]/{exit} /fs::write/{print FILENAME":"FNR}' "$f"; done`
- [ ] `cargo test --test lock` passes when run repeatedly (`for i in $(seq 10); do cargo
      test --test lock || break; done`) — these tests must not be flaky, because flaky
      lock tests get deleted.
- [ ] The TUI still runs: `CASSETTE_DATA_DIR=/tmp/probe cargo run -- stats` behaves as
      before.

## What this phase deliberately does not do

- **No sticky lock.** `locked_by` stays a dormant frontmatter field; exit 4 stays unused.
  It is a human-facing claim/release workflow and lands in Phase 4 with the CLI.
- **No `--json`.** Phase 4, with the rest of the queue surface.
- **No `--wait` and no retry.** The one place waiting is correct — the registry — is
  unconditional and private.
- **No TUI integration.** The TUI does not yet take locks; Phase 5 does that.
- **No Windows CI.** `fs4` is the cross-platform choice and correctness there is verified
  by hand later.
