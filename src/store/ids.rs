//! Identity: typed ids (`ses_`/`cas_`/`wri_` + a ULID), and the slug half of
//! a cassette's file name.

/// Longest slug allowed in a file name; the id and `.md` follow it.
pub const SLUG_MAX: usize = 32;

/// Used when a topic yields no usable slug characters.
const SLUG_FALLBACK: &str = "cassette";

/// Length of the ULID part of an id, after the `<prefix>_`.
const ULID_LEN: usize = 26;

/// The three kinds of identity the store mints. Each id carries its kind as
/// a prefix — `ses_`, `cas_`, `wri_` — so a person or an agent can tell a
/// session id from a cassette id from a writer id at a glance, and every
/// entry point can say *which* kind it was handed instead of "not found".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Session,
    Cassette,
    Writer,
}

impl IdKind {
    const ALL: [IdKind; 3] = [IdKind::Session, IdKind::Cassette, IdKind::Writer];

    pub fn prefix(self) -> &'static str {
        match self {
            IdKind::Session => "ses",
            IdKind::Cassette => "cas",
            IdKind::Writer => "wri",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            IdKind::Session => "session",
            IdKind::Cassette => "cassette",
            IdKind::Writer => "writer",
        }
    }
}

/// A fresh id of `kind`: `<prefix>_<ULID>`, roughly sortable by creation
/// time within a kind. Roughly is enough — the id is only ever a tiebreak,
/// never an ordering guarantee (see the spec's "Identity and file naming").
pub fn new(kind: IdKind) -> String {
    format!("{}_{}", kind.prefix(), ulid::Ulid::generate())
}

/// Why a string is not an id of the expected kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// Well-formed, but a different kind.
    WrongKind { expected: IdKind, found: IdKind },
    /// Not `<known prefix>_<ULID>` at all — a bare ULID included.
    Malformed { expected: IdKind },
}

impl IdError {
    /// The one wording every entry point uses. `slot` names where the id
    /// was given (`--session`, `<ID>`, `--before`).
    pub fn message(&self, input: &str, slot: &str) -> String {
        match *self {
            IdError::WrongKind { expected, found } => format!(
                "`{input}` is a {} id; {slot} takes a {} id ({}_…)",
                found.noun(),
                expected.noun(),
                expected.prefix()
            ),
            IdError::Malformed { expected } => format!(
                "malformed {} id '{input}': expected {}_ followed by a {ULID_LEN}-character ULID",
                expected.noun(),
                expected.prefix()
            ),
        }
    }
}

/// The kind of a well-formed id: a known lowercase prefix, `_`, then exactly
/// 26 Crockford base32 characters — the ten digits plus the letters, minus
/// `I`, `L`, `O` and `U`, which Crockford strikes out to keep `1`/`I`/`l`,
/// `0`/`O` and `U` from being confused by a human reading an id aloud.
/// `None` for anything else, a bare ULID included.
///
/// This is the guard between an outside string and a path join
/// (`Store::session_dir` joins a session id straight onto the store root):
/// nothing that passes it can contain `/` or `..`. Whether the thing so
/// named exists is the caller's half of the question.
///
/// The ULID part is case-insensitive, matching Crockford's own alphabet:
/// `new` mints uppercase, and a lowercased id is well-formed but simply
/// names nothing on a case-sensitive filesystem — which the existence check
/// then reports as missing, rather than as malformed. The prefix is
/// lowercase only.
pub fn kind_of(s: &str) -> Option<IdKind> {
    let (prefix, ulid) = s.split_once('_')?;
    let kind = IdKind::ALL.into_iter().find(|k| k.prefix() == prefix)?;
    // Byte-wise, not char-wise: every legal character is ASCII, so 26 legal
    // bytes are exactly 26 characters, and a multi-byte character fails
    // `is_crockford_byte` on its first byte.
    (ulid.len() == ULID_LEN && ulid.bytes().all(is_crockford_byte)).then_some(kind)
}

/// `Ok(())` when `s` is a well-formed id of `kind`.
pub fn check(kind: IdKind, s: &str) -> Result<(), IdError> {
    match kind_of(s) {
        Some(found) if found == kind => Ok(()),
        Some(found) => Err(IdError::WrongKind {
            expected: kind,
            found,
        }),
        None => Err(IdError::Malformed { expected: kind }),
    }
}

/// One Crockford base32 character. See [`kind_of`].
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
/// dash, since slugs contain dashes of their own — and only when that
/// segment is a well-formed `cas_` id. `None` otherwise: a name that does
/// not end in a cassette id names no cassette. `_` never appears in a slug
/// (`slug` maps every non-alphanumeric run to `-`), so the id's own
/// separator cannot confuse the split.
pub fn id_from_file_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (_, id) = stem.rsplit_once('-')?;
    check(IdKind::Cassette, id).is_ok().then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_carry_their_kind_and_check_as_it() {
        for kind in [IdKind::Session, IdKind::Cassette, IdKind::Writer] {
            let id = new(kind);
            assert_eq!(id.len(), 30, "{id}");
            assert!(id.starts_with(&format!("{}_", kind.prefix())), "{id}");
            assert!(check(kind, &id).is_ok(), "{id}");
            assert_eq!(kind_of(&id), Some(kind));
        }
        assert_ne!(new(IdKind::Session), new(IdKind::Session));
    }

    #[test]
    fn a_wrong_kind_is_reported_as_such() {
        let c = new(IdKind::Cassette);
        match check(IdKind::Session, &c) {
            Err(IdError::WrongKind {
                expected: IdKind::Session,
                found: IdKind::Cassette,
            }) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bare_and_misshapen_ids_are_malformed() {
        let ulid = "01K5GQ2R8VXM3T0000000000AB";
        for bad in [
            ulid.to_string(),                          // bare ULID
            format!("ses-{ulid}"),                     // dash separator
            format!("SES_{ulid}"),                     // prefix is lowercase only
            format!("xyz_{ulid}"),                     // unknown prefix
            format!("ses_{}", &ulid[..25]),            // 25-char ULID
            format!("ses_{ulid}A"),                    // 27-char ULID
            format!("ses_{}", ulid.replace('K', "I")), // I is not Crockford
            format!("ses_é{}", &ulid[..24]),           // multi-byte never passes on length
            String::new(),
        ] {
            assert!(
                matches!(check(IdKind::Session, &bad), Err(IdError::Malformed { .. })),
                "{bad:?}"
            );
            assert_eq!(kind_of(&bad), None, "{bad:?}");
        }
        assert!(check(IdKind::Session, &format!("ses_{}", ulid.to_lowercase())).is_ok());
    }

    #[test]
    fn traversal_and_path_separators_are_not_ids() {
        // `--session` is joined onto the store root, so anything that could
        // climb out of it or name a nested path must fail on shape alone.
        for bad in [
            "..",
            "../../escaped",
            "a/b",
            "/etc",
            "..\\x",
            ".",
            "ses_../../x",
        ] {
            assert_eq!(kind_of(bad), None, "must be rejected: {bad:?}");
        }
    }

    #[test]
    fn messages_name_both_kinds_and_the_slot() {
        let c = "cas_01K5GQ2R8VXM3T0000000000AB";
        let m = check(IdKind::Session, c)
            .unwrap_err()
            .message(c, "--session");
        assert_eq!(
            m,
            "`cas_01K5GQ2R8VXM3T0000000000AB` is a cassette id; --session takes a session id (ses_…)"
        );
        let bare = "01K5GQ2R8VXM3T0000000000AB";
        let m = check(IdKind::Session, bare)
            .unwrap_err()
            .message(bare, "--session");
        assert_eq!(
            m,
            "malformed session id '01K5GQ2R8VXM3T0000000000AB': expected ses_ followed by a 26-character ULID"
        );
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
        let id = new(IdKind::Cassette);
        let name = file_name(Some("loose thoughts"), &id);
        assert_eq!(id_from_file_name(&name), Some(id.as_str()));
    }

    #[test]
    fn a_file_name_yields_its_id_only_when_it_is_a_cassette_id() {
        let id = "cas_01K5GR7T2M9WPD0000000000AB";
        assert_eq!(
            id_from_file_name(&format!("morning-pages-{id}.md")),
            Some(id)
        );
        assert_eq!(
            id_from_file_name("morning-pages-01K5GR7T2M9WPD0000000000AB.md"),
            None
        );
        assert_eq!(
            id_from_file_name("x-ses_01K5GR7T2M9WPD0000000000AB.md"),
            None
        );
    }

    #[test]
    fn id_recovery_handles_dashed_slugs_and_rejects_junk() {
        // The slug itself contains dashes, so recovery must take the LAST one.
        let id = "cas_01K5GR7T2M9WPD0000000000AB";
        let name = format!("loose-thoughts-{id}.md");
        assert_eq!(id_from_file_name(&name), Some(id));
        assert_eq!(id_from_file_name("no-extension"), None);
        assert_eq!(id_from_file_name("nodash.md"), None);
    }
}
