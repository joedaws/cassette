use chrono::NaiveDateTime;

use crate::output;

/// One saved note as `cassette find` shows it.
pub struct NoteEntry {
    pub name: String,
    pub date: NaiveDateTime,
    pub words: usize,
    pub topics: Vec<String>,
    pub preview: String,
    pub draft: bool,
    haystack: String,
}

const PREVIEW_CHARS: usize = 72;

/// Frontmatter `date:` with or without a time part; anything else is None.
fn parse_date(v: &str) -> Option<NaiveDateTime> {
    let v = v.trim();
    NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(v.get(..10)?, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
        })
}

/// Build a note's listing entry from its content; `fallback` (the file's
/// mtime) stands in when the frontmatter has no parseable date, so notes the
/// writer didn't produce still browse.
pub fn parse_entry(name: &str, content: &str, fallback: NaiveDateTime) -> NoteEntry {
    let mut date = None;
    let mut words = 0;
    let mut in_frontmatter = content.lines().next() == Some("---");
    let mut past_opening = false;
    let mut topics = Vec::new();
    let mut preview = String::new();
    for line in content.lines() {
        if in_frontmatter {
            if !past_opening {
                past_opening = true;
                continue;
            }
            if line == "---" {
                in_frontmatter = false;
            } else if let Some(v) = line.strip_prefix("date:") {
                date = parse_date(v);
            } else if let Some(v) = line.strip_prefix("word_count:") {
                words = v.trim().parse().unwrap_or(0);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("# Cassette") {
            if let Some((_, topic)) = rest.split_once(" — ") {
                topics.push(topic.trim().to_string());
            }
        } else if preview.is_empty() && !line.trim().is_empty() && !line.starts_with('#') {
            preview = truncate(line.trim(), PREVIEW_CHARS);
        }
    }
    NoteEntry {
        name: name.to_string(),
        date: date.unwrap_or(fallback),
        words,
        topics,
        preview,
        draft: output::is_draft(content),
        haystack: format!("{}\n{}", name, content).to_lowercase(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    const NOTE: &str = "---\n\
        date: 2026-07-13T09:12:00\n\
        word_count: 412\n\
        cassettes: 2\n\
        ---\n\
        # Cassette 1 — gratitude\n\n\
        ## Side A\n\n\
        woke up thinking about the demo\n\n\
        # Cassette 2 — priorities\n\n\
        ## Side A\n\n\
        ship the find command\n";

    #[test]
    fn parse_entry_reads_frontmatter_topics_and_preview() {
        let e = parse_entry("2026-07-13.md", NOTE, dt("2000-01-01T00:00:00"));
        assert_eq!(e.name, "2026-07-13.md");
        assert_eq!(e.date, dt("2026-07-13T09:12:00"));
        assert_eq!(e.words, 412);
        assert_eq!(e.topics, vec!["gratitude", "priorities"]);
        assert_eq!(e.preview, "woke up thinking about the demo");
        assert!(!e.draft);
    }

    #[test]
    fn parse_entry_draft_and_date_only() {
        let note = "---\ndate: 2026-07-11\ndraft: true\nword_count: 188\n---\n\
                    # Cassette 1\n\n## Side A\n\ncan't sleep again\n";
        let e = parse_entry("late-night.md", note, dt("2000-01-01T00:00:00"));
        assert!(e.draft);
        assert_eq!(
            e.date,
            dt("2026-07-11T00:00:00"),
            "date-only lands on midnight"
        );
        assert!(e.topics.is_empty());
    }

    #[test]
    fn parse_entry_falls_back_to_mtime_and_skips_headings() {
        let e = parse_entry(
            "loose.md",
            "# just a heading\n\nreal first line\n",
            dt("2026-07-01T08:00:00"),
        );
        assert_eq!(e.date, dt("2026-07-01T08:00:00"), "no frontmatter date → fallback");
        assert_eq!(e.words, 0);
        assert_eq!(e.preview, "real first line", "headings never become previews");
    }

    #[test]
    fn parse_entry_truncates_long_previews() {
        let long = format!("---\ndate: 2026-07-13T09:12:00\n---\n{}\n", "x".repeat(100));
        let e = parse_entry("n.md", &long, dt("2000-01-01T00:00:00"));
        assert_eq!(e.preview.chars().count(), 73, "72 chars + ellipsis");
        assert!(e.preview.ends_with('…'));
    }

    #[test]
    fn parse_entry_no_body_text_means_no_preview() {
        let e = parse_entry(
            "n.md",
            "---\ndate: 2026-07-13T09:12:00\n---\n# Cassette 1\n\n## Side A\n\n\n",
            dt("2000-01-01T00:00:00"),
        );
        assert_eq!(e.preview, "");
    }
}
