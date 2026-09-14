//! Cassette and session identity: ULIDs, and the slug half of a file name.

/// Longest slug allowed in a file name; the ULID and `.md` follow it.
#[allow(dead_code)]
pub const SLUG_MAX: usize = 32;

/// Used when a topic yields no usable slug characters.
#[allow(dead_code)]
const SLUG_FALLBACK: &str = "cassette";

/// A fresh ULID: 26 Crockford base32 characters, roughly sortable by
/// creation time. Roughly is enough — the id is only ever a tiebreak, never
/// an ordering guarantee (see the spec's "Identity and file naming").
#[allow(dead_code)]
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

/// The filename-safe half of a cassette file name, derived from its topic at
/// creation and never recomputed afterward: `topic` in frontmatter stays the
/// source of truth for display, so a renamed topic leaves the file where it
/// is. ASCII alphanumerics are kept lowercased, every other run becomes a
/// single `-`, and a topic that leaves nothing behind (absent, blank, or
/// entirely non-ASCII) falls back to `cassette`.
#[allow(dead_code)]
pub fn slug(topic: Option<&str>) -> String {
    let mut out = String::with_capacity(SLUG_MAX);
    let mut pending_dash = false;
    for ch in topic.unwrap_or_default().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
            if out.len() == SLUG_MAX {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        return SLUG_FALLBACK.to_string();
    }
    out
}

/// `<slug>-<id>.md`. Priority is deliberately absent: encoding order in the
/// name would make every reprioritization a rename, and renames desynchronize
/// an flock from the path other writers resolve.
#[allow(dead_code)]
pub fn file_name(topic: Option<&str>, id: &str) -> String {
    format!("{}-{}.md", slug(topic), id)
}

/// Recover a cassette id from its file name — the segment after the LAST
/// dash, since slugs contain dashes of their own. `None` when the name is not
/// a `<slug>-<id>.md` pair.
#[allow(dead_code)]
pub fn id_from_file_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (_, id) = stem.rsplit_once('-')?;
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_twenty_six_char_ulids_and_unique() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 26, "ULIDs are 26 Crockford base32 chars: {a}");
        assert!(
            a.chars().all(|c| c.is_ascii_alphanumeric()),
            "no separators in a ULID: {a}"
        );
        assert_ne!(a, b, "two mints must differ");
    }

    #[test]
    fn slug_lowercases_and_dashes_the_topic() {
        assert_eq!(slug(Some("Gratitude")), "gratitude");
        assert_eq!(slug(Some("loose thoughts")), "loose-thoughts");
        assert_eq!(slug(Some("What's next?!")), "what-s-next");
    }

    #[test]
    fn slug_collapses_runs_and_trims_edges() {
        assert_eq!(slug(Some("  --a   b--  ")), "a-b");
        assert_eq!(slug(Some("a///b")), "a-b");
    }

    #[test]
    fn slug_falls_back_when_nothing_survives() {
        // No topic, empty topic, and topics with no ASCII alphanumerics all
        // need a name a filesystem accepts.
        assert_eq!(slug(None), "cassette");
        assert_eq!(slug(Some("")), "cassette");
        assert_eq!(slug(Some("   ")), "cassette");
        assert_eq!(slug(Some("日本語")), "cassette");
        assert_eq!(slug(Some("!!!")), "cassette");
    }

    #[test]
    fn slug_is_capped_without_a_trailing_dash() {
        let long = "a".repeat(100);
        assert_eq!(slug(Some(&long)).len(), SLUG_MAX);
        // Cutting mid-run must not leave a dangling separator.
        let words = "aaaaaaaaaa ".repeat(10);
        let s = slug(Some(&words));
        assert!(s.len() <= SLUG_MAX, "{s}");
        assert!(!s.ends_with('-'), "{s}");
    }

    #[test]
    fn file_name_joins_slug_and_id() {
        assert_eq!(
            file_name(Some("gratitude"), "01K5GR7T2M9WPD0000000000"),
            "gratitude-01K5GR7T2M9WPD0000000000.md"
        );
    }

    #[test]
    fn id_round_trips_through_the_file_name() {
        let id = new_id();
        let name = file_name(Some("loose thoughts"), &id);
        assert_eq!(id_from_file_name(&name), Some(id.as_str()));
    }

    #[test]
    fn id_recovery_handles_dashed_slugs_and_rejects_junk() {
        // The slug itself contains dashes, so recovery must take the LAST one.
        let id = "01K5GR7T2M9WPD0000000000AB";
        let name = format!("loose-thoughts-{id}.md");
        assert_eq!(id_from_file_name(&name), Some(id));
        assert_eq!(id_from_file_name("no-extension"), None);
        assert_eq!(id_from_file_name("nodash.md"), None);
    }
}
