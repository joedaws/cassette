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

impl Writers {
    /// The id registered under `name`, if any.
    pub fn find_by_name(&self, name: &str) -> Option<&str> {
        self.writers
            .iter()
            .find(|(_, w)| w.name == name)
            .map(|(id, _)| id.as_str())
    }
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

/// The id for `name`, registering it on first sight. Idempotent: calling it
/// twice with the same name returns the same id rather than minting a second.
pub(crate) fn ensure(root: &Path, name: &str, kind: Kind) -> io::Result<String> {
    let mut all = read(root)?;
    if let Some(id) = all.find_by_name(name) {
        return Ok(id.to_string());
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
    fn find_by_name_locates_an_existing_writer() {
        let mut w = Writers::default();
        w.writers.insert(
            "id-1".to_string(),
            Writer {
                name: "joseph".to_string(),
                kind: Kind::Human,
                created: String::new(),
            },
        );
        assert_eq!(w.find_by_name("joseph"), Some("id-1"));
        assert_eq!(w.find_by_name("nobody"), None);
    }
}
