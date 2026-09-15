//! The `cassette queue` commands. Only `write` exists in 4a; 4b adds `list`,
//! `next`, `new`, `show`, `close`, `reopen`, `move`.

use crate::store;
use crate::store::writers::WriterError;

/// Where a writer name came from. The distinction is load-bearing: an
/// unknown `$USER` is bootstrapped on first run, which the spec blesses,
/// while an unknown `--writer` is a typo and must fail loudly rather than
/// silently spawning a second identity — one auto-created as human, the
/// *privileged* kind, would fail open on exactly that typo.
///
/// Every queue command that resolves a writer takes this alongside the name,
/// so the seven commands 4b adds all pick the same way `write` does here
/// rather than each re-deriving it.
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
    /// `WriterError::KindMismatch` — a deliberate divergence, not an
    /// oversight: the sole caller needs only the text, and the holder
    /// formatting belongs beside the code that produces it. A `queue move`
    /// in 4b can render its own message while it still has the id. 4c's
    /// `--json` is what will likely need the fields back, since it emits
    /// `id` and `holder` raw rather than prose.
    Busy(String),
    /// Anything else. Exit 1.
    Io(String),
}

/// `cassette queue write <ID>`: acquire the cassette's lock, THEN read the body
/// from stdin, write, and release.
///
/// The ordering is deliberate and is what makes the concurrency tests
/// deterministic: a child spawned with an open stdin pipe is provably holding
/// the lock, with no sleeps and no polling, and closing the pipe releases it.
pub fn write(
    store: &store::Store,
    id: &str,
    session: Option<&str>,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    let session = match session
        .map(str::to_string)
        .map_or_else(|| store.active_session(), |s| Ok(Some(s)))
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Err(QueueError::Usage(
                "no active session; pass --session".into(),
            ))
        }
        Err(e) => {
            return Err(QueueError::Io(format!(
                "cannot read the active session: {e}"
            )))
        }
    };

    // Registration/lookup happens BEFORE acquisition, even though failing
    // fast on a bad lock looks more logical: two writers racing for the same
    // cassette must both land in writers.toml (or both fail cleanly) even
    // when one of them loses the lock.
    //
    // Which of `resolve_writer`/`require_writer` runs depends on where the
    // name came from, not on whether it happens to be new: an unknown
    // `$USER` (`WriterSource::Env`) is a first run (the spec's
    // "auto-registered from $USER on first run") and is created as human, but
    // an unknown `--writer` (`WriterSource::Flag`) is a typo and must fail
    // loudly — exit 2 via `QueueError::Usage` — rather than silently
    // spawning a second identity as human, the *privileged* kind. Only
    // `writer register` declares a kind; both paths here defer to whatever
    // is already on record for a name that already exists.
    //
    // `_kind` is unused today: nothing here needs to tell a human from an
    // agent yet. 4b's `queue close` is where it starts to matter (an agent
    // refuses to close a cassette whose `locked_by` is set; a human may), so
    // this is where that lookup will plug in rather than a second lookup.
    let (writer, _kind) = match source {
        WriterSource::Env => store.resolve_writer(who_name),
        WriterSource::Flag => store.require_writer(who_name),
    }
    .map_err(writer_error_to_queue_error)?;
    let who = store::lock::Attribution::for_now(&writer, who_name);

    let guard = match store.lock(&session, id, &who) {
        Ok(g) => g,
        Err(store::lock::LockError::Busy { holder, .. }) => {
            let who = holder
                .map(|a| format!("{} (since {})", a.name, a.since))
                .unwrap_or_else(|| "another writer".to_string());
            return Err(QueueError::Busy(format!(
                "'{id}' is open by {who} — try again later"
            )));
        }
        // A distinct variant from `LockError::Io`, so this is a usage error
        // (2) by construction rather than by matching on `io::ErrorKind` and
        // hoping `acquire` never surfaces `NotFound` for another reason.
        Err(store::lock::LockError::NoSuchCassette { .. }) => {
            return Err(QueueError::Usage(format!(
                "no cassette '{id}' in session '{session}'"
            )))
        }
        Err(store::lock::LockError::Io(e)) => {
            return Err(QueueError::Io(format!("cannot lock '{id}': {e}")))
        }
    };

    // Lock first, stdin second. Reversing these would make the tests racy.
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
        return Err(QueueError::Io(format!("cannot read stdin: {e}")));
    }

    let current = match guard.read() {
        Ok(c) => c,
        Err(e) => return Err(QueueError::Io(format!("cannot read '{id}': {e}"))),
    };
    let mut m = current.meta;
    m.last_writer = writer;
    m.updated_at = store::meta::now_utc();
    if let Err(e) = guard.write(&m, &body) {
        return Err(QueueError::Io(format!("cannot write '{id}': {e}")));
    }
    Ok(())
}

/// Render a writer-resolution failure as the exit code it deserves.
/// `EmptyName` and `Unregistered` are usage errors (2): both are about what
/// the caller asked for, not a system failure. `KindMismatch` cannot actually
/// reach here — neither `resolve_writer` nor `require_writer` declares a
/// kind — but the arm stays so this match stays exhaustive as `WriterError`
/// grows, rather than by a wildcard that would silently swallow a real new
/// variant into `Io`.
fn writer_error_to_queue_error(e: WriterError) -> QueueError {
    match e {
        WriterError::EmptyName => QueueError::Usage(e.to_string()),
        WriterError::Unregistered(_) => QueueError::Usage(e.to_string()),
        WriterError::KindMismatch { .. } => QueueError::Usage(e.to_string()),
        WriterError::Io(io_e) => QueueError::Io(format!("cannot resolve writer: {io_e}")),
    }
}
