//! `session.toml`.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A session's own metadata. The id is the directory name, not a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMeta {
    /// Human-facing name; the id is displayed when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// RFC3339 UTC.
    pub created: String,
    /// Session defaults, mirroring the `-t` and `-w` flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_goal: Option<usize>,
}

pub(crate) fn read(path: &Path) -> io::Result<SessionMeta> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub(crate) fn write(path: &Path, m: &SessionMeta) -> io::Result<()> {
    let text = toml::to_string(m).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::atomic_write(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_toml_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        let m = SessionMeta {
            alias: Some("freewriting-2026-09-13".to_string()),
            created: "2026-09-13T09:25:57Z".to_string(),
            timer_secs: Some(600),
            word_goal: Some(500),
        };
        write(&path, &m).expect("write");
        assert_eq!(read(&path).expect("read"), m);
    }

    #[test]
    fn optional_fields_are_omitted_not_nulled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        let m = SessionMeta {
            alias: None,
            created: "2026-09-13T09:25:57Z".to_string(),
            timer_secs: None,
            word_goal: None,
        };
        write(&path, &m).expect("write");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(!text.contains("alias"), "absent keys stay absent: {text}");
        assert!(!text.contains("timer_secs"), "{text}");
        assert_eq!(read(&path).expect("read"), m);
    }

    #[test]
    fn a_session_with_only_created_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "created = \"2026-09-13T09:25:57Z\"\n").expect("write");
        let m = read(&path).expect("read");
        assert_eq!(m.created, "2026-09-13T09:25:57Z");
        assert_eq!(m.alias, None);
    }
}
