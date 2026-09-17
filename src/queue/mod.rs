//! The `cassette queue` commands.
//!
//! Split on the lock boundary: `view` holds the read-only commands (`list`,
//! `show`, `next`) and never touches `LockGuard::write`, while `write` and
//! `edit` hold the commands that do — `write` replaces a cassette's body,
//! `edit` creates cassettes and renumbers a session's priorities.
//! `LockGuard::write` (see `store::lock`) is the only code in the crate that
//! writes a cassette file, and that invariant is meant to be checkable by
//! reading `view.rs` alone — no locks *held*, no writes, full stop. `next`
//! probes locks (`Store::is_free`) to skip busy cassettes, but a probe
//! acquires and releases within one call and never constructs a `LockGuard`,
//! so it never holds anything past a return.
//!
//! `pub use write::write` keeps `main.rs`'s existing `queue::write(...)` call
//! site unchanged even though `write` now names both a module and a function;
//! those live in separate namespaces, so this is ordinary Rust, not a
//! collision to design around.

pub mod edit;
pub mod json;
pub mod view;
pub mod write;

pub use edit::Placement;
pub use write::write;

use crate::store::{writers, Store};

/// The gate every `queue` command passes through before it touches the
/// store: `--session` names a well-formed ULID, and that session exists.
///
/// One helper rather than a check per command, and called from `main.rs`
/// against `cli::QueueCmd::session()` rather than from inside each command,
/// so the ninth queue command cannot forget it — `session()`'s match is
/// exhaustive, so a new variant fails to compile until it says where its
/// session id lives.
///
/// A malformed or unknown id is `Usage` (exit 2), which is the spec's exit
/// table: "unknown session" is listed there as 2, raised by any command. It
/// has to be checked up front, because the store reads the argument
/// permissively — `scan_session` maps a missing directory to an empty scan,
/// so without this `queue list --session <typo>` would print `no cassettes`
/// and exit 0, and `queue next` would exit 5, telling an agent loop "idle or
/// enqueue work" when the truth is a typo. A session directory the store
/// otherwise cannot read (permissions, a corrupt `session.toml`) is `Io`
/// (exit 1) instead — see `store::RequireSessionError`.
///
/// `session alias` calls `Store::require_session` too, so the two paths
/// cannot drift apart on what a session id is.
pub fn require_session(store: &Store, session: &str) -> Result<(), QueueError> {
    match store.require_session(session) {
        Ok(()) => Ok(()),
        Err(crate::store::RequireSessionError::Usage(m)) => Err(QueueError::Usage(m)),
        Err(crate::store::RequireSessionError::Io(m)) => Err(QueueError::Io(m)),
    }
}

/// Where a writer name came from. The distinction is load-bearing: an
/// unknown `$USER` is bootstrapped on first run, which the spec blesses,
/// while an unknown `--writer` is a typo and must fail loudly rather than
/// silently spawning a second identity — one auto-created as human, the
/// *privileged* kind, would fail open on exactly that typo.
///
/// Every queue command that resolves a writer takes this alongside the name,
/// so every command that needs an identity picks it the same way `write`
/// does here rather than each re-deriving it. `list` and `show` need no
/// identity at all — they attribute nothing — so they never touch this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterSource {
    /// Explicit `--writer`. An unknown name here is a usage error.
    Flag,
    /// Derived from `$USER`. An unknown name here is a first run.
    Env,
}

/// Why a queue command failed, in the shape `main.rs` maps to an exit code.
#[derive(Debug)]
pub enum QueueError {
    /// Bad invocation, unknown session, unknown cassette, unregistered
    /// writer, or a kind mismatch. Exit 2.
    Usage(String),
    /// Another writer holds the cassette. Exit 3. Carries the rendered
    /// message rather than `{ id, holder }`, unlike `LockError::Busy` and
    /// `writers::EnsureError::KindMismatch` — a deliberate divergence, not an
    /// oversight: the sole caller needs only the text, and the holder
    /// formatting belongs beside the code that produces it. A `queue move`
    /// in 4b can render its own message while it still has the id. 4c's
    /// `--json` is what will likely need the fields back, since it emits
    /// `id` and `holder` raw rather than prose.
    Busy(String),
    /// `queue next` found no open cassettes at all — a different situation
    /// from `Busy`, which means open cassettes exist but every one is
    /// currently locked. `Busy` says "wait and retry"; `Empty` says "there is
    /// nothing to wait for — the caller should sit idle or enqueue work".
    /// Exit 5, distinct from `Busy`'s exit 3 for exactly that reason.
    Empty(String),
    /// `queue new` found the session already holding `max_open` open
    /// cassettes. Exit 6, distinct from every other variant: it is neither a
    /// bad invocation, a lock fight, nor an empty queue — the queue is
    /// (over)full. Checked before any priority is computed, so a full queue
    /// is never charged a renumber.
    Full(String),
    /// `queue close` or `queue write` found the cassette's `locked_by` set
    /// and the acting writer is an agent; also `queue lock` finding the
    /// cassette already claimed by a different writer. Exit 4, distinct from
    /// `Busy`'s exit 3: `Busy` means another writer is actively holding the
    /// advisory `flock` right now (nobody, human or agent, may act on it);
    /// `Sticky` means nobody is writing it at this instant but a writer has
    /// claimed it via the sticky `locked_by` frontmatter field, and only a
    /// human may close or write over that claim (or, for `lock`, only a human
    /// may clear someone else's claim — see `queue unlock`).
    Sticky(String),
    /// Anything else. Exit 1.
    Io(String),
}

/// The process exit code this failure deserves. The single source of truth
/// for the spec's exit table: `main.rs` renders, this decides.
///
/// Kept here rather than in `main.rs` because `--json` puts the number in a
/// machine-readable field — a caller branches on it — so it is contract, not
/// presentation.
pub fn exit_code(e: &QueueError) -> i32 {
    match e {
        QueueError::Io(_) => 1,
        QueueError::Usage(_) => 2,
        QueueError::Busy(_) => 3,
        QueueError::Sticky(_) => 4,
        QueueError::Empty(_) => 5,
        QueueError::Full(_) => 6,
    }
}

/// The human-readable message, whatever the variant.
pub fn message(e: &QueueError) -> &str {
    match e {
        QueueError::Io(m)
        | QueueError::Usage(m)
        | QueueError::Busy(m)
        | QueueError::Sticky(m)
        | QueueError::Empty(m)
        | QueueError::Full(m) => m,
    }
}

/// Which cassettes `queue list` shows. This module takes no dependency on
/// the command-line parsing crate, same as the rest of `queue` and `store` —
/// `cli.rs` is the only place the command line is read, so its
/// command-line-facing counterpart is `cli::StatusArg`, converted via `From`
/// the same way `cli::WriterKindArg` converts into `store::writers::Kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    Open,
    Closed,
    All,
}

/// Render a `resolve_writer` failure as the exit code it deserves.
/// `resolve` declares no kind and always auto-creates, so `EmptyName` is its
/// only usage error (2) and everything else is `Io` (1).
fn resolve_error_to_queue_error(e: writers::ResolveError) -> QueueError {
    match e {
        writers::ResolveError::EmptyName => QueueError::Usage(e.to_string()),
        writers::ResolveError::Io(io_e) => QueueError::Io(format!("cannot resolve writer: {io_e}")),
    }
}

/// Render a `require_writer` failure as the exit code it deserves.
/// `EmptyName` and `Unregistered` are usage errors (2): both are about what
/// the caller asked for, not a system failure.
fn require_error_to_queue_error(e: writers::RequireError) -> QueueError {
    match e {
        writers::RequireError::EmptyName => QueueError::Usage(e.to_string()),
        writers::RequireError::Unregistered(_) => QueueError::Usage(e.to_string()),
        writers::RequireError::Io(io_e) => QueueError::Io(format!("cannot resolve writer: {io_e}")),
    }
}
