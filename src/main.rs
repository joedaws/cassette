use std::io;
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
mod ui;

use app::{App, Mode};

fn main() -> io::Result<()> {
    let args = parse_args();
    let cfg = config::load_config();

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
    app.resize(size.width, size.height);

    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result?;

    if args.print_stdout {
        for (i, cassette) in app.cassettes.iter().enumerate() {
            println!("Words recorded to Cassette {}:\n", i + 1);
            println!("{}", cassette.side_a_text());
            let side_b = cassette.side_b_text();
            if !side_b.trim().is_empty() {
                println!("\nSide B:\n\n{}", side_b);
            }
        }
    } else {
        let effective_notes_dir = cfg
            .notes_dir
            .clone()
            .or_else(config::default_notes_dir);
        let desired = config::resolve_output_path(
            args.note_name.as_deref(),
            &app.started_at,
            effective_notes_dir.as_deref(),
        );
        let (path, conflicted) = config::find_available_path(&desired);
        if conflicted {
            eprintln!(
                "cassette: '{}' already exists — saved to '{}' instead",
                desired.display(),
                path.display()
            );
        }
        match output::write_markdown(&app, &path) {
            Ok(()) if !conflicted => eprintln!("cassette: saved to '{}'", path.display()),
            Ok(()) => {}
            Err(e) => eprintln!("cassette: could not write '{}': {}", path.display(), e),
        }
    }

    Ok(())
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => handle_key(app, key),
                Event::Resize(w, h) => app.resize(w, h),
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick_timer();
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
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
        // Flip to the other side: Shift+Enter (kitty-protocol terminals) or Ctrl+F.
        (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| c.flip());
            app.advance_reel();
            return;
        }
        _ => {}
    }

    match app.mode {
        Mode::Insert => handle_insert_key(app, key),
        Mode::Normal => handle_normal_key(app, key),
    }
}

fn handle_insert_key(app: &mut App, key: KeyEvent) {
    let cw = app.cassette_width();
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.mode = Mode::Normal;
            app.pending = None;
        }
        (KeyCode::Enter, _) => {
            app.modify_focused(|c| c.insert('\n'));
            app.advance_reel();
        }
        (KeyCode::Left, KeyModifiers::NONE) => app.modify_focused(|c| c.move_left()),
        (KeyCode::Right, KeyModifiers::NONE) => app.modify_focused(|c| c.move_right()),
        (KeyCode::Up, KeyModifiers::NONE) => app.modify_focused(|c| c.move_up(cw)),
        (KeyCode::Down, KeyModifiers::NONE) => app.modify_focused(|c| c.move_down(cw)),
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            app.modify_focused(|c| c.backspace());
            app.advance_reel();
        }
        (KeyCode::Delete, KeyModifiers::NONE) => {
            app.modify_focused(|c| c.delete());
            app.advance_reel();
        }
        (KeyCode::Char(c), mods)
            if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.modify_focused(|cas| cas.insert(c));
            app.advance_reel();
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
                    app.modify_focused(|c| c.delete_line());
                    app.advance_reel();
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
                'i' => app.mode = Mode::Insert,
                'a' => {
                    app.modify_focused(|c| c.move_right());
                    app.mode = Mode::Insert;
                }
                'I' => {
                    app.modify_focused(|c| c.move_row_start(cw));
                    app.mode = Mode::Insert;
                }
                'A' => {
                    app.modify_focused(|c| c.move_row_end(cw));
                    app.mode = Mode::Insert;
                }
                'o' => {
                    app.modify_focused(|c| c.open_below());
                    app.mode = Mode::Insert;
                    app.advance_reel();
                }
                'O' => {
                    app.modify_focused(|c| c.open_above());
                    app.mode = Mode::Insert;
                    app.advance_reel();
                }
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
                    app.modify_focused(|c| c.delete());
                    app.advance_reel();
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
            app.modify_focused(|c| c.delete());
            app.advance_reel();
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
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut timer = None;
    let mut word_goal = None;
    let mut note_name: Option<String> = None;
    let mut print_stdout = false;
    let mut visible_lines = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" if i + 1 < args.len() => {
                if let Ok(mins) = args[i + 1].parse::<u32>() {
                    if mins > 0 {
                        timer = Some(mins * 60);
                    }
                }
                i += 2;
            }
            "-w" if i + 1 < args.len() => {
                if let Ok(goal) = args[i + 1].parse::<usize>() {
                    if goal > 0 {
                        word_goal = Some(goal);
                    }
                }
                i += 2;
            }
            "-l" if i + 1 < args.len() => {
                if let Ok(lines) = args[i + 1].parse::<usize>() {
                    if lines > 0 {
                        visible_lines = Some(lines);
                    }
                }
                i += 2;
            }
            "-o" | "--output" => {
                print_stdout = true;
                i += 1;
            }
            arg if !arg.starts_with('-') => {
                note_name = Some(arg.to_string());
                i += 1;
            }
            _ => i += 1,
        }
    }
    Args {
        timer_secs: timer,
        word_goal,
        note_name,
        print_stdout,
        visible_lines,
    }
}
