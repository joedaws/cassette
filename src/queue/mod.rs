//! The `cassette queue` commands.
//!
//! Split on the lock boundary: `view` holds the read-only commands (`list`,
//! `show`, `next`) and never touches `LockGuard::write`, while `write` holds
//! the one command that does. `LockGuard::write` (see `store::lock`) is the
//! only code in the crate that writes a cassette file, and that invariant is
//! meant to be checkable by reading `view.rs` alone — no locks *held*, no
//! writes, full stop. `next` probes locks (`Store::is_free`) to skip busy
//! cassettes, but a probe acquires and releases within one call and never
//! constructs a `LockGuard`, so it never holds anything past a return.
//!
//! `pub use write::write` keeps `main.rs`'s existing `queue::write(...)` call
//! site unchanged even though `write` now names both a module and a function;
//! those live in separate namespaces, so this is ordinary Rust, not a
//! collision to design around.

pub mod view;
pub mod write;

pub use write::write;

use crate::store::writers;

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
    /// Anything else. Exit 1.
    Io(String),
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
