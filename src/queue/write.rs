//! `cassette queue write`: the one command that mutates a cassette body.
//!
//! Everything shared with the read-only commands in `view.rs` — `QueueError`,
//! `WriterSource`, `StatusFilter`, and the writer-error mappers — lives in
//! `queue/mod.rs` instead, so this module can stay the only place a lock is
//! ever acquired for writing.

use super::{require_error_to_queue_error, resolve_error_to_queue_error, QueueError, WriterSource};
use crate::store;

/// `cassette queue write <ID>`: acquire the cassette's lock, THEN read the body
/// from stdin, write, and release.
///
/// The ordering is deliberate and is what makes the concurrency tests
/// deterministic: a child spawned with an open stdin pipe is provably holding
/// the lock, with no sleeps and no polling, and closing the pipe releases it.
pub fn write(
    store: &store::Store,
    id: &str,
    session: &str,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
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
        WriterSource::Env => store
            .resolve_writer(who_name)
            .map_err(resolve_error_to_queue_error)?,
        WriterSource::Flag => store
            .require_writer(who_name)
            .map_err(require_error_to_queue_error)?,
    };
    let who = store::lock::Attribution::for_now(&writer, who_name);

    let guard = match store.lock(session, id, &who) {
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
