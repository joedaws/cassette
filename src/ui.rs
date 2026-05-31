use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::cassette::Cassette;

const HUB_FRAMES: [char; 4] = ['◓', '◐', '◒', '◑'];

// Focused cassette: black text on yellow; unfocused: white text on blue.
const FOCUSED_BG: Color = Color::Rgb(170, 170, 0);
const FOCUSED_FG: Color = Color::Rgb(0, 0, 0);
const UNFOCUSED_BG: Color = Color::Rgb(0, 0, 170);
const UNFOCUSED_FG: Color = Color::Rgb(255, 255, 255);

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let n = app.cassettes.len();
    let rpc = app.rows_per_cassette();

    let mut constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Length(rpc)).collect();
    constraints.push(Constraint::Length(1)); // bottom separator
    constraints.push(Constraint::Length(1)); // reel stats
    constraints.push(Constraint::Length(1)); // status message
    constraints.push(Constraint::Length(1)); // help text
    constraints.push(Constraint::Min(0)); // absorb leftover space

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let cw = app.cassette_width();

    for (i, cassette) in app.cassettes.iter().enumerate() {
        render_cassette(frame, chunks[i], cassette, i == app.focus_idx, cw, app.visible_lines);
    }

    let sep = "─".repeat(area.width as usize);
    frame.render_widget(Paragraph::new(sep), chunks[n]);

    render_reel_stats(frame, chunks[n + 1], app);

    let status = app.status_msg.as_deref().unwrap_or(" ");
    frame.render_widget(Paragraph::new(status), chunks[n + 2]);

    frame.render_widget(
        Paragraph::new("Tab: next  Shift+Tab: prev  Ctrl+N: new cassette  Esc: quit"),
        chunks[n + 3],
    );
}

/// Render one cassette: a separator row followed by `visible_lines` rows of text.
///
/// Text is character-wrapped at `cw` columns. The viewport scrolls so the cursor
/// stays at the bottom row. Lines further from the cursor (older text, toward the top)
/// fade toward the cassette background color.
fn render_cassette(
    frame: &mut Frame,
    area: Rect,
    cassette: &Cassette,
    focused: bool,
    cw: usize,
    visible_lines: usize,
) {
    let sep_char = if focused { '═' } else { '─' };
    let sep = sep_char.to_string().repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(sep),
        Rect { height: 1, ..area },
    );

    if area.height <= 1 {
        return;
    }
    let text_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    let (bg, fg) = if focused {
        (FOCUSED_BG, FOCUSED_FG)
    } else {
        (UNFOCUSED_BG, UNFOCUSED_FG)
    };

    let left_chars: Vec<char> = cassette.left.chars().collect();
    let right_chars: Vec<char> = cassette.right.chars().collect();
    // Display = left chars + cursor marker '│' + right chars
    let cursor_disp = left_chars.len();
    let display_len = left_chars.len() + 1 + right_chars.len();

    let cursor_line = cursor_disp / cw;
    // Traditional scroll: cursor stays at the bottom of the viewport.
    let scroll_top = cursor_line.saturating_sub(visible_lines.saturating_sub(1));

    let mut lines: Vec<Line> = Vec::with_capacity(visible_lines);

    for line_idx in scroll_top..scroll_top + visible_lines {
        let disp_start = line_idx * cw;
        let disp_end = (disp_start + cw).min(display_len);

        // Lines above the cursor fade toward the background; cursor line is full brightness.
        let dist_above = cursor_line.saturating_sub(line_idx);
        let fade_t = (1.0 - dist_above as f64 / visible_lines as f64).clamp(0.15, 1.0);
        let line_fg = lerp_color(fade_t, fg, bg);

        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(" ", Style::new().bg(bg)));

        for di in disp_start..disp_end {
            let (c, is_cursor) = if di < cursor_disp {
                (left_chars[di], false)
            } else if di == cursor_disp {
                ('│', true)
            } else {
                (right_chars[di - cursor_disp - 1], false)
            };

            let style = if is_cursor {
                // Invert colors at the cursor position.
                Style::new().fg(bg).bg(line_fg)
            } else {
                Style::new().fg(line_fg).bg(bg)
            };
            spans.push(Span::styled(c.to_string(), style));
        }

        // Pad the remainder of the line with the background color.
        let rendered = disp_end.saturating_sub(disp_start);
        if rendered < cw {
            spans.push(Span::styled(" ".repeat(cw - rendered), Style::new().bg(bg)));
        }

        spans.push(Span::styled(" ", Style::new().bg(bg)));
        lines.push(Line::from(spans));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::new().bg(bg)),
        text_area,
    );
}

fn render_reel_stats(frame: &mut Frame, area: Rect, app: &App) {
    let ratio = app.focused_cursor_ratio();
    let hub = HUB_FRAMES[app.reel_rotation % HUB_FRAMES.len()];
    let left_reel = format!("{}{}", hub, reel_pattern(ratio));
    let right_reel = format!("{}{}", reel_pattern(1.0 - ratio), hub);
    let stats = app.format_stats();

    let total_w = area.width as usize;
    let left_len = left_reel.chars().count();
    let right_len = right_reel.chars().count();
    let stats_len = stats.chars().count();
    let center_w = total_w.saturating_sub(left_len + right_len);
    let left_pad = center_w.saturating_sub(stats_len) / 2;
    let right_pad = center_w.saturating_sub(left_pad + stats_len);

    let stats_style = match app.timer_secs {
        Some(0) => Style::new().fg(Color::Black).bg(Color::Red),
        Some(_) => Style::new().fg(Color::Green),
        None => Style::new(),
    };

    let line = Line::from(vec![
        Span::raw(left_reel),
        Span::raw(" ".repeat(left_pad)),
        Span::styled(stats, stats_style),
        Span::raw(" ".repeat(right_pad)),
        Span::raw(right_reel),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

fn reel_pattern(r: f64) -> &'static str {
    if r <= 0.0 {
        "·····"
    } else if r <= 0.25 {
        "·░░░·"
    } else if r <= 0.50 {
        "░░░░░"
    } else if r <= 0.75 {
        "▒▒▒▒▒"
    } else {
        "▓▓▓▓▓"
    }
}

/// Linearly interpolate between two colors: t=1.0 gives `a`, t=0.0 gives `b`.
fn lerp_color(t: f64, a: Color, b: Color) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    Color::Rgb(
        lerp_u8(t, ar, br),
        lerp_u8(t, ag, bg),
        lerp_u8(t, ab, bb),
    )
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

fn lerp_u8(t: f64, a: u8, b: u8) -> u8 {
    (a as f64 * t + b as f64 * (1.0 - t)).round() as u8
}
