//! `cassette queue write`: the one command that mutates a cassette body.
//!
//! Everything shared with the read-only commands in `view.rs` — `QueueError`,
//! `WriterSource`, `StatusFilter`, and the writer-error mappers — lives in
//! `queue/mod.rs` instead, so this module can stay the only place a lock is
//! ever acquired for writing.

use super::{require_error_to_queue_error, resolve_error_to_queue_error, QueueError, WriterSource};
use crate::store;
use crate::store::writers::Kind;
use crate::store::Store;

/// Whether `kind` may write over a cassette whose `locked_by` is set.
///
/// The same permission boundary `queue close`'s `close_permitted` enforces,
/// applied to `write` instead: a sticky lock is a claim on the cassette's
/// *content*, not just on closing it out, so an agent overwriting the body of
/// a cassette someone else claimed is exactly what the lock exists to
/// prevent. A human may always proceed — blocked by their own or another
/// human's lock, they can always `queue unlock` first, so refusing them here
/// would only let one terminal lock a person out of their own work.
fn write_permitted(kind: Kind, locked_by: Option<&str>) -> Result<(), QueueError> {
    match (kind, locked_by) {
        (Kind::Agent, Some(holder)) => Err(QueueError::Sticky(format!(
            "cassette is locked by '{holder}' — only a human may write it"
        ))),
        _ => Ok(()),
    }
}

/// Resolve `who_name`/`source` into `(writer, kind)`, matching `close`'s
/// rationale: which of `resolve_writer`/`require_writer` runs depends on
/// where the name came from, not on whether it happens to be new. Shared by
/// `write` and `write_body` so the two cannot resolve a writer differently.
fn resolve(
    store: &Store,
    who_name: &str,
    source: WriterSource,
) -> Result<(String, Kind), QueueError> {
    match source {
        WriterSource::Env => store
            .resolve_writer(who_name)
            .map_err(resolve_error_to_queue_error),
        WriterSource::Flag => store
            .require_writer(who_name)
            .map_err(require_error_to_queue_error),
    }
}

/// Acquire `id`'s lock, mapping every `LockError` to the `QueueError` `write`
/// and `write_body` both report it as.
fn acquire(
    store: &Store,
    session: &str,
    id: &str,
    who: &store::lock::Attribution,
) -> Result<store::lock::LockGuard, QueueError> {
    match store.lock(session, id, who) {
        Ok(g) => Ok(g),
        Err(store::lock::LockError::Busy { holder, .. }) => {
            let who = holder
                .map(|a| format!("{} (since {})", a.name, a.since))
                .unwrap_or_else(|| "another writer".to_string());
            Err(QueueError::Busy(format!(
                "'{id}' is open by {who} — try again later"
            )))
        }
        // A distinct variant from `LockError::Io`, so this is a usage error
        // (2) by construction rather than by matching on `io::ErrorKind` and
        // hoping `acquire` never surfaces `NotFound` for another reason.
        Err(store::lock::LockError::NoSuchCassette { .. }) => Err(QueueError::Usage(format!(
            "no cassette '{id}' in session '{session}'"
        ))),
        Err(store::lock::LockError::Io(e)) => {
            Err(QueueError::Io(format!("cannot lock '{id}': {e}")))
        }
    }
}

/// Read through `guard`, enforce `write_permitted` against its `locked_by`,
/// and write `body` back. The last step of both `write` and `write_body`,
/// once a guard is already held and a body is already in hand.
///
/// Reading `locked_by` here — only after the guard is held — is what makes
/// the check race-free: reading it from an unlocked scan would race a
/// concurrent `queue lock` setting it in between the check and the write.
fn read_check_and_write(
    guard: &store::lock::LockGuard,
    id: &str,
    body: &str,
    writer: String,
    kind: Kind,
) -> Result<(), QueueError> {
    let current = match guard.read() {
        Ok(c) => c,
        Err(e) => return Err(QueueError::Io(format!("cannot read '{id}': {e}"))),
    };
    write_permitted(kind, current.meta.locked_by.as_deref())?;

    let mut m = current.meta;
    m.last_writer = writer;
    m.updated_at = store::meta::now_utc();
    if let Err(e) = guard.write(&m, body) {
        return Err(QueueError::Io(format!("cannot write '{id}': {e}")));
    }
    Ok(())
}

/// `cassette queue write <ID>`: acquire the cassette's lock, THEN read the body
/// from stdin, write, and release.
///
/// The ordering is deliberate and is what makes the concurrency tests
/// deterministic: a child spawned with an open stdin pipe is provably holding
/// the lock, with no sleeps and no polling, and closing the pipe releases it.
/// **Lock before stdin, always** — reversing this would make every
/// `contend()`-based test in `tests/lock.rs` hang, since a losing racer would
/// no longer exit before ever reading its own stdin.
///
/// This is deliberately NOT built as "read stdin, then call `write_body`":
/// `write_body` (below) acquires its own lock so it stays a single
/// self-contained, testable unit, and calling it after stdin is read would
/// move this command's lock acquisition to after stdin — exactly the
/// ordering bug this doc warns about. Registration, locking and stdin are
/// interleaved here instead, and `read_check_and_write` is the tail both
/// this and `write_body` share.
pub fn write(
    store: &Store,
    id: &str,
    session: &str,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    // Registration/lookup happens BEFORE acquisition, even though failing
    // fast on a bad lock looks more logical: two writers racing for the same
    // cassette must both land in writers.toml (or both fail cleanly) even
    // when one of them loses the lock.
    let (writer, kind) = resolve(store, who_name, source)?;
    let who = store::lock::Attribution::for_now(&writer, who_name);

    let guard = acquire(store, session, id, &who)?;

    // Lock first, stdin second. Reversing these would make the tests racy.
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
        return Err(QueueError::Io(format!("cannot read stdin: {e}")));
    }

    read_check_and_write(&guard, id, &body, writer, kind)
}

/// The lock-read-modify-write core, with the body already in hand.
///
/// Split out from `write` so the sticky-lock rule is unit-testable: `write`
/// itself reads stdin, which a test cannot supply. This does its own
/// resolve-then-lock, exactly mirroring `write`'s own order of operations —
/// it is not called by `write` (see `write`'s doc comment for why), only by
/// tests that already have a body in hand and no stdin to provide. Gated on
/// `cfg(test)` since it has no production caller.
#[cfg(test)]
fn write_body(
    store: &Store,
    session: &str,
    id: &str,
    body: &str,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    let (writer, kind) = resolve(store, who_name, source)?;
    let who = store::lock::Attribution::for_now(&writer, who_name);
    let guard = acquire(store, session, id, &who)?;
    read_check_and_write(&guard, id, body, writer, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::writers::Kind;
    use crate::store::Store;

    fn sticky_cassette(store: &Store) -> (String, String) {
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let id = "aaa00000000000000000000000".to_string();
        let m = CassetteMeta {
            id: id.clone(),
            topic: Some("claimed".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: Some("01OTHERWRITER00000000000AB".to_string()),
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-16T09:00:00Z".to_string(),
        };
        store.add_cassette(&sid, &m, "original\n").expect("add");
        (sid, id)
    }

    #[test]
    fn an_agent_may_not_write_a_sticky_locked_cassette() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (sid, id) = sticky_cassette(&store);
        store.ensure_writer("bot", Kind::Agent).expect("agent");

        match write_body(
            &store,
            &sid,
            &id,
            "agent prose\n",
            "bot",
            WriterSource::Flag,
        ) {
            Err(QueueError::Sticky(_)) => {}
            other => panic!("expected Sticky, got {other:?}"),
        }
        let scan = store.scan_session(&sid).expect("scan");
        assert!(
            scan.cassettes[0].body.contains("original"),
            "the refused write must not have touched the body: {}",
            scan.cassettes[0].body
        );
    }

    #[test]
    fn a_human_may_write_a_sticky_locked_cassette() {
        // The lock keeps agents out; a human blocked by one can always
        // `queue unlock` and proceed, so blocking humans would only let one
        // terminal lock the same person out of their own work.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (sid, id) = sticky_cassette(&store);
        store.ensure_writer("joseph", Kind::Human).expect("human");

        write_body(
            &store,
            &sid,
            &id,
            "human prose\n",
            "joseph",
            WriterSource::Flag,
        )
        .expect("a human may write over a sticky lock");
        let scan = store.scan_session(&sid).expect("scan");
        assert!(scan.cassettes[0].body.contains("human prose"));
    }
}
