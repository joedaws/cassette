use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, Mode, GUTTER_WIDTH, MINIMIZED_ROWS};
use crate::cassette::{char_width, pos_to_row_col, wrap_spans, Cassette, Side};

/// Accent color for all "you are on side B" cues.
const SIDE_B_ACCENT: Color = Color::Yellow;

const HUB_FRAMES: [char; 4] = ['◓', '◐', '◒', '◑'];

// Focused cassette uses the terminal's default colors (issue #16); unfocused
// cassettes keep a colored background to visually separate themselves.
const UNFOCUSED_BG: Color = Color::Rgb(0, 0, 170);
const UNFOCUSED_FG: Color = Color::Rgb(255, 255, 255);

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let rpc = app.rows_per_cassette();

    // Only the window of cassettes starting at `cassette_scroll` is laid out:
    // the focused one full-height, the others minimized to their last line.
    let first = app
        .cassette_scroll
        .min(app.cassettes.len().saturating_sub(1));
    let n = app
        .visible_cassette_count()
        .min(app.cassettes.len() - first);

    let mut constraints: Vec<Constraint> = (first..first + n)
        .map(|i| {
            if i == app.focus_idx {
                Constraint::Length(rpc)
            } else {
                Constraint::Length(MINIMIZED_ROWS)
            }
        })
        .collect();
    constraints.push(Constraint::Length(1)); // bottom separator
    constraints.push(Constraint::Length(1)); // reel stats
    constraints.push(Constraint::Length(1)); // status / info line
    constraints.push(Constraint::Length(1)); // help text
    constraints.push(Constraint::Min(0)); // absorb leftover space

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let cw = app.cassette_width();

    for (chunk_idx, cassette_idx) in (first..first + n).enumerate() {
        let cassette = &app.cassettes[cassette_idx];
        if cassette_idx == app.focus_idx {
            render_cassette_focused(
                frame,
                chunks[chunk_idx],
                cassette,
                cw,
                app.visible_lines,
                app.mode,
            );
        } else {
            render_cassette_min(frame, chunks[chunk_idx], cassette, cw);
        }
    }

    let sep = "─".repeat(area.width as usize);
    frame.render_widget(Paragraph::new(sep), chunks[n]);

    // Overflow indicators for cassettes scrolled out of view.
    let (above, below) = app.hidden_cassettes();
    if above > 0 {
        render_overflow_hint(frame, chunks[0], above, '↑');
    }
    if below > 0 {
        render_overflow_hint(frame, chunks[n], below, '↓');
    }

    render_reel_stats(frame, chunks[n + 1], app);

    // Status messages take precedence; otherwise show a vim-style info line.
    let info;
    let status = match &app.status_msg {
        Some(m) => m.as_str(),
        None => {
            let c = &app.cassettes[app.focus_idx];
            let (ln, col) = c.cursor_line_col();
            let mode_str = match app.mode {
                Mode::Insert => "-- INSERT --",
                Mode::Normal => "-- NORMAL --",
            };
            let side = match c.side {
                Side::A => "",
                Side::B => "  ·  side B",
            };
            info = format!(
                "{}  ln {}, col {}  ·  {} chars  ·  cassette {}/{}{}",
                mode_str,
                ln,
                col,
                c.char_count(),
                app.focus_idx + 1,
                app.cassettes.len(),
                side
            );
            info.as_str()
        }
    };
    frame.render_widget(Paragraph::new(status), chunks[n + 2]);

    let help = match app.mode {
        Mode::Insert => "Esc:normal  Enter:newline  ^W:del word  ^F:flip side  Tab:next  ^N:new  ^C:quit",
        Mode::Normal => {
            "i/a/o:insert  hjkl:move  w/b:word  0/$:line  x/dd:del  u:undo  gg/G:jump  ^F:flip  q:quit"
        }
    };
    frame.render_widget(Paragraph::new(help), chunks[n + 3]);
}

/// Overlay a right-aligned "N more ↑/↓" hint on a separator row.
fn render_overflow_hint(frame: &mut Frame, sep_area: Rect, count: usize, arrow: char) {
    let hint = format!(" {} more {} ", count, arrow);
    let w = hint.chars().count() as u16;
    if sep_area.width <= w + 2 {
        return;
    }
    let hint_area = Rect {
        x: sep_area.x + sep_area.width - w - 2,
        y: sep_area.y,
        width: w,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::new().fg(Color::Black).bg(Color::Gray),
        ))),
        hint_area,
    );
}

/// A cassette's top separator row; on side B a yellow label is woven into it.
fn render_separator(frame: &mut Frame, area: Rect, ch: char, side: Side, label: &str) {
    let sep_area = Rect { height: 1, ..area };
    let total = area.width as usize;
    if side == Side::B {
        let used = 2 + label.chars().count();
        let line = Line::from(vec![
            Span::raw(ch.to_string().repeat(2)),
            Span::styled(label.to_string(), Style::new().fg(SIDE_B_ACCENT)),
            Span::raw(ch.to_string().repeat(total.saturating_sub(used))),
        ]);
        frame.render_widget(Paragraph::new(line), sep_area);
    } else {
        frame.render_widget(Paragraph::new(ch.to_string().repeat(total)), sep_area);
    }
}

/// Logical line number and whether the row starts that line, for each wrap span.
/// A gap between consecutive spans means a '\n' separated them.
fn span_line_numbers(spans: &[(usize, usize)]) -> Vec<(usize, bool)> {
    let mut out = Vec::with_capacity(spans.len());
    let mut line = 0usize;
    let mut prev_end: Option<usize> = None;
    for &(s, e) in spans {
        let is_start = prev_end.is_none_or(|pe| s > pe);
        if is_start {
            line += 1;
        }
        out.push((line, is_start));
        prev_end = Some(e);
    }
    out
}

fn fmt_line_no(n: usize) -> String {
    if n <= 999 {
        format!("{:>3} ", n)
    } else {
        "··· ".into()
    }
}

/// Terminal-default-colors fade: rows near the viewport edges step down in
/// brightness via modifiers instead of RGB lerp, since the real background
/// color of the terminal is unknown.
fn fade_style(t: f64) -> Style {
    if t >= 0.9 {
        Style::new()
    } else if t >= 0.55 {
        Style::new().add_modifier(Modifier::DIM)
    } else if t >= 0.3 {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)
    }
}

/// Render the focused cassette: a `═` separator followed by `visible_lines`
/// rows of word-wrapped text on the terminal's default background, with a
/// line-number gutter. The viewport scrolls typewriter-style (cursor row
/// centered, clamped at the ends) and rows fade toward both viewport edges.
fn render_cassette_focused(
    frame: &mut Frame,
    area: Rect,
    cassette: &Cassette,
    cw: usize,
    visible_lines: usize,
    mode: Mode,
) {
    render_separator(frame, area, '═', cassette.side, "╡ SIDE B ╞");

    if area.height <= 1 {
        return;
    }
    let text_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    // Insert mode shows the cursor as a dedicated bar cell between left and
    // right. Normal mode overlays a block on the char under the cursor instead,
    // so columns stay true; the text may reflow by one cell on mode switch.
    let cursor_disp = cassette.cursor_pos();
    let mut display: Vec<char> = cassette.left.chars().collect();
    if mode == Mode::Insert {
        display.push('│');
    }
    display.extend(cassette.right.chars());

    let row_spans = wrap_spans(&display, cw);
    let line_nums = span_line_numbers(&row_spans);
    let (cursor_row, _) = pos_to_row_col(&row_spans, cursor_disp);

    // Typewriter scroll: keep the cursor row centered, clamped so the viewport
    // never scrolls past the last row of text. While appending at the end this
    // degenerates to the classic cursor-at-bottom behavior.
    let scroll_top = cursor_row
        .saturating_sub(visible_lines / 2)
        .min(row_spans.len().saturating_sub(visible_lines));

    // Rows this many steps from a viewport edge (or closer) fade toward the background.
    let fade_zone = (visible_lines / 2).max(1);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_lines);

    for vp_row in 0..visible_lines {
        let line_idx = scroll_top + vp_row;

        // Fade toward both viewport edges; the cursor row stays at full brightness.
        let t_top = (vp_row + 1) as f64 / (fade_zone + 1) as f64;
        let t_bot = (visible_lines - vp_row) as f64 / (fade_zone + 1) as f64;
        let fade_t = if line_idx == cursor_row {
            1.0
        } else {
            t_top.min(t_bot).clamp(0.15, 1.0)
        };
        let text_style = fade_style(fade_t);
        // The gutter doubles as a persistent side indicator while writing.
        let gutter_style = match cassette.side {
            Side::A => Style::new().fg(Color::DarkGray),
            Side::B => Style::new().fg(SIDE_B_ACCENT),
        };

        let mut spans: Vec<Span> = vec![Span::raw(" ")];

        let gutter = match (row_spans.get(line_idx), line_nums.get(line_idx)) {
            (Some(_), Some(&(n, true))) => fmt_line_no(n),
            (Some(_), _) => " ".repeat(GUTTER_WIDTH),
            // vim-style '~' marker for rows past the end of the text
            (None, _) => format!("{:>2}  ", '~'),
        };
        spans.push(Span::styled(gutter, gutter_style));

        let mut rendered = 0;
        if let Some(&(disp_start, disp_end)) = row_spans.get(line_idx) {
            for (offset, &ch) in display[disp_start..disp_end].iter().enumerate() {
                // Insert mode: the bar cell. Normal mode: block over the char.
                let is_cursor = disp_start + offset == cursor_disp;
                let style = if is_cursor {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    text_style
                };
                spans.push(Span::styled(ch.to_string(), style));
                rendered += char_width(ch);
            }
            // Normal-mode cursor at the row's end (before a '\n' or at the end
            // of the text) has no char to sit on: draw a block on a space.
            if mode == Mode::Normal && line_idx == cursor_row && cursor_disp >= disp_end {
                spans.push(Span::styled(
                    " ",
                    Style::new().add_modifier(Modifier::REVERSED),
                ));
                rendered += 1;
            }
        }

        if rendered < cw {
            spans.push(Span::raw(" ".repeat(cw - rendered)));
        }
        spans.push(Span::raw(" "));
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), text_area);
}

/// Render an unfocused cassette minimized to a `─` separator plus its last
/// text row, keeping the colored background and showing the line number.
fn render_cassette_min(frame: &mut Frame, area: Rect, cassette: &Cassette, cw: usize) {
    render_separator(frame, area, '─', cassette.side, "╡ B ╞");

    if area.height <= 1 {
        return;
    }
    let text_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };

    let (bg, fg) = (UNFOCUSED_BG, UNFOCUSED_FG);
    let chars: Vec<char> = cassette
        .left
        .chars()
        .chain(cassette.right.chars())
        .collect();
    let row_spans = wrap_spans(&chars, cw);
    let line_nums = span_line_numbers(&row_spans);
    let last = row_spans.len() - 1;
    let (line_no, _) = line_nums[last];
    let (start, end) = row_spans[last];

    let gutter_fg = lerp_color(0.55, fg, bg);
    let mut spans: Vec<Span> = vec![Span::styled(" ", Style::new().bg(bg))];
    spans.push(Span::styled(
        fmt_line_no(line_no),
        Style::new().fg(gutter_fg).bg(bg),
    ));
    let text: String = chars[start..end].iter().collect();
    let rendered: usize = chars[start..end].iter().map(|&c| char_width(c)).sum();
    spans.push(Span::styled(text, Style::new().fg(fg).bg(bg)));
    if rendered < cw {
        spans.push(Span::styled(" ".repeat(cw - rendered), Style::new().bg(bg)));
    }
    spans.push(Span::styled(" ", Style::new().bg(bg)));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(bg)),
        text_area,
    );
}

fn render_reel_stats(frame: &mut Frame, area: Rect, app: &App) {
    // Tape winds from the supply reel (right) onto the take-up reel (left)
    // as words are recorded; both reels fill/empty from the hub outward.
    let ratio = app.tape_ratio();
    let hub = HUB_FRAMES[app.reel_rotation % HUB_FRAMES.len()];
    let left_reel = format!("{}{}", hub, reel_pattern(ratio));
    let right_reel = format!(
        "{}{}",
        reel_pattern(1.0 - ratio).chars().rev().collect::<String>(),
        hub
    );
    let stats = app.format_stats();

    let total_w = area.width as usize;
    let left_len = left_reel.chars().count();
    let right_len = right_reel.chars().count();
    let stats_len = stats.chars().count();
    let center_w = total_w.saturating_sub(left_len + right_len);
    let left_pad = center_w.saturating_sub(stats_len) / 2;
    let right_pad = center_w.saturating_sub(left_pad + stats_len);

    // A reached word goal is the happiest state and wins over the expired-timer
    // red; the reel staying fully wound keeps the durable cue.
    let goal_met = app.word_goal.is_some_and(|g| app.total_word_count() >= g);
    let stats_style = if goal_met {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        match app.timer_secs {
            Some(0) => Style::new().fg(Color::Black).bg(Color::Red),
            Some(_) => Style::new().fg(Color::Green),
            None => Style::new(),
        }
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

/// Five tape cells that fill `·→░→▒→▓` from the hub outward, so the reel
/// visibly winds a step for every ~1/15th of the spool.
fn reel_pattern(r: f64) -> String {
    (0..5)
        .map(|i| {
            let cell = (r.clamp(0.0, 1.0) * 5.0) - i as f64;
            if cell >= 1.0 {
                '▓'
            } else if cell >= 0.5 {
                '▒'
            } else if cell > 0.0 {
                '░'
            } else {
                '·'
            }
        })
        .collect()
}

/// Linearly interpolate between two colors: t=1.0 gives `a`, t=0.0 gives `b`.
fn lerp_color(t: f64, a: Color, b: Color) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    Color::Rgb(lerp_u8(t, ar, br), lerp_u8(t, ag, bg), lerp_u8(t, ab, bb))
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
