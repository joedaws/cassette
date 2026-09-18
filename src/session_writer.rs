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

/// The writer identity a TUI session attributes its work to:
/// `$CASSETTE_WRITER`, else `$USER`, resolved through the registry (which
/// registers an unseen name as a human on first sight — a person at a
/// terminal is exactly that).
///
/// Falls back to using the bare name as its own id when the registry cannot
/// be read. Attribution strings are not the words: a freewriting session
/// must not refuse to start because `writers.toml` has the wrong
/// permissions, and `Store::add_cassette` already takes the same stance for
/// the same reason.
fn identity(store: &Store) -> (String, String) {
    let name = std::env::var("CASSETTE_WRITER")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("USER").ok().filter(|v| !v.trim().is_empty()))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "cassette".to_string());
    let writer = store
        .resolve_writer(&name)
        .map(|(id, _kind)| id)
        .unwrap_or_else(|_| name.clone());
    (writer, name)
}

impl<'a> SessionWriter<'a> {
    /// Bind a writer to an existing store session. `created_here` says
    /// whether this run created it — see `finish`.
    pub fn open(store: &'a Store, session: &str, created_here: bool) -> Self {
        let (writer, name) = identity(store);
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

    /// Final flush, release the guard, and clean up after a session that
    /// recorded nothing.
    ///
    /// An empty session has always written nothing; the store equivalent is
    /// removing the session directory, so a mistaken launch does not litter
    /// `session list`. Only a session **this run created** qualifies — one
    /// that was opened holds a human's earlier work, whatever it currently
    /// contains. The guard is dropped before the directory goes, so the
    /// anchor it holds is not unlinked out from under it.
    pub fn finish(&mut self, app: &mut App) -> io::Result<()> {
        let flushed = self.flush_held(app).map_err(io::Error::from);
        self.guard = None;
        flushed?;

        if self.created_here && app.is_empty() {
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

    #[test]
    fn finish_writes_the_focused_cassette_through_the_held_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true);
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
        let mut w = SessionWriter::open(&store, &session, true);
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
    fn acquire_is_idempotent_for_the_cassette_already_held() {
        // flock is per-open-file-description: asking twice for a lock this
        // process holds reports Busy and blames a phantom writer.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 1);
        let mut w = SessionWriter::open(&store, &session, true);
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
        let w = SessionWriter::open(&store, &session, true);
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
        let mut w = SessionWriter::open(&store, &session, true);
        w.acquire(&mut app, 0).expect("acquire");
        app.modify_focused(|c| c.insert_str("something"));
        w.finish(&mut app).expect("finish");
        assert_eq!(store.list_sessions().expect("list").len(), 1);
    }
}
