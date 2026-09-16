//! Cassette and session identity: ULIDs, and the slug half of a file name.

/// Longest slug allowed in a file name; the ULID and `.md` follow it.
pub const SLUG_MAX: usize = 32;

/// Used when a topic yields no usable slug characters.
const SLUG_FALLBACK: &str = "cassette";

/// Length of a ULID in characters.
pub const ID_LEN: usize = 26;

/// A fresh ULID: 26 Crockford base32 characters, roughly sortable by
/// creation time. Roughly is enough — the id is only ever a tiebreak, never
/// an ordering guarantee (see the spec's "Identity and file naming").
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

/// Whether `s` is a well-formed ULID: exactly [`ID_LEN`] characters, each
/// from Crockford base32 — the ten digits plus the letters, minus `I`, `L`,
/// `O` and `U`, which Crockford strikes out to keep `1`/`I`/`l`, `0`/`O` and
/// `U` from being confused by a human reading an id aloud.
///
/// This is the **only** guard between a `--session` argument and a path
/// join: `Store::session_dir` joins the id straight onto the store root, so
/// an unvalidated `../../escaped` writes a cassette outside the store
/// entirely. Shape is checked here, beside `new_id`, because that is where
/// the crate's knowledge of what an id *is* lives; whether the session so
/// named exists is `Store::require_session`'s half of the question.
///
/// Case-insensitive, matching Crockford's own alphabet: `new_id` mints
/// uppercase, and a lowercased id is well-formed but simply names no
/// session on a case-sensitive filesystem — which the existence check then
/// reports as the missing session it is, rather than as a malformed id.
pub fn is_valid_id(s: &str) -> bool {
    // Byte-wise, not char-wise: every legal character is ASCII, so a
    // 26-byte string of legal bytes is exactly a 26-character id, and a
    // multi-byte character fails `is_crockford_byte` on its first byte.
    s.len() == ID_LEN && s.bytes().all(is_crockford_byte)
}

/// One Crockford base32 character. See [`is_valid_id`].
fn is_crockford_byte(b: u8) -> bool {
    match b {
        b'0'..=b'9' => true,
        b'A'..=b'Z' | b'a'..=b'z' => !matches!(b.to_ascii_uppercase(), b'I' | b'L' | b'O' | b'U'),
        _ => false,
    }
}

/// The filename-safe half of a cassette file name, derived from its topic at
/// creation and never recomputed afterward: `topic` in frontmatter stays the
/// source of truth for display, so a renamed topic leaves the file where it
/// is. ASCII alphanumerics are kept lowercased, every other run becomes a
/// single `-`, and a topic that leaves nothing behind (absent, blank, or
/// entirely non-ASCII) falls back to `cassette`.
pub fn slug(topic: Option<&str>) -> String {
    let mut out = String::with_capacity(SLUG_MAX);
    let mut pending_dash = false;
    for ch in topic.unwrap_or_default().chars() {
        if !ch.is_ascii_alphanumeric() {
            pending_dash = true;
            continue;
        }
        // Check the budget BEFORE pushing, counting the separator this char
        // would drag in with it. Checking afterwards for an exact `== SLUG_MAX`
        // lets a dash+char pair step from 31 straight to 33, and since the
        // length only grows the cap can then never be hit again.
        let needed = if pending_dash && !out.is_empty() {
            2
        } else {
            1
        };
        if out.len() + needed > SLUG_MAX {
            break;
        }
        if pending_dash && !out.is_empty() {
            out.push('-');
        }
        pending_dash = false;
        out.push(ch.to_ascii_lowercase());
    }
    if out.is_empty() {
        return SLUG_FALLBACK.to_string();
    }
    out
}

/// `<slug>-<id>.md`. Priority is deliberately absent: encoding order in the
/// name would make every reprioritization a rename, and renames desynchronize
/// an flock from the path other writers resolve.
pub fn file_name(topic: Option<&str>, id: &str) -> String {
    format!("{}-{}.md", slug(topic), id)
}

/// Recover a cassette id from its file name — the segment after the LAST
/// dash, since slugs contain dashes of their own. `None` when the name is not
/// a `<slug>-<id>.md` pair.
pub fn id_from_file_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (_, id) = stem.rsplit_once('-')?;
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
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
    fn a_minted_id_is_well_formed() {
        assert!(is_valid_id(&new_id()));
    }

    #[test]
    fn traversal_and_path_separators_are_not_ids() {
        // The whole point of the check: `--session` is joined onto the store
        // root, so anything that could climb out of it or name a nested path
        // must be rejected on shape alone, before any I/O.
        for bad in ["..", "../../escaped", "a/b", "/etc", "..\\x", "."] {
            assert!(!is_valid_id(bad), "must be rejected: {bad:?}");
        }
    }

    #[test]
    fn ids_are_exactly_twenty_six_characters() {
        assert!(!is_valid_id(""), "empty");
        assert!(!is_valid_id(&"A".repeat(ID_LEN - 1)), "25 chars");
        assert!(!is_valid_id(&"A".repeat(ID_LEN + 1)), "27 chars");
        assert!(is_valid_id(&"A".repeat(ID_LEN)), "26 chars");
    }

    #[test]
    fn crockford_excludes_i_l_o_and_u_in_either_case() {
        for excluded in ['I', 'L', 'O', 'U', 'i', 'l', 'o', 'u'] {
            let id = format!("{excluded}{}", "A".repeat(ID_LEN - 1));
            assert!(!is_valid_id(&id), "not Crockford base32: {id}");
        }
        // Lowercase is otherwise fine — Crockford's alphabet is
        // case-insensitive; naming no session is the existence check's
        // business, not this one's.
        assert!(is_valid_id(&"a".repeat(ID_LEN)));
        // And a multi-byte character never sneaks through on byte length.
        assert!(!is_valid_id(&format!("é{}", "A".repeat(ID_LEN - 2))));
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
    fn slug_cap_holds_when_a_word_boundary_straddles_it() {
        // Regression: a separator plus the char after it can step the length
        // from 31 to 33 in one iteration. An `== SLUG_MAX` check placed after
        // the push misses that and never fires again, uncapping the rest of
        // the topic. Any topic whose alnum run reaches 31 just before a
        // boundary reproduces it.
        let topic = format!("{} {}", "a".repeat(31), "c".repeat(100));
        let s = slug(Some(&topic));
        assert!(s.len() <= SLUG_MAX, "cap bypassed: {} chars — {s}", s.len());
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
