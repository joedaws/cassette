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
//!   sessions/
//!     <session ulid>/
//!       session.toml
//!       cassettes/
//!         <slug>-<cassette ulid>.md
//!       .locks/
//!         <cassette ulid>        # empty flock anchor (Phase 3)
//! ```
//!
//! Pure data and math live in `ids`, `meta` and `priority`; the thin I/O layer
//! is `session`, `writers`, and `Store` here. No locking yet — Phase 3 wraps
//! the write calls.

pub mod ids;
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
/// Per-session directory of flock anchors (Phase 3 opens these; this phase
/// only creates the directory so the layout is complete).
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            // Parents (e.g. ~/.local/share) keep their normal permissions —
            // only the store root is private.
            if let Some(parent) = self.root.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // 0700 is baked into the mkdir(2) call rather than chmod'd on
            // afterwards: create-then-tighten leaves a window where the
            // directory exists world-readable, and the spec says the data
            // directory *is created* 0700. umask can only narrow this further,
            // never widen it.
            match std::fs::DirBuilder::new().mode(0o700).create(&self.root) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            // A directory that already existed may still be loose — tighten it.
            let mut perms = std::fs::metadata(&self.root)?.permissions();
            if perms.mode() & 0o777 != 0o700 {
                perms.set_mode(0o700);
                std::fs::set_permissions(&self.root, perms)?;
            }
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Mint a session id, build its directory layout, write `session.toml`,
    /// and return the id.
    pub fn create_session(&self, m: &SessionMeta) -> io::Result<String> {
        self.ensure_root()?;
        let id = ids::new_id();
        std::fs::create_dir_all(self.cassettes_dir(&id))?;
        std::fs::create_dir_all(self.locks_dir(&id))?;
        session::write(&self.session_dir(&id).join("session.toml"), m)?;
        Ok(id)
    }

    /// Create a new cassette file, named `<slug>-<id>.md` from the topic at
    /// creation. Returns the path, which callers keep: the name is never
    /// recomputed, even when the topic changes.
    pub fn add_cassette(&self, session: &str, m: &CassetteMeta, body: &str) -> io::Result<PathBuf> {
        let path = self
            .cassettes_dir(session)
            .join(ids::file_name(m.topic.as_deref(), &m.id));
        self.write_cassette(&path, m, body)?;
        Ok(path)
    }

    /// Overwrite a cassette in place. Deliberately takes the path rather than
    /// deriving it: the file keeps the slug it was minted with, so a retopic
    /// updates frontmatter without a rename.
    pub fn write_cassette(&self, path: &Path, m: &CassetteMeta, body: &str) -> io::Result<()> {
        atomic_write(path, &format!("{}\n{}", meta::build_frontmatter(m), body))
    }

    /// Every cassette in a session, in queue order. A missing session, files
    /// that are not `.md`, and `.md` files without parseable frontmatter are
    /// all skipped rather than erroring — the store shares a directory with
    /// editors and their swap files.
    pub fn scan_session(&self, session: &str) -> io::Result<Vec<StoredCassette>> {
        let dir = self.cassettes_dir(session);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
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
            ".locks must exist before Phase 3"
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
        let read_back = session::read(&s.session_dir(&id).join("session.toml")).expect("read");
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
    fn write_cassette_updates_in_place_without_renaming() {
        // The slug is frozen at creation: retopicking must not move the file,
        // because an flock is held on the inode another writer resolved.
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let path = s.add_cassette(&sid, &m, "old\n").expect("add");

        m.topic = Some("completely different".to_string());
        s.write_cassette(&path, &m, "new\n").expect("write");

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
}
