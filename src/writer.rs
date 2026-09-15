//! The `cassette writer` commands: register, list, whoami.
//!
//! `kind` is fixed at registration and resolved from the registry everywhere
//! else, so a caller cannot claim a kind per-invocation — that is the property
//! the spec's permission boundary rests on.

use crate::store::writers::{lookup_by_name, Kind, WriterError, Writers};
use crate::store::Store;

/// Register `name`, or return its existing id when it is already registered
/// with this same kind. A kind mismatch is rejected — see `WriterError`.
///
/// Returns `Result<_, WriterError>` rather than `Result<_, String>`: unlike
/// `list`/`whoami` (whose only failure is an I/O error, always exit 1), a
/// mismatch here is exit 2 and an I/O failure is exit 1 — `main.rs` needs the
/// variant, not just a rendered message, to pick between them.
pub fn register(store: &Store, name: &str, kind: Kind) -> Result<String, WriterError> {
    let id = store.ensure_writer(name, kind)?;
    Ok(format!("{id}  {name} ({})", kind.as_str()))
}

/// One line per writer, sorted by name. The registry is a `BTreeMap` keyed by
/// id, so id order is not name order — sort explicitly.
pub fn render_list(all: &Writers) -> String {
    if all.writers.is_empty() {
        return "no writers registered".to_string();
    }
    let mut rows: Vec<(&str, &str, &str)> = all
        .writers
        .iter()
        .map(|(id, w)| (w.name.as_str(), w.kind.as_str(), id.as_str()))
        .collect();
    rows.sort_unstable();
    rows.iter()
        .map(|(name, kind, id)| format!("{name}  {kind}  {id}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn list(store: &Store) -> Result<String, String> {
    store
        .writers()
        .map(|all| render_list(&all))
        .map_err(|e| e.to_string())
}

/// What `name` resolves to. Reports plainly when the name is not registered
/// yet rather than inventing a kind for it.
pub fn render_whoami(all: &Writers, name: &str) -> String {
    match lookup_by_name(all, name) {
        Some((id, kind)) => format!("{name}  {}  {id}", kind.as_str()),
        None => format!("{name}  (not registered — 'cassette writer register' first)"),
    }
}

pub fn whoami(store: &Store, name: &str) -> Result<String, String> {
    store
        .writers()
        .map(|all| render_whoami(&all, name))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::writers::Writer;

    fn writers() -> Writers {
        let mut all = Writers::default();
        all.writers.insert(
            "zzz-id".to_string(),
            Writer {
                name: "alice".to_string(),
                kind: Kind::Human,
                created: "2026-09-14T09:00:00Z".to_string(),
            },
        );
        all.writers.insert(
            "aaa-id".to_string(),
            Writer {
                name: "bot".to_string(),
                kind: Kind::Agent,
                created: "2026-09-14T09:01:00Z".to_string(),
            },
        );
        all
    }

    #[test]
    fn list_sorts_by_name_not_by_id() {
        // The registry is keyed by id, and "aaa-id" (bot) sorts before
        // "zzz-id" (alice) — so relying on map order would list them backwards.
        let out = render_list(&writers());
        let alice = out.find("alice").expect("alice");
        let bot = out.find("bot").expect("bot");
        assert!(alice < bot, "sorted by name, not id: {out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&Writers::default()), "no writers registered");
    }

    #[test]
    fn whoami_reports_the_registered_kind() {
        let out = render_whoami(&writers(), "bot");
        assert!(out.contains("agent"), "{out}");
        assert!(out.contains("aaa-id"), "{out}");
    }

    #[test]
    fn whoami_is_plain_about_an_unregistered_name() {
        let out = render_whoami(&writers(), "nobody");
        assert!(out.contains("not registered"), "{out}");
        assert!(
            !out.contains("human") && !out.contains("agent"),
            "must not invent a kind: {out}"
        );
    }

    #[test]
    fn register_round_trips_through_a_real_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let msg = register(&store, "bot", Kind::Agent).expect("register");
        assert!(msg.contains("bot"), "{msg}");
        assert!(msg.contains("agent"), "{msg}");

        let listed = list(&store).expect("list");
        assert!(listed.contains("bot"), "{listed}");
        assert!(listed.contains("agent"), "{listed}");

        let who = whoami(&store, "bot").expect("whoami");
        assert!(who.contains("agent"), "{who}");
    }

    #[test]
    fn register_reports_a_kind_mismatch_and_leaves_the_registry_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        register(&store, "bot", Kind::Agent).expect("first registration");
        match register(&store, "bot", Kind::Human) {
            Err(WriterError::KindMismatch { name, .. }) => assert_eq!(name, "bot"),
            other => panic!("expected KindMismatch, got {other:?}"),
        }
        assert_eq!(store.writers().expect("read").writers.len(), 1);
    }

    #[test]
    fn whoami_on_a_store_with_no_registry_yet_is_plain_not_invented() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let who = whoami(&store, "nobody").expect("whoami");
        assert!(who.contains("not registered"), "{who}");
    }
}
