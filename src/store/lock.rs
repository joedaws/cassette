//! Cassette locking: `flock` on a `.locks/<id>` sidecar.
//!
//! The lock is on the sidecar and never on the `.md`, because `flock` attaches
//! to an inode and an atomic write replaces the inode via `rename()` — locking
//! the cassette file would silently hand two writers the same "lock". The
//! anchor is never renamed and never deleted, so its inode is stable, and its
//! *existence* carries no meaning: lockedness is kernel state, tested by
//! attempting acquisition.

use crate::store::meta;

/// Who holds a lock. Written into the anchor after acquiring — an ordering the
/// lock itself serializes — and read by a blocked writer for its message.
///
/// Display-only. Stale contents after a crash are harmless, because lockedness
/// is decided by kernel `flock` state and never by these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub writer: String,
    pub name: String,
    pub pid: u32,
    /// RFC3339 UTC.
    pub since: String,
}

impl Attribution {
    /// This process, holding a lock as of now.
    pub fn for_now(writer: &str, name: &str) -> Attribution {
        Attribution {
            writer: writer.to_string(),
            name: name.to_string(),
            pid: std::process::id(),
            since: meta::now_utc(),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "writer={} name={} pid={} since={}",
            self.writer, self.name, self.pid, self.since
        )
    }

    /// Parse the rendered form. `None` for anything else — a crashed holder can
    /// leave arbitrary bytes here and the caller degrades to a generic message.
    ///
    /// Fields are located by their markers rather than by splitting on
    /// whitespace, because `name` is free text and may contain spaces.
    pub fn parse(line: &str) -> Option<Attribution> {
        let line = line.trim();
        let rest = line.strip_prefix("writer=")?;
        let (writer, rest) = rest.split_once(" name=")?;
        let (name, rest) = rest.split_once(" pid=")?;
        let (pid, since) = rest.split_once(" since=")?;
        Some(Attribution {
            writer: writer.to_string(),
            name: name.to_string(),
            pid: pid.parse().ok()?,
            since: since.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribution() -> Attribution {
        Attribution {
            writer: "01K5H2WRITERID000000000000".to_string(),
            name: "joseph".to_string(),
            pid: 4242,
            since: "2026-09-14T14:02:11Z".to_string(),
        }
    }

    #[test]
    fn attribution_round_trips() {
        let a = attribution();
        assert_eq!(Attribution::parse(&a.render()).expect("parses"), a);
    }

    #[test]
    fn rendered_form_matches_the_spec() {
        assert_eq!(
            attribution().render(),
            "writer=01K5H2WRITERID000000000000 name=joseph pid=4242 since=2026-09-14T14:02:11Z"
        );
    }

    #[test]
    fn a_name_containing_spaces_survives() {
        // Names are free text and the fields are space-separated, so parsing
        // must key off the field markers rather than splitting on whitespace.
        let mut a = attribution();
        a.name = "joseph the writer".to_string();
        let parsed = Attribution::parse(&a.render()).expect("parses");
        assert_eq!(parsed.name, "joseph the writer");
        assert_eq!(parsed.pid, 4242, "fields after the name must still parse");
    }

    #[test]
    fn a_trailing_newline_is_tolerated() {
        let a = attribution();
        let parsed = Attribution::parse(&format!("{}\n", a.render())).expect("parses");
        assert_eq!(parsed, a);
    }

    #[test]
    fn garbage_does_not_parse() {
        // A crashed holder can leave anything here. Contents are display-only,
        // so unparseable bytes must be None rather than a panic or a partial.
        assert!(Attribution::parse("").is_none());
        assert!(Attribution::parse("not an attribution").is_none());
        assert!(Attribution::parse("writer=x name=y pid=notanumber since=z").is_none());
        assert!(Attribution::parse("writer=x name=y").is_none());
    }

    #[test]
    fn for_now_stamps_this_process() {
        let a = Attribution::for_now("writer-1", "joseph");
        assert_eq!(a.writer, "writer-1");
        assert_eq!(a.name, "joseph");
        assert_eq!(a.pid, std::process::id());
        assert!(a.since.ends_with('Z'), "{}", a.since);
    }
}
