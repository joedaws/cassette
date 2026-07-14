use std::io;
use std::path::Path;

use crate::app::App;
use crate::cassette::Cassette;

/// `draft` marks an in-flight autosave (`draft: true` in the frontmatter);
/// the final save on quit clears it, so a surviving draft flag means the
/// session crashed and the note is offered for `--resume` on next launch.
/// `note_date` carries a resumed note's original `date:` so saving doesn't
/// restamp it with this session's start time; `None` for fresh notes.
pub fn write_markdown(
    app: &App,
    path: &Path,
    draft: bool,
    note_date: Option<&str>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let content = format!(
        "{}\n{}",
        build_frontmatter(app, draft, note_date),
        build_body(app)
    );
    std::fs::write(path, content)
}

/// Whether a note's frontmatter carries the autosave `draft: true` marker.
pub fn is_draft(content: &str) -> bool {
    frontmatter_lines(content).any(|l| l.trim() == "draft: true")
}

/// Parse a saved note back into cassettes: `# Cassette N — topic` headings
/// with `## Side A` / `## Side B` sections, the inverse of `build_body`.
/// Daily-note `## Session` headings are treated as boundaries, so resuming a
/// multi-session note flattens it into one cassette list (the words all
/// survive; the session headings do not). Empty when nothing matches.
pub fn parse_markdown(content: &str) -> Vec<Cassette> {
    // Skip the frontmatter block and the blank line after it.
    let body = match content.strip_prefix("---\n") {
        Some(rest) => rest
            .split_once("\n---\n")
            .map(|(_, b)| b.strip_prefix('\n').unwrap_or(b))
            .unwrap_or(content),
        None => content,
    };

    enum Ev {
        Cassette(Option<String>),
        SideA,
        SideB,
        Boundary,
    }
    let mut events: Vec<(Ev, String)> = Vec::new();
    for line in body.split_inclusive('\n') {
        let heading = line.trim_end_matches('\n');
        if let Some(rest) = heading.strip_prefix("# Cassette ") {
            let topic = rest.split_once(" — ").map(|(_, t)| t.trim().to_string());
            events.push((Ev::Cassette(topic), String::new()));
        } else if heading == "## Side A" {
            events.push((Ev::SideA, String::new()));
        } else if heading == "## Side B" {
            events.push((Ev::SideB, String::new()));
        } else if heading.starts_with("## Session ") {
            events.push((Ev::Boundary, String::new()));
        } else if let Some(last) = events.last_mut() {
            last.1.push_str(line);
        }
    }

    let mut sides: Vec<(Option<String>, String, String)> = Vec::new();
    for (ev, raw) in events {
        match ev {
            Ev::Cassette(topic) => sides.push((topic, String::new(), String::new())),
            Ev::SideA => {
                if let Some(c) = sides.last_mut() {
                    c.1 = section_text(&raw);
                }
            }
            Ev::SideB => {
                if let Some(c) = sides.last_mut() {
                    c.2 = section_text(&raw);
                }
            }
            Ev::Boundary => {}
        }
    }
    sides
        .into_iter()
        .map(|(topic, a, b)| Cassette::from_sides(a, b, topic))
        .collect()
}

/// Recover a side's text from its raw section: `build_body` wraps every
/// section as `"\n{text}\n\n"`, so strip exactly that (leniently for
/// hand-edited files).
fn section_text(raw: &str) -> String {
    let s = raw.strip_prefix('\n').unwrap_or(raw);
    match s.strip_suffix("\n\n") {
        Some(t) => t.to_string(),
        None => s.trim_end_matches('\n').to_string(),
    }
}

/// Snapshot of an existing note that this session appends to (daily mode).
/// Parsed once at startup; every save rewrites `content` plus this session's
/// section, so repeated autosaves never double-count.
pub struct AppendBase {
    pub content: String,
    /// `word_count:` from the existing frontmatter (0 when absent).
    pub word_count: usize,
    /// `cassettes:` from the existing frontmatter (0 when absent).
    pub cassettes: usize,
    /// This session's ordinal: existing `## Session` headings + 2, because
    /// the first session of the day is written without a heading.
    pub session_no: usize,
}

pub fn parse_append_base(content: String) -> AppendBase {
    let fm_field = |name: &str| -> usize {
        frontmatter_lines(&content)
            .find_map(|l| l.strip_prefix(name)?.trim().parse().ok())
            .unwrap_or(0)
    };
    let sessions = content
        .lines()
        .filter(|l| l.starts_with("## Session "))
        .count();
    AppendBase {
        word_count: fm_field("word_count:"),
        cassettes: fm_field("cassettes:"),
        session_no: sessions + 2,
        content,
    }
}

/// The raw `date:` value from a note's frontmatter, if any.
pub fn frontmatter_date(content: &str) -> Option<String> {
    frontmatter_lines(content)
        .find_map(|l| l.strip_prefix("date:"))
        .map(|v| v.trim().to_string())
}

/// Lines inside the leading `---` frontmatter block (empty if there is none).
fn frontmatter_lines(content: &str) -> impl Iterator<Item = &str> {
    let mut lines = content.lines();
    let has_fm = lines.next() == Some("---");
    lines.take_while(move |l| has_fm && *l != "---")
}

/// Append this session to an existing note: the base content with its
/// frontmatter totals bumped, then a `## Session N — HH:MM` section holding
/// the usual cassette body.
pub fn write_markdown_appended(
    app: &App,
    path: &Path,
    base: &AppendBase,
    draft: bool,
) -> io::Result<()> {
    let mut in_fm = false;
    let mut out = String::new();
    for (i, line) in base.content.lines().enumerate() {
        if line == "---" {
            in_fm = i == 0;
            out.push_str("---\n");
            if i == 0 && draft {
                out.push_str("draft: true\n");
            }
            continue;
        }
        if in_fm && line.trim() == "draft: true" {
            continue; // stale marker from the base; re-added above if needed
        }
        if in_fm && line.starts_with("word_count:") {
            out.push_str(&format!(
                "word_count: {}\n",
                base.word_count + app.total_word_count()
            ));
        } else if in_fm && line.starts_with("cassettes:") {
            out.push_str(&format!(
                "cassettes: {}\n",
                base.cassettes + app.cassettes.len()
            ));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    let dt: chrono::DateTime<chrono::Local> = app.started_at.into();
    let out = format!(
        "{}\n## Session {} — {}\n\n{}",
        out.trim_end_matches('\n').to_string() + "\n",
        base.session_no,
        dt.format("%H:%M"),
        build_body(app)
    );
    std::fs::write(path, out)
}

fn build_frontmatter(app: &App, draft: bool, note_date: Option<&str>) -> String {
    let dt: chrono::DateTime<chrono::Local> = app.started_at.into();
    let date_str = match note_date {
        Some(d) => d.to_string(),
        None => dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
    };

    let mut fm = String::from("---\n");
    fm.push_str(&format!("date: {}\n", date_str));
    if draft {
        fm.push_str("draft: true\n");
    }

    if let Some(orig) = app.timer_original_secs {
        let label = if orig % 60 == 0 {
            format!("{}m", orig / 60)
        } else {
            format!("{}s", orig)
        };
        fm.push_str(&format!("timer: {}\n", label));
    }

    if let Some(goal) = app.word_goal {
        fm.push_str(&format!("word_goal: {}\n", goal));
    }

    fm.push_str(&format!("word_count: {}\n", app.total_word_count()));
    fm.push_str(&format!("cassettes: {}\n", app.cassettes.len()));
    fm.push_str("---\n");
    fm
}

fn build_body(app: &App) -> String {
    let mut out = String::new();
    for (i, cassette) in app.cassettes.iter().enumerate() {
        match &cassette.topic {
            Some(topic) => out.push_str(&format!("# Cassette {} — {}\n\n", i + 1, topic)),
            None => out.push_str(&format!("# Cassette {}\n\n", i + 1)),
        }
        out.push_str("## Side A\n\n");
        out.push_str(&cassette.side_a_text());
        out.push_str("\n\n");
        let side_b = cassette.side_b_text();
        if !side_b.trim().is_empty() {
            out.push_str("## Side B\n\n");
            out.push_str(&side_b);
            out.push_str("\n\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_text(text: &str) -> App {
        let mut app = App::new(None, None, None);
        app.modify_focused(|c| {
            for ch in text.chars() {
                c.insert(ch);
            }
        });
        app
    }

    #[test]
    fn frontmatter_date_reads_the_raw_value() {
        assert_eq!(
            frontmatter_date("---\ndate: 2026-07-09T09:25:57\nword_count: 5\n---\nbody\n"),
            Some("2026-07-09T09:25:57".to_string())
        );
        assert_eq!(frontmatter_date("no frontmatter here"), None);
        assert_eq!(frontmatter_date("---\nword_count: 5\n---\n"), None);
    }

    #[test]
    fn frontmatter_keeps_a_resumed_notes_date() {
        let app = app_with_text("hello again");
        let fm = build_frontmatter(&app, false, Some("2026-07-09T09:25:57"));
        assert!(
            fm.contains("date: 2026-07-09T09:25:57\n"),
            "resumed note keeps its original date: {fm}"
        );
    }

    #[test]
    fn body_labels_side_a_and_topic() {
        let mut app = app_with_text("thoughts");
        app.modify_focused(|c| c.topic = Some("morning pages".into()));
        let body = build_body(&app);
        assert!(body.contains("# Cassette 1 — morning pages\n"));
        assert!(body.contains("## Side A\n\nthoughts\n"));
        assert!(!body.contains("## Side B"), "empty side B stays out");
    }

    #[test]
    fn body_without_topic_keeps_plain_heading() {
        let app = app_with_text("hi");
        let body = build_body(&app);
        assert!(body.contains("# Cassette 1\n"));
        assert!(body.contains("## Side A\n"));
    }

    #[test]
    fn parse_markdown_round_trips_sides_topics_and_newlines() {
        let mut app = app_with_text("line one\n\nline three ends with newline\n");
        app.modify_focused(|c| {
            c.topic = Some("morning pages".into());
            c.flip();
            for ch in "\nside b starts blank".chars() {
                c.insert(ch);
            }
            c.flip();
        });
        app.add_cassette();
        app.modify_focused(|c| {
            for ch in "second cassette".chars() {
                c.insert(ch);
            }
        });

        let saved = format!("{}\n{}", build_frontmatter(&app, false, None), build_body(&app));
        let parsed = parse_markdown(&saved);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].topic.as_deref(), Some("morning pages"));
        assert_eq!(
            parsed[0].side_a_text(),
            "line one\n\nline three ends with newline\n"
        );
        assert_eq!(parsed[0].side_b_text(), "\nside b starts blank");
        assert_eq!(parsed[1].topic, None);
        assert_eq!(parsed[1].side_a_text(), "second cassette");
        assert_eq!(
            parsed[1].cursor_pos(),
            "second cassette".chars().count(),
            "cursor resumes at the end of side A"
        );
    }

    #[test]
    fn parse_markdown_flattens_daily_sessions_and_rejects_plain_files() {
        let daily = "---\ndate: x\n---\n# Cassette 1\n\n## Side A\n\nmorning\n\n## Session 2 — 14:00\n\n# Cassette 1\n\n## Side A\n\nafternoon\n\n";
        let parsed = parse_markdown(daily);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].side_a_text(), "morning");
        assert_eq!(parsed[1].side_a_text(), "afternoon");
        assert!(parse_markdown("just some notes\nnothing cassette-shaped\n").is_empty());
    }

    #[test]
    fn draft_flag_written_and_cleared() {
        let app = app_with_text("words");
        let draft = format!("{}\n{}", build_frontmatter(&app, true, None), build_body(&app));
        assert!(is_draft(&draft));
        let final_save = format!("{}\n{}", build_frontmatter(&app, false, None), build_body(&app));
        assert!(!is_draft(&final_save));
        assert!(
            !is_draft("no frontmatter\ndraft: true\n"),
            "marker only counts inside frontmatter"
        );
    }

    #[test]
    fn parse_append_base_reads_counts_and_session_no() {
        let content =
            "---\ndate: 2026-07-04T09:00:00\nword_count: 120\ncassettes: 2\n---\n# Cassette 1\n\n## Side A\n\nhello\n\n";
        let base = parse_append_base(content.into());
        assert_eq!(base.word_count, 120);
        assert_eq!(base.cassettes, 2);
        assert_eq!(base.session_no, 2, "no session headings yet");
        let later = format!("{content}## Session 2 — 09:00\n\nmore\n");
        assert_eq!(parse_append_base(later).session_no, 3);
    }

    #[test]
    fn parse_append_base_ignores_counts_outside_frontmatter() {
        let content = "---\nword_count: 7\n---\nbody says\nword_count: 999\n";
        assert_eq!(parse_append_base(content.into()).word_count, 7);
        let no_fm = "just text\n";
        assert_eq!(parse_append_base(no_fm.into()).word_count, 0);
    }

    #[test]
    fn append_bumps_totals_and_adds_session_heading() {
        let app = app_with_text("three new words");
        let base = parse_append_base(
            "---\ndate: 2026-07-04T09:00:00\nword_count: 10\ncassettes: 2\n---\n# Cassette 1\n\n## Side A\n\nmorning words\n\n"
                .into(),
        );
        let path = std::env::temp_dir().join(format!("cassette-append-{}.md", std::process::id()));
        write_markdown_appended(&app, &path, &base, false).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(out.contains("word_count: 13"), "10 base + 3 new:\n{out}");
        assert!(out.contains("cassettes: 3"), "2 base + 1 new:\n{out}");
        assert!(out.contains("morning words"), "base body kept");
        assert!(out.contains("\n## Session 2 — "), "session heading added");
        assert!(out.contains("three new words"), "new body appended");
        assert!(
            out.find("morning words") < out.find("## Session 2"),
            "session section comes after the base"
        );
    }

    #[test]
    fn body_includes_side_b_when_written() {
        let mut app = app_with_text("front");
        app.modify_focused(|c| {
            c.flip();
            for ch in "back".chars() {
                c.insert(ch);
            }
        });
        let body = build_body(&app);
        assert!(body.contains("## Side A\n\nfront\n"));
        assert!(body.contains("## Side B\n\nback\n"));
    }
}
