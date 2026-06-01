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
        out.push_str(&format!("# Cassette {}\n\n", i + 1));
        out.push_str(&cassette.text());
        out.push_str("\n\n");
    }
    out
}
