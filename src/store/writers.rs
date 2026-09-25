//! `writers.toml` — who is allowed to appear in `created_by` / `last_writer`.
//!
//! The read/write entry points are `pub(crate)` and reached through `Store`
//! rather than called directly: `Store` owns the data-dir root, so routing
//! every write through it is what keeps the root's `0700` creation in one
//! place. See `Store::writers`, `Store::ensure_writer`.
//!
//! Attribution is cooperative: this registry names writers, it does not
//! authenticate them. OS-level enforcement is explicitly deferred in the spec.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::ids;
use crate::store::meta;

/// File name of the registry, at the store root.
pub const WRITERS_FILE: &str = "writers.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Human,
    Agent,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Human => "human",
            Kind::Agent => "agent",
        }
    }
}

/// Failures of `ensure`, which declares a kind and may auto-create.
#[derive(Debug)]
pub enum EnsureError {
    /// This name is registered with a different `kind`. A distinct variant
    /// rather than an `Io(InvalidInput)` so a caller renders a usage error
    /// without matching on `io::ErrorKind`.
    KindMismatch {
        name: String,
        registered: Kind,
        requested: Kind,
    },
    /// The name has no characters left after trimming. A distinct variant
    /// rather than an `Io(InvalidInput)`, for the same reason as
    /// `KindMismatch`: the caller renders a usage error without matching on
    /// `io::ErrorKind`.
    EmptyName,
    Io(io::Error),
}

impl std::fmt::Display for EnsureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnsureError::KindMismatch {
                name,
                registered,
                requested,
            } => write!(
                f,
                "'{name}' is already registered as {} — cannot register as {}",
                registered.as_str(),
                requested.as_str()
            ),
            EnsureError::EmptyName => write!(f, "writer name cannot be blank"),
            EnsureError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for EnsureError {
    fn from(e: io::Error) -> EnsureError {
        EnsureError::Io(e)
    }
}

/// Failures of `resolve`, which declares no kind and may auto-create. It
/// cannot report a mismatch (it requests no kind) and cannot report an
/// unknown name (it creates one).
#[derive(Debug)]
pub enum ResolveError {
    EmptyName,
    Io(io::Error),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::EmptyName => write!(f, "writer name cannot be blank"),
            ResolveError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for ResolveError {
    fn from(e: io::Error) -> ResolveError {
        ResolveError::Io(e)
    }
}

/// Failures of `require_registered`, which never inserts.
#[derive(Debug)]
pub enum RequireError {
    EmptyName,
    /// `name` is not in the registry and this caller may not create it.
    Unregistered(String),
    Io(io::Error),
}

impl std::fmt::Display for RequireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequireError::EmptyName => write!(f, "writer name cannot be blank"),
            RequireError::Unregistered(name) => write!(
                f,
                "'{name}' is not a registered writer — run 'cassette writer register' first"
            ),
            RequireError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for RequireError {
    fn from(e: io::Error) -> RequireError {
        RequireError::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writer {
    pub name: String,
    pub kind: Kind,
    /// RFC3339 UTC.
    pub created: String,
}

/// The whole registry: writer id → writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Writers {
    #[serde(default)]
    pub writers: BTreeMap<String, Writer>,
}

/// A missing registry is an empty one — the first writer creates it. Every
/// OTHER read failure propagates: `read_to_string` also errors on
/// permission-denied, on a directory, and on non-UTF-8 content, and treating
/// those as "empty" is a data-loss path — the next `ensure` would write a
/// fresh single-entry registry over a file that was merely unreadable,
/// destroying every existing writer id.
pub(crate) fn read(root: &Path) -> io::Result<Writers> {
    let path = root.join(WRITERS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Writers::default()),
        Err(e) => return Err(e),
    };
    let all: Writers =
        toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Fail loudly rather than skip: a dropped entry would leave every
    // cassette crediting it resolving to a raw id, and make the sticky-lock
    // and authority checks treat a real writer as unknown.
    if let Some(bad) = all
        .writers
        .keys()
        .find(|k| crate::store::ids::check(crate::store::ids::IdKind::Writer, k).is_err())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "writers.toml: '{bad}' is not a writer id (expected wri_ followed by a 26-character ULID)"
            ),
        ));
    }
    Ok(all)
}

pub(crate) fn write(root: &Path, w: &Writers) -> io::Result<()> {
    crate::store::ensure_private_dir(root)?;
    let text = toml::to_string(w).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::atomic_write(&root.join(WRITERS_FILE), &text)
}

/// A writer id resolved to its registered display name, falling back to the
/// raw id when the registry doesn't know it — a damaged store, or a writer
/// that registered and was since removed by hand. `pub(crate)` so every
/// place that names a `locked_by` holder — `queue::write::write_permitted`'s
/// sticky-lock message and the TUI's own resolution (5b, `main.rs` and
/// `session_writer.rs`) — shares one lookup instead of reimplementing it.
pub(crate) fn display_name(writers: &Writers, id: &str) -> String {
    writers
        .writers
        .get(id)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// The id and kind registered under `name`, if any. Shared by `ensure` and
/// `resolve`, which differ in what they do with the result, not in how they
/// find it. The map is keyed by id, so this is a linear scan over values.
///
/// `pub(crate)` rather than private: `writer::render_whoami` needs this exact
/// shape (id + kind) too, and giving it a third copy of the same scan would
/// reintroduce the duplication `ensure`/`resolve` were collapsed onto this
/// helper to remove. The older `Writers::find_by_name` (id only, `pub`) was
/// removed rather than widened: once Task 5 needed the same lookup, keeping
/// two near-identical scans around — one narrow and public, one wider and
/// crate-only — would have been the very duplication this helper exists to
/// avoid, and `find_by_name` had no production caller of its own, only tests.
pub(crate) fn lookup_by_name(all: &Writers, name: &str) -> Option<(String, Kind)> {
    all.writers
        .iter()
        .find(|(_, w)| w.name == name)
        .map(|(id, w)| (id.clone(), w.kind))
}

/// The id for `name`, registering it on first sight. Idempotent for a matching
/// `kind`; a mismatch is rejected rather than silently updated — see
/// `EnsureError::KindMismatch`.
///
/// Trims `name` before doing anything else, and rejects an empty-after-trim
/// name outright: this is the store boundary, so it is the one place that
/// normalisation cannot be skipped by a caller that forgot to trim (the CLI,
/// or the Phase 5 TUI). Skipping the trim here let `" bot "` register as a
/// name distinct from `"bot"` — a silent identity split once anything trims
/// before comparing, which `--writer` already did.
pub(crate) fn ensure(root: &Path, name: &str, kind: Kind) -> Result<String, EnsureError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(EnsureError::EmptyName);
    }
    let mut all = read(root)?;
    if let Some((id, existing)) = lookup_by_name(&all, name) {
        if existing != kind {
            return Err(EnsureError::KindMismatch {
                name: name.to_string(),
                registered: existing,
                requested: kind,
            });
        }
        return Ok(id);
    }
    let id = ids::new(ids::IdKind::Writer);
    all.writers.insert(
        id.clone(),
        Writer {
            name: name.to_string(),
            kind,
            created: meta::now_utc(),
        },
    );
    write(root, &all)?;
    Ok(id)
}

/// The id and kind for `name`, registering it as a human on first sight.
/// Only for a name that is allowed to bootstrap itself — today, the `$USER`
/// default. See `require_registered` for a name that must already exist.
///
/// Unlike `ensure`, this declares nothing: an existing writer's kind is
/// returned as it stands, so a registered agent is not asked to claim it is a
/// human. Only `writer register` declares a kind, because only there does a
/// person choose one. A brand-new name defaults to human — the spec's
/// "auto-registered from $USER on first run" — and anyone who wants to be an
/// agent registers first.
///
/// Trims `name` first and rejects an empty-after-trim name, for the same
/// store-boundary reason as `ensure`.
///
/// Returns `Result<_, ResolveError>` rather than `io::Result`: it never
/// declares a kind, so a mismatch is not a state it can reach, but an empty
/// name is — and it needs `ResolveError::EmptyName` to say so distinctly
/// rather than via `io::ErrorKind`.
pub(crate) fn resolve(root: &Path, name: &str) -> Result<(String, Kind), ResolveError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ResolveError::EmptyName);
    }
    let mut all = read(root)?;
    if let Some((id, kind)) = lookup_by_name(&all, name) {
        return Ok((id, kind));
    }
    let id = ids::new(ids::IdKind::Writer);
    all.writers.insert(
        id.clone(),
        Writer {
            name: name.to_string(),
            kind: Kind::Human,
            created: meta::now_utc(),
        },
    );
    write(root, &all)?;
    Ok((id, Kind::Human))
}

/// The id and kind for `name`, requiring that it already be registered.
///
/// The strict counterpart to `resolve`: where `resolve` bootstraps an unknown
/// `$USER` as human on first run (the case the spec blesses), this is for a
/// name a caller named *explicitly* — `--writer` — where an unknown name is a
/// typo, not a first run. Auto-creating it would fail open, since the
/// default kind is `Human`, the *privileged* one; failing loudly here instead
/// means a typo is caught rather than silently spawning a second identity.
///
/// Trims `name` first and rejects an empty-after-trim name, same as `ensure`
/// and `resolve`.
pub(crate) fn require_registered(root: &Path, name: &str) -> Result<(String, Kind), RequireError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(RequireError::EmptyName);
    }
    let all = read(root)?;
    lookup_by_name(&all, name).ok_or_else(|| RequireError::Unregistered(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_key_that_is_not_a_writer_id_fails_loudly() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("writers.toml"),
            "[writers.01K5GQ00000000000000000001]\nname = \"joseph\"\nkind = \"human\"\ncreated = \"2026-09-24T09:00:00Z\"\n",
        )
        .expect("write");
        let err = read(dir.path()).expect_err("bare key");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("01K5GQ00000000000000000001"),
            "{err}"
        );
    }

    #[test]
    fn writers_toml_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = Writers::default();
        w.writers.insert(
            "wri_01K5GQ00000000000000000001".to_string(),
            Writer {
                name: "joseph".to_string(),
                kind: Kind::Human,
                created: "2026-09-13T09:20:00Z".to_string(),
            },
        );
        write(dir.path(), &w).expect("write");
        assert_eq!(read(dir.path()).expect("read"), w);
    }

    #[test]
    fn a_missing_registry_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read(dir.path()).expect("read").writers.is_empty());
    }

    #[test]
    fn an_unreadable_registry_errors_rather_than_reading_as_empty() {
        // Only NotFound may mean "empty". Any other read failure must
        // propagate: reporting an empty registry would let the next `ensure`
        // overwrite a real one, losing every writer id in the store. A
        // directory where the file should be is the portable way to make
        // `read_to_string` fail for a reason other than NotFound.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(WRITERS_FILE)).expect("mkdir");
        assert!(
            read(dir.path()).is_err(),
            "an unreadable registry must not read as empty"
        );
    }

    #[test]
    fn kind_serializes_lowercase() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = Writers::default();
        w.writers.insert(
            "01K5H3AGENTID00000000000000".to_string(),
            Writer {
                name: "refactor-agent".to_string(),
                kind: Kind::Agent,
                created: "2026-09-13T09:22:00Z".to_string(),
            },
        );
        write(dir.path(), &w).expect("write");
        let text = std::fs::read_to_string(dir.path().join(WRITERS_FILE)).expect("read");
        assert!(text.contains("kind = \"agent\""), "{text}");
    }

    #[test]
    fn ensure_creates_once_and_then_reuses_the_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        let again = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        assert_eq!(first, again, "the same name must not mint a second id");
        assert_eq!(read(dir.path()).expect("read").writers.len(), 1);
    }

    #[test]
    fn ensure_distinguishes_different_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let human = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        let agent = ensure(dir.path(), "refactor-agent", Kind::Agent).expect("ensure");
        assert_ne!(human, agent);
        let all = read(dir.path()).expect("read");
        assert_eq!(all.writers.len(), 2);
        assert_eq!(all.writers[&agent].kind, Kind::Agent);
    }

    #[test]
    fn re_registering_with_a_different_kind_is_rejected() {
        // `kind` is what the permission boundary rests on: an agent refuses to
        // close a cassette whose `locked_by` is set, a human may. Letting a
        // second registration silently flip it would let any caller change an
        // identity — including an agent re-registering itself as human.
        let dir = tempfile::tempdir().expect("tempdir");
        let id = ensure(dir.path(), "bot", Kind::Agent).expect("first");
        match ensure(dir.path(), "bot", Kind::Human) {
            Err(EnsureError::KindMismatch {
                name,
                registered,
                requested,
            }) => {
                assert_eq!(name, "bot");
                assert_eq!(registered, Kind::Agent);
                assert_eq!(requested, Kind::Human);
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
        // And the record must be untouched.
        let all = read(dir.path()).expect("read");
        assert_eq!(
            all.writers[&id].kind,
            Kind::Agent,
            "the registered kind stands"
        );
        assert_eq!(all.writers.len(), 1, "no second id was minted");
    }

    #[test]
    fn re_registering_with_the_same_kind_is_still_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure(dir.path(), "joseph", Kind::Human).expect("first");
        let again = ensure(dir.path(), "joseph", Kind::Human).expect("again");
        assert_eq!(first, again);
        assert_eq!(read(dir.path()).expect("read").writers.len(), 1);
    }

    #[test]
    fn resolve_returns_a_registered_agent_as_an_agent() {
        // The write path must not make a registered agent claim to be human —
        // that is what `queue write` was doing, and it locked agents out
        // entirely once mismatches started being rejected.
        let dir = tempfile::tempdir().expect("tempdir");
        let registered = ensure(dir.path(), "bot", Kind::Agent).expect("register");
        let (id, kind) = resolve(dir.path(), "bot").expect("resolve");
        assert_eq!(id, registered, "same writer, not a new id");
        assert_eq!(kind, Kind::Agent, "the registered kind is returned as-is");
    }

    #[test]
    fn resolve_auto_registers_an_unknown_name_as_human() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (id, kind) = resolve(dir.path(), "newcomer").expect("resolve");
        assert_eq!(kind, Kind::Human, "first sight defaults to human");
        let all = read(dir.path()).expect("read");
        assert_eq!(all.writers[&id].name, "newcomer");
        assert_eq!(all.writers.len(), 1);
    }

    #[test]
    fn resolve_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (first, _) = resolve(dir.path(), "joseph").expect("first");
        let (again, _) = resolve(dir.path(), "joseph").expect("again");
        assert_eq!(first, again, "no second id minted");
        assert_eq!(read(dir.path()).expect("read").writers.len(), 1);
    }

    #[test]
    fn a_blank_name_cannot_be_registered() {
        let dir = tempfile::tempdir().expect("tempdir");
        for blank in ["", "   ", "\t"] {
            match ensure(dir.path(), blank, Kind::Agent) {
                Err(EnsureError::EmptyName) => {}
                other => panic!("{blank:?} must be rejected, got {other:?}"),
            }
        }
        assert!(read(dir.path()).expect("read").writers.is_empty());
    }

    #[test]
    fn a_padded_name_normalises_to_one_identity() {
        // The identity-split bug: " bot " registered, then `--writer " bot "`
        // trims to "bot" and finds nothing — so a second writer is born.
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure(dir.path(), " bot ", Kind::Agent).expect("register padded");
        let again = ensure(dir.path(), "bot", Kind::Agent).expect("register trimmed");
        assert_eq!(first, again, "padded and trimmed must be the SAME writer");
        let all = read(dir.path()).expect("read");
        assert_eq!(all.writers.len(), 1, "no second identity");
        assert_eq!(all.writers[&first].name, "bot", "stored trimmed");
    }

    #[test]
    fn resolve_also_normalises() {
        let dir = tempfile::tempdir().expect("tempdir");
        let registered = ensure(dir.path(), "bot", Kind::Agent).expect("register");
        let (id, kind) = resolve(dir.path(), "  bot  ").expect("resolve padded");
        assert_eq!(id, registered);
        assert_eq!(kind, Kind::Agent, "not a fresh human identity");
    }

    #[test]
    fn require_registered_finds_a_known_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let registered = ensure(dir.path(), "bot", Kind::Agent).expect("register");
        let (id, kind) = require_registered(dir.path(), "  bot  ").expect("require");
        assert_eq!(id, registered, "same writer, and normalised");
        assert_eq!(kind, Kind::Agent);
    }

    #[test]
    fn require_registered_refuses_to_invent_an_unknown_name() {
        // The Finding 2 fix: an explicit `--writer` naming an unknown writer
        // must fail loudly (exit 2 upstream) rather than auto-creating a
        // human — the privileged kind — which would fail open on a typo.
        let dir = tempfile::tempdir().expect("tempdir");
        match require_registered(dir.path(), "nosuchwriter") {
            Err(RequireError::Unregistered(name)) => assert_eq!(name, "nosuchwriter"),
            other => panic!("expected Unregistered, got {other:?}"),
        }
        assert!(
            read(dir.path()).expect("read").writers.is_empty(),
            "must not have created anything"
        );
    }

    #[test]
    fn require_registered_rejects_a_blank_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        match require_registered(dir.path(), "   ") {
            Err(RequireError::EmptyName) => {}
            other => panic!("expected EmptyName, got {other:?}"),
        }
    }

    #[test]
    fn resolve_cannot_report_a_kind_mismatch() {
        // `resolve` declares no kind, so a mismatch is not one of its outcomes.
        // This is a type-level claim: it compiles only while ResolveError has
        // no KindMismatch variant, which is the whole point of the split.
        let dir = tempfile::tempdir().expect("tempdir");
        ensure(dir.path(), "bot", Kind::Agent).expect("ensure");
        let (id, kind) = resolve(dir.path(), "bot").expect("resolve");
        assert!(!id.is_empty());
        assert_eq!(kind, Kind::Agent, "resolve defers to the registered kind");

        match resolve(dir.path(), "   ") {
            Err(ResolveError::EmptyName) => {}
            other => panic!("a blank name is EmptyName, got {other:?}"),
        }
    }

    #[test]
    fn require_registered_reports_an_unknown_name_as_its_own_variant() {
        let dir = tempfile::tempdir().expect("tempdir");
        match require_registered(dir.path(), "ghost") {
            Err(RequireError::Unregistered(n)) => assert_eq!(n, "ghost"),
            other => panic!("expected Unregistered, got {other:?}"),
        }
    }
}
