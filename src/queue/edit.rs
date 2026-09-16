//! `cassette queue new`: create a cassette in a session, plus the shared
//! open-cassette cap and the renumbering helper `queue move` (a later task)
//! will also need.
//!
//! Alongside `write.rs`, this is the other place in `queue` that writes a
//! cassette file — `new` calls `Store::add_cassette`, which routes through
//! `LockGuard::write` like every other write in the crate, and
//! `renumber_all` writes through guards it takes itself via
//! `Store::lock_many`.

use super::{require_error_to_queue_error, resolve_error_to_queue_error, QueueError, WriterSource};
use crate::store::lock::Attribution;
use crate::store::meta::{CassetteMeta, Status};
use crate::store::{ids, priority, Store};

/// Where a new cassette lands in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Head of the queue.
    First,
    /// Tail of the queue — the default. An agent adding work cannot jump
    /// the human's line.
    Last,
    /// An exact sparse priority, validated positive at construction time by
    /// `validate_priority`.
    Explicit(i64),
}

/// Reject a non-positive priority. `priority::first`'s `below > 0` and
/// `halved > 0` guards already treat a non-positive result as "no room
/// left", so a stored `0` or negative priority would be a value the
/// placement functions cannot reason about — reject it here, at the
/// boundary, instead of letting it reach disk.
pub fn validate_priority(n: i64) -> Result<i64, QueueError> {
    if n <= 0 {
        Err(QueueError::Usage(format!(
            "--priority must be a positive integer, got {n}"
        )))
    } else {
        Ok(n)
    }
}

/// Whether a session already holds `max_open` open cassettes. Closed
/// cassettes never count toward the cap — a session full of closed
/// cassettes is not a full queue.
pub fn is_full(statuses: &[Status], max_open: usize) -> bool {
    statuses.iter().filter(|s| **s == Status::Open).count() >= max_open
}

/// `cassette queue new <TOPIC> --session <ID> [--first|--last|--priority N]`:
/// create an open cassette and print its id.
///
/// Order of operations, each deliberate:
/// 1. Resolve the writer first, matching `write`'s existing rationale — a
///    typo'd `--writer` or an I/O failure should fail before anything else
///    is touched.
/// 2. Check the open-cassette cap before computing a priority, so a full
///    queue is never charged a renumber.
/// 3. Compute the priority from `placement`. When `priority::first`/`last`
///    report no room left (`None`), renumber the whole session once and
///    retry the placement exactly once; a second `None` means the run is
///    genuinely unrepresentable (only reachable by hand-edited frontmatter),
///    which is `QueueError::Io`.
/// 4. Create the cassette through `Store::add_cassette`, which takes and
///    releases its own lock — a freshly minted id cannot be contended.
#[allow(clippy::too_many_arguments)]
pub fn new(
    store: &Store,
    session: &str,
    topic: &str,
    placement: Placement,
    who_name: &str,
    source: WriterSource,
    max_open: usize,
) -> Result<String, QueueError> {
    // Same rationale as `write::write`: registering/looking up the writer
    // happens before anything else, so a typo'd `--writer` or a registry
    // read failure is reported before the session is even scanned.
    let (writer, _kind) = match source {
        WriterSource::Env => store
            .resolve_writer(who_name)
            .map_err(resolve_error_to_queue_error)?,
        WriterSource::Flag => store
            .require_writer(who_name)
            .map_err(require_error_to_queue_error)?,
    };

    let scan = store
        .scan_session(session)
        .map_err(|e| QueueError::Io(format!("cannot scan session '{session}': {e}")))?;
    let statuses: Vec<Status> = scan.cassettes.iter().map(|c| c.meta.status).collect();
    if is_full(&statuses, max_open) {
        return Err(QueueError::Full(format!(
            "session '{session}' already holds {max_open} open cassettes"
        )));
    }

    let open_priorities: Vec<i64> = scan
        .cassettes
        .iter()
        .filter(|c| c.meta.status == Status::Open)
        .map(|c| c.meta.priority)
        .collect();

    let priority = match place(placement, &open_priorities)? {
        Some(p) => p,
        None => {
            let who = Attribution::for_now(&writer, who_name);
            renumber_all(store, session, &who)?;
            let rescanned = store
                .scan_session(session)
                .map_err(|e| QueueError::Io(format!("cannot scan session '{session}': {e}")))?;
            let open_priorities: Vec<i64> = rescanned
                .cassettes
                .iter()
                .filter(|c| c.meta.status == Status::Open)
                .map(|c| c.meta.priority)
                .collect();
            place(placement, &open_priorities)?.ok_or_else(|| {
                QueueError::Io(format!(
                    "session '{session}' cannot be placed even after renumbering"
                ))
            })?
        }
    };

    let meta = CassetteMeta {
        id: ids::new_id(),
        topic: Some(topic.to_string()),
        priority,
        status: Status::Open,
        locked_by: None,
        created_by: writer.clone(),
        last_writer: writer,
        updated_at: crate::store::meta::now_utc(),
    };

    store
        .add_cassette(session, &meta, "")
        .map_err(|e| QueueError::Io(format!("cannot create cassette in '{session}': {e}")))?;

    Ok(meta.id)
}

/// Resolve `placement` against the session's current open priorities.
/// `Explicit` is validated here too — not just at the CLI boundary — since
/// `new` is a public entry point in its own right, not only reachable
/// through `cli.rs`.
fn place(placement: Placement, open_priorities: &[i64]) -> Result<Option<i64>, QueueError> {
    Ok(match placement {
        Placement::Last => priority::last(open_priorities),
        Placement::First => priority::first(open_priorities),
        Placement::Explicit(n) => Some(validate_priority(n)?),
    })
}

/// Give every cassette in `session` fresh sparse priorities, in queue order.
/// The one caller of `Store::lock_many` in the crate.
///
/// **Call this holding no locks.** flock is per-open-file-description: a
/// second open-and-lock of a file this same process already holds through
/// another descriptor does not "already own" it — it reports `Busy` — so a
/// caller that renumbers while still holding one of the session's cassette
/// guards deadlocks against itself and misreports it as contention from
/// another writer. Every caller (today, `new` above; `queue move` in Task 8
/// as well) must therefore compute its placement first, release every guard
/// it holds, renumber, and retry.
pub fn renumber_all(store: &Store, session: &str, who: &Attribution) -> Result<(), QueueError> {
    let scan = store
        .scan_session(session)
        .map_err(|e| QueueError::Io(format!("cannot scan session '{session}': {e}")))?;

    let mut metas: Vec<CassetteMeta> = scan.cassettes.into_iter().map(|c| c.meta).collect();
    if metas.is_empty() {
        return Ok(());
    }
    priority::queue_order(&mut metas);
    let fresh = priority::renumber(metas.len());

    // Keyed by id rather than kept parallel to `metas` by position: below,
    // `lock_many`'s guards come back sorted into ascending id order, not in
    // `metas`' queue order, so the only safe way to pair a guard with its new
    // priority is to look it up by id.
    let priority_by_id: std::collections::HashMap<&str, i64> =
        metas.iter().map(|m| m.id.as_str()).zip(fresh).collect();

    let ids: Vec<&str> = metas.iter().map(|m| m.id.as_str()).collect();
    let guards = store.lock_many(session, &ids, who).map_err(|e| {
        QueueError::Io(format!(
            "cannot lock session '{session}' for renumbering: {e}"
        ))
    })?;

    for guard in &guards {
        let current = guard
            .read()
            .map_err(|e| QueueError::Io(format!("cannot read '{}': {e}", guard.id())))?;
        let mut m = current.meta;
        // Every guard's id came from `metas`, whose ids are exactly the keys
        // of `priority_by_id` — this can only miss on a bug in that
        // construction, not on anything a caller can trigger.
        let Some(&p) = priority_by_id.get(guard.id()) else {
            return Err(QueueError::Io(format!(
                "renumbering '{session}': no fresh priority computed for '{}'",
                guard.id()
            )));
        };
        m.priority = p;
        m.last_writer = who.writer.clone();
        m.updated_at = crate::store::meta::now_utc();
        guard
            .write(&m, &current.body)
            .map_err(|e| QueueError::Io(format!("cannot write '{}': {e}", guard.id())))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_positive_explicit_priority_is_rejected() {
        // priority::first treats non-positive results as "no room left"
        // (`below > 0`, `halved > 0`), so a stored 0 would be a value the
        // placement functions cannot reason about. Reject at the boundary.
        for bad in [0i64, -1, i64::MIN] {
            assert!(
                validate_priority(bad).is_err(),
                "{bad} must be rejected at the CLI boundary"
            );
        }
        assert!(validate_priority(1).is_ok());
    }

    #[test]
    fn the_cap_counts_open_cassettes_only() {
        // A session full of closed cassettes is not a full queue.
        assert!(!is_full(&[Status::Closed, Status::Closed], 2));
        assert!(is_full(&[Status::Open, Status::Open], 2));
        assert!(!is_full(&[Status::Open, Status::Closed], 2));
    }

    fn meta(id: &str, priority: i64, status: Status) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: Some(format!("topic-{id}")),
            priority,
            status,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-15T09:00:00Z".to_string(),
        }
    }

    fn new_session(store: &Store) -> String {
        store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session")
    }

    #[test]
    fn new_defaults_to_last_placement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        store
            .add_cassette(
                &sid,
                &meta("aaa00000000000000000000000", 10, Status::Open),
                "",
            )
            .expect("add");

        let id = new(
            &store,
            &sid,
            "gratitude",
            Placement::Last,
            "tester",
            WriterSource::Env,
            36,
        )
        .expect("new");

        let scan = store.scan_session(&sid).expect("scan");
        let created = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == id)
            .expect("found");
        assert_eq!(created.meta.priority, 20, "tail placement after p10");
        assert_eq!(created.meta.topic.as_deref(), Some("gratitude"));
        assert_eq!(created.meta.status, Status::Open);
    }

    #[test]
    fn new_rejects_a_non_positive_explicit_priority() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);

        match new(
            &store,
            &sid,
            "gratitude",
            Placement::Explicit(0),
            "tester",
            WriterSource::Env,
            36,
        ) {
            Err(QueueError::Usage(_)) => {}
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn new_refuses_to_exceed_the_open_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        store
            .add_cassette(
                &sid,
                &meta("aaa00000000000000000000000", 10, Status::Open),
                "",
            )
            .expect("add");

        match new(
            &store,
            &sid,
            "one too many",
            Placement::Last,
            "tester",
            WriterSource::Env,
            1,
        ) {
            Err(QueueError::Full(m)) => assert!(m.contains(&sid), "{m}"),
            other => panic!("expected Full, got {other:?}"),
        }
    }

    #[test]
    fn new_does_not_count_closed_cassettes_against_the_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        store
            .add_cassette(
                &sid,
                &meta("aaa00000000000000000000000", 10, Status::Closed),
                "",
            )
            .expect("add");

        let id = new(
            &store,
            &sid,
            "gratitude",
            Placement::Last,
            "tester",
            WriterSource::Env,
            1,
        )
        .expect("new");
        assert!(!id.is_empty());
    }

    #[test]
    fn renumber_all_matches_guards_to_metas_by_id_not_position() {
        // `lock_many` sorts its input into ascending id order, so the guards
        // it returns are not in queue order. Use ids where ascending-id order
        // and queue (priority) order disagree, so a position-based zip would
        // write the wrong priority to the wrong cassette.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);

        // Queue order (by priority): zzz (p10), aaa (p20).
        // Ascending id order: aaa, zzz — the opposite.
        store
            .add_cassette(
                &sid,
                &meta("zzz00000000000000000000000", 10, Status::Open),
                "",
            )
            .expect("add");
        store
            .add_cassette(
                &sid,
                &meta("aaa00000000000000000000000", 20, Status::Open),
                "",
            )
            .expect("add");

        let who = Attribution::for_now("w", "tester");
        renumber_all(&store, &sid, &who).expect("renumber");

        let scan = store.scan_session(&sid).expect("scan");
        let zzz = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == "zzz00000000000000000000000")
            .expect("zzz");
        let aaa = scan
            .cassettes
            .iter()
            .find(|c| c.meta.id == "aaa00000000000000000000000")
            .expect("aaa");
        assert_eq!(zzz.meta.priority, 10, "queue-first cassette gets p10");
        assert_eq!(aaa.meta.priority, 20, "queue-second cassette gets p20");
    }

    #[test]
    fn renumber_all_on_an_empty_session_is_a_no_op() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let who = Attribution::for_now("w", "tester");
        renumber_all(&store, &sid, &who).expect("renumber");
    }
}
