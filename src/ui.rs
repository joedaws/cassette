use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, Mode, GUTTER_WIDTH, MINIMIZED_ROWS};
use crate::cassette::{char_width, pos_to_row_col, wrap_spans, Cassette, Side};
use crate::theme::Theme;

// All colors come from the active `Theme` (side accents, unfocused cassette
// colors, and — when set — focused text/background; the default theme keeps
// the terminal's own colors for the focused cassette, issue #16).

pub fn render(frame: &mut Frame, app: &App, theme: &Theme) {
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
    constraints.push(Constraint::Length(1)); // bottom separator (closes the stack)
    constraints.push(Constraint::Min(0)); // filler: pins the footer to the window bottom
    constraints.push(Constraint::Length(1)); // reel stats
    constraints.push(Constraint::Length(1)); // status / info line
    constraints.push(Constraint::Length(1)); // help text

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
                theme,
            );
        } else {
            render_cassette_min(frame, chunks[chunk_idx], cassette, cw, theme);
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

    render_reel_stats(frame, chunks[n + 2], app);

    // The topic prompt owns the line while open; then status messages; then
    // the idle nudge; otherwise a vim-style info line.
    let info;
    let mut info_style = Style::new();
    let status = if app.mode == Mode::Topic {
        info = format!("topic: {}▏", app.topic_input);
        info.as_str()
    } else if let Some(m) = &app.status_msg {
        m.as_str()
    } else if app.idle_nudge() {
        info_style = Style::new().fg(Color::DarkGray);
        "· · ·  tape's still rolling — keep writing  · · ·"
    } else {
        let c = &app.cassettes[app.focus_idx];
        let (ln, col) = c.cursor_line_col();
        let mode_str = match app.mode {
            Mode::Insert if app.record => "-- RECORD --",
            Mode::Insert => "-- INSERT --",
            Mode::Normal => "-- NORMAL --",
            Mode::Topic => unreachable!("handled above"),
        };
        let side = match c.side {
            Side::A => "  ·  side A",
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
    };
    frame.render_widget(Paragraph::new(status).style(info_style), chunks[n + 3]);

    let help = match app.mode {
        Mode::Insert if app.record => {
            "type:the tape only rolls forward  Enter:newline  ^T:topic  ^B:flip side  Tab:next  ^N:new  ^C:quit & save"
        }
        Mode::Insert => "Esc:normal  Enter:newline  ^W:del word  ^T:topic  ^B:flip side  Tab:next  ^N:new  ^C:quit",
        Mode::Normal => {
            "i/a/o:insert  hjkl:move  w/b:word  0/$:line  x/dd:del  u:undo  gg/G:jump  t:topic  ^B:flip  q:quit"
        }
        Mode::Topic => "Enter:set topic  Esc:cancel  (empty input clears the topic)",
    };
    frame.render_widget(Paragraph::new(help_line(help, theme)), chunks[n + 4]);
}

/// Style a `key:description  key:description` help string: key combos get
/// the theme's `help_key` color (bold), descriptions and anything without a
/// `:` get the dimmer `help_text` color.
fn help_line<'a>(help: &'a str, theme: &Theme) -> Line<'a> {
    let mut key_style = Style::new().add_modifier(Modifier::BOLD);
    if let Some(c) = theme.help_key {
        key_style = key_style.fg(c);
    }
    let mut text_style = Style::new();
    if let Some(c) = theme.help_text {
        text_style = text_style.fg(c);
    }

    let mut spans = Vec::new();
    for (i, group) in help.split("  ").enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        match group.split_once(':') {
            Some((keys, desc)) => {
                spans.push(Span::styled(keys, key_style));
                spans.push(Span::styled(":", text_style));
                spans.push(Span::styled(desc, text_style));
            }
            None => spans.push(Span::styled(group, text_style)),
        }
    }
    Line::from(spans)
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

/// A cassette's top separator row: the active side's tag is woven into it
/// (accents from the theme; side A carries the loud one), followed by the
/// topic label when one is set. `short` picks the compact tags used on
/// minimized cassettes.
fn render_separator(
    frame: &mut Frame,
    area: Rect,
    ch: char,
    cassette: &Cassette,
    short: bool,
    theme: &Theme,
) {
    let sep_area = Rect { height: 1, ..area };
    let total = area.width as usize;
    let accent = match cassette.side {
        Side::A => theme.accent_a,
        Side::B => theme.accent_b,
    };
    let tag = match (cassette.side, short) {
        (Side::A, false) => "╡ SIDE A ╞",
        (Side::A, true) => "╡ A ╞",
        (Side::B, false) => "╡ SIDE B ╞",
        (Side::B, true) => "╡ B ╞",
    };
    let tag_style = Style::new().fg(accent);

    let mut used = 2 + tag.chars().count();
    let mut spans = vec![
        Span::raw(ch.to_string().repeat(2)),
        Span::styled(tag, tag_style),
    ];
    if let Some(topic) = &cassette.topic {
        let label = format!("╡ {} ╞", topic);
        used += 1 + label.chars().count();
        spans.push(Span::raw(ch.to_string()));
        spans.push(Span::styled(
            label,
            Style::new().add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw(ch.to_string().repeat(total.saturating_sub(used))));
    frame.render_widget(Paragraph::new(Line::from(spans)), sep_area);
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

/// Fade for the focused text under the active theme. With explicit text and
/// background colors the row fg lerps between them (floored so text stays
/// readable); with only a text color, DIM approximates the fade; with
/// neither, the terminal-default modifier fade applies.
fn themed_fade(t: f64, theme: &Theme) -> Style {
    match (theme.text, theme.background) {
        (Some(fg), Some(bg)) => Style::new().fg(lerp_color(t.clamp(0.35, 1.0), fg, bg)),
        (Some(fg), None) => {
            let s = Style::new().fg(fg);
            if t >= 0.9 {
                s
            } else {
                s.add_modifier(Modifier::DIM)
            }
        }
        _ => fade_style(t),
    }
}

/// Render the focused cassette: a `═` separator followed by `visible_lines`
/// rows of word-wrapped text on the terminal's default background, with a
/// line-number gutter. The viewport scrolls typewriter-style (cursor row
/// centered, clamped at the ends) and rows fade with distance from the
/// cursor row, so the bright band follows the cursor even when the scroll
/// clamp leaves it off-center.
#[allow(clippy::too_many_arguments)]
fn render_cassette_focused(
    frame: &mut Frame,
    area: Rect,
    cassette: &Cassette,
    cw: usize,
    visible_lines: usize,
    mode: Mode,
    theme: &Theme,
) {
    render_separator(frame, area, '═', cassette, false, theme);

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

    // Rows this many steps from the cursor row (or further) fade toward the background.
    let fade_zone = (visible_lines / 2).max(1);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_lines);

    for vp_row in 0..visible_lines {
        let line_idx = scroll_top + vp_row;

        // Fade with distance from the cursor row, which stays at full brightness.
        let dist = line_idx.abs_diff(cursor_row);
        let fade_t = (1.0 - dist as f64 / (fade_zone + 1) as f64).clamp(0.15, 1.0);
        let text_style = themed_fade(fade_t, theme);
        // The gutter doubles as a persistent side indicator while writing.
        let gutter_style = Style::new().fg(match cassette.side {
            Side::A => theme.accent_a,
            Side::B => theme.accent_b,
        });

        let mut spans: Vec<Span> = vec![Span::raw(" ")];

        let gutter = match (row_spans.get(line_idx), line_nums.get(line_idx)) {
            (Some(_), Some(&(n, true))) => fmt_line_no(n),
            (Some(_), _) => " ".repeat(GUTTER_WIDTH),
            // vim-style '~' marker for rows past the end of the text
            (None, _) => format!("{:>2}  ", '~'),
        };
        spans.push(Span::styled(gutter, gutter_style));

        // The cursor cell reverses the theme's own colors so the block reads
        // correctly on themed backgrounds too.
        let mut cursor_style = Style::new().add_modifier(Modifier::REVERSED);
        if let Some(fg) = theme.text {
            cursor_style = cursor_style.fg(fg);
        }
        if let Some(bg) = theme.background {
            cursor_style = cursor_style.bg(bg);
        }

        let mut rendered = 0;
        if let Some(&(disp_start, disp_end)) = row_spans.get(line_idx) {
            for (offset, &ch) in display[disp_start..disp_end].iter().enumerate() {
                // Insert mode: the bar cell. Normal mode: block over the char.
                let is_cursor = disp_start + offset == cursor_disp;
                let style = if is_cursor { cursor_style } else { text_style };
                spans.push(Span::styled(ch.to_string(), style));
                rendered += char_width(ch);
            }
            // Normal-mode cursor at the row's end (before a '\n' or at the end
            // of the text) has no char to sit on: draw a block on a space.
            if mode == Mode::Normal && line_idx == cursor_row && cursor_disp >= disp_end {
                spans.push(Span::styled(" ", cursor_style));
                rendered += 1;
            }
        }

        if rendered < cw {
            spans.push(Span::raw(" ".repeat(cw - rendered)));
        }
        spans.push(Span::raw(" "));
        lines.push(Line::from(spans));
    }

    let mut para = Paragraph::new(Text::from(lines));
    if let Some(bg) = theme.background {
        para = para.style(Style::new().bg(bg));
    }
    frame.render_widget(para, text_area);
}

/// Render an unfocused cassette minimized to a `─` separator plus its last
/// text row, keeping the colored background and showing the line number.
fn render_cassette_min(
    frame: &mut Frame,
    area: Rect,
    cassette: &Cassette,
    cw: usize,
    theme: &Theme,
) {
    render_separator(frame, area, '─', cassette, true, theme);

    if area.height <= 1 {
        return;
    }
    let text_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };

    let (bg, fg) = (theme.unfocused_bg, theme.unfocused_fg);
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
    // Tape winds from the supply reel (right) onto the take-up reel (left):
    // toward the word goal, or through the timer when only a timer is set.
    // Without either there is no progress to measure and no bars are drawn.
    let (left_reel, right_reel) = match app.tape_ratio() {
        Some(ratio) => (
            reel_pattern(ratio),
            reel_pattern(1.0 - ratio).chars().rev().collect::<String>(),
        ),
        None => (String::new(), String::new()),
    };
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
    let goal_met = app.word_goal.is_some_and(|g| app.session_word_count() >= g);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::{backend::TestBackend, Terminal};

    /// The reel/status/help footer pins to the bottom of the window; the
    /// filler sits between the cassette stack and the footer, not below it.
    #[test]
    fn footer_pins_to_window_bottom() {
        let mut app = App::new(None, None, Some(5));
        app.resize(80, 24);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render(f, &app, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();

        let row_text =
            |y: u16| -> String { (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect() };

        // Cassette stack: separator, 5 text rows, closing separator right below.
        assert!(row_text(6).starts_with("─"), "stack closes at row 6");
        // The gap down to the footer is blank filler.
        assert!(row_text(10).trim().is_empty(), "filler stays blank");
        // Footer occupies the last three rows of the window.
        assert!(row_text(21).contains("◆"), "stats row");
        assert!(row_text(22).contains("-- INSERT --"), "info line");
        assert!(row_text(23).contains("Esc:normal"), "help line");
    }

    /// Progress bars render only when a word goal or timer gives them
    /// something to measure; without either the stats row shows no bars.
    #[test]
    fn progress_bars_only_with_goal_or_timer() {
        let bars = ['·', '░', '▒', '▓'];
        let stats_row = |app: &App| -> String {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|f| render(f, app, &Theme::default()))
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..80)
                .map(|x| buf[(x, 21u16)].symbol().to_string())
                .collect()
        };

        let mut app = App::new(None, None, Some(5));
        app.resize(80, 24);
        let row = stats_row(&app);
        assert!(!row.contains(bars), "no goal, no timer: no bars");

        let mut app = App::new(None, Some(100), Some(5));
        app.resize(80, 24);
        let row = stats_row(&app);
        assert!(row.contains(bars), "word goal set: bars render");
        assert!(row.contains("0 / 100"), "goal stats render");
    }

    /// Issue #37: the help line separates key combos from descriptions —
    /// keys bold in `help_key`, descriptions in the dimmer `help_text`.
    #[test]
    fn help_line_styles_keys_and_descriptions() {
        let mut app = App::new(None, None, Some(5));
        app.resize(100, 24);
        let theme = crate::theme::resolve(Some("gruvbox"), &std::collections::HashMap::new())
            .expect("gruvbox is built in");
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| render(f, &app, &theme)).unwrap();
        let buf = terminal.backend().buffer();

        // Help row is the last row; insert help starts "Esc:normal".
        let key_cell = buf[(0u16, 23u16)].style(); // 'E' of "Esc"
        assert!(key_cell.add_modifier.contains(Modifier::BOLD));
        assert_eq!(key_cell.fg, Some(Color::Rgb(0xeb, 0xdb, 0xb2)));
        let desc_cell = buf[(4u16, 23u16)].style(); // 'n' of "normal"
        assert!(!desc_cell.add_modifier.contains(Modifier::BOLD));
        assert_eq!(desc_cell.fg, Some(Color::Rgb(0x92, 0x83, 0x74)));

        // Default theme: keys keep terminal fg, descriptions go DarkGray.
        terminal
            .draw(|f| render(f, &app, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(0u16, 23u16)].style().fg,
            Some(Color::Reset),
            "keys keep the terminal's own fg"
        );
        assert_eq!(buf[(4u16, 23u16)].style().fg, Some(Color::DarkGray));
    }

    /// Issue #34: the side accents are swapped — side A wears the yellow
    /// tag and gutter, side B the dark gray.
    #[test]
    fn side_accents_are_swapped() {
        let mut app = App::new(None, None, Some(5));
        app.resize(80, 24);
        app.modify_focused(|c| c.insert('x'));

        let styles = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|f| render(f, app, &Theme::default()))
                .unwrap();
            let buf = terminal.backend().buffer();
            // Tag char inside "══╡ SIDE X ╞" at x=2; gutter digit at x=1, y=1.
            (buf[(2u16, 0u16)].style().fg, buf[(3u16, 1u16)].style().fg)
        };

        let (tag, gutter) = styles(&app);
        assert_eq!(tag, Some(Color::Yellow), "side A tag is loud now");
        assert_eq!(gutter, Some(Color::Yellow), "side A gutter matches");

        app.modify_focused(|c| {
            c.flip();
            c.insert('y');
        });
        let (tag, gutter) = styles(&app);
        assert_eq!(tag, Some(Color::DarkGray), "side B tag went calm");
        assert_eq!(gutter, Some(Color::DarkGray));
    }

    /// A theme with explicit colors paints the focused cassette: background
    /// fill, text color at the cursor row, and themed side accents.
    #[test]
    fn themed_render_paints_focused_cassette() {
        let theme = crate::theme::resolve(Some("gruvbox"), &std::collections::HashMap::new())
            .expect("gruvbox is built in");
        let mut app = App::new(None, None, Some(5));
        app.resize(40, 20);
        app.modify_focused(|c| {
            for ch in "hi".chars() {
                c.insert(ch);
            }
        });

        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        terminal.draw(|f| render(f, &app, &theme)).unwrap();
        let buf = terminal.backend().buffer();

        // Text cell 'h' at x=5, y=1: gruvbox fg on gruvbox bg.
        let cell = &buf[(5u16, 1u16)];
        assert_eq!(cell.symbol(), "h");
        assert_eq!(cell.style().fg, Some(Color::Rgb(0xeb, 0xdb, 0xb2)));
        assert_eq!(cell.style().bg, Some(Color::Rgb(0x28, 0x28, 0x28)));
        // Padding cell far right of the text row is filled with the theme bg.
        assert_eq!(
            buf[(30u16, 1u16)].style().bg,
            Some(Color::Rgb(0x28, 0x28, 0x28))
        );
        // Side A separator tag uses the gruvbox accent.
        assert_eq!(
            buf[(2u16, 0u16)].style().fg,
            Some(Color::Rgb(0xfa, 0xbd, 0x2f))
        );
        // Faded row below the cursor row lerps toward the background:
        // still reddish-tinted, but darker than full text.
        let faded = buf[(5u16, 2u16)].style().fg;
        assert_ne!(faded, Some(Color::Rgb(0xeb, 0xdb, 0xb2)), "row 2 is faded");
    }

    /// Both sides announce themselves: side A tag on the focused separator,
    /// side B tag after a flip, and the topic label woven in when set.
    #[test]
    fn separator_shows_side_tag_and_topic() {
        let row_text = |app: &App, y: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|f| render(f, app, &Theme::default()))
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect()
        };

        let mut app = App::new(None, None, Some(5));
        app.resize(80, 24);
        assert!(row_text(&app, 0).contains("╡ SIDE A ╞"));

        app.modify_focused(|c| c.flip());
        assert!(row_text(&app, 0).contains("╡ SIDE B ╞"));

        app.modify_focused(|c| {
            c.flip();
            c.topic = Some("dream log".into());
        });
        let row = row_text(&app, 0);
        assert!(row.contains("╡ SIDE A ╞"));
        assert!(row.contains("╡ dream log ╞"));

        // Minimized cassettes get the compact tag plus the topic. After
        // adding a cassette, cassette 1 is minimized at the top of the stack.
        app.add_cassette();
        let row = row_text(&app, 0);
        assert!(row.contains("╡ A ╞"), "compact side tag: {row}");
        assert!(row.contains("╡ dream log ╞"));
    }

    /// With the cursor at the start of a long text the scroll clamp pins the
    /// viewport to the top, leaving the cursor row off-center: the fade must
    /// still be brightest at the cursor, not at the middle of the cassette.
    #[test]
    fn fade_follows_cursor_when_scroll_clamps() {
        let mut app = App::new(None, None, Some(5));
        app.resize(40, 20);
        app.modify_focused(|c| {
            for ch in "a\nb\nc\nd\ne\nf\ng".chars() {
                c.insert(ch);
            }
            c.move_text_start();
        });
        app.mode = Mode::Normal;

        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        terminal
            .draw(|f| render(f, &app, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();

        // Text rows start at y=1 (below the separator); text at x=5 (pad + gutter).
        let style_at = |y: u16| buf[(5, y)].style();

        // Cursor row at the viewport top: the block cursor.
        assert!(style_at(1).add_modifier.contains(Modifier::REVERSED));
        // One row down: dimmed but not grayed yet.
        assert!(style_at(2).add_modifier.contains(Modifier::DIM));
        assert_ne!(style_at(2).fg, Some(Color::DarkGray));
        // Two rows down — the viewport center. Under center-based fading this
        // was full brightness; it must fade like any row two steps away.
        assert_eq!(style_at(3).fg, Some(Color::DarkGray));
        assert!(!style_at(3).add_modifier.contains(Modifier::DIM));
        // Three or more rows down: darkest step.
        assert_eq!(style_at(4).fg, Some(Color::DarkGray));
        assert!(style_at(4).add_modifier.contains(Modifier::DIM));
        assert_eq!(style_at(5).fg, Some(Color::DarkGray));
        assert!(style_at(5).add_modifier.contains(Modifier::DIM));
    }
}
