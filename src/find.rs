use chrono::NaiveDateTime;

use crate::queue::json::split_sides;
use crate::store::{Store, StoredCassette};

/// One session as `cassette find` shows it.
pub struct NoteEntry {
    /// Openable form shown in the listing: the session id, which
    /// `cassette resume <id>` accepts as-is. Queries match this, the
    /// session's alias, its topics and its content — see `build_haystack`.
    pub path: String,
    pub date: NaiveDateTime,
    pub words: usize,
    pub topics: Vec<String>,
    pub preview: String,
    haystack: String,
}

const PREVIEW_CHARS: usize = 72;

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
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

/// The first non-blank, non-heading line of a cassette's body, truncated for
/// the listing.
fn first_body_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| truncate(l, PREVIEW_CHARS))
        .unwrap_or_default()
}

/// A session's topics (each cassette's, in queue order) and its preview,
/// drawn from the highest-priority cassette. `Store::scan_session` already
/// returns cassettes in queue order — open by ascending priority, closed
/// last — so the first cassette in the slice is that one.
fn topics_and_preview(cassettes: &[StoredCassette]) -> (Vec<String>, String) {
    let topics = cassettes
        .iter()
        .filter_map(|c| c.meta.topic.clone())
        .collect();
    let preview = cassettes
        .first()
        .map(|c| first_body_line(&c.body))
        .unwrap_or_default();
    (topics, preview)
}

const MAX_LISTED: usize = 10;

/// The plain-text `cassette find` listing: newest first, optionally filtered,
/// capped at `MAX_LISTED` with a "… N more" hint. `unreadable` — the count of
/// cassette files `scan_store` could not read or parse — is appended as a
/// trailing `N unreadable` line when nonzero, the same presentation
/// `queue::view::render_list` uses: never invent a new one for the same
/// failure mode.
pub fn render(entries: &[NoteEntry], query: Option<&str>, unreadable: usize) -> String {
    if entries.is_empty() {
        return "no notes yet — the first session starts the count".into();
    }
    let mut matched: Vec<&NoteEntry> = match query {
        Some(q) => {
            let q = q.to_lowercase();
            entries.iter().filter(|e| e.haystack.contains(&q)).collect()
        }
        None => entries.iter().collect(),
    };
    if matched.is_empty() {
        return format!("no notes match '{}'", query.unwrap_or_default());
    }
    matched.sort_by_key(|e| std::cmp::Reverse(e.date));

    let mut out = String::new();
    for e in matched.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "{}  {:>5} words  {}",
            e.date.format("%Y-%m-%d %H:%M"),
            e.words,
            e.path
        ));
        if !e.topics.is_empty() {
            out.push_str(&format!(" — {}", e.topics.join(", ")));
        }
        out.push('\n');
        if !e.preview.is_empty() {
            out.push_str(&format!("    {}\n", e.preview));
        }
    }
    if matched.len() > MAX_LISTED {
        out.push_str(&format!(
            "… {} more — 'cassette find <text>' narrows the list\n",
            matched.len() - MAX_LISTED
        ));
    }
    out.push_str("\nresume one: cassette resume <id>");
    if unreadable > 0 {
        out.push_str(&format!("\n{unreadable} unreadable"));
    }
    out
}

/// Everything a `find` query is matched against, lowercased once here so
/// `render` can compare against a lowercased query.
///
/// The session **id**, its **alias**, its cassettes' **topics** and their
/// **bodies** — every string the listing itself can print. A row that prints
/// `— gratitude, priorities` and is then missed by `cassette find gratitude`
/// is a discovery loop that closes on nothing, which is exactly what this
/// indexed before aliases and topics were added to it.
///
/// `scan_store` and the unit fixtures both build their haystacks through
/// this one function on purpose: a fixture that assembles the field by hand
/// tests a shape the real scanner never produces, and every gap in the real
/// one survives the suite.
fn build_haystack(id: &str, alias: Option<&str>, topics: &[String], bodies: &str) -> String {
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
/// topics summed/collected across its cassettes, and a preview from the
/// highest-priority one — plus the total count of cassette files across all
/// sessions that could not be read or parsed (`SessionScan::unreadable`, see
/// `Store::scan_session`).
///
/// That count must survive to `render`: a damaged cassette silently drops
/// its words, topics and preview from a session's entry, and a total that is
/// quietly too low is worse than one that visibly says so — the same
/// reasoning `queue list`'s `N unreadable` line already acts on
/// (`queue::view::render_list`).
///
/// The legacy notes dir is deliberately not consulted — see the design's
/// decision 7. A session whose `created` timestamp fails to parse is
/// skipped, the same treatment `Store::list_sessions` gives a `session.toml`
/// that fails to parse at all.
pub fn scan_store(store: &Store) -> (Vec<NoteEntry>, usize) {
    let Ok(sessions) = store.list_sessions() else {
        return (Vec::new(), 0);
    };
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
        let (topics, preview) = topics_and_preview(&scan.cassettes);
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
            path: id,
            date,
            words,
            topics,
            preview,
            haystack,
        });
    }
    (entries, unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::session::SessionMeta;
    use chrono::NaiveDateTime;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    /// A fixture entry for exercising `render` directly, bypassing
    /// `scan_store` (and so the store entirely) the way `NoteMeta`'s test
    /// fixtures in `stats.rs` do.
    ///
    /// The haystack goes through `build_haystack`, the same function
    /// `scan_store` uses. It used to be assembled here by hand as
    /// `path\npreview` — a shape the real scanner never produced — and that
    /// is precisely why the suite could not see that aliases and topics were
    /// never indexed. A fixture that invents its own shape tests the
    /// fixture.
    fn entry_with(
        path: &str,
        date: &str,
        words: usize,
        alias: Option<&str>,
        topics: &[&str],
    ) -> NoteEntry {
        let preview = format!("body of {path}");
        let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
        NoteEntry {
            path: path.to_string(),
            date: dt(date),
            words,
            haystack: build_haystack(path, alias, &topics, &preview),
            topics,
            preview,
        }
    }

    fn entry(path: &str, date: &str, words: usize) -> NoteEntry {
        entry_with(path, date, words, None, &[])
    }

    #[test]
    fn first_body_line_skips_blanks_and_headings() {
        assert_eq!(
            first_body_line("## Side A\n\n\nreal first line\nmore\n"),
            "real first line"
        );
        assert_eq!(first_body_line("## Side A\n\n"), "");
    }

    #[test]
    fn first_body_line_truncates_long_previews() {
        let long = "x".repeat(100);
        assert_eq!(first_body_line(&long).chars().count(), 73, "72 + ellipsis");
        assert!(first_body_line(&long).ends_with('…'));
    }

    #[test]
    fn a_session_becomes_one_find_entry_with_topics_and_preview() {
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
                id: crate::store::ids::new_id(),
                topic: Some(topic.to_string()),
                priority,
                status: Status::Open,
                locked_by: None,
                created_by: "w".to_string(),
                last_writer: "w".to_string(),
                updated_at: crate::store::meta::now_utc(),
            };
            store
                .add_cassette(&sid, &m, &format!("## Side A\n\n{body}\n"))
                .expect("add");
        }

        let (entries, unreadable) = scan_store(&store);
        assert_eq!(
            entries.len(),
            1,
            "one session is one entry, not one per cassette"
        );
        let e = &entries[0];
        assert_eq!(e.path, sid);
        assert_eq!(e.words, 7, "words sum across the session's cassettes");
        assert_eq!(
            e.topics,
            vec!["gratitude".to_string(), "priorities".to_string()],
            "topics come from every cassette, in queue order"
        );
        assert_eq!(
            e.preview, "first thing to say",
            "preview comes from the highest-priority (first-in-queue) cassette"
        );
        assert_eq!(unreadable, 0, "no damaged cassettes in this fixture");
    }

    #[test]
    fn scan_store_indexes_the_alias_and_the_topics() {
        // Against a real store, not a hand-built fixture: `cassette find
        // gratitude` must return the session whose row prints `— gratitude`,
        // and `cassette find morning-pages` the one aliased that way.
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
            id: crate::store::ids::new_id(),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\nsomething else entirely\n")
            .expect("add");

        let (entries, _) = scan_store(&store);
        for q in ["gratitude", "morning-pages", &sid.to_lowercase()] {
            assert!(
                render(&entries, Some(q), 0).contains(&sid),
                "query '{q}' must match: {}",
                render(&entries, Some(q), 0)
            );
        }
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
            id: crate::store::ids::new_id(),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
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

        let (entries, unreadable) = scan_store(&store);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].words, 3,
            "the readable cassette's words still count"
        );
        assert_eq!(
            unreadable, 1,
            "the damaged cassette must be counted, not silently dropped"
        );

        let out = render(&entries, None, unreadable);
        assert!(
            out.contains("1 unreadable"),
            "the reader must see the count: {out}"
        );
    }

    #[test]
    fn render_sorts_newest_first_with_footer() {
        let entries = [
            entry("old.md", "2026-07-01T08:00:00", 10),
            entry("new.md", "2026-07-13T09:12:00", 412),
        ];
        let out = render(&entries, None, 0);
        let new_pos = out.find("new.md").unwrap();
        let old_pos = out.find("old.md").unwrap();
        assert!(new_pos < old_pos, "{out}");
        assert!(
            out.contains("2026-07-13 09:12    412 words  new.md"),
            "{out}"
        );
        assert!(out.contains("    body of new.md"), "{out}");
        assert!(out.ends_with("resume one: cassette resume <id>"), "{out}");
    }

    #[test]
    fn render_shows_topics() {
        let e = entry_with("d.md", "2026-07-13T09:12:00", 5, None, &["gratitude"]);
        let out = render(std::slice::from_ref(&e), None, 0);
        assert!(out.contains("d.md — gratitude"), "{out}");
    }

    #[test]
    fn a_query_matches_the_alias_and_the_topics_the_row_prints() {
        // The row prints `— gratitude, priorities`; a reader who types one
        // of those words back must land on this session. Same for the alias
        // they named it with.
        let e = entry_with(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "2026-07-13T09:12:00",
            5,
            Some("morning-pages"),
            &["gratitude", "priorities"],
        );
        let entries = std::slice::from_ref(&e);
        for q in ["gratitude", "PRIORITIES", "morning-pages", "01arz3ndek"] {
            assert!(
                render(entries, Some(q), 0).contains("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
                "query '{q}' must find the session whose listing row shows it"
            );
        }
        assert_eq!(
            render(entries, Some("nothing-like-this"), 0),
            "no notes match 'nothing-like-this'"
        );
    }

    #[test]
    fn render_filters_case_insensitively() {
        let entries = [
            entry("morning.md", "2026-07-13T09:12:00", 10),
            entry("evening.md", "2026-07-12T21:00:00", 10),
        ];
        let out = render(&entries, Some("MORNING"), 0);
        assert!(out.contains("morning.md"), "{out}");
        assert!(!out.contains("evening.md"), "{out}");
        assert_eq!(render(&entries, Some("zzz"), 0), "no notes match 'zzz'");
    }

    #[test]
    fn render_caps_at_ten_with_more_line() {
        let entries: Vec<NoteEntry> = (1..=12)
            .map(|i| {
                entry(
                    &format!("n{i:02}.md"),
                    &format!("2026-07-{i:02}T08:00:00"),
                    1,
                )
            })
            .collect();
        let out = render(&entries, None, 0);
        assert!(out.contains("n12.md") && out.contains("n03.md"), "{out}");
        assert!(!out.contains("n02.md"), "{out}");
        assert!(
            out.contains("… 2 more — 'cassette find <text>' narrows the list"),
            "{out}"
        );
    }

    #[test]
    fn render_empty_dir_message() {
        assert_eq!(
            render(&[], None, 0),
            "no notes yet — the first session starts the count"
        );
        assert_eq!(
            render(&[], Some("x"), 0),
            "no notes yet — the first session starts the count"
        );
    }

    #[test]
    fn render_shows_the_session_id_the_reader_can_resume() {
        // The listed form is the whole point of the listing: it is what
        // `cassette resume <id>` takes, so it is printed verbatim and the
        // footer names it.
        let e = entry("01ARZ3NDEKTSV4RRFFQ69G5FAV", "2026-07-13T09:12:00", 412);
        let out = render(std::slice::from_ref(&e), None, 0);
        assert!(
            out.contains("2026-07-13 09:12    412 words  01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "{out}"
        );
        assert!(out.ends_with("resume one: cassette resume <id>"), "{out}");
    }
}
