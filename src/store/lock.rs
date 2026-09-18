//! Cassette locking: `flock` on a `.locks/<id>` sidecar.
//!
//! The lock is on the sidecar and never on the `.md`, because `flock` attaches
//! to an inode and an atomic write replaces the inode via `rename()` — locking
//! the cassette file would silently hand two writers the same "lock". The
//! anchor is never renamed and never deleted, so its inode is stable, and its
//! *existence* carries no meaning: lockedness is kernel state, tested by
//! attempting acquisition.

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use fs4::FileExt;

use crate::store::meta;
use crate::store::meta::CassetteMeta;
use crate::store::StoredCassette;

/// The cassettes (`"<session>/<id>"`) this *process* currently holds a
/// [`LockGuard`] for.
///
/// `Store::holds` exists to answer "do we hold this lock?", and `flock`
/// itself has no way to answer that: a second `try_lock` from the same
/// process on an anchor it already holds fails exactly as it would for a
/// genuinely different process (`flock` is per open-file-description, not
/// per-process), so probing the anchor can only ever tell you "is this
/// locked at all" — which is trivially always true once we hold it. The
/// question `holds` actually needs answered is not visible to the kernel, so
/// it is tracked here instead: `acquire` inserts the key once a lock is
/// genuinely won, and `LockGuard::drop` removes it, so the set exactly
/// mirrors the guards this process is currently holding. Process-global
/// (not a per-`Store` field) because that is the true granularity of the
/// question — two `Store` values over the same root, or a lock acquired
/// through a clone, are still the same process holding the same flock.
static HELD: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn held_key(session: &str, id: &str) -> String {
    format!("{session}/{id}")
}

/// Record that this process now holds `session`/`id`'s lock. Called only
/// from `acquire`, only after the `flock` is genuinely won.
fn record_held(session: &str, id: &str) {
    HELD.lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(held_key(session, id));
}

/// The inverse of `record_held`, called from `LockGuard::drop`.
fn clear_held(session: &str, id: &str) {
    HELD.lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&held_key(session, id));
}

/// Whether this process's bookkeeping says it holds `session`/`id` right
/// now. See the `HELD` doc comment for why this is a registry lookup and
/// never a probe of the anchor itself.
pub(crate) fn is_held(session: &str, id: &str) -> bool {
    HELD.lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&held_key(session, id))
}

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

    /// Free-text fields (`writer` is normally a ULID and `since` an RFC3339
    /// stamp we generated ourselves, but `name` comes from `$USER` — run
    /// everything through `meta::one_line` anyway, the same normaliser the
    /// frontmatter builder runs every free-text field through. The anchor is
    /// line-oriented exactly like frontmatter is: an embedded newline would
    /// end the record early, and a crafted `name` could inject a fake
    /// `pid=`/`since=` for the next field to "parse".
    pub fn render(&self) -> String {
        format!(
            "writer={} name={} pid={} since={}",
            meta::one_line(&self.writer),
            meta::one_line(&self.name),
            self.pid,
            meta::one_line(&self.since)
        )
    }

    /// Parse the rendered form. `None` for anything else — a crashed holder can
    /// leave arbitrary bytes here and the caller degrades to a generic message.
    ///
    /// Fields are located by their markers rather than by splitting on
    /// whitespace, because `name` is free text and may contain spaces —
    /// and, since `one_line` only strips line breaks, may also contain the
    /// literal text " pid=" or " since=". `writer`/`name` are split on the
    /// *first* " name=", because that marker is written immediately after
    /// `writer=` and anything past it — markers included — belongs to
    /// `name`. `pid`/`since` are split from the *last* " since=" and then
    /// the last " pid=" in what remains, because `render` appends them in
    /// that fixed order after `name`: the rightmost occurrences are always
    /// the real ones, so a `name` forged to contain its own "pid=…
    /// since=…" cannot shift what the real trailing fields parse as.
    pub fn parse(line: &str) -> Option<Attribution> {
        let line = line.trim();
        let rest = line.strip_prefix("writer=")?;
        let (writer, rest) = rest.split_once(" name=")?;
        let (rest, since) = rest.rsplit_once(" since=")?;
        let (name, pid) = rest.rsplit_once(" pid=")?;
        Some(Attribution {
            writer: writer.to_string(),
            name: name.to_string(),
            pid: pid.parse().ok()?,
            since: since.to_string(),
        })
    }
}

/// Why a lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another live writer holds it. `holder` is `None` when they left no
    /// readable attribution — a crash before writing their line, or garbled
    /// bytes. It still blocks us; we just cannot name them. `id` is always
    /// present, so a caller locking several cassettes can say which one.
    Busy {
        id: String,
        holder: Option<Attribution>,
    },
    /// No cassette with this id in this session. A distinct variant rather
    /// than an `Io(NotFound)` so callers can render a usage error without
    /// matching on `io::ErrorKind` — that heuristic silently depends on
    /// `acquire` never surfacing `NotFound` itself, which nothing enforces.
    NoSuchCassette {
        session: String,
        id: String,
    },
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy {
                id,
                holder: Some(a),
            } => {
                write!(f, "'{id}' is held by {} (since {})", a.name, a.since)
            }
            LockError::Busy { id, holder: None } => {
                write!(f, "'{id}' is held by another writer")
            }
            LockError::NoSuchCassette { session, id } => {
                write!(f, "no cassette '{id}' in session '{session}'")
            }
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for LockError {
    fn from(e: io::Error) -> LockError {
        LockError::Io(e)
    }
}

impl From<LockError> for io::Error {
    fn from(e: LockError) -> io::Error {
        // `e.to_string()` borrows `e` via `Display`, so this is computed
        // before `e` is moved into the match below.
        let msg = e.to_string();
        match e {
            LockError::Io(e) => e,
            LockError::NoSuchCassette { .. } => io::Error::new(io::ErrorKind::NotFound, msg),
            LockError::Busy { .. } => io::Error::new(io::ErrorKind::WouldBlock, msg),
        }
    }
}

/// A held cassette lock. The only way to write an existing cassette.
///
/// The lock is released when this value is dropped, and by the kernel if the
/// process dies for any reason including `SIGKILL` — which is why there is no
/// reaper, no pid file and no `--force-unlock`.
#[derive(Debug)]
pub struct LockGuard {
    id: String,
    /// `Some(session)` for an ordinary cassette lock, so `Drop` knows the
    /// `HELD` key to clear; `None` for the one lock that is not
    /// session-scoped, the writer registry (`Store::lock_registry`) — that
    /// guard is never exposed past `ensure_writer`/`resolve_writer`, and
    /// `Store::holds` only ever asks about cassettes, so there is nothing
    /// for the registry lock to bookkeep.
    session: Option<String>,
    /// The cassette file. Resolved once at acquisition: the slug is frozen at
    /// creation, so this path cannot be derived from a (possibly retopicked)
    /// `CassetteMeta`.
    path: PathBuf,
    /// Holding the `File` IS holding the lock: dropping it closes the fd and
    /// the kernel releases — that part needs no `Drop` impl. The `Drop` impl
    /// below exists only to clear this guard's `HELD` bookkeeping (see the
    /// module-level `HELD` doc comment); it must never attempt to release
    /// the flock itself, which remains the kernel's job via `_anchor`'s own
    /// drop.
    _anchor: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            clear_held(session, &self.id);
        }
    }
}

impl LockGuard {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cassette's current state, read under the lock.
    ///
    /// Assumes `path` holds a cassette file (frontmatter + body). The guard
    /// returned by the registry lock (`Store::lock_registry`) points at
    /// `writers.toml` instead — calling `read`/`write` on that guard would
    /// try to parse or overwrite the registry as a cassette. That guard is
    /// never exposed past `ensure_writer` or `resolve_writer`, which only
    /// hold it and never call either method, so this cannot happen today; it
    /// stays a trap for a future caller to avoid, not a bug to fix.
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
    ///
    /// See the note on `read`: calling this on the registry lock's guard
    /// would clobber `writers.toml` with cassette-shaped frontmatter. Not
    /// reachable today — neither `ensure_writer` nor `resolve_writer` calls
    /// it.
    pub fn write(&self, m: &CassetteMeta, body: &str) -> io::Result<()> {
        crate::store::atomic_write(
            &self.path,
            &format!("{}\n{}", crate::store::meta::build_frontmatter(m), body),
        )
    }
}

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
///
/// `session` is `Some` for every ordinary cassette lock and `None` only for
/// the writer registry (`Store::lock_registry`), which is not session-scoped.
/// It is threaded through purely for the `HELD` bookkeeping `Store::holds`
/// reads — see the module doc comment — and plays no part in the `flock`
/// itself.
pub(crate) fn acquire(
    id: &str,
    path: PathBuf,
    anchor_path: &Path,
    as_writer: &Attribution,
    blocking: Blocking,
    session: Option<&str>,
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

    // Bound via the trait explicitly (UFCS), not `anchor.lock()`/`.try_lock()`:
    // this toolchain's `std::fs::File` has stabilized inherent methods of the
    // same names, and an inherent method always wins over a trait method on
    // method-call syntax regardless of which trait is imported. Calling
    // through `FileExt::` keeps this bound to fs4's `TryLockError`, which is
    // what the `Busy` match arms below are typed against.
    match blocking {
        Blocking::Yes => FileExt::lock(&anchor).map_err(LockError::Io)?,
        Blocking::No => {
            if let Err(e) = FileExt::try_lock(&anchor) {
                return Err(match e {
                    fs4::TryLockError::WouldBlock => {
                        // Read the holder's line without the lock: it is
                        // display-only, so a torn read costs us a message, not
                        // correctness.
                        let held = std::fs::read_to_string(anchor_path)
                            .ok()
                            .and_then(|s| Attribution::parse(&s));
                        LockError::Busy {
                            id: id.to_string(),
                            holder: held,
                        }
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

    if let Some(session) = session {
        record_held(session, id);
    }

    Ok(LockGuard {
        id: id.to_string(),
        session: session.map(str::to_string),
        path,
        _anchor: anchor,
    })
}

/// Is this lock free right now? Opens the anchor without creating it and
/// tries the lock, releasing immediately when it succeeds.
///
/// Deliberately NOT `acquire` with the guard dropped: `acquire` stamps the
/// holder into the anchor after locking, so using it to ask a question would
/// write to every cassette the caller merely looked at. The answer is a
/// snapshot — the lock may be taken the instant after this returns — which is
/// why the only caller, `queue next`, reports rather than claims.
pub(crate) fn probe(anchor_path: &Path) -> io::Result<bool> {
    let anchor = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(anchor_path)
    {
        Ok(f) => f,
        // No anchor means nobody has ever locked this cassette.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    // UFCS, not `anchor.try_lock()` — see the comment in `acquire` above:
    // this toolchain's `std::fs::File` has an inherent `try_lock` that would
    // otherwise shadow fs4's trait method.
    match FileExt::try_lock(&anchor) {
        Ok(()) => Ok(true), // released when `anchor` drops
        Err(fs4::TryLockError::WouldBlock) => Ok(false),
        Err(fs4::TryLockError::Error(e)) => Err(e),
    }
}

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
    fn a_name_containing_a_newline_cannot_break_the_anchor_line() {
        // The anchor is line-oriented like frontmatter is, and `name` comes
        // from $USER. Display-only, so this is tidiness rather than a
        // vulnerability — but the frontmatter builder already normalises every
        // free-text field and the two should not disagree.
        let a = Attribution {
            writer: "w1".to_string(),
            name: "joseph\nwriter=evil name=mallory pid=1 since=x".to_string(),
            pid: 42,
            since: "2026-09-14T14:02:11Z".to_string(),
        };
        let rendered = a.render();
        assert_eq!(
            rendered.lines().count(),
            1,
            "must stay one line: {rendered}"
        );
        let parsed = Attribution::parse(&rendered).expect("parses");
        assert_eq!(parsed.pid, 42, "later fields must survive");
        assert_eq!(parsed.since, "2026-09-14T14:02:11Z");
    }

    #[test]
    fn for_now_stamps_this_process() {
        let a = Attribution::for_now("writer-1", "joseph");
        assert_eq!(a.writer, "writer-1");
        assert_eq!(a.name, "joseph");
        assert_eq!(a.pid, std::process::id());
        assert!(a.since.ends_with('Z'), "{}", a.since);
    }

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
            Err(LockError::Busy {
                holder: Some(a), ..
            }) => {
                assert_eq!(
                    a.name, "joseph",
                    "the blocked writer must learn who holds it"
                );
                assert_eq!(a.pid, std::process::id());
            }
            other => panic!("expected Busy with attribution, got {other:?}"),
        }
    }

    #[test]
    fn busy_names_the_cassette_as_well_as_the_holder() {
        // With `lock_many` behind `queue move`, "held by joseph" is not
        // actionable unless it says which cassette.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        let _held = s.lock(&sid, ID, &who).expect("first");
        match s.lock(&sid, ID, &Attribution::for_now("writer-2", "agent")) {
            Err(LockError::Busy { id, holder }) => {
                assert_eq!(id, ID, "the blocked caller must learn which cassette");
                assert_eq!(holder.expect("holder").name, "joseph");
            }
            other => panic!("expected Busy with an id, got {other:?}"),
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
                tx.send(matches!(r, Err(LockError::Busy { .. })))
                    .expect("send");
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
        // `add_cassette` creates the anchor at birth (see mod.rs), so remove
        // it to force this test down the on-demand path in `lock` — the
        // fallback that keeps a hand-written or pre-Phase-3 cassette lockable.
        let _ = std::fs::remove_file(s.locks_dir(&sid).join(ID));
        let who = Attribution::for_now("writer-1", "joseph");
        s.lock(&sid, ID, &who)
            .expect("must create the anchor on demand");
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
            Err(LockError::Busy { holder: None, .. }) => {}
            other => panic!("expected Busy with no holder, got {other:?}"),
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

    #[test]
    fn locking_an_unknown_cassette_reports_no_such_cassette() {
        // A distinct variant, not Io(NotFound): callers render this as a usage
        // error and must not have to match on io::ErrorKind to find out.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        let who = Attribution::for_now("writer-1", "joseph");
        match s.lock(&sid, "nosuchcassette0000000000AB", &who) {
            Err(LockError::NoSuchCassette { session, id }) => {
                assert_eq!(session, sid);
                assert_eq!(id, "nosuchcassette0000000000AB");
            }
            other => panic!("expected NoSuchCassette, got {other:?}"),
        }
    }

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
    fn lock_many_returns_guards_in_ascending_id_order() {
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
        match r {
            Err(LockError::Busy { id, .. }) => assert_eq!(
                id, "bbb00000000000000000000000",
                "must name the cassette that actually blocked, not the first requested"
            ),
            other => panic!("expected Busy, got {other:?}"),
        }
        // a must be free again — if lock_many kept it, this would be Busy.
        s.lock(&sid, "aaa00000000000000000000000", &other)
            .expect("a must have been released");
        drop(held);
    }

    #[test]
    fn lock_many_rejects_a_duplicate_id() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta("aaa00000000000000000000000"), "")
            .expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        let r = s.lock_many(
            &sid,
            &["aaa00000000000000000000000", "aaa00000000000000000000000"],
            &who,
        );
        match r {
            Err(LockError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{e}")
            }
            other => panic!("a duplicate id must be rejected, got {other:?}"),
        }
        // And it must reject before taking anything, so the id is still free.
        s.lock(&sid, "aaa00000000000000000000000", &who)
            .expect("nothing should have been acquired");
    }

    #[test]
    fn lock_many_attempts_the_lowest_id_first() {
        // Stronger than checking the returned order, which a
        // "acquire in request order, then sort the results" refactor would
        // also satisfy — while reintroducing the livelock this exists to
        // prevent. The anchor keeps the last holder's stamp even after
        // release, so an anchor carrying nobody's stamp but its creator's is
        // proof that writer never acquired it.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        for id in ["aaa00000000000000000000000", "bbb00000000000000000000000"] {
            s.add_cassette(&sid, &cassette_meta(id), "").expect("add");
        }
        let holder = Attribution::for_now("writer-1", "joseph");
        let _held = s
            .lock(&sid, "aaa00000000000000000000000", &holder)
            .expect("hold the LOWEST id");

        // Request b first. Ascending order must try a, fail, and never reach b.
        let other = Attribution::for_now("writer-2", "agent");
        let r = s.lock_many(
            &sid,
            &["bbb00000000000000000000000", "aaa00000000000000000000000"],
            &other,
        );
        match r {
            Err(LockError::Busy { id, .. }) => assert_eq!(
                id, "aaa00000000000000000000000",
                "acquisition is ascending-id, so the blocked id is the LOWEST — not \
                 'bbb…', which was requested first but never reached"
            ),
            other => panic!("expected Busy, got {other:?}"),
        }

        let b_anchor =
            std::fs::read_to_string(s.locks_dir(&sid).join("bbb00000000000000000000000"))
                .expect("read b's anchor");
        assert!(
            !b_anchor.contains("writer-2"),
            "b must never have been acquired — acquisition did not start at the \
             lowest id: {b_anchor}"
        );
    }

    #[test]
    fn probe_reports_free_without_creating_or_stamping_the_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let anchor = dir.path().join("01K5GR7T2M9WPD0000000000AB");

        // A cassette nobody has ever locked has no anchor file. That is free,
        // and probing must not bring the file into existence.
        assert!(probe(&anchor).expect("probe"), "an absent anchor is free");
        assert!(
            !anchor.exists(),
            "probe must not create the anchor: it is a read"
        );
    }

    #[test]
    fn probe_reports_busy_while_the_lock_is_held() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        const ID: &str = "aaa00000000000000000000000";
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");

        let holder = Attribution::for_now("writer-1", "joseph");
        let _held = s.lock(&sid, ID, &holder).expect("hold it");

        assert!(
            !probe(&s.locks_dir(&sid).join(ID)).expect("probe"),
            "the lock is held, so probe must report busy"
        );
    }

    #[test]
    fn probe_does_not_stamp_the_anchor_it_finds_free() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        const ID: &str = "aaa00000000000000000000000";
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");

        // Acquire and release once, so the anchor exists and carries a real
        // holder's stamp — the state a cassette is normally found in.
        let holder = Attribution::for_now("writer-1", "joseph");
        drop(s.lock(&sid, ID, &holder).expect("acquire once"));
        let anchor_path = s.locks_dir(&sid).join(ID);
        let before = std::fs::read_to_string(&anchor_path).expect("read anchor");
        assert!(before.contains("writer-1"), "{before}");

        assert!(probe(&anchor_path).expect("probe"), "released, so free");

        let after = std::fs::read_to_string(&anchor_path).expect("read anchor");
        assert_eq!(
            before, after,
            "probe must not rewrite the anchor's attribution: it only reads"
        );
    }
}
