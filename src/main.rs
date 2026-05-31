use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

mod app;
mod cassette;
mod ui;

use app::App;

fn main() -> io::Result<()> {
    let (timer_secs, word_goal) = parse_args();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let mut app = App::new(timer_secs, word_goal);
    app.resize(size.width, size.height);

    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result?;

    for (i, cassette) in app.cassettes.iter().enumerate() {
        println!("Words recorded to Cassette {}:\n", i + 1);
        println!("{}", cassette.text());
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
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => app.should_quit = true,
        (KeyCode::Tab, KeyModifiers::NONE) => app.focus_next(),
        (KeyCode::BackTab, _) => app.focus_prev(),
        (KeyCode::Char('n'), KeyModifiers::CONTROL) => app.add_cassette(),
        (KeyCode::Left, KeyModifiers::NONE) => app.modify_focused(|c| c.move_left()),
        (KeyCode::Right, KeyModifiers::NONE) => app.modify_focused(|c| c.move_right()),
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

fn parse_args() -> (Option<u32>, Option<usize>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut timer = None;
    let mut word_goal = None;
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
            _ => i += 1,
        }
    }
    (timer, word_goal)
}
