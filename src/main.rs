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
    app.resize(size.width, size.height);

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
        run(&mut terminal, &mut app, sink.as_mut())
    }));

    restore_terminal();

    // Save before propagating any error or panic: the words matter most.
    finish_session(&app, sink.as_mut());

    match result {
        Ok(r) => r?,
        Err(payload) => panic::resume_unwind(payload),
    }
    Ok(())
}

/// Deliver the session's words: stdout in `-o` mode, the sink file otherwise.
/// Empty sessions write nothing (and clean up an autosaved draft).
fn finish_session(app: &App, sink: Option<&mut Sink>) {
    let Some(sink) = sink else {
        for (i, cassette) in app.cassettes.iter().enumerate() {
            println!("Words recorded to Cassette {}:\n", i + 1);
            println!("{}", cassette.side_a_text());
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

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mut sink: Option<&mut Sink>,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();
    let mut last_autosave = Instant::now();

    loop {
        terminal.draw(|f| ui::render(f, app))?;

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
            // The tape rolls while the session runs, not only on keystrokes.
            app.advance_reel();
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
        // Readline muscle memory: delete word / to line start.
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| {
                c.snapshot();
                c.delete_word_back();
            });
            app.advance_reel();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.modify_focused(|c| {
                c.snapshot();
                c.delete_to_line_start();
            });
            app.advance_reel();
        }
        (KeyCode::Char(c), mods) if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
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
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.delete_line();
                    });
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
                    app.advance_reel();
                }
                'O' => {
                    app.modify_focused(|c| {
                        c.snapshot();
                        c.open_above();
                    });
                    app.mode = Mode::Insert;
                    app.advance_reel();
                }
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
            app.modify_focused(|c| {
                c.snapshot();
                c.delete();
            });
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
  -o, --output   print to stdout on quit instead of writing a file
  -h, --help     print this help
  -V, --version  print version
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
    }
}
