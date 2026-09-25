use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, Mode, ReadOnly, GUTTER_WIDTH, MINIMIZED_ROWS};
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
    // The window covers the STACK, which excludes closed cassettes while the
    // fold is shut — so a folded session never lays one out at all.
    let stack = app.stack_len();
    let first = app.cassette_scroll.min(stack.saturating_sub(1));
    let n = app.visible_cassette_count().min(stack - first);

    let mut constraints: Vec<Constraint> = (first..first + n)
        .map(|i| {
            if i == app.focus_idx {
                Constraint::Length(rpc)
            } else {
                Constraint::Length(MINIMIZED_ROWS)
            }
        })
        .collect();
    let closed_row = closed_row_text(app);
    if closed_row.is_some() {
        constraints.push(Constraint::Length(1)); // the closed fold's own row
    }
    for _ in &app.damaged {
        constraints.push(Constraint::Length(1)); // one row per damaged file
    }
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

    // Everything after the stack shifts down by one when the fold row is
    // drawn, so the offset is computed once rather than spelled per chunk.
    let after_stack = if let Some(row) = &closed_row {
        let style = Style::new().fg(theme.unfocused_fg);
        frame.render_widget(Paragraph::new(row.clone()).style(style), chunks[n]);
        n + 1
    } else {
        n
    };

    // Damaged files sit below the fold row: both are outside the working
    // set, which is what they have in common.
    let damaged_style = Style::new().fg(theme.unfocused_fg);
    for (i, (label, reason)) in app.damaged.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(format!("⚠ {label} — {reason}")).style(damaged_style),
            chunks[after_stack + i],
        );
    }
    let after_stack = after_stack + app.damaged.len();

    let sep = "─".repeat(area.width as usize);
    frame.render_widget(Paragraph::new(sep), chunks[after_stack]);

    // Overflow indicators for cassettes scrolled out of view.
    let (above, below) = app.hidden_cassettes();
    if above > 0 {
        render_overflow_hint(frame, chunks[0], above, '↑');
    }
    if below > 0 {
        render_overflow_hint(frame, chunks[after_stack], below, '↓');
    }

    render_reel_stats(frame, chunks[after_stack + 2], app);

    // The topic prompt owns the line while open; then status messages; then
    // the idle nudge; otherwise a vim-style info line. `info_text` is the
    // pure text; the idle-nudge dimming is the only style decision left here.
    let mut info_style = Style::new();
    if app.status_msg.is_none()
        && app.mode != Mode::Topic
        && app.idle_nudge()
        && !app.read_only.is_read_only()
    {
        info_style = Style::new().fg(Color::DarkGray);
    }
    frame.render_widget(
        Paragraph::new(info_text(app)).style(info_style),
        chunks[after_stack + 3],
    );

    let help = help_text(app);
    frame.render_widget(
        Paragraph::new(help_line(help, theme)),
        chunks[after_stack + 4],
    );
}

/// The help row's text for the current mode and read-only reason. Pure, so
/// tests can assert the remedy each state names without a terminal —
/// `render` applies the key/description styling.
pub(crate) fn help_text(app: &App) -> &'static str {
    match app.mode {
        _ if matches!(app.read_only, ReadOnly::Closed) => {
            "this cassette is closed  `cassette queue reopen` to write in it again  z:fold  Tab:next  ^C:quit & save"
        }
        _ if app.read_only.is_read_only() => {
            "keys ignored: another writer holds this cassette's lock  Tab:next  ^N:new  ^C:quit & save"
        }
        Mode::Insert if app.record => {
            "type:the tape only rolls forward  Enter:newline  ^T:topic  ^B:flip side  Tab:next  ^N:new  ^C:quit & save"
        }
        Mode::Insert => "Esc:normal  Enter:newline  ^W:del word  ^T:topic  ^B:flip side  Tab:next  ^N:new  ^C:quit",
        Mode::Normal => {
            "i/a/o:insert  hjkl:move  w/b:word  0/$:line  x/dd:del  u:undo  gg/G:jump  t:topic  z:fold  ^B:flip  q:quit"
        }
        Mode::Topic => "Enter:set topic  Esc:cancel  (empty input clears the topic)",
    }
}

/// Draw the `cassette sessions` picker: a title, one line per visible
/// session, the filter prompt when it is open, and a key hint.
///
/// Rows show the alias where a session has one and the id otherwise — the
/// id is what `resume` takes, but a human who named a session should see
/// the name. All colours come from the `Theme`, like everything else here.
pub(crate) fn render_picker(frame: &mut Frame, picker: &crate::picker::Picker, theme: &Theme) {
    let area = frame.area();
    let rows = picker.visible();

    let mut lines: Vec<Line> = Vec::new();
    let dim = Style::new().fg(theme.unfocused_fg);
    let bold = Style::new().add_modifier(Modifier::BOLD);

    // The `a` hint only when there is something hidden to expand to:
    // offering "all" on a list that is already all of them reads as a
    // control that does nothing.
    let hidden = !picker.show_all && picker.total() > rows.len();
    let of = if picker.query.is_empty() {
        format!("{} of {}", rows.len(), picker.total())
    } else {
        // Under a filter, "of <total>" would misread as "N of M matches".
        format!("{} of {} matching", rows.len(), picker.total())
    };
    lines.push(Line::from(Span::styled(
        format!(
            "cassette sessions — {of}{}",
            if hidden { "  (a: all)" } else { "" }
        ),
        bold,
    )));
    lines.push(Line::from(""));

    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            if picker.total() == 0 {
                "no sessions yet — the first session starts the count".to_string()
            } else {
                format!("nothing matches '{}'", picker.query)
            },
            dim,
        )));
    }

    // One row per session. The marker carries the highlight rather than a
    // background colour: a themed background can be indistinguishable from
    // the terminal's own, and the marker is legible on every theme.
    let capacity = crate::picker::Picker::rows_capacity(area.height);
    let (above, below) = picker.hidden_rows(capacity);
    if above > 0 {
        lines.push(Line::from(Span::styled(format!("  ↑ {above} more"), dim)));
    }
    for (i, e) in rows
        .iter()
        .enumerate()
        .skip(picker.scroll)
        .take(capacity.saturating_sub(usize::from(above > 0) + usize::from(below > 0)))
    {
        let marker = if i == picker.cursor { "> " } else { "  " };
        let name = e.alias.as_deref().unwrap_or(&e.id);
        let mut text = format!(
            "{marker}{}  {:>5} words  {name}",
            e.date.format("%Y-%m-%d %H:%M"),
            e.words
        );
        if !e.topics.is_empty() {
            text.push_str(&format!("  — {}", e.topics.join(", ")));
        }
        let style = if i == picker.cursor {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    if below > 0 {
        lines.push(Line::from(Span::styled(format!("  ↓ {below} more"), dim)));
    }

    lines.push(Line::from(""));
    if picker.filtering {
        lines.push(Line::from(Span::styled(
            format!("/{}▏", picker.query),
            bold,
        )));
    } else if !picker.query.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("filter: {}  (/ to edit)", picker.query),
            dim,
        )));
    }

    if picker.unreadable > 0 {
        lines.push(Line::from(Span::styled(
            format!("{} unreadable", picker.unreadable),
            dim,
        )));
    }
    if let Some(line) = crate::store::skipped_line(picker.skipped) {
        lines.push(Line::from(Span::styled(line, dim)));
    }

    let help = if picker.filtering {
        "type:filter  Enter:apply  Esc:done"
    } else {
        "j/k:move  Enter:open  /:filter  a:all  q:quit"
    };
    lines.push(help_line(help, theme));

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// The closed fold's row — `▸ 12 closed` shut, `▾ 12 closed` open — or
/// `None` when the session has no closed cassettes, since an affordance for
/// nothing is noise. Pure text; `render` applies the theme's `unfocused_fg`,
/// following the `info_text`/`separator_text` precedent so production and
/// tests can never disagree about the wording.
pub(crate) fn closed_row_text(app: &App) -> Option<String> {
    let n = app.closed_count();
    if n == 0 {
        return None;
    }
    let arrow = if app.closed_expanded { '▾' } else { '▸' };
    Some(format!("{arrow} {n} closed"))
}

/// The vim-style info line's text: topic prompt, status message, or idle
/// nudge when one of those is showing, otherwise mode/cursor/cassette
/// position. Pure — returns the text only, no styling — so tests can assert
/// on its content without a terminal; `render` applies the one remaining
/// style decision (dimming the idle nudge).
///
/// A read-only cassette names its holder — `open by refactor-agent` — rather
/// than showing the bare `-- READ ONLY --` 5a left: `ReadOnly::Busy`'s holder is
/// `None` only when there is genuinely no name to show (a garbled lock
/// anchor), in which case the bare label is all that's left to say.
///
/// That arm is reachable only because `try_acquire` records a busy cassette
/// in `read_only`'s own variant and leaves `status_msg` — checked above it —
/// for transient news. `pub(crate)` so the cross-process contention test in
/// `session_writer` can assert the banner reaches the screen under a real
/// held lock, rather than only from hand-set fields.
pub(crate) fn info_text(app: &App) -> String {
    if app.mode == Mode::Topic {
        return format!("topic: {}▏", app.topic_input);
    }
    if let Some(m) = &app.status_msg {
        return m.clone();
    }
    // Not while locked out: "keep writing" is advice the user cannot take,
    // and it would shadow the one line saying why — the same shadowing the
    // busy banner was rescued from one branch below. Watching another writer
    // work is not idling, so the nudge is wrong here on its own terms.
    if app.idle_nudge() && !app.read_only.is_read_only() {
        return "· · ·  tape's still rolling — keep writing  · · ·".to_string();
    }
    let c = &app.cassettes[app.focus_idx];
    let (ln, col) = c.cursor_line_col();
    let mode_str = match app.mode {
        _ if app.read_only.is_read_only() => match &app.read_only {
            ReadOnly::Busy { holder: Some(h) } => format!("-- READ ONLY (open by {h}) --"),
            ReadOnly::Busy { holder: None } => "-- READ ONLY --".to_string(),
            ReadOnly::Closed => "-- CLOSED --".to_string(),
            ReadOnly::No => unreachable!("guarded by is_read_only above"),
        },
        Mode::Insert if app.record => "-- RECORD --".to_string(),
        Mode::Insert => "-- INSERT --".to_string(),
        Mode::Normal => "-- NORMAL --".to_string(),
        Mode::Topic => unreachable!("handled above"),
    };
    let side = match c.side {
        Side::A => "  ·  side A",
        Side::B => "  ·  side B",
    };
    format!(
        "{}  ln {}, col {}  ·  {} chars  ·  cassette {}/{}{}",
        mode_str,
        ln,
        col,
        c.char_count(),
        app.focus_idx + 1,
        // The STACK, not the whole list: with twelve closed cassettes folded
        // away, `cassette 1/13` beside a Tab cycle of one contradicts the
        // fold's whole premise.
        app.stack_len(),
        side
    )
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

/// Side tag text for a cassette's separator: `╡ SIDE A ╞`/`╡ SIDE B ╞`, or the
/// compact `╡ A ╞`/`╡ B ╞` used on minimized cassettes (`short`).
fn side_tag_text(side: Side, short: bool) -> &'static str {
    match (side, short) {
        (Side::A, false) => "╡ SIDE A ╞",
        (Side::A, true) => "╡ A ╞",
        (Side::B, false) => "╡ SIDE B ╞",
        (Side::B, true) => "╡ B ╞",
    }
}

fn topic_label_text(topic: &str) -> String {
    format!("╡ {topic} ╞")
}

/// A sticky lock's label — worded "locked by", never "open by": a **busy**
/// cassette (`info_text`'s `open by <holder>`) is transiently held by a live
/// process and frees itself; this is a durable claim only a human can clear
/// (`queue unlock`). The two must not read the same.
fn sticky_lock_label_text(holder: &str) -> String {
    format!("╡ locked by {holder} ╞")
}

/// A cassette's separator content, in display order: the side tag, then the
/// topic label if set, then the sticky-lock label if set. No fill characters
/// (those depend on the terminal width, which this doesn't take) — shared by
/// `render_separator` (styles and joins these with `ch` fill) and
/// `separator_text` (just concatenates them, for tests), so the two can never
/// disagree about what the separator says.
fn separator_pieces(cassette: &Cassette, short: bool) -> Vec<String> {
    let mut pieces = vec![side_tag_text(cassette.side, short).to_string()];
    if let Some(topic) = &cassette.topic {
        pieces.push(topic_label_text(topic));
    }
    if let Some(holder) = &cassette.locked_by {
        pieces.push(sticky_lock_label_text(holder));
    }
    pieces
}

/// The plain text of cassette `idx`'s separator (long-form tags, as on the
/// focused cassette) — pure, no fill, no styling, so a test can assert on
/// content without a terminal. `render_separator` gets the same content
/// (styled, with fill) straight from `separator_pieces`; this wrapper has no
/// production caller of its own, only tests — gated on `cfg(test)` for the
/// same reason `queue::write::write_body` is.
#[cfg(test)]
fn separator_text(app: &App, idx: usize) -> String {
    separator_pieces(&app.cassettes[idx], false).concat()
}

/// A cassette's top separator row: the active side's tag is woven into it
/// (accents from the theme; side A carries the loud one), followed by the
/// topic label when one is set and a sticky-lock label when `locked_by` is
/// set. `short` picks the compact tags used on minimized cassettes.
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
    let tag_style = Style::new().fg(accent);
    let label_style = Style::new().add_modifier(Modifier::BOLD);

    let pieces = separator_pieces(cassette, short);
    let mut used = 2 + pieces[0].chars().count();
    let mut spans = vec![
        Span::raw(ch.to_string().repeat(2)),
        Span::styled(pieces[0].clone(), tag_style),
    ];
    for label in &pieces[1..] {
        used += 1 + label.chars().count();
        spans.push(Span::raw(ch.to_string()));
        spans.push(Span::styled(label.clone(), label_style));
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

    /// Build an app with `open` open cassettes, `closed` closed ones and
    /// `damaged` damaged rows, laid out at 80x24 — the shape the fold and
    /// the error rows are actually seen at.
    fn folded_app(open: usize, closed: usize, damaged: usize) -> App {
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
        app.cassettes.clear();
        // Topics, not just ids: a cassette's id is never drawn, so a
        // fixture without one of these has nothing on screen to assert the
        // absence of — the test would pass whether or not it was laid out.
        for i in 0..open {
            let mut c = Cassette::new();
            c.id = format!("open-{i:02}");
            c.topic = Some(format!("opentopic{i}"));
            c.priority = i as i64 * 10;
            app.cassettes.push(c);
        }
        for i in 0..closed {
            let mut c = Cassette::new();
            c.id = format!("shut-{i:02}");
            c.topic = Some(format!("shuttopic{i}"));
            c.priority = 1000 + i as i64;
            c.closed = true;
            app.cassettes.push(c);
        }
        app.damaged = (0..damaged)
            .map(|i| (format!("bad-{i}"), "could not be read".to_string()))
            .collect();
        app.sort_queue();
        app.resize(80, 24);
        app
    }

    fn drawn(app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render(f, app, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..24)
            .map(|y| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    /// 5c shipped two display bugs a green suite could not see. The fixture
    /// must make rows distinguishable ON SCREEN — a row whose alias is
    /// absent proves nothing by being absent.
    #[test]
    fn the_picker_draws_rows_and_marks_the_cursor() {
        use crate::picker::Picker;
        let d = chrono::NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap();
        let mut p = Picker::new_for_test(vec![
            crate::catalog::NoteEntry::for_test(
                "01M3AAA0000000000000000AAA",
                Some("morningpages"),
                d,
                120,
                &["gratitude"],
            ),
            crate::catalog::NoteEntry::for_test(
                "01M3BBB0000000000000000BBB",
                Some("eveningreview"),
                d,
                80,
                &["review"],
            ),
        ]);
        p.move_down();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_picker(f, &p, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..24)
            .map(|y| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect();
        let screen = rows.join("\n");

        assert!(screen.contains("morningpages"), "{screen}");
        assert!(screen.contains("eveningreview"), "{screen}");
        assert!(screen.contains("gratitude"), "topics are shown: {screen}");

        let cursor_row = rows
            .iter()
            .position(|r| r.contains("eveningreview"))
            .expect("the second row is drawn");
        assert!(
            rows[cursor_row].trim_start().starts_with('>'),
            "the highlighted row carries the marker: {:?}",
            rows[cursor_row]
        );
        let other = rows
            .iter()
            .position(|r| r.contains("morningpages"))
            .expect("the first row is drawn");
        assert!(
            !rows[other].trim_start().starts_with('>'),
            "and only that row: {:?}",
            rows[other]
        );
    }

    /// Offering "a: all" on a list that already shows everything reads as
    /// a control that does nothing.
    #[test]
    fn the_all_hint_appears_only_when_rows_are_hidden() {
        use crate::picker::Picker;
        let d = chrono::NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap();
        let draw = |p: &Picker| -> String {
            let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
            t.draw(|f| render_picker(f, p, &Theme::default())).unwrap();
            let buf = t.backend().buffer();
            (0..24)
                .map(|y| {
                    (0..80)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let few: Vec<_> = (0..3)
            .map(|i| crate::catalog::NoteEntry::for_test(&format!("id{i}"), None, d, 1, &["t"]))
            .collect();
        assert!(
            !draw(&Picker::new_for_test(few)).contains("a: all"),
            "everything is already shown"
        );

        let many: Vec<_> = (0..20)
            .map(|i| crate::catalog::NoteEntry::for_test(&format!("id{i:02}"), None, d, 1, &["t"]))
            .collect();
        assert!(
            draw(&Picker::new_for_test(many)).contains("a: all"),
            "five are hidden, so the hint earns its place"
        );
    }

    /// The prompt line, the help row, the word count and the damaged-file
    /// footer were each deletable with the suite green. The last two phases
    /// both shipped a display bug a green suite could not see.
    #[test]
    fn the_picker_draws_its_chrome() {
        use crate::picker::Picker;
        let d = chrono::NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap();
        let draw = |p: &Picker| -> String {
            let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
            t.draw(|f| render_picker(f, p, &Theme::default())).unwrap();
            let buf = t.backend().buffer();
            (0..24)
                .map(|y| {
                    (0..80)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let mut p = Picker::new(
            vec![crate::catalog::NoteEntry::for_test(
                "01M3AAA0000000000000000AAA",
                Some("morningpages"),
                d,
                137,
                &["gratitude"],
            )],
            2,
        );

        let idle = draw(&p);
        assert!(idle.contains("137"), "the word count is shown: {idle}");
        assert!(idle.contains("j/k:move"), "the help row is drawn: {idle}");
        assert!(
            idle.contains("2 unreadable"),
            "damaged files are reported ON SCREEN, not to a wiped stderr: {idle}"
        );
        assert!(!idle.contains("skipped"), "no skipped line at zero: {idle}");
        p.skipped = 1;
        let with_skipped = draw(&p);
        assert!(
            with_skipped.contains("1 session directory skipped: names are not ses_ ids"),
            "skipped directories are reported on screen too: {with_skipped}"
        );
        p.skipped = 0;

        p.start_filter();
        p.push_filter('m');
        let filtering = draw(&p);
        assert!(
            filtering.contains("/m"),
            "the prompt shows what is being typed: {filtering}"
        );
        assert!(
            filtering.contains("Esc:done"),
            "and the prompt's own help: {filtering}"
        );
        assert!(
            filtering.contains("matching"),
            "the header counts matches, not the whole store: {filtering}"
        );
    }

    /// An empty store is a normal state; the picker says so rather than
    /// drawing a blank screen.
    #[test]
    fn the_picker_says_so_when_there_are_no_sessions() {
        use crate::picker::Picker;
        let p = Picker::new_for_test(Vec::new());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_picker(f, &p, &Theme::default()))
            .unwrap();
        let buf = terminal.backend().buffer();
        let screen: String = (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("no sessions yet"), "{screen}");
    }

    /// The fold's central rendering guarantee: a folded session lays out no
    /// closed cassette at all. `render`'s window spans `stack_len()`, and
    /// swapping that back to `cassettes.len()` must fail a test.
    #[test]
    fn a_folded_session_draws_no_closed_cassette() {
        let app = folded_app(2, 3, 0);
        assert!(!app.closed_expanded);
        let screen = drawn(&app).join("\n");

        assert!(
            screen.contains("opentopic0"),
            "the open ones ARE drawn, so absence below means something:\n{screen}"
        );
        assert!(
            !screen.contains("shuttopic"),
            "closed cassettes must not be drawn while folded:\n{screen}"
        );
        assert!(screen.contains("▸ 3 closed"), "{screen}");
    }

    /// The fold row sits directly under the stack, and the damaged rows
    /// directly under it — both above the separator that closes the stack.
    #[test]
    fn the_fold_row_and_damaged_rows_sit_between_the_stack_and_the_separator() {
        let app = folded_app(1, 2, 2);
        let rows = drawn(&app);
        let fold = rows
            .iter()
            .position(|r| r.contains("▸ 2 closed"))
            .expect("fold row is drawn");
        let first_damaged = rows
            .iter()
            .position(|r| r.contains("bad-0"))
            .expect("damaged row is drawn");
        let second_damaged = rows
            .iter()
            .position(|r| r.contains("bad-1"))
            .expect("second damaged row is drawn");

        assert_eq!(first_damaged, fold + 1, "damaged rows follow the fold row");
        assert_eq!(second_damaged, fold + 2, "and each other, in order");
    }

    /// Every widget after the stack indexes off one `after_stack` offset.
    /// Dropping the `+ damaged.len()` term silently slid the footer up over
    /// the damaged rows — the exact failure CLAUDE.md's `ui.rs` entry warns
    /// about — so the footer's position is pinned with them on screen.
    #[test]
    fn the_footer_still_lands_below_the_damaged_rows() {
        let app = folded_app(1, 1, 3);
        let rows = drawn(&app);
        let last_damaged = rows
            .iter()
            .rposition(|r| r.contains("bad-2"))
            .expect("last damaged row is drawn");

        // The info line carries the mode; it must sit below every ⚠ row and
        // must not have overwritten one.
        let info = rows
            .iter()
            .rposition(|r| r.contains("-- INSERT --"))
            .expect("info line is drawn");
        assert!(
            info > last_damaged,
            "footer must stay below the damaged rows (info {info}, last ⚠ {last_damaged})"
        );
        assert_eq!(
            rows.iter().filter(|r| r.contains("⚠")).count(),
            3,
            "all three damaged rows survive the footer's layout"
        );
    }

    /// The reel/status/help footer pins to the bottom of the window; the
    /// filler sits between the cassette stack and the footer, not below it.
    #[test]
    fn footer_pins_to_window_bottom() {
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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

        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
        app.resize(80, 24);
        let row = stats_row(&app);
        assert!(!row.contains(bars), "no goal, no timer: no bars");

        let mut app = App::new(
            None,
            Some(100),
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
        app.resize(80, 24);
        let row = stats_row(&app);
        assert!(row.contains(bars), "word goal set: bars render");
        assert!(row.contains("0 / 100"), "goal stats render");
    }

    /// Issue #37: the help line separates key combos from descriptions —
    /// keys bold in `help_key`, descriptions in the dimmer `help_text`.
    #[test]
    fn help_line_styles_keys_and_descriptions() {
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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

        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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

    /// A busy read-only cassette must name its holder, not just say
    /// `-- READ ONLY --` — the user needs to know WHO to wait on.
    #[test]
    fn the_busy_indicator_names_the_holder() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = ReadOnly::Busy {
            holder: Some("refactor-agent".to_string()),
        };
        let line = crate::ui::info_text(&app);
        assert!(
            line.contains("refactor-agent"),
            "the user must know WHO holds it: {line}"
        );
    }

    /// Ten quiet seconds on a cassette an agent holds must not replace the
    /// reason the keyboard is dead with advice to keep typing. The nudge
    /// sits ABOVE the read-only arm in `info_text`, so this is the same
    /// shadowing class the busy banner was rescued from.
    #[test]
    fn the_idle_nudge_does_not_shadow_the_busy_banner() {
        let mut app = App::new(
            Some(600),
            None,
            None,
            "01JTESTSESSN00000000000000".to_string(),
        );
        app.idle_secs = App::IDLE_NUDGE_SECS + 5;
        assert!(
            app.idle_nudge(),
            "fixture must actually be nudging, or this proves nothing"
        );

        app.read_only = ReadOnly::Busy {
            holder: Some("refactor-agent".to_string()),
        };
        let line = crate::ui::info_text(&app);
        assert!(
            line.contains("READ ONLY (open by refactor-agent)"),
            "the banner must win over the nudge: {line}"
        );
        assert!(!line.contains("still rolling"), "{line}");
    }

    /// The fold's own row: how much history is there, and which way it
    /// opens.
    #[test]
    fn the_closed_row_counts_and_points() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        let mut open = Cassette::new();
        open.id = "a".to_string();
        open.priority = 1;
        app.cassettes.push(open);
        for i in 0..12 {
            let mut c = Cassette::new();
            c.id = format!("c{i:02}");
            c.priority = 10 + i;
            c.closed = true;
            app.cassettes.push(c);
        }
        app.sort_queue();

        assert_eq!(
            crate::ui::closed_row_text(&app).as_deref(),
            Some("▸ 12 closed")
        );
        app.toggle_closed_fold();
        assert_eq!(
            crate::ui::closed_row_text(&app).as_deref(),
            Some("▾ 12 closed")
        );
    }

    /// No closed cassettes, no row — an affordance for nothing is noise.
    #[test]
    fn no_closed_row_without_closed_cassettes() {
        let app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        assert_eq!(crate::ui::closed_row_text(&app), None);
    }

    /// `z` is the only way into the closed fold, and the only way out of a
    /// session whose cassettes are all closed. A binding discoverable
    /// nowhere on screen is close to not shipped.
    #[test]
    fn the_help_row_advertises_the_fold_key() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.mode = Mode::Normal;
        assert!(crate::ui::help_text(&app).contains("z:fold"));

        app.read_only = ReadOnly::Closed;
        assert!(
            crate::ui::help_text(&app).contains("z:fold"),
            "especially here, where the fold is how you get back out"
        );
    }

    /// The help row must name the remedy. Closed needs an action
    /// (`queue reopen`); busy only needs patience, and telling a human to
    /// wait for a closed cassette leaves them waiting forever.
    #[test]
    fn the_help_row_names_the_remedy_for_closed() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = ReadOnly::Closed;
        let help = crate::ui::help_text(&app);
        assert!(help.contains("queue reopen"), "{help}");

        app.read_only = ReadOnly::Busy { holder: None };
        let busy = crate::ui::help_text(&app);
        assert!(busy.contains("another writer"), "{busy}");
        assert!(!busy.contains("queue reopen"), "{busy}");
    }

    /// Busy, sticky and closed are three states with three different
    /// remedies: wait, go unlock it, go reopen it. 5b established that the
    /// display must tell busy and sticky apart; closed is the third, and a
    /// human sent to wait for a closed cassette waits forever.
    #[test]
    fn a_closed_cassette_does_not_read_like_a_busy_one() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = ReadOnly::Busy {
            holder: Some("refactor-agent".to_string()),
        };
        let busy = crate::ui::info_text(&app);

        app.read_only = ReadOnly::Closed;
        let closed = crate::ui::info_text(&app);

        assert!(
            busy.contains("READ ONLY (open by refactor-agent)"),
            "{busy}"
        );
        assert!(closed.contains("-- CLOSED --"), "{closed}");
        assert!(
            !closed.contains("READ ONLY"),
            "closed is not 'someone else has it': {closed}"
        );
        assert!(
            closed.contains("cassette 1/"),
            "and it keeps the rest of the info line: {closed}"
        );
    }

    /// Busy means wait; sticky means go and unlock it. A display that blurs
    /// them sends the user to wait for something that will never free itself.
    #[test]
    fn a_busy_cassette_and_a_sticky_one_do_not_read_the_same() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = ReadOnly::Busy {
            holder: Some("refactor-agent".to_string()),
        };
        let busy = crate::ui::info_text(&app);

        app.read_only = ReadOnly::No;
        app.cassettes[0].locked_by = Some("joseph".to_string());
        let sticky = crate::ui::separator_text(&app, 0);

        assert_ne!(busy, sticky);
        assert!(sticky.contains("joseph"), "{sticky}");
    }

    /// With the cursor at the start of a long text the scroll clamp pins the
    /// viewport to the top, leaving the cursor row off-center: the fade must
    /// still be brightest at the cursor, not at the middle of the cassette.
    #[test]
    fn fade_follows_cursor_when_scroll_clamps() {
        let mut app = App::new(
            None,
            None,
            Some(5),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
