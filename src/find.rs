use chrono::NaiveDateTime;

use crate::queue::json::split_sides;
use crate::store::{Store, StoredCassette};

/// One session as `cassette find` shows it.
pub struct NoteEntry {
    /// Openable form shown in the listing: the session id. Queries match
    /// this and the session's own content, never a directory part — there
    /// is none any more.
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
/// capped at `MAX_LISTED` with a "… N more" hint.
pub fn render(entries: &[NoteEntry], query: Option<&str>) -> String {
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
    out.push_str("\nresume one: cassette resume <name>");
    out
}

/// One `NoteEntry` per session in the store: the session id as its openable
/// form, its date from `session.toml`'s `created` (local time), words and
/// topics summed/collected across its cassettes, and a preview from the
/// highest-priority one.
///
/// The legacy notes dir is deliberately not consulted — see the design's
/// decision 7. A session whose `created` timestamp fails to parse is
/// skipped, the same treatment `Store::list_sessions` gives a `session.toml`
/// that fails to parse at all.
pub fn scan_store(store: &Store) -> Vec<NoteEntry> {
    let Ok(sessions) = store.list_sessions() else {
        return Vec::new();
    };
    sessions
        .into_iter()
        .filter_map(|(id, meta)| {
            let date = chrono::DateTime::parse_from_rfc3339(&meta.created)
                .ok()?
                .with_timezone(&chrono::Local)
                .naive_local();
            let scan = store.scan_session(&id).ok()?;
            let (topics, preview) = topics_and_preview(&scan.cassettes);
            let words = session_word_count(&scan.cassettes);
            let haystack = format!(
                "{id}\n{}",
                scan.cassettes
                    .iter()
                    .map(|c| c.body.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
            .to_lowercase();
            Some(NoteEntry {
                path: id,
                date,
                words,
                topics,
                preview,
                haystack,
            })
        })
        .collect()
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
    fn entry(path: &str, date: &str, words: usize) -> NoteEntry {
        let preview = format!("body of {path}");
        NoteEntry {
            path: path.to_string(),
            date: dt(date),
            words,
            topics: Vec::new(),
            haystack: format!("{path}\n{preview}").to_lowercase(),
            preview,
        }
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

        let entries = scan_store(&store);
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
    }

    #[test]
    fn render_sorts_newest_first_with_footer() {
        let entries = [
            entry("old.md", "2026-07-01T08:00:00", 10),
            entry("new.md", "2026-07-13T09:12:00", 412),
        ];
        let out = render(&entries, None);
        let new_pos = out.find("new.md").unwrap();
        let old_pos = out.find("old.md").unwrap();
        assert!(new_pos < old_pos, "{out}");
        assert!(
            out.contains("2026-07-13 09:12    412 words  new.md"),
            "{out}"
        );
        assert!(out.contains("    body of new.md"), "{out}");
        assert!(out.ends_with("resume one: cassette resume <name>"), "{out}");
    }

    #[test]
    fn render_shows_topics() {
        let mut e = entry("d.md", "2026-07-13T09:12:00", 5);
        e.topics = vec!["gratitude".to_string()];
        let out = render(std::slice::from_ref(&e), None);
        assert!(out.contains("d.md — gratitude"), "{out}");
    }

    #[test]
    fn render_filters_case_insensitively() {
        let entries = [
            entry("morning.md", "2026-07-13T09:12:00", 10),
            entry("evening.md", "2026-07-12T21:00:00", 10),
        ];
        let out = render(&entries, Some("MORNING"));
        assert!(out.contains("morning.md"), "{out}");
        assert!(!out.contains("evening.md"), "{out}");
        assert_eq!(render(&entries, Some("zzz")), "no notes match 'zzz'");
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
        let out = render(&entries, None);
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
            render(&[], None),
            "no notes yet — the first session starts the count"
        );
        assert_eq!(
            render(&[], Some("x")),
            "no notes yet — the first session starts the count"
        );
    }

    #[test]
    fn render_shows_the_full_path() {
        let mut e = entry("new.md", "2026-07-13T09:12:00", 412);
        e.path = "~/.local/share/cassette/notes/new.md".into();
        let out = render(std::slice::from_ref(&e), None);
        assert!(
            out.contains("2026-07-13 09:12    412 words  ~/.local/share/cassette/notes/new.md"),
            "{out}"
        );
    }

    #[test]
    fn filter_ignores_the_directory_part() {
        let mut e = entry("morning.md", "2026-07-13T09:12:00", 10);
        e.path = "~/.local/share/cassette/notes/morning.md".into();
        let out = render(std::slice::from_ref(&e), Some("notes"));
        assert_eq!(
            out, "no notes match 'notes'",
            "directory names must not satisfy queries"
        );
    }
}
