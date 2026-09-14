// Nothing in the non-test build reaches the store until Phase 4 wires the CLI
// to it, so every item here is dead code to clippy until then. Lint attributes
// are inherited by nested modules, so this one covers the whole subtree.
// Remove it when Phase 4 lands.
#![allow(dead_code)]

//! The session store: session directories of per-cassette markdown files.
//!
//! Layout under the data dir (created `0700`):
//!
//! ```text
//! ~/.local/share/cassette/
//!   writers.toml
//!   active                       # single line: active session id
//!   .locks/
//!     writers                    # flock anchor for the writer registry
//!   sessions/
//!     <session ulid>/
//!       session.toml
//!       cassettes/
//!         <slug>-<cassette ulid>.md
//!       .locks/
//!         <cassette ulid>        # flock anchor, holder stamped after acquiring
//! ```
//!
//! Pure data and math live in `ids`, `meta` and `priority`; the thin I/O layer
//! is `session`, `writers`, and `Store` here. Writing an existing cassette
//! goes through `LockGuard::write` (see `lock`) — `Store` has no write method
//! of its own, so there is exactly one way onto disk.
//!
//! **`Store` owns the data-dir root.** The `session` and `writers` modules hold
//! the file formats, but their entry points are `pub(crate)` and everything
//! outside this module goes through a `Store` method. That is deliberate: when
//! `writers` and the active pointer took a bare root path of their own, they
//! created the store root through `atomic_write` at the process umask, leaving
//! a freewriting journal world-readable until some later call happened to
//! tighten it. One owner, one place that creates the root.

pub mod ids;
pub mod lock;
pub mod meta;
pub mod priority;
pub mod session;
pub mod writers;

use std::io;
use std::path::{Path, PathBuf};

use crate::store::meta::CassetteMeta;
use crate::store::session::SessionMeta;

/// Directory holding session directories, under the store root.
pub const SESSIONS_DIR: &str = "sessions";
/// Per-session directory of cassette files.
pub const CASSETTES_DIR: &str = "cassettes";
/// Per-session directory of flock anchors — see `lock`.
pub const LOCKS_DIR: &str = ".locks";

/// Write via a temp file in the same directory, then `rename()` over the
/// target. Spec invariant 4: a reader either sees the old file whole or the
/// new one whole, never a half-written mix. Same directory matters — `rename`
/// is only atomic within a filesystem.
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // The temp name carries a ULID so two writers never collide on it.
    let tmp = dir.join(format!(".tmp-{}", ids::new_id()));
    // Never leave a stray temp file behind on failure — including a partial
    // one from the initial write itself (e.g. disk-full), not just a failed
    // rename.
    if let Err(e) = std::fs::write(&tmp, contents) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Create `dir` as a private (`0700`) directory, tightening it if it already
/// exists with looser permissions. The mode is baked into `mkdir(2)` rather
/// than chmod'd afterwards, so the directory is never briefly world-readable;
/// parents keep their own permissions.
///
/// Free-standing rather than a `Store` method because `writers` and the active
/// pointer take a bare root path and would otherwise create the store root
/// through `atomic_write` at the process umask.
pub(crate) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let mut perms = std::fs::metadata(dir)?.permissions();
        if perms.mode() & 0o777 != 0o700 {
            perms.set_mode(0o700);
            std::fs::set_permissions(dir, perms)?;
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// One cassette as it exists on disk.
#[derive(Debug, Clone)]
pub struct StoredCassette {
    pub path: PathBuf,
    pub meta: CassetteMeta,
    /// Everything after the frontmatter, byte-for-byte.
    pub body: String,
}

/// The store rooted at a data dir. Holds no state beyond the path: every
/// method reads or writes the filesystem directly, which is what makes
/// concurrent writers possible.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    /// `~/.local/share/cassette` — the `data_dir` config key overrides it.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("cassette"))
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join(SESSIONS_DIR)
    }

    pub fn session_dir(&self, session: &str) -> PathBuf {
        self.sessions_dir().join(session)
    }

    pub fn cassettes_dir(&self, session: &str) -> PathBuf {
        self.session_dir(session).join(CASSETTES_DIR)
    }

    pub fn locks_dir(&self, session: &str) -> PathBuf {
        self.session_dir(session).join(LOCKS_DIR)
    }

    /// Create the store root `0700` if it is not there yet. A freewriting
    /// journal is private by default; tightening it later would leave a
    /// window where other local users can read it.
    fn ensure_root(&self) -> io::Result<()> {
        ensure_private_dir(&self.root)
    }

    /// Mint a session id, build its directory layout, write `session.toml`,
    /// and return the id.
    pub fn create_session(&self, m: &SessionMeta) -> io::Result<String> {
        self.ensure_root()?;
        let id = ids::new_id();
        ensure_private_dir(&self.cassettes_dir(&id))?;
        ensure_private_dir(&self.locks_dir(&id))?;
        session::write(&self.session_dir(&id).join("session.toml"), m)?;
        Ok(id)
    }

    /// Create a new cassette, named `<slug>-<id>.md` from the topic at
    /// creation, and its lock anchor. Returns the path, which callers keep:
    /// the name is never recomputed, even when the topic changes.
    ///
    /// Takes and releases the cassette's own lock like every other write. A
    /// freshly minted ULID cannot be contended, so this cannot fail on `Busy`
    /// — routing it through the guard means there is exactly one way a
    /// cassette file is ever written.
    ///
    /// There is no empty-placeholder write before acquiring: `lock::acquire`
    /// takes this path directly rather than resolving it from disk, and
    /// `LockGuard::write` creates the file itself via `atomic_write`'s
    /// temp-then-rename. A placeholder would only open a window where a
    /// failed acquire or write leaves a stray zero-byte cassette behind.
    pub fn add_cassette(&self, session: &str, m: &CassetteMeta, body: &str) -> io::Result<PathBuf> {
        let path = self
            .cassettes_dir(session)
            .join(ids::file_name(m.topic.as_deref(), &m.id));
        if let Some(parent) = path.parent() {
            ensure_private_dir(parent)?;
        }
        let anchor = self.locks_dir(session).join(&m.id);
        // `created_by` is a writer id, and it is deliberately used for the
        // display name too rather than resolved through `writers.toml`.
        // Nothing reads this stamp except a contender in the microsecond
        // window between `guard.write`'s rename and the guard dropping, and
        // resolving it would make creating any cassette depend on the
        // registry being readable — coupling the creation path to a resource
        // Task 5 puts a blocking lock on. Where a human actually waits on a
        // "held by" message is `Store::lock`, whose callers pass a properly
        // resolved name.
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

    /// The session's own metadata (`session.toml`).
    pub fn session_meta(&self, session: &str) -> io::Result<SessionMeta> {
        session::read(&self.session_dir(session).join("session.toml"))
    }

    /// The writer registry. A store with no `writers.toml` yet has an empty
    /// one; any other read failure propagates.
    pub fn writers(&self) -> io::Result<writers::Writers> {
        writers::read(&self.root)
    }

    /// Replace the writer registry wholesale.
    pub fn write_writers(&self, w: &writers::Writers) -> io::Result<()> {
        writers::write(&self.root, w)
    }

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

    /// The id for `name`, registering it on first sight. Idempotent: the same
    /// name never mints a second id.
    ///
    /// Holds the registry lock across the read-modify-write. Without it two
    /// writers registering at once both read a registry lacking the other,
    /// both insert, and the second write clobbers the first — which is exactly
    /// the first-run case, where the TUI registers from `$USER` while an agent
    /// registers itself.
    pub fn ensure_writer(&self, name: &str, kind: writers::Kind) -> io::Result<String> {
        // `_registry`, NOT `_`: a bare underscore drops the guard immediately
        // and silently reopens the lost-update race this whole function exists
        // to close. No test catches the difference — the critical section is
        // too fast to lose reliably — so this comment is the pin.
        let _registry = self.lock_registry()?;
        writers::ensure(&self.root, name, kind)
    }

    /// The active session id, or `None` when no session is active.
    pub fn active_session(&self) -> io::Result<Option<String>> {
        session::read_active(&self.root)
    }

    /// Point `active` at `session`.
    pub fn set_active_session(&self, session: &str) -> io::Result<()> {
        session::write_active(&self.root, session)
    }

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
        let path = match self.cassette_path(session, id)? {
            Some(path) => path,
            None => {
                return Err(lock::LockError::NoSuchCassette {
                    session: session.to_string(),
                    id: id.to_string(),
                })
            }
        };
        let anchor_path = self.locks_dir(session).join(id);
        lock::acquire(id, path, &anchor_path, as_writer, lock::Blocking::No)
    }

    /// Acquire several cassette locks at once, all or nothing.
    ///
    /// Locks are always taken in **ascending id order**, regardless of the
    /// order requested. This is a liveness device and nothing more: a total
    /// order makes livelock between two overlapping multi-lock operations
    /// impossible — with a non-blocking primitive nobody waits, so without
    /// this both sides would get `Busy`, both retry, and both fail forever.
    /// It is emphatically **not** queue order — that is `priority` first,
    /// closed last, with the id only as a tiebreak; nothing here is ever
    /// displayed or used to decide which cassette a writer sees next.
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
        let before = ordered.len();
        ordered.dedup();
        if ordered.len() != before {
            // A repeated id is almost never a real request to hold one lock
            // twice — it is an indexing bug in whatever computed the run.
            // Deduping silently would return fewer guards than ids, and a
            // caller zipping guards against new priorities would then write
            // to fewer cassettes than it meant to. Fail loudly instead; a
            // caller that legitimately produces duplicates should collapse
            // them itself, where the intent is visible.
            return Err(lock::LockError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lock_many: duplicate cassette id",
            )));
        }
        let mut guards = Vec::with_capacity(ordered.len());
        for id in ordered {
            // `?` drops `guards` on the way out, releasing everything taken.
            guards.push(self.lock(session, id, as_writer)?);
        }
        Ok(guards)
    }

    /// Every cassette in a session, in queue order. A missing session, files
    /// that are not `.md`, and `.md` files without parseable frontmatter are
    /// all skipped rather than erroring — the store shares a directory with
    /// editors and their swap files.
    pub fn scan_session(&self, session: &str) -> io::Result<Vec<StoredCassette>> {
        let dir = self.cassettes_dir(session);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut found = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (Some(meta), body) = meta::split(&content) else {
                continue;
            };
            found.push(StoredCassette {
                path,
                meta,
                body: body.to_string(),
            });
        }
        let mut metas: Vec<CassetteMeta> = found.iter().map(|c| c.meta.clone()).collect();
        priority::queue_order(&mut metas);
        let order: Vec<&str> = metas.iter().map(|m| m.id.as_str()).collect();
        found.sort_by_key(|c| {
            order
                .iter()
                .position(|id| *id == c.meta.id)
                .unwrap_or(usize::MAX)
        });
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::session::SessionMeta;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn cassette_meta(id: &str, priority: i64) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: Some("gratitude".to_string()),
            priority,
            status: Status::Open,
            locked_by: None,
            created_by: "writer-1".to_string(),
            last_writer: "writer-1".to_string(),
            updated_at: meta::now_utc(),
        }
    }

    fn session_meta() -> SessionMeta {
        SessionMeta {
            alias: None,
            created: meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        }
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let (dir, _s) = store();
        let path = dir.path().join("f.txt");
        atomic_write(&path, "hello").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "f.txt")
            .collect();
        assert!(strays.is_empty(), "temp files left behind: {strays:?}");
    }

    #[test]
    fn atomic_write_replaces_existing_content_whole() {
        let (dir, _s) = store();
        let path = dir.path().join("f.txt");
        atomic_write(&path, "first").expect("write");
        atomic_write(&path, "second").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn create_session_builds_the_directory_layout() {
        let (_dir, s) = store();
        let id = s.create_session(&session_meta()).expect("create");
        assert_eq!(id.len(), 26);
        assert!(s.session_dir(&id).is_dir());
        assert!(s.cassettes_dir(&id).is_dir());
        assert!(
            s.locks_dir(&id).is_dir(),
            ".locks must exist for the lock protocol"
        );
        assert!(s.session_dir(&id).join("session.toml").is_file());
    }

    #[test]
    fn create_session_reads_its_metadata_back() {
        let (_dir, s) = store();
        let m = SessionMeta {
            alias: Some("morning".to_string()),
            ..session_meta()
        };
        let id = s.create_session(&m).expect("create");
        let read_back = s.session_meta(&id).expect("read");
        assert_eq!(read_back.alias.as_deref(), Some("morning"));
    }

    #[test]
    fn add_cassette_names_the_file_by_slug_and_id() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let path = s
            .add_cassette(&sid, &m, "## Side A\n\nhello\n")
            .expect("add");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "gratitude-01K5GR7T2M9WPD0000000000AB.md"
        );
    }

    #[test]
    fn a_cassette_round_trips_through_the_store() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let body = "## Side A\n\nhello\n\n## Side B\n\nscratch\n";
        s.add_cassette(&sid, &m, body).expect("add");

        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].meta, m);
        assert_eq!(found[0].body, body, "body must survive byte-for-byte");
    }

    #[test]
    fn scan_returns_cassettes_in_queue_order() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut closed = cassette_meta("aaa00000000000000000000000", 5);
        closed.status = Status::Closed;
        s.add_cassette(&sid, &cassette_meta("ccc00000000000000000000000", 30), "")
            .expect("add");
        s.add_cassette(&sid, &closed, "").expect("add");
        s.add_cassette(&sid, &cassette_meta("bbb00000000000000000000000", 10), "")
            .expect("add");

        let ids: Vec<String> = s
            .scan_session(&sid)
            .expect("scan")
            .into_iter()
            .map(|c| c.meta.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "bbb00000000000000000000000",
                "ccc00000000000000000000000",
                "aaa00000000000000000000000"
            ],
            "open by priority, closed last — even with the best priority"
        );
    }

    #[test]
    fn scan_ignores_non_cassette_files() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        s.add_cassette(&sid, &cassette_meta("aaa00000000000000000000000", 10), "")
            .expect("add");
        // An editor swap file and a file with no frontmatter must not appear.
        std::fs::write(s.cassettes_dir(&sid).join("notes.txt"), "stray").expect("write");
        std::fs::write(s.cassettes_dir(&sid).join("broken.md"), "no frontmatter\n").expect("write");
        assert_eq!(s.scan_session(&sid).expect("scan").len(), 1);
    }

    #[test]
    fn scanning_a_missing_session_is_empty_not_an_error() {
        let (_dir, s) = store();
        assert!(s.scan_session("nope").expect("scan").is_empty());
    }

    #[test]
    fn an_unreadable_cassettes_dir_errors_rather_than_scanning_empty() {
        // Only NotFound may mean "no cassettes". Reporting an empty queue for a
        // directory we merely could not read would let a caller insert at the
        // head of a queue it never saw. A file where the directory should be is
        // the portable way to force a non-NotFound failure.
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        std::fs::remove_dir_all(s.cassettes_dir(&sid)).expect("rmdir");
        std::fs::write(s.cassettes_dir(&sid), "not a directory").expect("write");
        assert!(
            s.scan_session(&sid).is_err(),
            "an unreadable cassettes dir must not scan as empty"
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn an_existing_loose_data_dir_is_tightened() {
        // A store root that already exists with loose permissions must be
        // brought back to 0700 rather than left as found.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        std::fs::create_dir(&root).expect("mkdir");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        Store::new(root.clone())
            .create_session(&session_meta())
            .expect("create");
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "a loose existing root must be tightened"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_data_dirs_parent_keeps_its_own_permissions() {
        // Only the store root is private; creating it must not tighten
        // ~/.local/share (or whatever the parent happens to be).
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("share");
        let root = parent.join("cassette");
        Store::new(root)
            .create_session(&session_meta())
            .expect("create");
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode();
        assert_ne!(mode & 0o777, 0o700, "the parent must not be forced to 0700");
    }

    #[test]
    fn the_writer_registry_round_trips_through_the_store() {
        let (_dir, s) = store();
        assert!(
            s.writers().expect("read").writers.is_empty(),
            "empty to start"
        );
        let id = s
            .ensure_writer("joseph", writers::Kind::Human)
            .expect("ensure");
        let again = s
            .ensure_writer("joseph", writers::Kind::Human)
            .expect("ensure");
        assert_eq!(id, again, "the same name must not mint a second id");
        let all = s.writers().expect("read");
        assert_eq!(all.writers.len(), 1);
        assert_eq!(all.writers[&id].name, "joseph");
    }

    #[test]
    fn the_active_session_round_trips_through_the_store() {
        let (_dir, s) = store();
        assert_eq!(s.active_session().expect("read"), None, "none to start");
        let id = s.create_session(&session_meta()).expect("create");
        s.set_active_session(&id).expect("set");
        assert_eq!(
            s.active_session().expect("read").as_deref(),
            Some(id.as_str())
        );
    }

    #[test]
    fn an_unreadable_active_pointer_errors_rather_than_reading_as_absent() {
        // Moving the pointer onto Store was the moment to stop collapsing
        // "no active session" into "could not read it" — the same conflation
        // that made writers::read a data-loss path.
        let (dir, s) = store();
        s.create_session(&session_meta()).expect("create");
        std::fs::create_dir(dir.path().join(session::ACTIVE_FILE)).expect("mkdir");
        assert!(
            s.active_session().is_err(),
            "an unreadable pointer must not read as 'no active session'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn registering_a_writer_creates_a_private_root() {
        // The spec auto-registers a writer on first run, which can happen
        // before any session exists — that path must not leave the root loose.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        Store::new(root.clone())
            .ensure_writer("joseph", writers::Kind::Human)
            .expect("ensure");
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "writer registration must create a private root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn setting_the_active_session_creates_a_private_root() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        Store::new(root.clone())
            .set_active_session("01K5GQ2R8V3XQZ0000000000AB")
            .expect("write");
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "the active pointer must not create a loose root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_data_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let s = Store::new(root.clone());
        s.create_session(&session_meta()).expect("create");
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "a private journal is not world-readable"
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn session_subdirectories_are_private_too() {
        // The 0700 root already stops another user traversing in, so this is
        // defence in depth: a root loosened by a restore or a sync tool must
        // not expose every session beneath it.
        use std::os::unix::fs::PermissionsExt;
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        for d in [s.cassettes_dir(&sid), s.locks_dir(&sid)] {
            let mode = std::fs::metadata(&d).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{} must be private", d.display());
        }
    }
}
