//! Read-only `cassette queue` commands: `list`, `show`, and `next`.
//!
//! **This module performs no writes and holds no lock.** Every mutation to a
//! cassette file goes through `LockGuard::write` (see `store::lock`), and
//! this module never constructs a `LockGuard` at all — `list` and `show`
//! take no lock, cassette or registry. `next` calls `Store::is_free`, which
//! opens a cassette's lock anchor, tries the flock, and releases it within
//! that single call to answer "is this free right now" (a snapshot, not a
//! claim) — so no lock is ever held across a return from this module. A
//! reviewer can confirm the whole invariant by reading this file alone:
//! there is no `guard.write(`, no `atomic_write(`, no `std::fs::write(`,
//! nothing.

use crate::queue::{QueueError, StatusFilter};
use crate::store::meta::{CassetteMeta, Status};
use crate::store::{priority, Store, StoredCassette};

/// Order `cassettes` (open before closed, then priority, then id — see
/// `store::priority::queue_order`) and render one line per cassette:
/// `<id>  p<priority>  <status>  <topic>`. `unreadable` is the count of
/// cassette files the store could not read or parse, appended as a final
/// `N unreadable` line when nonzero.
///
/// A damaged cassette must never just vanish: `queue list` is the operator's
/// only view of the store, and a silently shorter list reads identically to
/// an empty queue. Counting and reporting it here is the point; rendering it
/// row-by-row (path, error) is a later phase's job.
pub fn render_list(cassettes: &[StoredCassette], unreadable: usize) -> String {
    if cassettes.is_empty() && unreadable == 0 {
        return "no cassettes".to_string();
    }
    let mut metas: Vec<CassetteMeta> = cassettes.iter().map(|c| c.meta.clone()).collect();
    priority::queue_order(&mut metas);
    let mut lines: Vec<String> = metas
        .iter()
        .map(|m| {
            format!(
                "{}  p{}  {}  {}",
                m.id,
                m.priority,
                m.status.as_str(),
                m.topic.as_deref().unwrap_or("")
            )
        })
        .collect();
    if unreadable > 0 {
        lines.push(format!("{unreadable} unreadable"));
    }
    lines.join("\n")
}

/// `cassette queue list --session <ID> [--status open|closed|all] [--since <TIME>]`.
///
/// Filters the session's cassettes by status and, when given, by
/// `updated_at >= since` (an unparseable `--since` is a usage error, not an
/// empty result) before rendering. Takes no lock: a scan is many independent
/// reads, and a torn read of one cassette mid-write is expected, not an
/// error — see `show`.
pub fn list(
    store: &Store,
    session: &str,
    status: StatusFilter,
    since: Option<&str>,
) -> Result<String, QueueError> {
    let since = match since {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    QueueError::Usage(format!("invalid --since '{s}': expected RFC3339"))
                })?,
        ),
        None => None,
    };

    let scan = store
        .scan_session(session)
        .map_err(|e| QueueError::Io(format!("cannot scan session '{session}': {e}")))?;

    let filtered: Vec<StoredCassette> = scan
        .cassettes
        .into_iter()
        .filter(|c| match status {
            StatusFilter::All => true,
            StatusFilter::Open => c.meta.status == Status::Open,
            StatusFilter::Closed => c.meta.status == Status::Closed,
        })
        .filter(|c| match since {
            None => true,
            // A cassette whose own `updated_at` cannot be parsed (a
            // hand-edited file) is included rather than silently dropped by
            // a filter it cannot be checked against.
            Some(since) => chrono::DateTime::parse_from_rfc3339(&c.meta.updated_at)
                .map(|dt| dt.with_timezone(&chrono::Utc) >= since)
                .unwrap_or(true),
        })
        .collect();

    Ok(render_list(&filtered, scan.unreadable))
}

/// `cassette queue show <ID> --session <ID>`: the cassette's frontmatter and
/// body, verbatim.
///
/// Reads the raw file rather than reconstructing it from a parsed
/// `CassetteMeta` — showing exactly what is on disk is more honest than
/// normalizing a hand-edited file back through `build_frontmatter`, and it
/// works even for a cassette whose frontmatter is damaged in some way that
/// doesn't stop `id` lookup (this reads by id via `cassette_path`, not via
/// `scan_session`, so a broken frontmatter body isn't in play at all).
///
/// Takes no lock: a torn read here shows stale or half-written text, which is
/// what a viewer of a live session should expect, and taking a lock would
/// make viewing fail while someone else is writing.
pub fn show(store: &Store, session: &str, id: &str) -> Result<String, QueueError> {
    let path = store
        .cassette_path(session, id)
        .map_err(|e| QueueError::Io(format!("cannot look up '{id}': {e}")))?
        .ok_or_else(|| QueueError::Usage(format!("no cassette '{id}' in session '{session}'")))?;
    std::fs::read_to_string(&path).map_err(|e| QueueError::Io(format!("cannot read '{id}': {e}")))
}

/// `cassette queue next --session <ID>`: the id of the next cassette a
/// writer should take.
///
/// A one-shot CLI process cannot hold a lock for its caller — flock dies
/// with the process — so this reports an id rather than claiming one; the
/// caller races for it with `queue write`. That window is real and is
/// accepted by design: `queue write` already returns exit 3 with the
/// holder's attribution when it loses the race, so the loser just retries.
/// Closing the window needs a sticky lock, which is a later phase.
///
/// Walks the session's open cassettes in queue order (`store::priority`,
/// same comparator `queue list` uses) and returns the first whose lock
/// `Store::is_free` reports free. Distinguishes two empty-handed outcomes,
/// on purpose (see `QueueError::Empty`):
/// - open cassettes exist but every one is currently locked → `Busy` (exit 3,
///   wait and retry)
/// - no open cassettes at all → `Empty` (exit 5, idle or enqueue)
pub fn next(store: &Store, session: &str) -> Result<String, QueueError> {
    let scan = store
        .scan_session(session)
        .map_err(|e| QueueError::Io(format!("cannot scan session '{session}': {e}")))?;

    let mut open: Vec<CassetteMeta> = scan
        .cassettes
        .into_iter()
        .map(|c| c.meta)
        .filter(|m| m.status == Status::Open)
        .collect();

    if open.is_empty() {
        return Err(QueueError::Empty(format!(
            "no open cassettes in session '{session}'"
        )));
    }

    priority::queue_order(&mut open);

    for m in &open {
        let free = store
            .is_free(session, &m.id)
            .map_err(|e| QueueError::Io(format!("cannot check lock on '{}': {e}", m.id)))?;
        if free {
            return Ok(m.id.clone());
        }
    }

    Err(QueueError::Busy(
        "every open cassette is being written — try again shortly".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};

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

    fn stored(m: CassetteMeta) -> StoredCassette {
        StoredCassette {
            path: std::path::PathBuf::from(format!("{}.md", m.id)),
            meta: m,
            body: "## Side A\n\nwords here\n".to_string(),
        }
    }

    #[test]
    fn list_orders_open_by_priority_then_closed_last() {
        let rows = vec![
            stored(meta("c", 5, Status::Closed)),
            stored(meta("b", 20, Status::Open)),
            stored(meta("a", 10, Status::Open)),
        ];
        let out = render_list(&rows, 0);
        let a = out.find("topic-a").expect("a");
        let b = out.find("topic-b").expect("b");
        let c = out.find("topic-c").expect("c");
        assert!(a < b, "priority 10 before 20: {out}");
        assert!(b < c, "closed last despite priority 5: {out}");
    }

    #[test]
    fn list_reports_unreadable_cassettes_instead_of_hiding_them() {
        // A cassette the store could not parse must never be silently
        // absent: `queue list` is the operator's only view of the store.
        let out = render_list(&[stored(meta("a", 10, Status::Open))], 2);
        assert!(out.contains("2 unreadable"), "{out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&[], 0), "no cassettes");
    }

    #[test]
    fn list_filters_by_status() {
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
        let mut open = meta("aaa00000000000000000000000", 10, Status::Open);
        open.topic = Some("open-one".to_string());
        let mut closed = meta("bbb00000000000000000000000", 20, Status::Closed);
        closed.topic = Some("closed-one".to_string());
        store.add_cassette(&sid, &open, "").expect("add");
        store.add_cassette(&sid, &closed, "").expect("add");

        let open_only = list(&store, &sid, StatusFilter::Open, None).expect("list");
        assert!(open_only.contains("open-one"), "{open_only}");
        assert!(!open_only.contains("closed-one"), "{open_only}");

        let all = list(&store, &sid, StatusFilter::All, None).expect("list");
        assert!(
            all.contains("open-one") && all.contains("closed-one"),
            "{all}"
        );
    }

    #[test]
    fn list_rejects_an_unparseable_since() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        match list(&store, "nope", StatusFilter::All, Some("not-a-date")) {
            Err(QueueError::Usage(m)) => assert!(m.contains("not-a-date"), "{m}"),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn show_returns_frontmatter_and_body_for_a_known_id() {
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
        let m = meta("01K5GR7T2M9WPD0000000000AB", 10, Status::Open);
        store
            .add_cassette(&sid, &m, "## Side A\n\nhello\n")
            .expect("add");

        let out = show(&store, &sid, &m.id).expect("show");
        assert!(out.contains("id: 01K5GR7T2M9WPD0000000000AB"), "{out}");
        assert!(out.contains("hello"), "{out}");
    }

    #[test]
    fn show_on_an_unknown_id_is_a_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        match show(&store, "nope", "nope") {
            Err(QueueError::Usage(m)) => assert!(m.contains("nope"), "{m}"),
            other => panic!("expected Usage, got {other:?}"),
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
    fn next_returns_empty_when_there_are_no_open_cassettes() {
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

        match next(&store, &sid) {
            Err(QueueError::Empty(m)) => assert!(m.contains(&sid), "{m}"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn next_returns_empty_for_a_session_with_no_cassettes_at_all() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        match next(&store, &sid) {
            Err(QueueError::Empty(_)) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn next_skips_a_locked_cassette_and_returns_the_next_free_one() {
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
        store
            .add_cassette(
                &sid,
                &meta("bbb00000000000000000000000", 20, Status::Open),
                "",
            )
            .expect("add");

        // Lock the higher-priority (lower id) cassette, so `next` must skip
        // it and report the other open one instead of reporting Busy.
        let holder = crate::store::lock::Attribution::for_now("writer-1", "joseph");
        let _held = store
            .lock(&sid, "aaa00000000000000000000000", &holder)
            .expect("hold it");

        assert_eq!(
            next(&store, &sid).expect("next"),
            "bbb00000000000000000000000"
        );
    }

    #[test]
    fn next_returns_busy_when_every_open_cassette_is_locked() {
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

        let holder = crate::store::lock::Attribution::for_now("writer-1", "joseph");
        let _held = store
            .lock(&sid, "aaa00000000000000000000000", &holder)
            .expect("hold it");

        match next(&store, &sid) {
            Err(QueueError::Busy(_)) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
    }
}
