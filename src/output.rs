use std::io;
use std::path::Path;

use crate::app::App;

pub fn write_markdown(app: &App, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let content = format!("{}\n{}", build_frontmatter(app), build_body(app));
    std::fs::write(path, content)
}

fn build_frontmatter(app: &App) -> String {
    let dt: chrono::DateTime<chrono::Local> = app.started_at.into();
    let date_str = dt.format("%Y-%m-%dT%H:%M:%S").to_string();

    let mut fm = String::from("---\n");
    fm.push_str(&format!("date: {}\n", date_str));

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
