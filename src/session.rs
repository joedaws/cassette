//! The `cassette session` commands: new, list, alias.
//!
//! Sessions are named by ULID only. An alias is a display label shown in
//! `session list` and nowhere else — it never resolves in place of an id, so
//! there is no uniqueness rule on it and no id-prefix matching anywhere in
//! this module. That is a deliberate design decision, not an oversight.

use std::io;

use crate::store::session::SessionMeta;
use crate::store::Store;

/// How many rows `list` shows by default before hinting at `--all`.
/// How many sessions a listing shows before it needs asking. Shared with
/// the `sessions` picker so the two cannot disagree about "recent".
pub(crate) const DEFAULT_LIST_LIMIT: usize = 15;

/// Why a session command failed, in the shape `main.rs` maps to an exit
/// code. Shaped like `queue::QueueError` minus `Busy`, which no session
/// command can produce — a session command never contends a lock, so a
/// distinct type here keeps that impossibility visible in the signature
/// rather than borrowing a variant that could never be constructed.
#[derive(Debug)]
pub enum SessionError {
    /// Bad invocation — e.g. an unknown session id. Exit 2.
    Usage(String),
    /// Anything else. Exit 1.
    Io(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Usage(m) | SessionError::Io(m) => write!(f, "{m}"),
        }
    }
}

/// Create a session and return its id. `alias` is a display label only —
/// see the module docs — and is never validated for uniqueness.
pub fn new_session(store: &Store, alias: Option<&str>) -> Result<String, String> {
    let meta = SessionMeta {
        alias: alias.map(str::to_string),
        created: crate::store::meta::now_utc(),
        timer_secs: None,
        word_goal: None,
    };
    store.create_session(&meta).map_err(|e| e.to_string())
}

/// Render one line per row (`<id>  <created>  <alias or "">`), in the order
/// given. With a `limit` that hides rows, the last line says how many were
/// hidden rather than dropping them silently.
///
/// Deliberately does **not** sort: `Store::list_sessions` already returns
/// rows newest-first by `created`, ties broken by id descending, and sorting
/// belongs with the fetch. This function had an identical second sort, which
/// only meant two places to keep a comparator in step. Ordering is pinned by
/// `store::tests::list_sessions_is_newest_first`; the tests here feed
/// pre-sorted rows, as the only caller does.
pub fn render_list(rows: &[(String, SessionMeta)], limit: Option<usize>) -> String {
    if rows.is_empty() {
        return "no sessions".to_string();
    }
    let total = rows.len();
    let shown = limit.map(|l| l.min(total)).unwrap_or(total);
    let mut lines: Vec<String> = rows[..shown]
        .iter()
        .map(|(id, m)| format!("{id}  {}  {}", m.created, m.alias.as_deref().unwrap_or("")))
        .collect();
    if shown < total {
        lines.push(format!("… {} more (--all)", total - shown));
    }
    lines.join("\n")
}

/// `all` shows every session; otherwise the `DEFAULT_LIST_LIMIT` most recent.
pub fn list(store: &Store, all: bool) -> Result<String, String> {
    let rows = store.list_sessions().map_err(|e| e.to_string())?;
    let limit = if all { None } else { Some(DEFAULT_LIST_LIMIT) };
    Ok(render_list(&rows, limit))
}

/// Set `id`'s display alias. An unknown session id is a usage error (exit
/// 2) — the caller named something that does not exist — distinct from an
/// I/O failure reading or writing an existing one (exit 1).
///
/// The id goes through `Store::require_session`, the same gate every `queue`
/// command passes (see `queue::require_session`), so the two code paths
/// cannot drift on what a session id is: `alias` is the other command that
/// takes an id a human typed, and it joins it onto the store root just the
/// same.
pub fn set_alias(store: &Store, id: &str, alias: &str) -> Result<String, SessionError> {
    match store.require_session(id) {
        Ok(()) => {}
        Err(crate::store::RequireSessionError::Usage(m)) => return Err(SessionError::Usage(m)),
        Err(crate::store::RequireSessionError::Io(m)) => return Err(SessionError::Io(m)),
    }
    match store.set_session_alias(id, alias) {
        Ok(()) => Ok(format!("{id}  {alias}")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(SessionError::Usage(format!("no session '{id}'")))
        }
        Err(e) => Err(SessionError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::session::SessionMeta;

    fn meta(created: &str, alias: Option<&str>) -> SessionMeta {
        SessionMeta {
            alias: alias.map(str::to_string),
            created: created.to_string(),
            timer_secs: None,
            word_goal: None,
        }
    }

    #[test]
    fn list_keeps_the_stores_order_and_shows_the_alias() {
        // Fed in the order `Store::list_sessions` returns — newest first —
        // because that is the order the only caller passes. `render_list`
        // does not re-sort; it must not reorder what it is given either.
        let rows = vec![
            (
                "01BBB".to_string(),
                meta("2026-09-15T09:00:00Z", Some("today")),
            ),
            ("01AAA".to_string(), meta("2026-09-14T09:00:00Z", None)),
        ];
        let out = render_list(&rows, None);
        let bbb = out.find("01BBB").expect("bbb");
        let aaa = out.find("01AAA").expect("aaa");
        assert!(bbb < aaa, "rows render in the order given: {out}");
        assert!(out.contains("today"), "alias must show: {out}");
    }

    #[test]
    fn list_end_to_end_is_newest_first() {
        // The ordering itself is the store's job, so pin it through the real
        // path `session list` takes rather than through `render_list` alone.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        // Explicit timestamps: two sessions minted back to back can share a
        // `created` second, and the id tiebreak is not creation order.
        let old_id = store
            .create_session(&meta("2026-09-14T09:00:00Z", Some("older")))
            .expect("create");
        let new_id = store
            .create_session(&meta("2026-09-15T09:00:00Z", Some("newer")))
            .expect("create");

        let out = list(&store, true).expect("list");
        let newer = out.find(&new_id).expect("newer");
        let older = out.find(&old_id).expect("older");
        assert!(newer < older, "newest first: {out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&[], None), "no sessions");
    }

    #[test]
    fn the_default_listing_is_capped_and_says_how_many_it_hid() {
        let rows: Vec<_> = (0..20)
            .map(|i| (format!("01{i:03}"), meta("2026-09-15T09:00:00Z", None)))
            .collect();
        let out = render_list(&rows, Some(15));
        assert_eq!(out.lines().filter(|l| l.starts_with("01")).count(), 15);
        assert!(out.contains("5 more"), "must not hide rows silently: {out}");
    }

    #[test]
    fn new_session_and_list_round_trip_through_a_real_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let id = new_session(&store, Some("morning")).expect("new");
        assert_eq!(id.len(), 30, "sessions are named by ses_ id");
        assert!(id.starts_with("ses_"), "{id}");

        let out = list(&store, false).expect("list");
        assert!(out.contains(&id), "{out}");
        assert!(out.contains("morning"), "{out}");
    }

    #[test]
    fn set_alias_round_trips_and_shows_up_in_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let id = new_session(&store, None).expect("new");
        let msg = set_alias(&store, &id, "monday").expect("set alias");
        assert!(msg.contains("monday"), "{msg}");

        let out = list(&store, false).expect("list");
        assert!(out.contains("monday"), "{out}");
    }

    #[test]
    fn set_alias_on_an_unknown_id_is_a_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        match set_alias(&store, "nope", "x") {
            Err(SessionError::Usage(m)) => assert!(m.contains("nope"), "{m}"),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn set_alias_on_an_unreadable_session_toml_is_an_io_error() {
        // Mirrors store::tests::an_unreadable_session_toml_is_an_io_error_not_a_missing_session:
        // `set_alias` routes through `Store::require_session` too, so its
        // `RequireSessionError::Io` arm must reach `SessionError::Io` rather
        // than being flattened into `Usage` (the "typo" case).
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let id = new_session(&store, None).expect("new");
        let toml = dir.path().join("sessions").join(&id).join("session.toml");

        let mut perms = std::fs::metadata(&toml).expect("metadata").permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&toml, perms).expect("chmod");

        // Root ignores the mode bits; probe the real effect rather than
        // guessing from $USER, same reasoning as the store-level test.
        if std::fs::read_to_string(&toml).is_ok() {
            return;
        }

        match set_alias(&store, &id, "monday") {
            Err(SessionError::Io(m)) => assert!(
                !m.contains("no session"),
                "an unreadable store is I/O, not a typo: {m}"
            ),
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
