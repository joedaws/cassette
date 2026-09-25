//! The session store: session directories of per-cassette markdown files.
//!
//! Layout under the data dir (created `0700`):
//!
//! ```text
//! ~/.local/share/cassette/
//!   writers.toml
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
//! `writers` took a bare root path of its own, it created the store root
//! through `atomic_write` at the process umask, leaving a freewriting journal
//! world-readable until some later call happened to tighten it. One owner,
//! one place that creates the root.

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

/// The most open cassettes one session may hold, overridable by the
/// `max_open` config key.
///
/// Defined here and NOT taken from `app::MAX_CASSETTES`, which happens to be
/// the same number: that one is a TUI display concern (how many cassettes the
/// stack can show and select), and binding the store's cap to it would assert
/// a relationship the code does not have.
pub const MAX_OPEN: usize = 36;

/// Write via a temp file in the same directory, then `rename()` over the
/// target. Spec invariant 4: a reader either sees the old file whole or the
/// new one whole, never a half-written mix. Same directory matters — `rename`
/// is only atomic within a filesystem.
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // The temp name carries a ULID so two writers never collide on it.
    let tmp = dir.join(format!(".tmp-{}", ulid::Ulid::generate()));
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
/// Free-standing rather than a `Store` method because `writers` takes a bare
/// root path and would otherwise create the store root through `atomic_write`
/// at the process umask.
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
        // Tighten an existing directory that is looser than 0700 — a store
        // that is already world-readable holds private writing that stays
        // exposed until somebody notices, which is worse than the surprise
        // of narrowing it.
        //
        // But narrow ONLY the group and other bits: `set_mode(0o700)`
        // wholesale also strips setgid and the sticky bit, silently undoing
        // a deliberate `2770` on a shared directory on every single run.
        // Clearing `0o077` leaves everything outside the permission triad
        // alone, so 2770 becomes 2700 rather than 700.
        let perms = std::fs::metadata(dir)?.permissions();
        let mode = perms.mode();
        if mode & 0o077 != 0 {
            let mut tightened = perms;
            tightened.set_mode(mode & !0o077);
            std::fs::set_permissions(dir, tightened)?;
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// The result of reading a session's cassettes, including how many files
/// could not be read or parsed. The count is carried rather than logged: a
/// damaged cassette that vanishes from `queue list` is invisible work, and
/// the operator has no other view of the store.
#[derive(Debug, Default)]
pub struct SessionScan {
    pub cassettes: Vec<StoredCassette>,
    /// Files that exist but could not be loaded. Carried as a list rather
    /// than a count so the TUI can render one row per damaged file naming
    /// it: an operator told "1 unreadable" with no filename has nothing to
    /// act on.
    pub damaged: Vec<DamagedCassette>,
}

impl SessionScan {
    /// How many files could not be loaded. Derived from `damaged` rather
    /// than tracked beside it, so the count and the list cannot disagree —
    /// the two-records-of-one-fact shape this codebase has already paid for
    /// once in the TUI's read-only banner.
    pub fn unreadable(&self) -> usize {
        self.damaged.len()
    }
}

/// Why a cassette file could not be turned into a `StoredCassette`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReason {
    /// Could not be read at all: permissions, I/O, or invalid UTF-8.
    Unreadable,
    /// Read, but carries no parseable frontmatter block.
    BadFrontmatter,
}

impl std::fmt::Display for DamageReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DamageReason::Unreadable => write!(f, "could not be read"),
            DamageReason::BadFrontmatter => write!(f, "frontmatter is unparseable"),
        }
    }
}

/// One cassette file that exists but could not be loaded.
#[derive(Debug, Clone)]
pub struct DamagedCassette {
    pub path: PathBuf,
    /// The id from the filename stem, where there is one. The frontmatter is
    /// exactly what could not be trusted, so the name is all there is.
    pub id: Option<String>,
    pub reason: DamageReason,
}

impl DamagedCassette {
    /// What to show for this file: its id when the name yields one, else the
    /// bare filename.
    pub fn label(&self) -> String {
        self.id.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }
}

/// One cassette as it exists on disk.
#[derive(Debug, Clone)]
pub struct StoredCassette {
    pub meta: CassetteMeta,
    /// Everything after the frontmatter, byte-for-byte.
    pub body: String,
}

/// Why `Store::require_session` rejected a session id.
///
/// A malformed id or a well-formed one naming no session are both usage
/// errors (exit 2) — the caller typed something wrong. Anything else
/// `session_meta` can fail with — permissions, a corrupt `session.toml` — is
/// a store the caller cannot be blamed for and gets exit 1 instead,
/// distinguished by `kind() == NotFound`: `session::read` opens the file
/// with `std::fs::read_to_string` before it ever parses anything, so a
/// missing session directory *or* a missing `session.toml` both surface as
/// `NotFound`, and every other failure (permission denied, or a parse error
/// `session::read` maps to `InvalidData`) is something else.
#[derive(Debug)]
pub enum RequireSessionError {
    /// Malformed id, or a well-formed one naming no session. Exit 2.
    Usage(String),
    /// A session directory the store cannot read for some other reason
    /// (permissions, a corrupt `session.toml`). Exit 1: not a typo.
    Io(String),
}

impl std::fmt::Display for RequireSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequireSessionError::Usage(m) | RequireSessionError::Io(m) => write!(f, "{m}"),
        }
    }
}

/// `Store::list_sessions`' result: the sessions it could read, and how
/// many directories under `sessions/` it skipped because their names are
/// not `ses_` ids.
#[derive(Debug)]
pub struct SessionListing {
    pub sessions: Vec<(String, session::SessionMeta)>,
    pub skipped: usize,
}

/// The trailing line every listing prints when `list_sessions` skipped
/// directories whose names are not session ids — work on disk must not
/// vanish from every view silently, the same rule as `N unreadable`.
pub(crate) fn skipped_line(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some("1 session directory skipped: names are not ses_ ids".to_string()),
        n => Some(format!(
            "{n} session directories skipped: names are not ses_ ids"
        )),
    }
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
        let id = ids::new(ids::IdKind::Session);
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
            Some(session),
        )
        .map_err(io::Error::from)?;
        guard.write(m, body)?;
        Ok(path)
    }

    /// Resolve a caller-supplied session id to a session that actually
    /// exists, on the two axes a `--session` argument can be wrong on.
    ///
    /// **Shape.** The id is joined straight onto the store root by
    /// `session_dir`, so `ids::check` is what stands between a
    /// `--session ../../escaped` and a cassette written outside the store.
    /// The spec's "sessions are named by ULID only" is asserted in half a
    /// dozen doc comments; this is where it is enforced.
    ///
    /// **Existence.** A well-formed id for a session nobody created is a
    /// typo, never an implicit create: `create_session` is the only code
    /// that builds a session directory, and letting a command materialize
    /// one by side effect produced cassettes in a directory `session list`
    /// could never show, since it has no `session.toml` to list. So the
    /// session directory must be there *and* carry a readable
    /// `session.toml`; anything else is reported as the missing session it
    /// is.
    ///
    /// **I/O failures are not typos.** A `session.toml` the store cannot
    /// read — wrong permissions, a corrupt file — is a different situation
    /// from a session nobody created: `--json` puts the exit code in a
    /// field an agent branches on, and an agent that sees exit 2 will "fix"
    /// its argument and retry forever, where exit 1 tells it to escalate
    /// instead. See `RequireSessionError`.
    pub fn require_session(&self, session: &str) -> Result<(), RequireSessionError> {
        if let Err(e) = ids::check(ids::IdKind::Session, session) {
            return Err(RequireSessionError::Usage(e.message(session, "--session")));
        }
        match self.session_meta(session) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(RequireSessionError::Usage(
                format!("no session '{session}' — `cassette session list` shows what exists"),
            )),
            Err(e) => Err(RequireSessionError::Io(format!(
                "cannot read session '{session}': {e}"
            ))),
        }
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
    /// Deliberately private. `ensure_writer` and `resolve_writer` are its only
    /// callers, so blocking acquisition cannot leak into a code path where a
    /// human could hold it.
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
            // Not session-scoped, so nothing for `Store::holds` to bookkeep.
            None,
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
    pub fn ensure_writer(
        &self,
        name: &str,
        kind: writers::Kind,
    ) -> Result<String, writers::EnsureError> {
        // `_registry`, NOT `_`: a bare underscore drops the guard immediately
        // and silently reopens the lost-update race this whole function exists
        // to close. No test catches the difference — the critical section is
        // too fast to lose reliably — so this comment is the pin.
        let _registry = self.lock_registry()?;
        writers::ensure(&self.root, name, kind)
    }

    /// The id and kind for `name`, registering as a human on first sight.
    /// Only for a name allowed to bootstrap itself — today, the `$USER`
    /// default. See `writers::resolve` and, for a name that must already be
    /// registered (an explicit `--writer`), `require_writer`.
    pub fn resolve_writer(
        &self,
        name: &str,
    ) -> Result<(String, writers::Kind), writers::ResolveError> {
        // `_registry`, NOT `_`: see `ensure_writer`. Same read-modify-write,
        // same race if the guard drops early.
        let _registry = self.lock_registry()?;
        writers::resolve(&self.root, name)
    }

    /// The id and kind for `name`, requiring that it already be registered —
    /// no auto-create. For a name a caller named explicitly (`--writer`),
    /// where an unknown name is a typo (usage error) rather than a first run.
    /// See `writers::require_registered`.
    ///
    /// Read-only, so unlike `ensure_writer`/`resolve_writer` this does not
    /// take the registry lock: there is no write for a concurrent registration
    /// to race.
    pub fn require_writer(
        &self,
        name: &str,
    ) -> Result<(String, writers::Kind), writers::RequireError> {
        writers::require_registered(&self.root, name)
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
        lock::acquire(
            id,
            path,
            &anchor_path,
            as_writer,
            lock::Blocking::No,
            Some(session),
        )
    }

    /// Whether *this process* currently holds `id`'s lock in `session` — a
    /// live `LockGuard` it has not yet dropped.
    ///
    /// Answered from in-process bookkeeping (`store::lock`'s `HELD` set),
    /// never by probing the anchor: `flock` cannot tell our own lock apart
    /// from another process's, so a second `try_lock` on an anchor we
    /// already hold reports `Busy` exactly as it would for a stranger — that
    /// would only answer "is this locked", which is trivially always true
    /// while we hold it, not "do *we* hold it", which is what callers like
    /// the TUI need in order to skip re-acquiring a lock they already have.
    pub fn holds(&self, session: &str, id: &str) -> bool {
        lock::is_held(session, id)
    }

    /// Whether `id`'s lock is free right now. A snapshot, not a claim: the
    /// lock may be taken the instant after this returns, and a caller that
    /// needs to act on the answer (`queue write`) still has to race for it
    /// with a real `lock`. Never stamps or creates the anchor — see
    /// `lock::probe`.
    pub fn is_free(&self, session: &str, id: &str) -> io::Result<bool> {
        lock::probe(&self.locks_dir(session).join(id))
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

    /// Every cassette in a session, in queue order, plus a count of files
    /// that could not be read or parsed. A missing session directory scans as
    /// empty; a file that is not `.md`, or a `.md` file without parseable
    /// frontmatter, increments `unreadable` rather than erroring outright —
    /// the store shares a directory with editors and their swap files, but a
    /// cassette that once had valid frontmatter and lost it is exactly the
    /// kind of damage `queue list` exists to surface (see `SessionScan`).
    pub fn scan_session(&self, session: &str) -> io::Result<SessionScan> {
        let dir = self.cassettes_dir(session);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(SessionScan::default()),
            Err(e) => return Err(e),
        };
        let mut found = Vec::new();
        let mut damaged: Vec<DamagedCassette> = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(ids::id_from_file_name)
                .map(str::to_string);
            let Ok(content) = std::fs::read_to_string(&path) else {
                damaged.push(DamagedCassette {
                    path,
                    id: stem,
                    reason: DamageReason::Unreadable,
                });
                continue;
            };
            let (Some(meta), body) = meta::split(&content) else {
                damaged.push(DamagedCassette {
                    path,
                    id: stem,
                    reason: DamageReason::BadFrontmatter,
                });
                continue;
            };
            // The file name and the frontmatter must name the same cassette.
            // Drift between them — a hand-edited or hand-migrated file — would
            // otherwise make one file two identities: listed under one id,
            // locked and written under the other.
            if stem.as_deref() != Some(meta.id.as_str()) {
                damaged.push(DamagedCassette {
                    path,
                    id: stem,
                    reason: DamageReason::BadFrontmatter,
                });
                continue;
            }
            found.push(StoredCassette {
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
        // Stable order so a damaged row does not jump between renders.
        damaged.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(SessionScan {
            cassettes: found,
            damaged,
        })
    }

    /// Every session, newest first by `created` (ties broken by id,
    /// descending, so the order is total and deterministic). A session
    /// directory whose `session.toml` is missing or unparseable is skipped:
    /// `session list` is a listing, not a repair tool, and one damaged
    /// session must not hide the rest.
    pub fn list_sessions(&self) -> io::Result<SessionListing> {
        let dir = self.sessions_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(SessionListing {
                    sessions: Vec::new(),
                    skipped: 0,
                })
            }
            Err(e) => return Err(e),
        };
        let mut found = Vec::new();
        let mut skipped = 0;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if ids::check(ids::IdKind::Session, id).is_err() {
                skipped += 1;
                continue;
            }
            let Ok(meta) = session::read(&path.join("session.toml")) else {
                continue;
            };
            found.push((id.to_string(), meta));
        }
        found
            .sort_by(|(id_a, a), (id_b, b)| b.created.cmp(&a.created).then_with(|| id_b.cmp(id_a)));
        Ok(SessionListing {
            sessions: found,
            skipped,
        })
    }

    /// Set a session's display alias. The alias never resolves — it is shown
    /// in `session list` and nowhere else — so no uniqueness check applies.
    pub fn set_session_alias(&self, session: &str, alias: &str) -> io::Result<()> {
        let path = self.session_dir(session).join("session.toml");
        let mut meta = session::read(&path)?;
        meta.alias = Some(alias.to_string());
        session::write(&path, &meta)
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
            created_by: crate::store::ids::TEST_WRITER.to_string(),
            last_writer: crate::store::ids::TEST_WRITER.to_string(),
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

    /// A damaged cassette must be nameable, not merely countable: an
    /// operator told "1 unreadable" with no filename has nothing to act on.
    /// The count stays exact by being derived from the list, not kept beside
    /// it — the two-records-of-one-fact shape this codebase already paid for.
    #[test]
    fn scan_session_names_damaged_cassettes_and_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let session = store.create_session(&session_meta()).expect("session");
        std::fs::write(
            store
                .cassettes_dir(&session)
                .join("morning-cas_01M38000000000000000000BAD.md"),
            "no frontmatter here at all\n",
        )
        .expect("write");

        let scan = store.scan_session(&session).expect("scan");

        assert_eq!(scan.unreadable(), 1);
        assert_eq!(scan.damaged.len(), 1, "the count is derived from the list");
        assert_eq!(
            scan.damaged[0].id.as_deref(),
            Some("cas_01M38000000000000000000BAD"),
            "recovered from the filename, since the frontmatter is what failed"
        );
        assert_eq!(scan.damaged[0].reason, DamageReason::BadFrontmatter);
        assert_eq!(scan.damaged[0].label(), "cas_01M38000000000000000000BAD");
        assert!(
            scan.cassettes.is_empty(),
            "and it is not served as a cassette"
        );
    }

    #[test]
    fn a_cassette_whose_frontmatter_id_disagrees_with_its_file_name_is_damaged() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        std::fs::write(
            s.cassettes_dir(&sid).join("x-cas_01K5GR7T2M9WPD0000000000AB.md"),
            "---\nid: cas_01K5GR7T2M9WPD0000000000ZZ\ntopic: x\npriority: 10\nstatus: open\n\
             locked_by:\ncreated_by: wri_01K5GQ00000000000000000001\n\
             last_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-24T09:00:00Z\n---\n\n",
        )
        .expect("write");
        let scan = s.scan_session(&sid).expect("scan");
        assert!(
            scan.cassettes.is_empty(),
            "one file must not become a second identity"
        );
        assert_eq!(scan.damaged.len(), 1);
        assert_eq!(scan.damaged[0].reason, DamageReason::BadFrontmatter);
        assert_eq!(
            scan.damaged[0].id.as_deref(),
            Some("cas_01K5GR7T2M9WPD0000000000AB")
        );
    }

    #[test]
    fn a_bare_ulid_frontmatter_id_is_damaged() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        std::fs::write(
            s.cassettes_dir(&sid).join("x-cas_01K5GR7T2M9WPD0000000000AB.md"),
            "---\nid: 01K5GR7T2M9WPD0000000000AB\ntopic: x\npriority: 10\nstatus: open\n\
             locked_by:\ncreated_by: wri_01K5GQ00000000000000000001\n\
             last_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-24T09:00:00Z\n---\n\n",
        )
        .expect("write");
        let scan = s.scan_session(&sid).expect("scan");
        assert!(scan.cassettes.is_empty());
        assert_eq!(scan.damaged.len(), 1);
        assert_eq!(scan.damaged[0].reason, DamageReason::BadFrontmatter);
    }

    #[test]
    fn directories_that_are_not_session_ids_are_skipped_and_counted() {
        let (_d, s) = store();
        let real = s.create_session(&session_meta()).expect("session");
        for name in [
            "01K5GQ2R8VXM3T0000000000AB",
            "cas_01K5GQ2R8VXM3T0000000000AB",
        ] {
            let dir = s.sessions_dir().join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(
                dir.join("session.toml"),
                "created = \"2026-09-24T09:00:00Z\"\n",
            )
            .expect("toml");
        }
        let listing = s.list_sessions().expect("list");
        assert_eq!(listing.sessions.len(), 1);
        assert_eq!(listing.sessions[0].0, real);
        assert_eq!(listing.skipped, 2);
    }

    #[test]
    fn the_skipped_line_is_singular_plural_and_absent_at_zero() {
        assert_eq!(skipped_line(0), None);
        assert_eq!(
            skipped_line(1).as_deref(),
            Some("1 session directory skipped: names are not ses_ ids")
        );
        assert_eq!(
            skipped_line(2).as_deref(),
            Some("2 session directories skipped: names are not ses_ ids")
        );
    }

    #[test]
    fn create_session_builds_the_directory_layout() {
        let (_dir, s) = store();
        let id = s.create_session(&session_meta()).expect("create");
        assert_eq!(id.len(), 30);
        assert!(id.starts_with("ses_"), "{id}");
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
    fn list_sessions_is_newest_first() {
        let (_dir, s) = store();
        let older = s
            .create_session(&SessionMeta {
                created: "2026-09-14T09:00:00Z".to_string(),
                ..session_meta()
            })
            .expect("create");
        let newer = s
            .create_session(&SessionMeta {
                created: "2026-09-15T09:00:00Z".to_string(),
                ..session_meta()
            })
            .expect("create");
        let rows = s.list_sessions().expect("list").sessions;
        let ids: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec![newer.as_str(), older.as_str()]);
    }

    #[test]
    fn listing_sessions_with_none_yet_is_empty_not_an_error() {
        let (_dir, s) = store();
        assert!(s.list_sessions().expect("list").sessions.is_empty());
    }

    #[test]
    fn a_session_with_unparseable_metadata_is_skipped_not_fatal() {
        let (_dir, s) = store();
        let good = s.create_session(&session_meta()).expect("create");
        std::fs::create_dir_all(s.session_dir("broken")).expect("mkdir");
        std::fs::write(s.session_dir("broken").join("session.toml"), "not toml").expect("write");
        let rows = s.list_sessions().expect("list").sessions;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, good);
    }

    #[test]
    fn require_session_accepts_a_session_that_exists() {
        let (_dir, s) = store();
        let id = s.create_session(&session_meta()).expect("create");
        assert!(s.require_session(&id).is_ok());
    }

    #[test]
    fn require_session_rejects_a_traversal_without_touching_the_disk() {
        let (dir, s) = store();
        for bad in ["..", "../../escaped", "a/b", "", "nope"] {
            let err = s.require_session(bad).expect_err("must be rejected");
            match err {
                RequireSessionError::Usage(msg) => assert!(
                    msg.contains("malformed session id"),
                    "shape failure must say so, not 'no session': {msg}"
                ),
                RequireSessionError::Io(msg) => {
                    panic!("a shape failure is a usage error, not I/O: {msg}")
                }
            }
        }
        // Nothing was created anywhere on the way out — in particular not
        // the `escaped/` directory the unvalidated path join produced.
        assert!(!dir.path().join("escaped").exists());
        assert!(!dir.path().parent().unwrap().join("escaped").exists());
    }

    #[test]
    fn require_session_rejects_a_well_formed_id_that_names_no_session() {
        // The phantom-session case: shape alone cannot be the whole check,
        // or a typo'd but well-formed id creates an unreachable session.
        let (_dir, s) = store();
        let ghost = ids::new(ids::IdKind::Session);
        let err = s.require_session(&ghost).expect_err("must be rejected");
        match err {
            RequireSessionError::Usage(msg) => {
                assert!(msg.contains("no session"), "{msg}");
                assert!(msg.contains(&ghost), "{msg}");
            }
            RequireSessionError::Io(msg) => panic!("a missing session is exit 2, not I/O: {msg}"),
        }
        assert!(!s.session_dir(&ghost).exists(), "must not create it");
    }

    #[test]
    fn require_session_rejects_a_directory_with_no_session_toml() {
        // A bare directory under `sessions/` — what an unvalidated
        // `--session <fresh ulid>` used to leave behind — is not a session:
        // `session list` skips it, so accepting it would hand cassettes to a
        // place nothing can ever list.
        let (_dir, s) = store();
        let ghost = ids::new(ids::IdKind::Session);
        std::fs::create_dir_all(s.cassettes_dir(&ghost)).expect("mkdir");
        let err = s.require_session(&ghost).expect_err("must be rejected");
        match err {
            RequireSessionError::Usage(msg) => assert!(msg.contains("no session"), "{msg}"),
            RequireSessionError::Io(msg) => {
                panic!("a missing session.toml is exit 2, not I/O: {msg}")
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_session_toml_is_an_io_error_not_a_missing_session() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let toml = s.session_dir(&sid).join("session.toml");

        let mut perms = std::fs::metadata(&toml).expect("metadata").permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&toml, perms).expect("chmod");

        // Root ignores the mode bits, so the chmod proves nothing there.
        // Probe the actual effect rather than guessing from $USER, which can
        // be unset or lie in a container and does not track effective uid
        // anyway.
        if std::fs::read_to_string(&toml).is_ok() {
            return; // running with privileges that defeat the test's premise
        }

        let err = s
            .require_session(&sid)
            .expect_err("must not read as present");
        match err {
            RequireSessionError::Io(msg) => assert!(
                !msg.contains("no session"),
                "an unreadable store is an I/O failure, not a typo: {msg}"
            ),
            RequireSessionError::Usage(msg) => {
                panic!("an unreadable session.toml must be exit 1, not exit 2: {msg}")
            }
        }
    }

    #[test]
    fn set_session_alias_updates_it_in_place() {
        let (_dir, s) = store();
        let id = s.create_session(&session_meta()).expect("create");
        s.set_session_alias(&id, "monday").expect("set alias");
        let read_back = s.session_meta(&id).expect("read");
        assert_eq!(read_back.alias.as_deref(), Some("monday"));
    }

    #[test]
    fn set_session_alias_on_an_unknown_id_is_not_found() {
        let (_dir, s) = store();
        let err = s.set_session_alias("nope", "x").expect_err("must fail");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn add_cassette_names_the_file_by_slug_and_id() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("cas_01K5GR7T2M9WPD0000000000AB", 10);
        let path = s
            .add_cassette(&sid, &m, "## Side A\n\nhello\n")
            .expect("add");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "gratitude-cas_01K5GR7T2M9WPD0000000000AB.md"
        );
    }

    #[test]
    fn a_cassette_round_trips_through_the_store() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("cas_01K5GR7T2M9WPD0000000000AB", 10);
        let body = "## Side A\n\nhello\n\n## Side B\n\nscratch\n";
        s.add_cassette(&sid, &m, body).expect("add");

        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.cassettes.len(), 1);
        assert_eq!(found.cassettes[0].meta, m);
        assert_eq!(
            found.cassettes[0].body, body,
            "body must survive byte-for-byte"
        );
    }

    #[test]
    fn scan_returns_cassettes_in_queue_order() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut closed = cassette_meta("cas_aaa00000000000000000000000", 5);
        closed.status = Status::Closed;
        s.add_cassette(
            &sid,
            &cassette_meta("cas_ccc00000000000000000000000", 30),
            "",
        )
        .expect("add");
        s.add_cassette(&sid, &closed, "").expect("add");
        s.add_cassette(
            &sid,
            &cassette_meta("cas_bbb00000000000000000000000", 10),
            "",
        )
        .expect("add");

        let ids: Vec<String> = s
            .scan_session(&sid)
            .expect("scan")
            .cassettes
            .into_iter()
            .map(|c| c.meta.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "cas_bbb00000000000000000000000",
                "cas_ccc00000000000000000000000",
                "cas_aaa00000000000000000000000"
            ],
            "open by priority, closed last — even with the best priority"
        );
    }

    #[test]
    fn scan_ignores_non_cassette_files() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        s.add_cassette(
            &sid,
            &cassette_meta("cas_aaa00000000000000000000000", 10),
            "",
        )
        .expect("add");
        // An editor swap file and a file with no frontmatter must not appear.
        std::fs::write(s.cassettes_dir(&sid).join("notes.txt"), "stray").expect("write");
        std::fs::write(s.cassettes_dir(&sid).join("broken.md"), "no frontmatter\n").expect("write");
        assert_eq!(s.scan_session(&sid).expect("scan").cassettes.len(), 1);
    }

    #[test]
    fn scanning_a_missing_session_is_empty_not_an_error() {
        let (_dir, s) = store();
        assert!(s.scan_session("nope").expect("scan").cassettes.is_empty());
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
        let m = cassette_meta("cas_01K5GR7T2M9WPD0000000000AB", 10);
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
        let mut m = cassette_meta("cas_01K5GR7T2M9WPD0000000000AB", 10);
        let path = s.add_cassette(&sid, &m, "old\n").expect("add");

        m.topic = Some("completely different".to_string());
        let who = lock::Attribution::for_now("writer-1", "joseph");
        let guard = s.lock(&sid, &m.id, &who).expect("acquire");
        guard.write(&m, "new\n").expect("write");
        drop(guard);

        assert!(path.is_file(), "the file must not have been renamed");
        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.cassettes.len(), 1);
        assert_eq!(
            found.cassettes[0].meta.topic.as_deref(),
            Some("completely different")
        );
        assert_eq!(found.cassettes[0].body, "new\n");
        assert_eq!(
            s.cassette_path(&sid, &m.id)
                .expect("io")
                .expect("exists")
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "gratitude-cas_01K5GR7T2M9WPD0000000000AB.md",
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

    /// Tightening must not strip setgid. A `2770` shared directory losing
    /// its setgid bit on every run is a side effect nobody asked for, and it
    /// would silently undo a deliberate group-sharing setup.
    #[cfg(unix)]
    #[test]
    fn tightening_preserves_bits_outside_the_permission_triad() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("shared");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o2770)).expect("chmod");

        ensure_private_dir(&root).expect("ensure");

        let mode = std::fs::metadata(&root).expect("root").permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "group and other are cleared");
        assert_eq!(mode & 0o2000, 0o2000, "but setgid survives: {mode:o}");
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
        assert!(writers::lookup_by_name(&all, "joseph").is_some());
        assert!(writers::lookup_by_name(&all, "agent").is_some());
    }

    #[test]
    fn holds_is_true_only_while_this_process_guards_the_cassette() {
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        const ID: &str = "cas_aaa00000000000000000000000";
        s.add_cassette(&sid, &cassette_meta(ID, 10), "")
            .expect("add");

        assert!(!s.holds(&sid, ID), "nothing held yet");
        {
            let who = lock::Attribution::for_now("writer-1", "joseph");
            let _guard = s.lock(&sid, ID, &who).expect("acquire");
            assert!(s.holds(&sid, ID), "we are holding it now");
            assert!(
                !s.holds(&sid, "cas_bbb00000000000000000000000"),
                "a different id"
            );
        }
        assert!(
            !s.holds(&sid, ID),
            "the guard dropped, so we no longer hold it"
        );
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
