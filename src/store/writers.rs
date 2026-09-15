//! `writers.toml` — who is allowed to appear in `created_by` / `last_writer`.
//!
//! The read/write entry points are `pub(crate)` and reached through `Store`
//! rather than called directly: `Store` owns the data-dir root, so routing
//! every write through it is what keeps the root's `0700` creation in one
//! place. See `Store::writers`, `Store::write_writers`, `Store::ensure_writer`.
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

/// Why a writer could not be resolved.
#[derive(Debug)]
pub enum WriterError {
    /// This name is registered with a different `kind`. A distinct variant
    /// rather than an `Io(InvalidInput)` so a caller renders a usage error
    /// without matching on `io::ErrorKind`.
    KindMismatch {
        name: String,
        registered: Kind,
        requested: Kind,
    },
    Io(io::Error),
}

impl std::fmt::Display for WriterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriterError::KindMismatch {
                name,
                registered,
                requested,
            } => write!(
                f,
                "'{name}' is already registered as {} — cannot register as {}",
                registered.as_str(),
                requested.as_str()
            ),
            WriterError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for WriterError {
    fn from(e: io::Error) -> WriterError {
        WriterError::Io(e)
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
    toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub(crate) fn write(root: &Path, w: &Writers) -> io::Result<()> {
    crate::store::ensure_private_dir(root)?;
    let text = toml::to_string(w).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::atomic_write(&root.join(WRITERS_FILE), &text)
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
/// `WriterError::KindMismatch`.
pub(crate) fn ensure(root: &Path, name: &str, kind: Kind) -> Result<String, WriterError> {
    let mut all = read(root)?;
    if let Some((id, existing)) = lookup_by_name(&all, name) {
        if existing != kind {
            return Err(WriterError::KindMismatch {
                name: name.to_string(),
                registered: existing,
                requested: kind,
            });
        }
        return Ok(id);
    }
    let id = ids::new_id();
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
///
/// Unlike `ensure`, this declares nothing: an existing writer's kind is
/// returned as it stands, so a registered agent is not asked to claim it is a
/// human. Only `writer register` declares a kind, because only there does a
/// person choose one. A brand-new name defaults to human — the spec's
/// "auto-registered from $USER on first run" — and anyone who wants to be an
/// agent registers first.
///
/// Returns `io::Result` rather than `Result<_, WriterError>`: this function
/// declares no kind, so a mismatch is not a state it can reach. Using the
/// shared error type would put an unreachable `KindMismatch` arm in every
/// caller, enforced by a comment — which is the same thing
/// `WriterError::KindMismatch` exists to avoid.
pub(crate) fn resolve(root: &Path, name: &str) -> io::Result<(String, Kind)> {
    let mut all = read(root)?;
    if let Some((id, kind)) = lookup_by_name(&all, name) {
        return Ok((id, kind));
    }
    let id = ids::new_id();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writers_toml_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = Writers::default();
        w.writers.insert(
            "01K5H2WRITERID000000000000".to_string(),
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
            Err(WriterError::KindMismatch {
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
}
