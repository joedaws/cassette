use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, Event, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

mod app;
mod cassette;
mod config;
mod output;
mod theme;
mod ui;

use app::{App, Mode};

/// Seconds between autosaves of a dirty session.
const AUTOSAVE_SECS: u64 = 30;

/// Where the session's markdown goes on quit (and during autosave).
struct Sink {
    path: PathBuf,
    desired: PathBuf,
    conflicted: bool,
    /// Whether an autosave has created `path` already.
    wrote: bool,
}

/// Best-effort terminal restore; must be safe to call twice and mid-panic.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

fn main() -> io::Result<()> {
    let args = parse_args();
    let cfg = config::load_config();

    if args.list_themes {
        print_themes(&cfg);
        return Ok(());
    }

    // Resolve the theme and topic template before touching the terminal so
    // an unknown name can die() cleanly.
    let theme = match theme::resolve(args.theme.as_deref().or(cfg.theme.as_deref()), &cfg.themes) {
        Ok(t) => t,
        Err(e) => die(&e),
    };
    let template_topics: Option<Vec<String>> = args.template.as_deref().map(|name| {
        cfg.templates.get(name).cloned().unwrap_or_else(|| {
            die(&format!(
                "unknown template '{name}' — define it under [templates] in config.toml"
            ))
        })
    });

    // Restore the terminal before the default hook prints, so the panic
    // message is readable and the shell isn't left in raw mode.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Enable the kitty keyboard protocol so Shift+Enter is distinguishable from
    // plain Enter (used to flip sides). Pushed unconditionally: querying support
    // first (crossterm's supports_keyboard_enhancement) blocks startup for seconds
    // on terminals that never answer, and terminals without the protocol ignore
    // these sequences by design.
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let visible_lines = args.visible_lines.or(cfg.visible_lines);
    let mut app = App::new(args.timer_secs, args.word_goal, visible_lines);
    app.record = args.record;
    app.resize(size.width, size.height);
    if let Some(topics) = &template_topics {
        app.apply_topics(topics);
    }

    // Resolve the output file up front so the session can autosave to it.
    let mut sink = if args.print_stdout {
        None
    } else {
        let effective_notes_dir = cfg.notes_dir.clone().or_else(config::default_notes_dir);
        let desired = config::resolve_output_path(
            args.note_name.as_deref(),
            &app.started_at,
            effective_notes_dir.as_deref(),
        );
        let (path, conflicted) = config::find_available_path(&desired);
        Some(Sink {
            path,
            desired,
            conflicted,
            wrote: false,
        })
    };

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        run(&mut terminal, &mut app, sink.as_mut(), &theme)
    }));

    restore_terminal();

    // Save before propagating any error or panic: the words matter most.
    finish_session(&app, sink.as_mut());
    if let Some(summary) = session_summary(&app) {
        eprintln!("{summary}");
    }

    match result {
        Ok(r) => r?,
        Err(payload) => panic::resume_unwind(payload),
    }
    Ok(())
}

/// One-line session recap (plus a per-cassette breakdown when there are
/// several): words, duration, and pace. `None` for empty sessions.
fn session_summary(app: &App) -> Option<String> {
    if app.is_empty() {
        return None;
    }
    let words = app.total_word_count();
    let secs = app.started_at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    let mut line = format!("{} words in {}:{:02}", words, secs / 60, secs % 60);
    // Pace needs a meaningful denominator; skip it for very short sessions.
    if secs >= 30 {
        line.push_str(&format!(" — {:.0} wpm", words as f64 * 60.0 / secs as f64));
    }
    if app.cassettes.len() > 1 {
        let parts: Vec<String> = app
            .cassettes
            .iter()
            .enumerate()
            .map(|(i, c)| match &c.topic {
                Some(t) => format!("{}: {}", t, c.word_count()),
                None => format!("cassette {}: {}", i + 1, c.word_count()),
            })
            .collect();
        line.push_str(&format!("\n  {}", parts.join(" · ")));
    }
    Some(line)
}

/// Deliver the session's words: stdout in `-o` mode, the sink file otherwise.
/// Empty sessions write nothing (and clean up an autosaved draft).
fn finish_session(app: &App, sink: Option<&mut Sink>) {
    let Some(sink) = sink else {
        for (i, cassette) in app.cassettes.iter().enumerate() {
            match &cassette.topic {
                Some(topic) => println!("Words recorded to Cassette {} — {}:\n", i + 1, topic),
                None => println!("Words recorded to Cassette {}:\n", i + 1),
            }
            println!("Side A:\n\n{}", cassette.side_a_text());
            let side_b = cassette.side_b_text();
            if !side_b.trim().is_empty() {
                println!("\nSide B:\n\n{}", side_b);
            }
        }
        return;
    };

    if app.is_empty() {
        if sink.wrote {
            let _ = std::fs::remove_file(&sink.path);
        }
        eprintln!("cassette: nothing recorded — no file written");
        return;
    }

    if sink.conflicted {
        eprintln!(
            "cassette: '{}' already exists — saved to '{}' instead",
            sink.desired.display(),
            sink.path.display()
        );
    }
    match output::write_markdown(app, &sink.path) {
        Ok(()) if !sink.conflicted => eprintln!("cassette: saved to '{}'", sink.path.display()),
        Ok(()) => {}
        Err(e) => eprintln!("cassette: could not write '{}': {}", sink.path.display(), e),
    }
}

/// Print the `+themes` listing: every selectable theme with a color swatch
/// (RGB themes only — the default theme has no colors of its own to show).
fn print_themes(cfg: &config::Config) {
    let active = cfg.theme.as_deref().unwrap_or("default");
    println!("available themes:");
    for (name, theme, user) in theme::all(&cfg.themes) {
        let mut line = format!("  {:<18}", name);
        if let (
            Some(ratatui::style::Color::Rgb(fr, fg_, fb)),
            Some(ratatui::style::Color::Rgb(br, bg_, bb)),
        ) = (theme.text, theme.background)
        {
            line.push_str(&format!(
                " \x1b[38;2;{fr};{fg_};{fb}m\x1b[48;2;{br};{bg_};{bb}m Aa \x1b[0m"
            ));
        } else {
            line.push_str("     ");
        }
        if name == active {
            line.push_str("  (active)");
        }
        if user {
            line.push_str("  (user)");
        }
        println!("{line}");
    }
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mut sink: Option<&mut Sink>,
    theme: &theme::Theme,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();
    let mut last_autosave = Instant::now();

    loop {
        terminal.draw(|f| ui::render(f, app, theme))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    handle_key(app, key);
                    app.check_goal();
                }
                Event::Resize(w, h) => app.resize(w, h),
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick_timer();
            app.tick_status();
            app.tick_idle();
            last_tick = Instant::now();
        }

        if std::mem::take(&mut app.bell) {
            let mut out = io::stdout();
            let _ = out.write_all(b"\x07");
            let _ = out.flush();
        }

        // Crash safety: flush dirty text to the note file every AUTOSAVE_SECS.
        if let Some(s) = sink.as_deref_mut() {
            if app.dirty
                && !app.is_empty()
                && last_autosave.elapsed() >= Duration::from_secs(AUTOSAVE_SECS)
            {
                if output::write_markdown(app, &s.path).is_ok() {
                    s.wrote = true;
                    app.dirty = false;
                }
                last_autosave = Instant::now();
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    app.idle_secs = 0;

    // Topic entry owns the keyboard: no cassette switching or flipping while
    // the prompt is open, so the topic lands on the cassette it was opened for.
    if app.mode == Mode::Topic {
        handle_topic_key(app, key);
        return;
    }

    // Bindings that work in both modes.
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.should_quit = true;
            return;
        }
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.focus_next();
            return;
        }
        (KeyCode::BackTab, _) => {
            app.focus_prev();
            return;
        }
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            app.add_cassette();
            return;
        }
        // Flip to the other side: Shift+Enter (kitty-protocol terminals) or Ctrl+B.
        (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| c.flip());
            return;
        }
        // Topic prompt from either mode; `t` in normal mode does the same.
        (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
            open_topic_prompt(app);
            return;
        }
        _ => {}
    }

    match app.mode {
        Mode::Insert => handle_insert_key(app, key),
        Mode::Normal => handle_normal_key(app, key),
        Mode::Topic => unreachable!("topic mode is dispatched above"),
    }
}

/// Open the topic prompt pre-filled with the focused cassette's topic,
/// remembering which mode to drop back into when it closes.
fn open_topic_prompt(app: &mut App) {
    app.topic_input = app
        .cassettes
        .get(app.focus_idx)
        .and_then(|c| c.topic.clone())
        .unwrap_or_default();
    app.topic_return = app.mode;
    app.mode = Mode::Topic;
}

/// Status-line prompt for the focused cassette's topic: Enter commits
/// (a blank input clears the topic), Esc cancels. Either way the editor
/// drops back into the mode the prompt was opened from.
fn handle_topic_key(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,
        (KeyCode::Esc, _) => {
            app.topic_input.clear();
            app.mode = app.topic_return;
        }
        (KeyCode::Enter, _) => {
            let trimmed = app.topic_input.trim().to_string();
            let topic = (!trimmed.is_empty()).then_some(trimmed);
            app.modify_focused(|c| c.topic = topic);
            app.topic_input.clear();
            app.mode = app.topic_return;
        }
        (KeyCode::Backspace, _) => {
            app.topic_input.pop();
        }
        (KeyCode::Char(c), mods) if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
            app.topic_input.push(c);
        }
        _ => {}
    }
}

fn handle_insert_key(app: &mut App, key: KeyEvent) {
    // Record mode: the tape only rolls forward. No deletions, no normal
    // mode, no cursor movement — typing and Enter only.
    if app.record {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) => app.modify_focused(|c| c.insert('\n')),
            (KeyCode::Char(c), mods)
                if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                app.modify_focused(|cas| cas.insert(c));
            }
            _ => {}
        }
        return;
    }

    let cw = app.cassette_width();
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.mode = Mode::Normal;
            app.pending = None;
        }
        (KeyCode::Enter, _) => {
            app.modify_focused(|c| c.insert('\n'));
        }
        (KeyCode::Left, KeyModifiers::NONE) => app.modify_focused(|c| c.move_left()),
        (KeyCode::Right, KeyModifiers::NONE) => app.modify_focused(|c| c.move_right()),
        (KeyCode::Up, KeyModifiers::NONE) => app.modify_focused(|c| c.move_up(cw)),
        (KeyCode::Down, KeyModifiers::NONE) => app.modify_focused(|c| c.move_down(cw)),
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            app.modify_focused(|c| c.backspace());
        }
        (KeyCode::Delete, KeyModifiers::NONE) => {
            app.modify_focused(|c| c.delete());
        }
        // Readline muscle memory: delete word / to line start.
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| {
                c.snapshot();
                c.delete_word_back();
            });
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| {
                c.snapshot();
                c.delete_to_line_start();
            });
        }
        (KeyCode::Char(c), mods) if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
            app.modify_focused(|cas| cas.insert(c));
        }
        _ => {}
    }
}

fn handle_normal_key(app: &mut App, key: KeyEvent) {
    let cw = app.cassette_width();

    // Resolve two-key sequences (dd, gg); any other key cancels the prefix
    // and is then handled normally.
    if let Some(prefix) = app.pending.take() {
        if let KeyCode::Char(c) = key.code {
            match (prefix, c) {
                ('d', 'd') => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.delete_line();
                    });
                    return;
                }
                ('g', 'g') => {
                    app.modify_focused(|c| c.move_text_start());
                    return;
                }
                _ => {}
            }
        }
    }

    match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            match c {
                'q' => app.should_quit = true,
                // Entering insert mode snapshots, so `u` undoes the whole burst.
                'i' => {
                    app.modify_focused(|c| c.snapshot());
                    app.mode = Mode::Insert;
                }
                'a' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.move_right();
                    });
                    app.mode = Mode::Insert;
                }
                'I' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.move_row_start(cw);
                    });
                    app.mode = Mode::Insert;
                }
                'A' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.move_row_end(cw);
                    });
                    app.mode = Mode::Insert;
                }
                'o' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.open_below();
                    });
                    app.mode = Mode::Insert;
                }
                'O' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.open_above();
                    });
                    app.mode = Mode::Insert;
                }
                't' => open_topic_prompt(app),
                'u' => app.modify_focused(|c| c.undo()),
                'h' => app.modify_focused(|c| c.move_left()),
                'l' => app.modify_focused(|c| c.move_right()),
                'j' => app.modify_focused(|c| c.move_down(cw)),
                'k' => app.modify_focused(|c| c.move_up(cw)),
                '0' => app.modify_focused(|c| c.move_row_start(cw)),
                '$' => app.modify_focused(|c| c.move_row_end(cw)),
                'w' => app.modify_focused(|c| c.move_word_forward()),
                'b' => app.modify_focused(|c| c.move_word_back()),
                'G' => app.modify_focused(|c| c.move_text_end()),
                'x' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.delete();
                    });
                }
                'd' => app.pending = Some('d'),
                'g' => app.pending = Some('g'),
                _ => {}
            }
        }
        KeyCode::Left => app.modify_focused(|c| c.move_left()),
        KeyCode::Right => app.modify_focused(|c| c.move_right()),
        KeyCode::Up => app.modify_focused(|c| c.move_up(cw)),
        KeyCode::Down => app.modify_focused(|c| c.move_down(cw)),
        KeyCode::Delete => {
            app.modify_focused(|c| {
                c.snapshot();
                c.delete();
            });
        }
        _ => {}
    }
}

struct Args {
    timer_secs: Option<u32>,
    word_goal: Option<usize>,
    note_name: Option<String>,
    print_stdout: bool,
    visible_lines: Option<usize>,
    template: Option<String>,
    theme: Option<String>,
    list_themes: bool,
    record: bool,
}

const USAGE: &str = "\
cassette — a freewriting TUI

Usage: cassette [OPTIONS] [NAME]

Arguments:
  [NAME]         output note name or path
                 (default: timestamped file in the notes dir)

Options:
  -t <MINUTES>   countdown timer in minutes
  -w <WORDS>     word goal (winds the tape reel)
  -l <LINES>     visible text rows per cassette (2-40)
  -T <TEMPLATE>  start with one cassette per topic from the named
                 [templates] entry in config.toml
  --theme <NAME> color theme for this session (overrides config)
  -R, --record   record mode: no deletions, the tape only rolls forward
  -o, --output   print to stdout on quit instead of writing a file
  -h, --help     print this help
  -V, --version  print version

Actions:
  +themes        list available themes (built-in and from config.toml)
";

fn die(msg: &str) -> ! {
    eprintln!("cassette: {msg}");
    eprintln!("try 'cassette --help'");
    std::process::exit(2);
}

/// Parse the value after a flag as a positive number, or exit with an error.
fn positive<T: std::str::FromStr + PartialOrd + From<u8>>(flag: &str, val: Option<&String>) -> T {
    let Some(v) = val else {
        die(&format!("option '{flag}' needs a value"));
    };
    match v.parse::<T>() {
        Ok(n) if n >= T::from(1u8) => n,
        _ => die(&format!(
            "invalid value '{v}' for '{flag}': expected a number >= 1"
        )),
    }
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut timer = None;
    let mut word_goal = None;
    let mut note_name: Option<String> = None;
    let mut print_stdout = false;
    let mut visible_lines = None;
    let mut template = None;
    let mut theme = None;
    let mut list_themes = false;
    let mut record = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("cassette {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-t" => {
                timer = Some(positive::<u32>("-t", args.get(i + 1)) * 60);
                i += 2;
            }
            "-w" => {
                word_goal = Some(positive::<u32>("-w", args.get(i + 1)) as usize);
                i += 2;
            }
            "-l" => {
                visible_lines = Some(positive::<u32>("-l", args.get(i + 1)) as usize);
                i += 2;
            }
            "-T" => {
                let Some(name) = args.get(i + 1) else {
                    die("option '-T' needs a value");
                };
                template = Some(name.clone());
                i += 2;
            }
            "--theme" => {
                let Some(name) = args.get(i + 1) else {
                    die("option '--theme' needs a value");
                };
                theme = Some(name.clone());
                i += 2;
            }
            "+themes" => {
                list_themes = true;
                i += 1;
            }
            "-R" | "--record" => {
                record = true;
                i += 1;
            }
            "-o" | "--output" => {
                print_stdout = true;
                i += 1;
            }
            arg if arg.starts_with('-') => {
                die(&format!("unknown option '{arg}'"));
            }
            arg => {
                if note_name.is_some() {
                    die(&format!("unexpected extra argument '{arg}'"));
                }
                note_name = Some(arg.to_string());
                i += 1;
            }
        }
    }
    Args {
        timer_secs: timer,
        word_goal,
        note_name,
        print_stdout,
        visible_lines,
        template,
        theme,
        list_themes,
        record,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cassette::Side;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            handle_key(app, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn ctrl_b_flips_side_and_ctrl_f_does_not() {
        let mut app = App::new(None, None, None);
        handle_key(&mut app, key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::B);
        handle_key(&mut app, key(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::B, "^F is no longer bound");
        handle_key(&mut app, key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::A);
    }

    #[test]
    fn topic_prompt_sets_edits_and_clears() {
        let mut app = App::new(None, None, None);
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE)); // -> normal

        // Set a topic.
        type_str(&mut app, "t");
        assert_eq!(app.mode, Mode::Topic);
        type_str(&mut app, "morning pages");
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("morning pages"));

        // Reopening pre-fills; backspace edits.
        type_str(&mut app, "t");
        assert_eq!(app.topic_input, "morning pages");
        for _ in 0..6 {
            handle_key(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("morning"));

        // A blank commit clears the topic.
        type_str(&mut app, "t");
        for _ in 0..7 {
            handle_key(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].topic, None);
    }

    #[test]
    fn ctrl_t_opens_topic_prompt_from_both_modes_and_returns() {
        let mut app = App::new(None, None, None);
        // From insert mode: Ctrl+T opens the prompt, Enter returns to insert.
        assert_eq!(app.mode, Mode::Insert);
        handle_key(&mut app, key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Topic);
        type_str(&mut app, "flow");
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert, "returns to the mode it came from");
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("flow"));

        // From normal mode: Esc-cancel returns to normal.
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        handle_key(&mut app, key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Topic);
        assert_eq!(app.topic_input, "flow", "pre-filled");
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn record_mode_only_rolls_forward() {
        let mut app = App::new(None, None, None);
        app.record = true;
        type_str(&mut app, "no going back");
        // Deletions and mode switches are ignored.
        handle_key(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        handle_key(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        handle_key(&mut app, key(KeyCode::Char('w'), KeyModifiers::CONTROL));
        handle_key(&mut app, key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert, "no normal mode while recording");
        // Cursor can't move back either.
        handle_key(&mut app, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].cursor_pos(), 13);
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        type_str(&mut app, "ok");
        assert_eq!(app.cassettes[0].text(), "no going back\nok");
    }

    #[test]
    fn record_mode_keeps_flip_topic_and_quit() {
        let mut app = App::new(None, None, None);
        app.record = true;
        handle_key(&mut app, key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::B, "flipping is not editing");
        handle_key(&mut app, key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Topic);
        // Backspace still works inside the topic prompt (metadata, not tape).
        type_str(&mut app, "xy");
        handle_key(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("x"));
        assert_eq!(app.mode, Mode::Insert);
        handle_key(&mut app, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn keypress_resets_idle_counter() {
        let mut app = App::new(Some(60), None, None);
        for _ in 0..App::IDLE_NUDGE_SECS {
            app.tick_idle();
        }
        assert!(app.idle_nudge());
        type_str(&mut app, "a");
        assert!(!app.idle_nudge(), "typing clears the nudge");
    }

    #[test]
    fn session_summary_reports_words_and_breakdown() {
        let mut app = App::new(None, None, None);
        assert!(session_summary(&app).is_none(), "empty session: no summary");
        app.modify_focused(|c| {
            for ch in "one two three".chars() {
                c.insert(ch);
            }
            c.topic = Some("morning".into());
        });
        app.add_cassette();
        app.modify_focused(|c| {
            for ch in "four five".chars() {
                c.insert(ch);
            }
        });
        let s = session_summary(&app).unwrap();
        assert!(s.starts_with("5 words in 0:0"), "summary was: {s}");
        assert!(!s.contains("wpm"), "no pace on a sub-30s session");
        assert!(s.contains("morning: 3"));
        assert!(s.contains("cassette 2: 2"));
    }

    #[test]
    fn topic_prompt_esc_cancels_without_change() {
        let mut app = App::new(None, None, None);
        app.cassettes[0].topic = Some("keep me".into());
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE)); // -> normal
        type_str(&mut app, "toverwrite");
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("keep me"));
        assert!(app.topic_input.is_empty());
    }

    #[test]
    fn topic_prompt_captures_tab_and_letters_as_text() {
        let mut app = App::new(None, None, None);
        app.add_cassette();
        app.focus_idx = 0;
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE)); // -> normal
        type_str(&mut app, "tq"); // 'q' must not quit inside the prompt
        assert!(!app.should_quit);
        handle_key(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus_idx, 0, "Tab must not switch cassettes mid-prompt");
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("q"));
    }
}
