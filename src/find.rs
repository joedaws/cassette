use std::path::Path;

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

/// Read every `.md` note in the notes dir (non-recursive, like the writer),
/// using each file's mtime as the date fallback.
pub fn scan_notes_dir(dir: &Path) -> Vec<NoteEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| {
            let content = std::fs::read_to_string(e.path()).ok()?;
            let mtime: chrono::DateTime<chrono::Local> =
                e.metadata().and_then(|m| m.modified()).ok()?.into();
            let name = e.file_name().to_string_lossy().into_owned();
            Some(parse_entry(&name, &content, mtime.naive_local()))
        })
        .collect()
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
            e.name
        ));
        if e.draft {
            out.push_str(" (draft)");
        }
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
    out.push_str("\nresume one: cassette --resume <name>");
    out
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

    fn entry(name: &str, date: &str, words: usize) -> NoteEntry {
        parse_entry(
            name,
            &format!(
                "---\ndate: {date}\nword_count: {words}\n---\n# Cassette 1\n\n## Side A\n\nbody of {name}\n"
            ),
            dt("2000-01-01T00:00:00"),
        )
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
        assert!(out.contains("2026-07-13 09:12    412 words  new.md"), "{out}");
        assert!(out.contains("    body of new.md"), "{out}");
        assert!(out.ends_with("resume one: cassette --resume <name>"), "{out}");
    }

    #[test]
    fn render_marks_drafts_and_topics() {
        let e = parse_entry(
            "d.md",
            "---\ndate: 2026-07-13T09:12:00\ndraft: true\nword_count: 5\n---\n# Cassette 1 — gratitude\n\n## Side A\n\nhi\n",
            dt("2000-01-01T00:00:00"),
        );
        let out = render(std::slice::from_ref(&e), None);
        assert!(out.contains("d.md (draft) — gratitude"), "{out}");
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
            .map(|i| entry(&format!("n{i:02}.md"), &format!("2026-07-{i:02}T08:00:00"), 1))
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
    fn parse_entry_no_body_text_means_no_preview() {
        let e = parse_entry(
            "n.md",
            "---\ndate: 2026-07-13T09:12:00\n---\n# Cassette 1\n\n## Side A\n\n\n",
            dt("2000-01-01T00:00:00"),
        );
        assert_eq!(e.preview, "");
    }
}
