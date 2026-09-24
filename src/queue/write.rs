//! `cassette queue write`: the one command that mutates a cassette body.
//!
//! Everything shared with the read-only commands in `view.rs` — `QueueError`,
//! `WriterSource`, `StatusFilter`, and the writer-error mappers — lives in
//! `queue/mod.rs` instead, so this module can stay the only place a lock is
//! ever acquired for writing.

use super::{resolve_acting, Acting, QueueError, WriterSource};
use crate::queue::json;
use crate::store;
use crate::store::writers::Kind;
use crate::store::Store;

/// Which side of a cassette `queue write` targets. Mirrors `Cassette`'s two
/// sides in `src/cassette.rs` — `A` is the default, matching every existing
/// caller's behaviour before `--side` existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// How `queue write` combines the incoming text with what is already on the
/// named side. `Replace` is the default, matching `queue write`'s behaviour
/// before `--append` existed: the whole side is overwritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Append,
    Replace,
}

/// Whether `acting` may write over a cassette whose `locked_by` is set.
/// Reads `acting.authority`, never the registered kind: a writer taken from
/// `$USER` is refused here even when that name is registered human (see
/// `queue::Acting`), and the refusal says how to act as yourself.
///
/// The same permission boundary `queue close`'s `close_permitted` enforces,
/// applied to `write` instead: a sticky lock is a claim on the cassette's
/// *content*, not just on closing it out, so an agent overwriting the body of
/// a cassette someone else claimed is exactly what the lock exists to
/// prevent. A human may always proceed — blocked by their own or another
/// human's lock, they can always `queue unlock` first, so refusing them here
/// would only let one terminal lock a person out of their own work.
///
/// `store` resolves `locked_by`'s writer id to a display name for the
/// message, the same lookup `queue::view::build_view` does for its
/// `sticky_lock` field — falling back to the raw id when the registry does
/// not know it (a damaged store, or a registry `store.writers()` itself
/// could not read), the same stance `build_view` takes rather than erroring
/// the whole write out over a cosmetic lookup.
fn write_permitted(
    store: &Store,
    acting: &Acting,
    who_name: &str,
    locked_by: Option<&str>,
) -> Result<(), QueueError> {
    match (acting.authority, locked_by) {
        (Kind::Agent, Some(holder)) => {
            let name = store
                .writers()
                .map(|w| store::writers::display_name(&w, holder))
                .unwrap_or_else(|_| holder.to_string());
            Err(QueueError::Sticky(format!(
                "cassette is locked by '{name}' — only a human may write it{}",
                acting.hint(who_name)
            )))
        }
        _ => Ok(()),
    }
}

/// Rebuild a cassette body in canonical form from its two sides: `## Side A`
/// always, `## Side B` only when non-empty. The write-side counterpart of
/// `json::split_sides`, which reads this same shape back apart — the two
/// must never disagree about what a side is.
///
/// `pub(crate)` because the TUI writes the same store through
/// `output::cassette_body`, which delegates here. Two writers emitting
/// subtly different bytes for identical content would make a cassette's
/// body depend on which one touched it last, so there is one function and
/// both callers reach it.
pub(crate) fn build_body(side_a: &str, side_b: &str) -> String {
    let mut out = format!("## Side A\n\n{}\n", side_a.trim());
    if !side_b.trim().is_empty() {
        out.push_str(&format!("## Side B\n\n{}\n", side_b.trim()));
    }
    out
}

/// Compute the new body for `current` after writing `incoming` to `side`
/// under `mode`. Splits `current` via `json::split_sides` (the same parser
/// `--json` reads a body with), replaces or appends to the named side, and
/// rebuilds through `build_body` — the other side is carried through
/// untouched either way.
fn apply_write(current: &str, incoming: &str, side: Side, mode: WriteMode) -> String {
    let (mut side_a, mut side_b) = json::split_sides(current);
    let target = match side {
        Side::A => &mut side_a,
        Side::B => &mut side_b,
    };
    *target = match mode {
        WriteMode::Replace => incoming.to_string(),
        WriteMode::Append => {
            let mut joined = target.trim_end_matches('\n').to_string();
            if !joined.is_empty() {
                joined.push('\n');
            }
            joined.push_str(incoming);
            joined
        }
    };
    build_body(&side_a, &side_b)
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

/// What to write and where it goes: the text a caller supplied, plus the
/// side and mode it targets. `incoming`/`side`/`mode` always travel together
/// from `write`/`write_body` through to `apply_write`, so — unlike `write`'s
/// and `write_body`'s own parameter lists, pinned verbatim by this task's
/// brief — `read_check_and_write` (a private helper with no externally fixed
/// shape) takes them bundled rather than as three more loose parameters.
struct WriteRequest<'a> {
    incoming: &'a str,
    side: Side,
    mode: WriteMode,
}

/// Read through `guard`, enforce `write_permitted` against its `locked_by`,
/// and write the result of `request` back. The last step of both `write` and
/// `write_body`, once a guard is already held and a body is already in hand.
///
/// Reading `locked_by` here — only after the guard is held — is what makes
/// the check race-free: reading it from an unlocked scan would race a
/// concurrent `queue lock` setting it in between the check and the write.
fn read_check_and_write(
    store: &Store,
    guard: &store::lock::LockGuard,
    id: &str,
    request: WriteRequest,
    acting: &Acting,
    who_name: &str,
) -> Result<(), QueueError> {
    let current = match guard.read() {
        Ok(c) => c,
        Err(e) => return Err(QueueError::Io(format!("cannot read '{id}': {e}"))),
    };
    write_permitted(store, acting, who_name, current.meta.locked_by.as_deref())?;

    let body = apply_write(&current.body, request.incoming, request.side, request.mode);
    let mut m = current.meta;
    m.last_writer = acting.id.clone();
    m.updated_at = store::meta::now_utc();
    if let Err(e) = guard.write(&m, &body) {
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
    side: Side,
    mode: WriteMode,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    // Registration/lookup happens BEFORE acquisition, even though failing
    // fast on a bad lock looks more logical: two writers racing for the same
    // cassette must both land in writers.toml (or both fail cleanly) even
    // when one of them loses the lock.
    let acting = resolve_acting(store, who_name, source)?;
    let who = store::lock::Attribution::for_now(&acting.id, who_name);

    let guard = acquire(store, session, id, &who)?;

    // Lock first, stdin second. Reversing these would make the tests racy.
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
        return Err(QueueError::Io(format!("cannot read stdin: {e}")));
    }

    read_check_and_write(
        store,
        &guard,
        id,
        WriteRequest {
            incoming: &body,
            side,
            mode,
        },
        &acting,
        who_name,
    )
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
#[allow(clippy::too_many_arguments)]
fn write_body(
    store: &Store,
    session: &str,
    id: &str,
    body: &str,
    side: Side,
    mode: WriteMode,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    let acting = resolve_acting(store, who_name, source)?;
    let who = store::lock::Attribution::for_now(&acting.id, who_name);
    let guard = acquire(store, session, id, &who)?;
    read_check_and_write(
        store,
        &guard,
        id,
        WriteRequest {
            incoming: body,
            side,
            mode,
        },
        &acting,
        who_name,
    )
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
            Side::A,
            WriteMode::Replace,
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
            Side::A,
            WriteMode::Replace,
            "joseph",
            WriterSource::Flag,
        )
        .expect("a human may write over a sticky lock");
        let scan = store.scan_session(&sid).expect("scan");
        assert!(scan.cassettes[0].body.contains("human prose"));
    }

    #[test]
    fn the_sticky_refusal_names_the_holder_by_name_not_raw_ulid() {
        // Step 5: the message used to read "cassette is locked by
        // '01OTHERWRITER...'" — a raw ULID nobody but the store can act on.
        // `queue::view::build_view` already resolves the same field to a
        // name for `--json`'s `sticky_lock`; the prose path must do the same.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let holder_id = store
            .ensure_writer("beyonce", Kind::Human)
            .expect("register the holder");
        let id = "aaa00000000000000000000000".to_string();
        let m = CassetteMeta {
            id: id.clone(),
            topic: Some("claimed".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: Some(holder_id.clone()),
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-16T09:00:00Z".to_string(),
        };
        store.add_cassette(&sid, &m, "original\n").expect("add");
        store.ensure_writer("bot", Kind::Agent).expect("agent");

        match write_body(
            &store,
            &sid,
            &id,
            "agent prose\n",
            Side::A,
            WriteMode::Replace,
            "bot",
            WriterSource::Flag,
        ) {
            Err(QueueError::Sticky(msg)) => {
                assert!(msg.contains("beyonce"), "must name the holder: {msg}");
                assert!(
                    !msg.contains(&holder_id),
                    "must not leak the raw ulid: {msg}"
                );
            }
            other => panic!("expected Sticky, got {other:?}"),
        }
    }

    #[test]
    fn a_sticky_holder_unknown_to_the_registry_falls_back_to_the_raw_id() {
        // Same stance as `build_view`: a `locked_by` id the registry does not
        // know (a damaged store) must not turn a sticky refusal into some
        // other kind of error — degrade to the raw id instead.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (sid, id) = sticky_cassette(&store);
        store.ensure_writer("bot", Kind::Agent).expect("agent");

        match write_body(
            &store,
            &sid,
            &id,
            "agent prose\n",
            Side::A,
            WriteMode::Replace,
            "bot",
            WriterSource::Flag,
        ) {
            Err(QueueError::Sticky(msg)) => {
                assert!(msg.contains("01OTHERWRITER00000000000AB"), "{msg}");
            }
            other => panic!("expected Sticky, got {other:?}"),
        }
    }

    #[test]
    fn writing_side_b_leaves_side_a_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let id = "aaa00000000000000000000000";
        let m = CassetteMeta {
            id: id.to_string(),
            topic: Some("sides".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-17T09:00:00Z".to_string(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\nfront\n")
            .expect("add");
        store.ensure_writer("joseph", Kind::Human).expect("human");

        write_body(
            &store,
            &sid,
            id,
            "back\n",
            Side::B,
            WriteMode::Replace,
            "joseph",
            WriterSource::Flag,
        )
        .expect("write side b");

        let body = &store.scan_session(&sid).expect("scan").cassettes[0].body;
        assert!(body.contains("front"), "side A must survive: {body}");
        assert!(body.contains("## Side B"), "side B heading written: {body}");
        assert!(body.contains("back"), "{body}");
    }

    #[test]
    fn append_adds_to_a_side_rather_than_replacing_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let id = "aaa00000000000000000000000";
        let m = CassetteMeta {
            id: id.to_string(),
            topic: Some("sides".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-17T09:00:00Z".to_string(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\nfirst\n")
            .expect("add");
        store.ensure_writer("joseph", Kind::Human).expect("human");

        write_body(
            &store,
            &sid,
            id,
            "second\n",
            Side::A,
            WriteMode::Append,
            "joseph",
            WriterSource::Flag,
        )
        .expect("append");

        let body = &store.scan_session(&sid).expect("scan").cassettes[0].body;
        assert!(body.contains("first"), "the original text survives: {body}");
        assert!(body.contains("second"), "{body}");
    }
}
