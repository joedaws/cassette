//! The TUI's half of the store protocol: one held lock, one cassette file
//! per cassette on screen.
//!
//! Everything the editor persists goes through here, and everything here
//! goes through a `LockGuard` — `LockGuard::write` is the only code in the
//! crate that writes a cassette file, and the TUI is not an exception to
//! that. `App` and `Cassette` stay pure (CLAUDE.md); the guard lives in this
//! module and in `main.rs`'s event loop, never in `App`.
//!
//! **Focus means held.** The writer holds the focused cassette's lock from
//! the moment focus lands on it until focus moves or the process exits. No
//! idle timeout and no reacquisition: `flock` is per-open-file-description,
//! so a second acquire of a lock this process already holds reports `Busy`
//! and blames a phantom writer. `Store::holds` answers "do *we* hold it"
//! from in-process bookkeeping, which is why the checks below ask it rather
//! than probing the anchor.
//!
//! **Any flush happens through the guard that is held, before that guard is
//! dropped.** Dropping first would either lose the text or force a
//! re-acquire `flock` may refuse.

use std::io;

use crate::app::App;
use crate::output;
use crate::store::lock::{Attribution, LockError, LockGuard};
use crate::store::meta::{self, CassetteMeta, Status};
use crate::store::{ids, priority, Store};

/// Writes an `App`'s cassettes into one store session, holding the focused
/// cassette's lock for as long as it stays focused.
pub struct SessionWriter<'a> {
    store: &'a Store,
    session: String,
    /// Whether this run created the session, which is the only case in which
    /// `finish` may remove it. A session that was *opened* is somebody's
    /// earlier work.
    created_here: bool,
    /// The registry id writes are attributed to, and the display name a
    /// contender sees stamped in the lock anchor.
    writer: String,
    name: String,
    /// The cassette index the guard belongs to, alongside the guard. `None`
    /// when nothing is held — before the first `acquire`, after `finish`, or
    /// when acquiring failed and the session is read-only.
    guard: Option<(usize, LockGuard)>,
}

impl<'a> SessionWriter<'a> {
    /// Bind a writer to an existing store session. `created_here` says
    /// whether this run created it — see `finish`.
    ///
    /// `writer` is the registry id and `name` the display name, both
    /// resolved by the caller through the same path every queue command
    /// uses (`main.rs`'s `resolve_tui_writer`). They are not cosmetic: the
    /// id lands in each cassette's `last_writer`, and the name is stamped
    /// into the lock anchor another writer reads when it is told who holds a
    /// cassette. Resolving them here instead would either re-implement that
    /// precedence — dropping `--writer` and its rule that an explicitly
    /// named writer must already be registered — or make `open` fallible.
    pub fn open(
        store: &'a Store,
        session: &str,
        created_here: bool,
        writer: &str,
        name: &str,
    ) -> Self {
        let (writer, name) = (writer.to_string(), name.to_string());
        SessionWriter {
            store,
            session: session.to_string(),
            created_here,
            writer,
            name,
            guard: None,
        }
    }

    /// The store session being written.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// The cassette index whose lock is currently held, if any. `main.rs`
    /// compares it against `App::focus_idx` to notice that focus moved.
    pub fn held_idx(&self) -> Option<usize> {
        self.guard.as_ref().map(|(idx, _)| *idx)
    }

    fn attribution(&self) -> Attribution {
        Attribution::for_now(&self.writer, &self.name)
    }

    /// Mint the store cassette backing `app.cassettes[idx]` and record its
    /// id on the in-memory cassette. A no-op for a cassette that already has
    /// one, so it is safe to sweep the whole list.
    ///
    /// Placed at the tail of the queue like `queue new`'s default: a
    /// cassette appearing mid-session must not jump ahead of work already
    /// queued.
    pub fn create_cassette(&self, app: &mut App, idx: usize) -> io::Result<()> {
        if app.cassettes.get(idx).is_none_or(|c| !c.id.is_empty()) {
            return Ok(());
        }
        let scan = self.store.scan_session(&self.session)?;
        let open: Vec<i64> = scan
            .cassettes
            .iter()
            .filter(|c| c.meta.status == Status::Open)
            .map(|c| c.meta.priority)
            .collect();
        // `None` means the sparse run has no gap left above the maximum,
        // only reachable from a hand-edited priority near `i64::MAX`.
        // Renumbering the session would mean taking every lock, including
        // ones agents may hold; landing on the maximum instead keeps the new
        // cassette last without touching anybody else's file.
        let priority = priority::last(&open).unwrap_or(i64::MAX);
        let meta = CassetteMeta {
            id: ids::new_id(),
            topic: app.cassettes[idx].topic.clone(),
            priority,
            status: Status::Open,
            locked_by: None,
            created_by: self.writer.clone(),
            last_writer: self.writer.clone(),
            updated_at: meta::now_utc(),
        };
        let body = output::cassette_body(&app.cassettes[idx]);
        self.store.add_cassette(&self.session, &meta, &body)?;
        app.cassettes[idx].id = meta.id;
        app.clear_dirty(idx);
        Ok(())
    }

    /// Give every cassette in `app` a store counterpart. `Ctrl+N` pushes a
    /// cassette with no id from the pure key handlers, which cannot reach a
    /// `Store`; this restores the invariant afterwards instead of threading
    /// the store through them.
    pub fn create_missing_cassettes(&self, app: &mut App) -> io::Result<()> {
        for idx in 0..app.cassettes.len() {
            self.create_cassette(app, idx)?;
        }
        Ok(())
    }

    /// Take the lock for cassette `idx` and hold it.
    ///
    /// When a different cassette's lock is already held, it is flushed
    /// **through that guard** and only then dropped — the ordering is the
    /// correctness, not a style preference. Re-acquiring a lock we already
    /// hold is skipped outright: `flock` is per-open-file-description, so
    /// asking twice reports `Busy` against ourselves.
    pub fn acquire(&mut self, app: &mut App, idx: usize) -> Result<(), LockError> {
        if let Some((held, _)) = &self.guard {
            if *held == idx {
                return Ok(());
            }
            // Flush first, drop second. The write goes through the guard.
            self.flush_held(app)?;
            self.guard = None;
        }
        let Some(id) = app.cassettes.get(idx).map(|c| c.id.clone()) else {
            return Ok(());
        };
        if id.is_empty() {
            return Err(LockError::NoSuchCassette {
                session: self.session.clone(),
                id: String::new(),
            });
        }
        debug_assert!(
            !self.store.holds(&self.session, &id),
            "re-acquiring a lock this process already holds would report Busy against itself"
        );
        let guard = self.store.lock(&self.session, &id, &self.attribution())?;
        self.guard = Some((idx, guard));
        Ok(())
    }

    /// Write the focused cassette through the held guard when it has unsaved
    /// edits. The guard's index *is* the focused index — `acquire` keeps
    /// them equal — and the write is keyed on the guard's, so a focus change
    /// that has not yet moved the lock can never write one cassette's words
    /// into another's file.
    pub fn flush_focused(&mut self, app: &mut App) -> io::Result<()> {
        self.flush_held(app).map_err(io::Error::from)
    }

    /// The flush itself, keyed on the guard rather than on `App::focus_idx`.
    ///
    /// Frontmatter is re-read under the lock and only the fields the TUI
    /// owns are replaced: `priority`, `status` and `locked_by` belong to the
    /// queue commands, and an agent that reprioritized or closed a cassette
    /// while the human was writing in it must not have that undone by the
    /// next autosave.
    fn flush_held(&mut self, app: &mut App) -> Result<(), LockError> {
        let Some((idx, guard)) = self.guard.as_ref() else {
            return Ok(());
        };
        let idx = *idx;
        debug_assert!(
            app.dirty_indices().iter().all(|i| *i == idx),
            "only the cassette whose lock is held can be dirty: nothing edits an \
             unfocused cassette, and `acquire` flushes the one being left before \
             the guard moves. A dirty unfocused cassette is a design hole, not \
             something to paper over with a loop."
        );
        let Some(c) = app.cassettes.get(idx) else {
            return Ok(());
        };
        if !c.dirty {
            return Ok(());
        }
        let mut meta = guard.read()?.meta;
        meta.topic = c.topic.clone();
        meta.last_writer = self.writer.clone();
        meta.updated_at = meta::now_utc();
        guard.write(&meta, &output::cassette_body(c))?;
        app.clear_dirty(idx);
        Ok(())
    }

    /// Whether the **store** agrees this session holds nothing — the
    /// question `finish` has to answer before it deletes a directory.
    ///
    /// `App::is_empty` cannot answer it. `App` only ever knows the cassettes
    /// the TUI itself created, and this whole phase exists because other
    /// writers touch the same session concurrently: an agent that found the
    /// session through `session list` and ran `queue new` + `queue write`
    /// is completely invisible in `app.cassettes`. Judging from memory and
    /// then calling `remove_dir_all` destroyed its words and its live lock
    /// anchor along with the directory.
    ///
    /// So the store is re-scanned, and every one of these blocks removal:
    ///
    /// - a cassette with text on either side;
    /// - a cassette id `app.cassettes` has never seen, **even when it is
    ///   empty** — somebody else created it and may be about to write it;
    /// - a file the scanner could not parse, which is still a file somebody
    ///   wrote;
    /// - a lock anchor somebody currently holds;
    /// - any I/O error at all. A store we cannot read is a store we must not
    ///   delete from, so every failure below answers "not empty".
    fn store_is_empty(&self, app: &App) -> bool {
        let Ok(scan) = self.store.scan_session(&self.session) else {
            return false;
        };
        if scan.unreadable > 0 {
            return false;
        }
        for stored in &scan.cassettes {
            let (side_a, side_b) = crate::queue::json::split_sides(&stored.body);
            if !side_a.trim().is_empty() || !side_b.trim().is_empty() {
                return false;
            }
            if !app.cassettes.iter().any(|c| c.id == stored.meta.id) {
                return false;
            }
        }
        // Anchors are named by cassette id, so the directory listing is the
        // id list `Store::is_free` wants — including ids `scan_session`
        // never saw, which is exactly the case that matters.
        match std::fs::read_dir(self.store.locks_dir(&self.session)) {
            Ok(entries) => {
                for entry in entries {
                    let Ok(entry) = entry else { return false };
                    let name = entry.file_name();
                    let Some(id) = name.to_str() else {
                        return false;
                    };
                    if !self.store.is_free(&self.session, id).unwrap_or(false) {
                        return false;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
        true
    }

    /// Final flush, release the guard, and clean up after a session that
    /// recorded nothing.
    ///
    /// An empty session has always written nothing; the store equivalent is
    /// removing the session directory, so a mistaken launch does not litter
    /// `session list`. Three things must all agree first:
    ///
    /// 1. **this run created the session** — one that was merely *opened*
    ///    holds a human's earlier work, whatever it currently contains;
    /// 2. **the TUI wrote nothing** (`App::is_empty`);
    /// 3. **the store holds nothing either** (`store_is_empty`) — see its
    ///    doc comment; without this a concurrent writer's cassettes go with
    ///    the directory.
    ///
    /// The guard is dropped before the directory goes, so the anchor it
    /// holds is not unlinked out from under it.
    pub fn finish(&mut self, app: &mut App) -> io::Result<()> {
        let flushed = self.flush_held(app).map_err(io::Error::from);
        self.guard = None;
        flushed?;

        if self.created_here && app.is_empty() && self.store_is_empty(app) {
            let dir = self.store.session_dir(&self.session);
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cassette::Cassette;
    use crate::store::meta::{CassetteMeta, Status};
    use std::io::Write;

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
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
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
        let mut w = SessionWriter::open(&store, &session, false, "w", "w");
        w.finish(&mut app).expect("finish");
        assert_eq!(store.list_sessions().expect("list").len(), 1);
    }

    #[test]
    fn finish_writes_the_focused_cassette_through_the_held_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire");
        app.modify_focused(|c| c.insert_str("words that must survive quit"));

        w.finish(&mut app).expect("finish");

        let scan = store.scan_session(&session).expect("scan");
        assert_eq!(
            scan.cassettes.len(),
            1,
            "the session is not empty, so it stays"
        );
        assert!(
            scan.cassettes[0]
                .body
                .contains("words that must survive quit"),
            "body: {}",
            scan.cassettes[0].body
        );
        assert!(!app.cassettes[0].dirty, "a flushed cassette is clean");
    }

    #[test]
    fn a_flush_preserves_priority_status_and_a_sticky_lock() {
        // An agent may reprioritize or close a cassette while the human is
        // writing in it. The TUI owns the body and the topic; everything
        // else in the frontmatter is the queue's, and an autosave that
        // rewrote it from stale memory would silently undo their work.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire");
        app.modify_focused(|c| c.insert_str("text"));
        w.flush_focused(&mut app).expect("flush");

        let scan = store.scan_session(&session).expect("scan");
        assert_eq!(scan.cassettes[0].meta.priority, 10);
        assert_eq!(scan.cassettes[0].meta.status, Status::Open);
        assert_eq!(
            scan.cassettes[0].meta.created_by, "w",
            "creation is not re-attributed"
        );
    }

    #[test]
    fn moving_focus_flushes_the_cassette_being_left() {
        // Text typed into cassette 0 must reach disk when focus moves to 1,
        // not wait for the next 30-second autosave. `acquire` *is* the
        // "focus" entry point: called with a different index than the one
        // currently held, it flushes the outgoing cassette through its
        // guard, drops that guard, and only then takes the new one — see
        // its doc comment. There is no separate `focus` method to test.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 2);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire 0");

        app.focus_idx = 0;
        app.modify_focused(|c| c.insert_str("typed into zero"));

        w.acquire(&mut app, 1).expect("move focus to 1");

        let scan = store.scan_session(&session).expect("scan");
        let zero = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == app.cassettes[0].id)
            .expect("found");
        assert!(
            zero.body.contains("typed into zero"),
            "blur must flush before the guard is dropped: {}",
            zero.body
        );
        assert!(
            !app.cassettes[0].dirty,
            "and the cassette is clean afterwards"
        );
    }

    /// Locate the `cassette` binary Cargo built alongside this test binary.
    ///
    /// `CARGO_BIN_EXE_cassette` — what `tests/lock.rs` uses — is only set for
    /// integration tests; Cargo does not define it for a bin crate's own
    /// unit tests (confirmed directly: the same `env!` there fails to
    /// compile from inside this module). This crate has no `src/lib.rs`, so
    /// `tests/lock.rs` cannot link `SessionWriter` either — the cross-process
    /// test below has to live here instead, and locate the binary itself:
    /// the test binary runs from `target/<profile>/deps/`, and the plain
    /// binary Cargo builds alongside it (so integration tests have one to
    /// exec) sits one directory up, exactly as `CARGO_BIN_EXE_cassette`
    /// would resolve.
    fn bin_path() -> std::path::PathBuf {
        let mut path = std::env::current_exe().expect("current test exe");
        path.pop(); // drop the test binary's own file name
        if path.ends_with("deps") {
            path.pop();
        }
        path.push("cassette");
        path
    }

    /// Cross-process proof that the guard `SessionWriter::acquire` holds for
    /// the focused cassette is a real `flock`, binding on another process,
    /// and that it is released the moment focus moves.
    ///
    /// Driving the actual TUI end-to-end needs a pty (`.claude/skills/verify`);
    /// a pty harness for a `cargo test` is a heavier, flakier dependency than
    /// this phase needs, so — as the task allows — this holds the lock with
    /// an in-process `SessionWriter` standing in for "the TUI has focus",
    /// exercising the real `acquire` code path, and spawns the real
    /// `cassette` binary's `queue write` as the contending agent. The
    /// contention itself is still genuinely cross-process: two OS processes
    /// racing the same on-disk `flock`.
    ///
    /// Unlike `tests/lock.rs`'s `contend`, there is no start-order race to
    /// eliminate here: the in-process guard is acquired synchronously, by a
    /// blocking Rust call that has already returned `Ok` before the agent
    /// process is even spawned, so the agent's denial is deterministic, not
    /// a race outcome. Its proof is still exactly what the module doc
    /// demands — output written *after* the child's own lock attempt, never
    /// the parent's successful write to its stdin: `queue write` acquires
    /// the lock before it ever reads stdin (see `queue/write.rs`'s doc
    /// comment), so the denied child's exit code 3 and stderr are produced
    /// by, and only by, its own failed acquisition.
    #[test]
    fn an_agents_write_is_denied_the_cassette_the_tui_has_focused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 2);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0)
            .expect("acquire 0: the TUI focuses cassette 0");
        let held_id = app.cassettes[0].id.clone();

        // The lock is already held, deterministically, before this child is
        // even spawned: its failure is not a race outcome.
        let denied = std::process::Command::new(bin_path())
            .args(["queue", "write", &held_id, "--session", &session])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "agent")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn denied write")
            .wait_with_output()
            .expect("wait denied write");
        assert_eq!(
            denied.status.code(),
            Some(3),
            "an agent must be denied the cassette the TUI has focused: {denied:?}"
        );
        let err = String::from_utf8_lossy(&denied.stderr);
        assert!(err.contains("is open by"), "{err}");
        assert!(err.contains("try again later"), "{err}");

        // Focus moves to cassette 1: `acquire` flushes and drops cassette
        // 0's guard, through the guard, before taking the new one.
        w.acquire(&mut app, 1).expect("move focus to 1");

        let mut allowed = std::process::Command::new(bin_path())
            .args(["queue", "write", &held_id, "--session", &session])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "agent")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn allowed write");
        allowed
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"the agent's words\n")
            .expect("write body");
        let allowed = allowed.wait_with_output().expect("wait allowed write");
        assert_eq!(
            allowed.status.code(),
            Some(0),
            "the cassette must be writable once focus has moved off it: {allowed:?}"
        );

        let scan = store.scan_session(&session).expect("scan");
        let zero = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == held_id)
            .expect("found");
        assert!(
            zero.body.contains("the agent's words"),
            "body: {}",
            zero.body
        );
    }

    #[test]
    fn acquire_is_idempotent_for_the_cassette_already_held() {
        // flock is per-open-file-description: asking twice for a lock this
        // process holds reports Busy and blames a phantom writer.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire");
        w.acquire(&mut app, 0)
            .expect("re-acquiring the held cassette must not fail");
        assert_eq!(w.held_idx(), Some(0));
    }

    #[test]
    fn a_new_runtime_cassette_becomes_a_store_cassette() {
        // Ctrl+N: the pure key handler pushes a cassette with no id, and the
        // event loop mints its store counterpart.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let w = SessionWriter::open(&store, &session, true, "w", "w");
        app.add_cassette();
        assert!(app.cassettes[1].id.is_empty(), "pure code mints no ids");

        w.create_missing_cassettes(&mut app).expect("create");

        assert!(!app.cassettes[1].id.is_empty());
        let scan = store.scan_session(&session).expect("scan");
        assert_eq!(scan.cassettes.len(), 2);
        assert!(
            scan.cassettes
                .iter()
                .any(|c| c.meta.id == app.cassettes[1].id && c.meta.priority == 20),
            "a cassette added mid-session lands at the tail of the queue"
        );
    }

    #[test]
    fn finish_keeps_a_created_session_that_holds_words() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire");
        app.modify_focused(|c| c.insert_str("something"));
        w.finish(&mut app).expect("finish");
        assert_eq!(store.list_sessions().expect("list").len(), 1);
    }

    /// A cassette added to the store behind the TUI's back, as a concurrent
    /// agent's `queue new` + `queue write` would leave it.
    fn add_behind_the_tuis_back(store: &Store, session: &str, body: &str) -> String {
        let id = crate::store::ids::new_id();
        let m = CassetteMeta {
            id: id.clone(),
            topic: Some("agentwork".to_string()),
            priority: 900,
            status: Status::Open,
            locked_by: None,
            created_by: "agent".to_string(),
            last_writer: "agent".to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store.add_cassette(session, &m, body).expect("add");
        id
    }

    #[test]
    fn finish_never_removes_a_session_another_writer_has_written_into() {
        // The reproduction: a bare launch creates session S; an agent finds
        // S through `session list` and runs `queue new` + `queue write`; the
        // user quits without typing. Judging emptiness from `App` — which
        // only ever knows the cassettes the TUI itself created — deleted the
        // directory, taking the agent's words and its lock anchor with it.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let agent_id =
            add_behind_the_tuis_back(&store, &session, "## Side A\n\nwords the agent wrote\n");

        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.finish(&mut app).expect("finish");

        assert_eq!(
            store.list_sessions().expect("list").len(),
            1,
            "a session another writer has written into is not this run's to delete"
        );
        let scan = store.scan_session(&session).expect("scan");
        assert!(
            scan.cassettes
                .iter()
                .any(|c| c.meta.id == agent_id && c.body.contains("words the agent wrote")),
            "and the agent's cassette survives intact"
        );
    }

    #[test]
    fn finish_never_removes_a_session_holding_a_cassette_the_tui_never_saw() {
        // Empty today is not empty tomorrow: the agent created it and may be
        // about to write it. `session_writer` has no way to tell the two
        // moments apart, so an unknown id blocks removal on its own.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        add_behind_the_tuis_back(&store, &session, "");

        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.finish(&mut app).expect("finish");

        assert_eq!(store.list_sessions().expect("list").len(), 1);
    }

    #[test]
    fn finish_never_removes_a_session_with_a_live_lock_anchor() {
        // `remove_dir_all` takes `.locks/` with it, so a held anchor would be
        // unlinked out from under whoever is holding it.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let held = store
            .lock(
                &session,
                &app.cassettes[0].id.clone(),
                &Attribution::for_now("other", "other"),
            )
            .expect("another holder takes the lock");

        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.finish(&mut app).expect("finish");

        assert_eq!(store.list_sessions().expect("list").len(), 1);
        drop(held);
    }
}
