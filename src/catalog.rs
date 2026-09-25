//! The session catalog behind `cassette sessions`: one `NoteEntry` per
//! session in the store, with what the picker shows and what its `/` filter
//! matches. `Picker` stays pure over these entries; this module is the thin
//! part that reads the `Store`. (It was `find.rs` until the `find` command
//! was removed — the picker covered the same ground, interactively.)

use chrono::NaiveDateTime;

use crate::queue::json::split_sides;
use crate::store::{Store, StoredCassette};

/// One session as the `cassette sessions` picker shows it.
pub struct NoteEntry {
    /// The session id, which `cassette resume <id>` accepts as-is. Queries
    /// match this, the session's alias, its topics and its content — see
    /// `build_haystack`. (Named `path` until 5d; it has held an id since 5a,
    /// and the old name described the flat-note era.)
    pub id: String,
    /// The session's display label where it has one. `build_haystack` has
    /// always matched on this; the picker shows it in place of the id so a
    /// row never matches a word that appears nowhere on screen.
    pub alias: Option<String>,
    pub date: NaiveDateTime,
    pub words: usize,
    pub topics: Vec<String>,
    haystack: String,
}

impl NoteEntry {
    /// Whether this session matches an already-lowercased query, against
    /// the haystack `build_haystack` assembled.
    pub(crate) fn matches(&self, lowercased_query: &str) -> bool {
        self.haystack.contains(lowercased_query)
    }

    /// A fixture entry for tests in other modules, which cannot build
    /// `haystack` themselves. It goes through `build_haystack` for the
    /// reason that function's own doc gives: a fixture that assembles the
    /// field by hand tests a shape the real scanner never produces.
    #[cfg(test)]
    pub(crate) fn for_test(
        id: &str,
        alias: Option<&str>,
        date: NaiveDateTime,
        words: usize,
        topics: &[&str],
    ) -> Self {
        let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
        Self {
            haystack: build_haystack(id, alias, &topics, &format!("body of {id}")),
            id: id.to_string(),
            alias: alias.map(|a| a.to_string()),
            date,
            words,
            topics,
        }
    }
}

/// A session's total words, summed across its cassettes the same way
/// `Cassette::word_count` does — see `stats::session_word_count`, which this
/// deliberately matches so the two commands never disagree about a
/// session's length.
fn session_word_count(cassettes: &[StoredCassette]) -> usize {
    cassettes
        .iter()
        .map(|c| {
            let (side_a, side_b) = split_sides(&c.body);
            side_a.split_whitespace().count() + side_b.split_whitespace().count()
        })
        .sum()
}

/// Everything a picker filter is matched against, lowercased once here so
/// `NoteEntry::matches` can compare against a lowercased query.
///
/// The session **id**, its **alias**, its cassettes' **topics** and their
/// **bodies** — every string a row can show. A row that shows
/// `gratitude, priorities` and is then missed by a `gratitude` filter is a
/// discovery loop that closes on nothing, which is exactly what this indexed
/// before aliases and topics were added to it.
///
/// `scan_store` and the unit fixtures both build their haystacks through
/// this one function on purpose: a fixture that assembles the field by hand
/// tests a shape the real scanner never produces, and every gap in the real
/// one survives the suite.
pub(crate) fn build_haystack(
    id: &str,
    alias: Option<&str>,
    topics: &[String],
    bodies: &str,
) -> String {
    let mut out = String::from(id);
    if let Some(alias) = alias {
        out.push('\n');
        out.push_str(alias);
    }
    for topic in topics {
        out.push('\n');
        out.push_str(topic);
    }
    out.push('\n');
    out.push_str(bodies);
    out.to_lowercase()
}

/// One `NoteEntry` per session in the store — the session id as its openable
/// form, its date from `session.toml`'s `created` (local time), words and
/// topics summed/collected across its cassettes (in queue order) — plus the total count of cassette files across all
/// sessions that could not be read or parsed (`SessionScan::unreadable`, see
/// `Store::scan_session`).
///
/// That count must survive to the picker: a damaged cassette silently drops
/// its words and topics from a session's entry, and a total that is
/// quietly too low is worse than one that visibly says so — the same
/// reasoning `queue list`'s `N unreadable` line already acts on
/// (`queue::view::render_list`).
///
/// The legacy notes dir is deliberately not consulted — see the design's
/// decision 7. A session whose `created` timestamp fails to parse is
/// skipped, the same treatment `Store::list_sessions` gives a `session.toml`
/// that fails to parse at all.
pub(crate) fn scan_store(store: &Store) -> (Vec<NoteEntry>, usize, usize) {
    let Ok(listing) = store.list_sessions() else {
        return (Vec::new(), 0, 0);
    };
    let sessions = listing.sessions;
    let mut entries = Vec::new();
    let mut unreadable = 0usize;
    for (id, meta) in sessions {
        let Some(date) = chrono::DateTime::parse_from_rfc3339(&meta.created)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Local).naive_local())
        else {
            continue;
        };
        let Ok(scan) = store.scan_session(&id) else {
            continue;
        };
        unreadable += scan.unreadable();
        let topics: Vec<String> = scan
            .cassettes
            .iter()
            .filter_map(|c| c.meta.topic.clone())
            .collect();
        let words = session_word_count(&scan.cassettes);
        let haystack = build_haystack(
            &id,
            meta.alias.as_deref(),
            &topics,
            &scan
                .cassettes
                .iter()
                .map(|c| c.body.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        entries.push(NoteEntry {
            alias: meta.alias.clone(),
            id,
            date,
            words,
            topics,
            haystack,
        });
    }
    (entries, unreadable, listing.skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::session::SessionMeta;

    #[test]
    fn a_session_becomes_one_entry_with_its_topics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        for (body, priority, topic) in [
            ("first thing to say", 10, "gratitude"),
            ("second cassette body", 20, "priorities"),
        ] {
            let m = CassetteMeta {
                id: crate::store::ids::new(crate::store::ids::IdKind::Cassette),
                topic: Some(topic.to_string()),
                priority,
                status: Status::Open,
                locked_by: None,
                created_by: crate::store::ids::TEST_WRITER.to_string(),
                last_writer: crate::store::ids::TEST_WRITER.to_string(),
                updated_at: crate::store::meta::now_utc(),
            };
            store
                .add_cassette(&sid, &m, &format!("## Side A\n\n{body}\n"))
                .expect("add");
        }

        let (entries, unreadable, _) = scan_store(&store);
        assert_eq!(
            entries.len(),
            1,
            "one session is one entry, not one per cassette"
        );
        let e = &entries[0];
        assert_eq!(e.id, sid);
        assert_eq!(e.words, 7, "words sum across the session's cassettes");
        assert_eq!(
            e.topics,
            vec!["gratitude".to_string(), "priorities".to_string()],
            "topics come from every cassette, in queue order"
        );
        assert_eq!(unreadable, 0, "no damaged cassettes in this fixture");
    }

    #[test]
    fn scan_store_indexes_the_alias_and_the_topics() {
        // Against a real store, not a hand-built fixture: filtering by
        // `gratitude` must keep the session whose row shows that topic, and
        // `morning-pages` the one aliased that way; a word it never shows
        // must not.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&SessionMeta {
                alias: Some("morning-pages".to_string()),
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let m = CassetteMeta {
            id: crate::store::ids::new(crate::store::ids::IdKind::Cassette),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: crate::store::ids::TEST_WRITER.to_string(),
            last_writer: crate::store::ids::TEST_WRITER.to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\nsomething else entirely\n")
            .expect("add");

        let (entries, _, _) = scan_store(&store);
        assert_eq!(entries.len(), 1);
        for q in [
            "gratitude",
            "morning-pages",
            &sid.to_lowercase(),
            "else entirely",
        ] {
            assert!(entries[0].matches(q), "query '{q}' must match");
        }
        assert!(!entries[0].matches("nothing-like-this"));
    }

    #[test]
    fn scan_store_surfaces_unreadable_cassettes_rather_than_dropping_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let m = CassetteMeta {
            id: crate::store::ids::new(crate::store::ids::IdKind::Cassette),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: crate::store::ids::TEST_WRITER.to_string(),
            last_writer: crate::store::ids::TEST_WRITER.to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\none two three\n")
            .expect("add");
        // A cassette file with no parseable frontmatter: `Store::scan_session`
        // counts it as unreadable rather than erroring the whole scan out.
        std::fs::write(
            store.cassettes_dir(&sid).join("damaged.md"),
            "not a cassette file\n",
        )
        .expect("write damaged file");

        let (entries, unreadable, _) = scan_store(&store);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].words, 3,
            "the readable cassette's words still count"
        );
        assert_eq!(
            unreadable, 1,
            "the damaged cassette must be counted, not silently dropped"
        );
    }
}
