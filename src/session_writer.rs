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
//!
//! **Every acquire re-reads.** The other half of the parent spec's
//! invariant 1 — "every cassette you do not hold, you re-read from disk" —
//! is `refresh_from_disk`, run the instant the lock is won. While a cassette
//! is unfocused the TUI holds nothing and an agent may rewrite it; without
//! the re-read, one keystroke after tabbing back would republish the stale
//! in-memory copy over the agent's words and warn nobody.

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
    /// The guard currently held, if any. `None` when nothing is held —
    /// before the first `acquire`, after `finish`, or when acquiring failed
    /// and the session is read-only.
    ///
    /// Bound to the cassette's id, not its position in `app.cassettes`: the
    /// list can grow at arbitrary positions (an agent's `queue new` inserted
    /// by live sync in priority order), and an index would silently start
    /// naming a different cassette the moment something is inserted ahead of
    /// it. `LockGuard::id` is the guard's own id, so wherever a position is
    /// still needed it is resolved fresh by id rather than cached.
    guard: Option<LockGuard>,
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

    /// The id of the cassette whose lock is currently held, if any.
    /// `main.rs` compares it against the focused cassette's id to notice
    /// that focus moved.
    pub fn held_id(&self) -> Option<&str> {
        self.guard.as_ref().map(LockGuard::id)
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
        if let Some(held) = &self.guard {
            let already_this_one = app.cassettes.get(idx).is_some_and(|c| c.id == held.id());
            if already_this_one {
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
        // The lock is ours; the in-memory copy may not be. Re-read before
        // anything can be written back through this guard.
        self.refresh_from_disk(app, idx, &guard)?;
        self.guard = Some(guard);
        Ok(())
    }

    /// Refresh `app.cassettes[idx]` from the file whose lock was just won.
    ///
    /// The parent spec's invariant 1 is "you may only write a cassette whose
    /// lock you hold; every cassette you do not hold, you re-read from
    /// disk". Between losing focus and regaining it the TUI holds nothing,
    /// so any agent may have run `queue write` on that cassette — and
    /// `flush_held` writes the body straight out of memory. Without this
    /// step, tabbing back and typing one character republishes the stale
    /// copy over the agent's words, reverts `last_writer`, and warns nobody.
    ///
    /// This is not 5b's `merge_external`: nothing is merged. An unfocused
    /// cassette can never be dirty — `modify_focused` only ever touches the
    /// focused one, and `acquire` flushes the outgoing cassette through its
    /// own guard before the guard moves (the `debug_assert!` in `flush_held`
    /// states exactly that) — so the in-memory copy holds no keystrokes the
    /// disk lacks, and adopting the disk's version loses nothing. The
    /// `dirty` guard below is a belt to that brace: if the invariant ever
    /// breaks, unwritten words are kept rather than silently discarded.
    ///
    /// The refresh is skipped when the disk agrees with memory, which is the
    /// overwhelmingly common case (nobody else wrote). That keeps the
    /// cursor, the undo stack and the active side across an ordinary
    /// Tab-away-and-back; only genuinely changed text resets them, the same
    /// state `resume` starts a loaded cassette with.
    fn refresh_from_disk(
        &self,
        app: &mut App,
        idx: usize,
        guard: &LockGuard,
    ) -> Result<(), LockError> {
        let Some(c) = app.cassettes.get(idx) else {
            return Ok(());
        };
        debug_assert!(
            !c.dirty,
            "a cassette whose lock we do not hold cannot have unsaved edits: \
             `acquire` flushes the outgoing cassette before the guard moves"
        );
        if c.dirty {
            return Ok(());
        }
        let stored = guard.read()?;
        let (disk_a, disk_b) = crate::queue::json::split_sides(&stored.body);
        let (disk_a, disk_b) = (disk_a.trim(), disk_b.trim());
        // Resolved once here rather than on every render: `Cassette` carries
        // no store knowledge of its own, so the id-to-name lookup (the same
        // fallback-to-raw-id `store::writers::display_name` gives
        // `queue::write::write_permitted`'s sticky-lock message) happens the
        // one time this cassette's data is read from disk.
        let locked_by = stored
            .meta
            .locked_by
            .as_deref()
            .map(|id| match self.store.writers() {
                Ok(w) => crate::store::writers::display_name(&w, id),
                Err(_) => id.to_string(),
            });
        if disk_a == c.side_a_text().trim()
            && disk_b == c.side_b_text().trim()
            && stored.meta.topic == c.topic
        {
            // Text and topic are unchanged, so the cursor/undo short-circuit
            // still applies — but a sticky lock is metadata, not prose, and
            // can change (`queue lock`/`unlock`) with nothing else moving.
            if app.cassettes[idx].locked_by != locked_by {
                app.cassettes[idx].locked_by = locked_by;
            }
            return Ok(());
        }
        let id = c.id.clone();
        let mut fresh = crate::cassette::Cassette::from_sides(
            disk_a.to_string(),
            disk_b.to_string(),
            stored.meta.topic,
        );
        fresh.id = id;
        fresh.locked_by = locked_by;
        app.cassettes[idx] = fresh;
        app.clear_dirty(idx);
        Ok(())
    }

    /// Write the focused cassette through the held guard when it has unsaved
    /// edits. The guard names the focused cassette by id — `acquire` keeps
    /// them in step — and the write is keyed on the guard's id, so a focus
    /// change that has not yet moved the lock can never write one cassette's
    /// words into another's file, and an insertion elsewhere in the list
    /// cannot repoint the write either.
    pub fn flush_focused(&mut self, app: &mut App) -> io::Result<()> {
        self.flush_held(app).map_err(io::Error::from)
    }

    /// The flush itself, keyed on the guard's id rather than on
    /// `App::focus_idx` or a cached position.
    ///
    /// Frontmatter is re-read under the lock and only the fields the TUI
    /// owns are replaced: `priority`, `status` and `locked_by` belong to the
    /// queue commands, and an agent that reprioritized or closed a cassette
    /// while the human was writing in it must not have that undone by the
    /// next autosave.
    ///
    /// The *body* is written from memory, and that is safe only because the
    /// lock has been held continuously since `acquire` re-read it
    /// (`refresh_from_disk`): nobody else can have written this file in
    /// between. Every window in which somebody could have is a window in
    /// which this guard did not exist.
    fn flush_held(&mut self, app: &mut App) -> Result<(), LockError> {
        let Some(guard) = self.guard.as_ref() else {
            return Ok(());
        };
        // A cassette cannot actually be removed from `app.cassettes` today,
        // so this lookup is not known to ever fail — but the event loop is
        // the wrong place to discover that assumption was wrong. Treat a
        // failed lookup as "nothing to flush" rather than panicking or
        // indexing blindly.
        let Some(idx) = app.cassettes.iter().position(|c| c.id == guard.id()) else {
            return Ok(());
        };
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

    /// Locate the `cassette` binary this test must exec — rebuilding it
    /// first, so the path this returns is guaranteed *fresh*, not just
    /// guaranteed to exist.
    ///
    /// `CARGO_BIN_EXE_cassette` — what `tests/lock.rs` uses — is only set for
    /// integration tests; Cargo does not define it for a bin crate's own
    /// unit tests (confirmed directly: the same `env!` there fails to
    /// compile from inside this module). This crate has no `src/lib.rs`, so
    /// `tests/lock.rs` cannot link `SessionWriter` either — the cross-process
    /// test below has to live here instead, and locate the binary itself:
    /// the test binary runs from `target/<profile>/deps/`, and the plain
    /// binary sits one directory up, exactly as `CARGO_BIN_EXE_cassette`
    /// would resolve.
    ///
    /// Locating it is not, on its own, proof of anything about *current*
    /// code. Nothing forces Cargo to have rebuilt `cassette` before this
    /// function runs — a plain `cargo test` happens to, but only because
    /// `tests/lock.rs` also exists in this workspace and its own
    /// `CARGO_BIN_EXE_cassette` reference forces that target fresh in the
    /// same invocation. That is an incidental sibling, not a guarantee this
    /// file controls: scope the run to just this binary (`cargo test --bin
    /// cassette an_agents_write_is_denied...`) and `tests/lock.rs` never
    /// gets compiled, so nothing rebuilds `cassette` — this test would then
    /// exec whatever stale binary happens to sit in `target/`, silently
    /// proving nothing about the code as it stands. A reviewer confirmed
    /// this by editing an error string in `queue/write.rs` and watching a
    /// `--bin`-scoped run leave the binary's mtime untouched.
    ///
    /// So this function rebuilds the target itself before resolving its
    /// path, via the `CARGO` environment variable Cargo sets for every test
    /// binary it runs (confirmed present at runtime, unlike
    /// `CARGO_BIN_EXE_*`) rather than assuming `cargo` is on `PATH`. That
    /// makes freshness a property this test enforces on its own, regardless
    /// of `tests/lock.rs`'s existence or how narrowly the run is scoped — at
    /// the cost of one `cargo build` per run of this test. The profile name
    /// is read back off the running test binary's own path (the directory
    /// Cargo built it into) rather than guessed from `cfg!(debug_assertions)`,
    /// so this does the right thing under `--release` or a custom `--profile`
    /// too, respecting whatever `CARGO_TARGET_DIR` is already in the
    /// environment since the child inherits it; the one irregular case Cargo
    /// itself carves out is the default profile, whose flag is `dev` but
    /// whose output directory is named `debug`.
    fn bin_path() -> std::path::PathBuf {
        let mut dir = std::env::current_exe().expect("current test exe");
        dir.pop(); // drop the test binary's own file name
        if dir.ends_with("deps") {
            dir.pop();
        }
        let profile_dir = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("profile directory name")
            .to_string();
        let profile_flag: &str = if profile_dir == "debug" {
            "dev"
        } else {
            profile_dir.as_str()
        };

        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = std::process::Command::new(&cargo)
            .args(["build", "--bin", "cassette", "--profile", profile_flag])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("rebuild the cassette binary before exec'ing it");
        assert!(
            status.success(),
            "cargo build --bin cassette failed; cannot prove anything about stale code"
        );

        dir.join("cassette")
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

    /// A sticky lock (`queue lock`) set before the TUI ever focuses the
    /// cassette must show up resolved to a display name, not left as `None`
    /// or as the raw writer id — `ui.rs`'s separator has nothing else to
    /// show. This cassette's body is empty and its topic is `None` on both
    /// sides, so `refresh_from_disk`'s no-op short circuit is the one that
    /// fires here; `locked_by` must still be applied even though the
    /// rebuild it guards is skipped.
    #[test]
    fn acquiring_a_cassette_resolves_its_sticky_lock_to_a_writer_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 1);
        let cid = app.cassettes[0].id.clone();

        let lock = std::process::Command::new(bin_path())
            .args(["queue", "lock", &cid, "--session", &session])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn queue lock");
        assert_eq!(lock.status.code(), Some(0), "{lock:?}");

        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire 0");

        assert_eq!(
            app.cassettes[0].locked_by.as_deref(),
            Some("joseph"),
            "the sticky lock's writer id must resolve to its display name"
        );
    }

    /// Spawn two `queue write` invocations racing for the same cassette lock
    /// and return `(loser_output, winner_child)` — the same technique
    /// `tests/lock.rs`'s `contend` uses, reproduced here since that helper
    /// lives in a separate integration-test binary this module can't reach.
    /// See its doc comment for why "spawn a holder first, then assume a
    /// freshly spawned second process loses to it" is flaky by measurement
    /// (~1 failure in 6-15 runs observed): two freshly forked processes
    /// racing the same instruction are close enough in startup cost that
    /// either can win. Racing their *completions* instead needs no
    /// assumption about who acquires first — exactly one `try_lock` on the
    /// same flock must fail, so exactly one process exits almost immediately
    /// (before ever reading its own stdin) while the other blocks reading
    /// stdin, which nothing has closed, and cannot exit on its own. By the
    /// time this returns, `winner` is *proven* — by the loser's own exit, not
    /// by anything this function wrote to either child's stdin — to hold the
    /// cassette's lock.
    fn contend_for_lock(
        root: &std::path::Path,
        session: &str,
        id: &str,
    ) -> (std::process::Output, std::process::Child) {
        let spawn = |user: &str| -> std::process::Child {
            let mut child = std::process::Command::new(bin_path())
                .args(["queue", "write", id, "--session", session])
                .env("CASSETTE_DATA_DIR", root)
                .env("USER", user)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn queue write");
            match child.stdin.as_mut().expect("stdin").write_all(b"body\n") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => panic!("write: {e}"),
            }
            child
        };
        let mut a = spawn("contender-a");
        let mut b = spawn("contender-b");
        let a_id = a.id();
        let b_id = b.id();
        let a_out = a.stdout.take().expect("stdout a");
        let b_out = b.stdout.take().expect("stdout b");

        let (tx, rx) = std::sync::mpsc::channel();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            let mut out = a_out;
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut buf);
            let _ = tx.send(a_id);
        });
        std::thread::spawn(move || {
            let mut out = b_out;
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut buf);
            let _ = tx2.send(b_id);
        });
        // Blocks until whichever child's stdout closes first — the loser's,
        // since the winner's stays open until this function's caller acts.
        let first = rx.recv().expect("recv");
        let (loser, winner) = if first == a_id { (a, b) } else { (b, a) };
        let out = loser.wait_with_output().expect("wait loser");
        (out, winner)
    }

    /// The spec's required integration proof: a busy cassette becomes
    /// editable after its holder releases, with no keypress — `retry_lock`'s
    /// (`main.rs`) whole reason to exist. The holder here must be a real
    /// subprocess, never a second in-process `SessionWriter`:
    /// `store::lock`'s `HELD` bookkeeping is a process-global set keyed only
    /// by `(session, id)`, so two `SessionWriter`s in this one test process
    /// racing for the *same* cassette would trip `acquire`'s own
    /// `debug_assert!("re-acquiring a lock this process already holds...")`
    /// — a false alarm, not the real contention this test needs. The
    /// subprocess's own `HELD` bookkeeping lives in its own process, so it
    /// never touches this one's.
    #[test]
    fn retry_lock_wins_once_the_external_holder_releases_no_keypress_needed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 1);
        let id = app.cassettes[0].id.clone();

        // Deterministic by construction (see `contend_for_lock`): by the time
        // this returns, `holder` is proven to hold `id`'s lock, blocked
        // reading its own stdin, which nothing has closed yet.
        let (loser_out, mut holder) = contend_for_lock(&root, &session, &id);
        assert_eq!(loser_out.status.code(), Some(3), "{loser_out:?}");
        let err = String::from_utf8_lossy(&loser_out.stderr);
        assert!(err.contains("is open by"), "{err}");

        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        crate::try_acquire(&mut app, &mut w, 0);
        assert!(
            app.read_only.is_read_only(),
            "busy while the subprocess holds the lock"
        );
        // `holder` is already the subprocess here, so bind the name apart.
        let crate::app::ReadOnly::Busy { holder: who } = &app.read_only else {
            panic!("a held lock must read as Busy, not {:?}", app.read_only);
        };
        let holder_name = who
            .clone()
            .expect("the user must know WHO holds it, not just that it's busy");

        // The spec's banner must actually REACH the screen under a real held
        // lock, not merely exist as an arm of `info_text`. It was unreachable
        // once: `try_acquire` also wrote `status_msg`, which `info_text`
        // checks first, so every live busy cassette rendered the raw
        // `LockError` instead and this arm was dead on the only path that
        // can reach it.
        let line = crate::ui::info_text(&app);
        assert!(
            line.contains(&format!("-- READ ONLY (open by {holder_name}) --")),
            "the busy banner must name the holder on screen: {line}"
        );
        assert!(
            line.contains("cassette 1/"),
            "and must not cost the user the rest of the info line: {line}"
        );

        // Still busy on a tick that finds nothing changed.
        crate::retry_lock(&mut app, Some(&mut w));
        assert!(
            app.read_only.is_read_only(),
            "still blocked: the holder hasn't let go"
        );

        // Contention is a standing condition with a banner of its own, so a
        // retry that stays blocked must not spend `status_msg` on saying so
        // again — that is where transient news lives, and news showing over
        // a busy cassette has to survive the ticks that keep failing.
        app.flash("goal reached — 500 words. keep rolling!".to_string());
        crate::retry_lock(&mut app, Some(&mut w));
        assert_eq!(
            app.status_msg.as_deref(),
            Some("goal reached — 500 words. keep rolling!"),
            "a still-contended retry must not clobber an unrelated flash"
        );
        app.status_msg = None;

        // The holder releases: closing its stdin hands it EOF, so it
        // finishes its write and exits normally — no sleep, no poll; the
        // pipe close is the synchronisation.
        drop(holder.stdin.take());
        let done = holder.wait().expect("wait holder");
        assert!(done.success(), "{done:?}");

        // The next tick's retry wins with no keypress at all — the point of
        // this task.
        crate::retry_lock(&mut app, Some(&mut w));
        assert!(
            !app.read_only.is_read_only(),
            "editable once the holder released"
        );
        assert_eq!(app.read_only, crate::app::ReadOnly::No);
    }

    /// The reviewer's data-loss reproduction, end to end: the TUI must not
    /// republish a stale in-memory body over words another writer put on
    /// disk while the cassette was unfocused.
    ///
    /// Two cassettes; focus starts on 0, moves to 1 (which releases 0's
    /// lock), the real `cassette queue write` binary rewrites 0 from another
    /// process, focus returns to 0 and the human types one character. Before
    /// `refresh_from_disk` the final file was the human's stale text plus
    /// that character, with the agent's words gone and nobody warned.
    #[test]
    fn regaining_focus_re_reads_a_cassette_another_writer_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 2);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");

        // The human writes on cassette 0 …
        w.acquire(&mut app, 0).expect("acquire 0");
        app.focus_idx = 0;
        app.modify_focused(|c| c.insert_str("the human's first draft"));
        let zero_id = app.cassettes[0].id.clone();

        // … then tabs to cassette 1, which flushes and releases 0.
        w.acquire(&mut app, 1).expect("focus 1");
        app.focus_idx = 1;

        // An agent rewrites cassette 0 behind the TUI's back.
        let mut agent = std::process::Command::new(bin_path())
            .args(["queue", "write", &zero_id, "--session", &session])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "agent")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn agent write");
        agent
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"words the agent wrote\n")
            .expect("write body");
        let agent = agent.wait_with_output().expect("wait agent write");
        assert_eq!(
            agent.status.code(),
            Some(0),
            "the unfocused cassette must be writable: {agent:?}"
        );

        // The human tabs back and types one character.
        w.acquire(&mut app, 0).expect("focus 0 again");
        app.focus_idx = 0;
        app.modify_focused(|c| c.insert_str("!"));
        w.flush_focused(&mut app).expect("flush");

        let scan = store.scan_session(&session).expect("scan");
        let zero = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == zero_id)
            .expect("cassette 0");
        assert!(
            zero.body.contains("words the agent wrote"),
            "the external writer's words must survive the human's next keystroke: {}",
            zero.body
        );
        assert!(
            !zero.body.contains("the human's first draft"),
            "and the stale in-memory copy must not come back: {}",
            zero.body
        );
        assert!(
            zero.body.contains('!'),
            "the keystroke lands: {}",
            zero.body
        );
    }

    #[test]
    fn regaining_focus_keeps_cursor_and_undo_when_nothing_changed_on_disk() {
        // The re-read must not cost the writer their place on every Tab: an
        // unchanged file leaves the in-memory cassette — cursor, undo stack
        // and active side — exactly as it was.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (mut app, session) = fixture(&store, 2);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");
        w.acquire(&mut app, 0).expect("acquire 0");
        app.focus_idx = 0;
        app.modify_focused(|c| {
            c.snapshot(); // what entering insert mode does
            c.insert_str("hello world");
            c.move_word_back();
        });
        let cursor = app.cassettes[0].cursor_pos();

        w.acquire(&mut app, 1).expect("focus 1");
        w.acquire(&mut app, 0).expect("focus 0 again");

        assert_eq!(
            app.cassettes[0].cursor_pos(),
            cursor,
            "an unchanged file must not reset the cursor"
        );
        app.focus_idx = 0;
        app.modify_focused(|c| c.undo());
        assert_eq!(
            app.cassettes[0].text(),
            "",
            "and the undo stack must survive the round trip"
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
        assert_eq!(w.held_id(), Some(app.cassettes[0].id.as_str()));
    }

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
