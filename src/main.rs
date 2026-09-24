use std::collections::HashMap;
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

mod app;
mod cassette;
mod cli;
mod config;
mod export;
mod find;
mod output;
mod picker;
mod queue;
mod session;
mod session_writer;
mod stats;
mod store;
mod theme;
mod ui;
mod writer;

use app::{App, Mode};

/// Seconds between autosaves of a dirty session.
const AUTOSAVE_SECS: u64 = 30;

/// Signal flags checked by the event loop (unix only): SIGTERM/SIGHUP ask for
/// a save-and-quit, SIGTSTP for a clean suspend. Registered handlers only set
/// these `AtomicBool`s; all real work happens on the main thread.
#[cfg(unix)]
struct SignalFlags {
    terminate: std::sync::Arc<std::sync::atomic::AtomicBool>,
    suspend: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(unix)]
fn install_signal_handlers() -> io::Result<SignalFlags> {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let terminate = Arc::new(AtomicBool::new(false));
    let suspend = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, terminate.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, terminate.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGTSTP, suspend.clone())?;
    Ok(SignalFlags { terminate, suspend })
}

/// Best-effort terminal restore; must be safe to call twice and mid-panic.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

fn main() -> io::Result<()> {
    let mut args = cli::parse();
    let cfg = config::load_config().unwrap_or_else(|e| exit_with(2, &e, args.json));

    if args.list_themes {
        print_themes(&cfg);
        return Ok(());
    }

    // `stats` and `find` read only the session store — the legacy notes dir
    // (the deprecated `cfg.notes_dir`) is deliberately not
    // consulted here. See the design's decision 7: existing notes stop
    // appearing in these two commands, on purpose, with no fallback and no
    // migration; the files themselves are untouched.
    if args.stats {
        let store = store::Store::new(store_root(args.json));
        let (metas, unreadable) = stats::scan_store(&store);
        println!(
            "{}",
            stats::render(&metas, chrono::Local::now().date_naive(), unreadable)
        );
        return Ok(());
    }

    if let Some(session) = &args.export {
        let store = store::Store::new(store_root(args.json));
        // The same validation gate the queue commands pass through: an
        // unvalidated id is joined straight onto the store root, and
        // `scan_session` treats a missing directory as an empty session — so
        // a typo would export silence instead of failing.
        if let Err(e) = queue::require_session(&store, session) {
            exit_queue_err(&e, args.json);
        }
        // Exit 1, not 2: these are I/O failures the caller cannot fix by
        // trying a different invocation, and `--help` does not help with
        // `Is a directory`. Routed through `exit_with` so `--json` gets the
        // {error, code} envelope an agent branches on, rather than prose.
        let scan = store.scan_session(session).unwrap_or_else(|e| {
            exit_with(
                1,
                &format!("cannot read session '{session}': {e}"),
                args.json,
            )
        });
        let rendered = export::render(&scan);
        match &args.export_out {
            Some(path) => std::fs::write(path, &rendered).unwrap_or_else(|e| {
                exit_with(
                    1,
                    &format!("cannot write '{}': {e}", path.display()),
                    args.json,
                )
            }),
            // `write_all`, not `print!`: Rust ignores SIGPIPE, so `print!`
            // PANICS when the reader closes the pipe — and `export … | head`
            // or `| less` is the documented way to read one. A closed pipe
            // is the reader saying "enough", which is a clean exit, not an
            // error worth a backtrace.
            None => {
                use std::io::Write;
                if let Err(e) = io::stdout().write_all(rendered.as_bytes()) {
                    if e.kind() != io::ErrorKind::BrokenPipe {
                        exit_with(1, &format!("cannot write to stdout: {e}"), args.json);
                    }
                }
            }
        }
        return Ok(());
    }

    if let Some(words) = &args.find {
        let store = store::Store::new(store_root(args.json));
        let (entries, unreadable) = find::scan_store(&store);
        let query = (!words.is_empty()).then(|| words.join(" "));
        println!("{}", find::render(&entries, query.as_deref(), unreadable));
        return Ok(());
    }

    if let Some(cmd) = &args.queue_cmd {
        let store = store::Store::new(store_root(args.json));
        // Before any command touches the store: `--session` must be a
        // well-formed ULID naming a session that exists. Done here, once,
        // against `QueueCmd::session()` rather than inside each command —
        // see `queue::require_session`. Unvalidated, the id is joined
        // straight onto the store root by `Store::session_dir`, and the
        // permissive reads below (`scan_session` treats a missing directory
        // as an empty session) would otherwise report a typo as an empty
        // queue.
        if let Err(e) = queue::require_session(&store, cmd.session()) {
            exit_queue_err(&e, args.json);
        }
        match cmd {
            // The only command that creates a cassette, so — like `write` —
            // it needs a writer identity to attribute the creation to.
            cli::QueueCmd::New {
                topic,
                session,
                placement,
            } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                let max_open = cfg.max_open.unwrap_or(store::MAX_OPEN);
                match queue::edit::new(
                    &store,
                    session,
                    topic,
                    *placement,
                    &who_name,
                    writer_source,
                    max_open,
                ) {
                    Ok(id) => exit_queue_ok(Some(id)),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            // The only queue command that writes an existing cassette, so
            // it's the only one that needs a writer identity to attribute the
            // write to.
            cli::QueueCmd::Write {
                id,
                session,
                side,
                mode,
            } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::write(&store, id, session, *side, *mode, &who_name, writer_source) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            // `list` and `show` attribute nothing, so unlike `write` they
            // never resolve a writer — a viewer with no $USER and no
            // registered identity can still read the queue.
            cli::QueueCmd::List {
                session,
                status,
                since,
            } => {
                if args.json {
                    match queue::view::list_view(&store, session, *status, since.as_deref()) {
                        Ok(listing) => exit_queue_ok(Some(
                            serde_json::to_string(&listing).expect("Listing always serializes"),
                        )),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                } else {
                    match queue::view::list(&store, session, *status, since.as_deref()) {
                        Ok(msg) => exit_queue_ok(Some(msg)),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                }
            }
            cli::QueueCmd::Show { id, session } => {
                if args.json {
                    match queue::view::show_view(&store, session, id) {
                        Ok(view) => exit_queue_ok(Some(
                            serde_json::to_string(&view).expect("CassetteView always serializes"),
                        )),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                } else {
                    match queue::view::show(&store, session, id) {
                        Ok(msg) => exit_queue_ok(Some(msg)),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                }
            }
            // No writer identity: `next` reports an id, it attributes
            // nothing.
            cli::QueueCmd::Next { session } => {
                if args.json {
                    match queue::view::next_view(&store, session) {
                        Ok(view) => exit_queue_ok(Some(
                            serde_json::to_string(&view).expect("CassetteView always serializes"),
                        )),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                } else {
                    match queue::view::next(&store, session) {
                        Ok(msg) => exit_queue_ok(Some(msg)),
                        Err(e) => exit_queue_err(&e, args.json),
                    }
                }
            }
            // `close` attributes the closure and, when the acting writer is
            // an agent, needs its `Kind` to enforce the sticky-lock
            // boundary — see `queue::edit::close_permitted`.
            cli::QueueCmd::Close {
                id,
                session,
                message,
            } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::edit::close(
                    &store,
                    session,
                    id,
                    message.as_deref(),
                    &who_name,
                    writer_source,
                ) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            // `reopen` raises the session's open count, so — like `new` — it
            // needs a writer identity to attribute the change to and the
            // configured `max_open` cap.
            cli::QueueCmd::Reopen { id, session } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                let max_open = cfg.max_open.unwrap_or(store::MAX_OPEN);
                match queue::edit::reopen(&store, session, id, &who_name, writer_source, max_open) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            // `move` reprioritizes an existing cassette, so — like `close`
            // and `reopen` — it needs a writer identity to attribute the
            // change to.
            cli::QueueCmd::Move {
                id,
                session,
                anchor,
            } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::edit::move_cassette(
                    &store,
                    session,
                    id,
                    anchor.clone(),
                    &who_name,
                    writer_source,
                ) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            // `lock`/`unlock` are human-only: `edit::lock`/`edit::unlock`
            // reject a non-human writer with `Usage` (exit 2) before
            // acquiring anything, so they need the resolved writer identity
            // exactly like `close`/`reopen`/`move` do.
            cli::QueueCmd::Lock { id, session } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::edit::lock(&store, session, id, &who_name, writer_source) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
            cli::QueueCmd::Unlock { id, session } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::edit::unlock(&store, session, id, &who_name, writer_source) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
        }
    }

    if let Some(cmd) = &args.writer_cmd {
        run_writer_cmd(cmd, args.writer.as_deref(), args.json);
    }

    if let Some(cmd) = &args.session_cmd {
        run_session_cmd(cmd, args.json);
    }

    // Resolve the theme and topic template before touching the terminal so
    // an unknown name can die() cleanly.
    let theme = match theme::resolve(args.theme.as_deref().or(cfg.theme.as_deref()), &cfg.themes) {
        Ok(t) => t,
        Err(e) => die(&e),
    };
    let daily_name: Option<String> = args.daily.then(|| {
        let fmt = cfg.daily_format.as_deref().unwrap_or("%Y-%m-%d");
        match daily_note_name(fmt) {
            Ok(n) => n,
            Err(e) => die(&e),
        }
    });
    let template_topics: Option<Vec<String>> = args.template.as_deref().map(|name| {
        cfg.templates.get(name).cloned().unwrap_or_else(|| {
            die(&format!(
                "unknown template '{name}' — define it under [templates] in config.toml"
            ))
        })
    });

    // Refused BEFORE the picker runs, and named after the flag the user
    // actually typed. Letting it through would make the human choose a
    // session and only then be told the combination is impossible — and the
    // refusal would cite 'resume', which they never typed, because the
    // picker sets it. This codebase's rule is that refusals come before
    // anything is touched.
    if args.pick_session && args.template.is_some() {
        die("'sessions' opens an existing session, so it cannot be combined with '-T'");
    }
    if args.pick_session && args.print_stdout {
        die("'-o' persists nothing, so there is no session for 'sessions' to open");
    }

    // After the theme is resolved, so the picker is drawn in the theme the
    // session will open in — and so an unknown `--theme` dies before the
    // human picks anything rather than after. It returns a session id and
    // falls through to the ordinary TUI path rather than launching it:
    // returning a choice keeps `Picker` a state machine tests can drive
    // without a terminal.
    if args.pick_session {
        let store = store::Store::new(store_root(args.json));
        let (entries, unreadable) = find::scan_store(&store);
        match run_picker(picker::Picker::new(entries, unreadable), &theme)? {
            // Handed to `resume`, which already resolves a session id and
            // loads its cassettes. A second opening path would be a second
            // set of rules about what an id means.
            Some(id) => args.resume = Some(Some(id)),
            // Quitting the picker is a normal exit, not a refusal.
            None => return Ok(()),
        }
    }

    if args.resume.is_some() && args.template.is_some() {
        die("'resume' cannot be combined with '-T'");
    }

    // `-o` prints to stdout and persists nothing, so it opens no store
    // session at all — which makes every subcommand that *names* one
    // meaningless under it. Silently ignoring the subcommand is what this
    // used to do: `cassette -o resume` opened a blank editor and printed
    // only what was typed in that sitting, with the resumed words nowhere.
    // Honouring them instead is not available: `new` and `today` have to
    // create a session when none exists, which is exactly the persistence
    // `-o` promises not to do, and honouring only `resume` would make one
    // flag mean two things. So the combination is refused, in one rule, the
    // way `resume` + `-T` already is.
    if args.print_stdout {
        let named = if args.resume.is_some() {
            Some("resume")
        } else if args.daily {
            Some("today")
        } else if args.note_name.is_some() {
            Some("new")
        } else {
            None
        };
        if let Some(named) = named {
            die(&format!(
                "'-o' persists nothing, so it cannot be combined with '{named}' — drop '-o' \
                 to write to the store, or drop '{named}' to print this sitting \
                 to stdout"
            ));
        }
    }

    // Resolve the store session, build the app and take the focused
    // cassette's lock before the terminal is touched, so anything that goes
    // wrong here dies cleanly onto a normal shell.
    //
    // `-o` prints to stdout and persists nothing, so it opens no session at
    // all: `app.session` stays empty rather than naming a session that was
    // never created.
    let store = (!args.print_stdout).then(|| store::Store::new(store_root(args.json)));
    let (session, created_here, loaded) = match &store {
        Some(store) => resolve_session(store, &args, daily_name.as_deref()),
        None => (String::new(), false, None),
    };

    let visible_lines = args.visible_lines.or(cfg.visible_lines);
    let mut app = App::new(args.timer_secs, args.word_goal, visible_lines, session);
    app.record = args.record;
    if let Some(topics) = &template_topics {
        app.apply_topics(topics);
    }
    if let Some(loaded) = loaded {
        app.load_cassettes(loaded.cassettes);
        app.damaged = loaded.damaged;
    }

    let mut writer = store.as_ref().map(|s| {
        let (writer_id, writer_name) = resolve_tui_writer(s, &args);
        session_writer::SessionWriter::open(s, &app.session, created_here, &writer_id, &writer_name)
    });
    if let Some(w) = writer.as_mut() {
        // Every cassette on screen is a store cassette — the ones a template
        // seeded and the single empty one a bare launch starts with alike.
        if let Err(e) = w.create_missing_cassettes(&mut app) {
            die_with(
                1,
                &format!("cannot create cassettes in session '{}': {e}", app.session),
            );
        }
        // Focus means held. A cassette another writer already holds opens
        // read-only rather than refusing to start: refusing would let a
        // running agent lock a human out of their own session, and accepting
        // keystrokes with no guard to write them through would lose them.
        // Through `try_acquire`, not by hand: it is the one place that
        // decides what a failed acquire looks like, and hand-setting
        // `read_only` here used to skip `busy_holder` — so the opening
        // frames named nobody until the first tick's retry filled it in,
        // and a non-contention failure was reported only by an `eprintln!`
        // the alternate screen immediately wiped.
        let focus = app.focus_idx;
        try_acquire(&mut app, w, focus);
    }

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
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableBracketedPaste
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    app.resize(size.width, size.height);

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        run(
            &mut terminal,
            &mut app,
            writer.as_mut(),
            store.as_ref(),
            &theme,
        )
    }));

    restore_terminal();

    // Save before propagating any error or panic: the words matter most.
    // `finish` flushes through the guard it holds and only then drops it —
    // dropping first would lose the text or force a re-acquire `flock` may
    // refuse.
    if let Some(w) = writer.as_mut() {
        if let Err(e) = w.finish(&mut app) {
            eprintln!("cassette: could not save the session: {e}");
        }
    }
    finish_session(&app, writer.as_ref().map(|w| w.session()));
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
/// several): words, duration, and pace. `None` for empty sessions. A resumed
/// session counts only the words added on top of `baseline_words` — and stays
/// quiet when nothing new was written.
fn session_summary(app: &App) -> Option<String> {
    if app.is_empty() {
        return None;
    }
    let total = app.total_word_count();
    let words = total.saturating_sub(app.baseline_words);
    if app.baseline_words > 0 && words == 0 {
        return None;
    }
    let secs = app.started_at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    let label = if app.baseline_words > 0 { " new" } else { "" };
    let mut line = format!("{}{} words in {}:{:02}", words, label, secs / 60, secs % 60);
    // Pace needs a meaningful denominator; skip it for very short sessions.
    if secs >= 30 {
        line.push_str(&format!(" — {:.0} wpm", words as f64 * 60.0 / secs as f64));
    }
    if app.baseline_words > 0 {
        line.push_str(&format!(" ({} total)", total));
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

/// Report where the session's words went. In `-o` mode (`session` is `None`)
/// nothing was persisted, so they go to stdout here; otherwise
/// `SessionWriter::finish` has already written them and this only names the
/// session they landed in.
fn finish_session(app: &App, session: Option<&str>) {
    let Some(session) = session else {
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
        // `finish` has already removed a session this run created, so there
        // is nothing left to point at.
        eprintln!("cassette: nothing recorded — no session written");
        return;
    }
    eprintln!("cassette: saved to session {session}");
}

/// The writer identity a TUI session attributes its work to, resolved
/// exactly the way every queue command resolves it: `--writer`, else
/// `$CASSETTE_WRITER`, else `$USER` (`resolve_writer_name`), then through
/// the registry — auto-registering a bare `$USER` as a human, and requiring
/// that an explicitly named writer already exist, so a typo'd `--writer` is
/// exit 2 here as it is for `queue write`.
///
/// It is not cosmetic. The returned id lands in each cassette's
/// `last_writer`, and the name is stamped into the lock anchor another
/// writer reads when it is told who holds a cassette — so a TUI that
/// resolved this differently would attribute the same session to a
/// different writer than the CLI does.
///
/// Returns `(writer id, display name)`.
fn resolve_tui_writer(store: &store::Store, args: &cli::Args) -> (String, String) {
    let (name, source) = resolve_writer_name(args.writer.as_deref())
        .unwrap_or_else(|msg| exit_usage(&msg, args.json));
    let resolved = match source {
        queue::WriterSource::Env => store
            .resolve_writer(&name)
            .map_err(queue::resolve_error_to_queue_error),
        queue::WriterSource::Flag => store
            .require_writer(&name)
            .map_err(queue::require_error_to_queue_error),
    };
    match resolved {
        Ok((id, _kind)) => (id, name),
        Err(e) => exit_queue_err(&e, args.json),
    }
}

/// The store session this launch writes to, per the spec's entry-point
/// table:
///
/// | `cassette`               | a new session |
/// | `cassette new <NAME>`    | a new session aliased `<NAME>` |
/// | `cassette today`         | the session aliased with today's date, created if absent |
/// | `cassette resume [NAME]` | the most recent session, or the one with that alias |
/// | `cassette -T <template>` | a new session with one cassette per topic |
///
/// Returns the session id, whether **this run created it** — the only case
/// in which `SessionWriter::finish` may remove it again — and the cassettes
/// to load when an existing session was opened.
///
/// Aliases are not unique (`Store::set_session_alias` deliberately enforces
/// nothing), so a name resolves to the **most recent** session carrying it:
/// `list_sessions` is newest-first, and picking the newest is the only
/// answer that keeps `cassette today` idempotent across a day.
/// What opening an existing session read off disk: its cassettes, plus the
/// files that could not be loaded. Damaged files travel with the cassettes
/// rather than being logged, because an `eprintln!` here is wiped by the
/// alternate screen before a human can read it — `ui.rs` puts them on screen.
struct LoadedSession {
    cassettes: Vec<cassette::Cassette>,
    damaged: Vec<(String, String)>,
}

fn resolve_session(
    store: &store::Store,
    args: &cli::Args,
    daily_name: Option<&str>,
) -> (String, bool, Option<LoadedSession>) {
    let by_alias = |alias: &str| -> Option<String> {
        store
            .list_sessions()
            .unwrap_or_else(|e| die_with(1, &format!("cannot list sessions: {e}")))
            .into_iter()
            .find(|(_, m)| m.alias.as_deref() == Some(alias))
            .map(|(id, _)| id)
    };
    let open = |id: String| -> (String, bool, Option<LoadedSession>) {
        let (cassettes, damaged) = load_session_cassettes(store, &id);
        (id, false, Some(LoadedSession { cassettes, damaged }))
    };

    if let Some(name) = &args.resume {
        let id = match name {
            // An alias first, then the id itself. `cassette find` prints
            // session ids, and an id it printed that `resume` then rejected
            // would be a discovery loop that closes on nothing. 4b's
            // "sessions are named by ULID only, an alias never resolves"
            // governs `--session` on the queue commands, where an ambiguous
            // name would be resolved silently by a machine; `resume` is the
            // human entry point the 5a spec defines as "the most recent
            // session, or the one with that alias", and accepting the
            // printed id there adds an opening, never an ambiguity: aliases
            // are checked first, and a ULID-shaped alias would have to be
            // typed deliberately.
            Some(alias) => by_alias(alias)
                .or_else(|| {
                    store::ids::is_valid_id(alias)
                        .then(|| store.session_meta(alias).ok().map(|_| alias.clone()))
                        .flatten()
                })
                .unwrap_or_else(|| {
                    die(&format!(
                        "no session named '{alias}' — `cassette session list` shows what exists"
                    ))
                }),
            None => store
                .list_sessions()
                .unwrap_or_else(|e| die_with(1, &format!("cannot list sessions: {e}")))
                .into_iter()
                .next()
                .map(|(id, _)| id)
                .unwrap_or_else(|| die("no sessions to resume")),
        };
        return open(id);
    }

    if let Some(date) = daily_name {
        if let Some(id) = by_alias(date) {
            // `-T` seeds one cassette per topic into a *new* session;
            // `load_cassettes` then replaces everything `apply_topics`
            // built, so on a day that already has a session the topics were
            // silently discarded. `resume` + `-T` already dies rather than
            // discard them, and this is the same situation: the flag cannot
            // be honoured, so it is refused instead of ignored. On the first
            // launch of a day there is nothing to open and `-T` works
            // normally.
            if args.template.is_some() {
                die(&format!(
                    "today's session ('{date}') already exists, so '-T' has nothing to seed \
                     — drop '-T' to continue today's session, or start a \
                     separate one with 'cassette -T <template>'"
                ));
            }
            return open(id);
        }
        return (create_session(store, args, Some(date)), true, None);
    }

    (
        create_session(store, args, args.note_name.as_deref()),
        true,
        None,
    )
}

fn create_session(store: &store::Store, args: &cli::Args, alias: Option<&str>) -> String {
    store
        .create_session(&store::session::SessionMeta {
            alias: alias.map(|a| a.to_string()),
            created: store::meta::now_utc(),
            timer_secs: args.timer_secs,
            word_goal: args.word_goal,
        })
        .unwrap_or_else(|e| die_with(1, &format!("cannot create a session: {e}")))
}

/// Read a session's cassettes back into the editor, in queue order.
///
/// Bodies are split with `json::split_sides` — the same parser the JSON
/// contract reads a cassette with, and the inverse of the `output::
/// cassette_body` that wrote them — and trimmed, because `cassette_body`
/// trims on the way out; without it every resume would grow a leading blank
/// line that the next save silently removed again.
///
/// Closed cassettes are loaded too. The TUI has no notion of closed yet
/// (5c adds the collapsed row), and hiding text a human wrote would look
/// exactly like losing it.
fn load_session_cassettes(
    store: &store::Store,
    session: &str,
) -> (Vec<cassette::Cassette>, Vec<(String, String)>) {
    let scan = store
        .scan_session(session)
        .unwrap_or_else(|e| die_with(1, &format!("cannot read session '{session}': {e}")));
    // Not an eprintln any more: the alternate screen wipes one before a
    // human can read it. These ride out to `App.damaged` and onto the screen.
    let damaged: Vec<(String, String)> = scan
        .damaged
        .iter()
        .map(|d| (d.label(), d.reason.to_string()))
        .collect();
    // Resolved once for the whole session rather than per cassette; a
    // registry read failure degrades to showing raw writer ids rather than
    // failing the load.
    let writers = store.writers().unwrap_or_default();
    let cassettes: Vec<cassette::Cassette> = scan
        .cassettes
        .into_iter()
        .map(|c| {
            let (side_a, side_b) = queue::json::split_sides(&c.body);
            let mut loaded = cassette::Cassette::from_sides(
                side_a.trim().to_string(),
                side_b.trim().to_string(),
                c.meta.topic,
            );
            loaded.id = c.meta.id;
            loaded.priority = c.meta.priority;
            loaded.closed = c.meta.status == store::meta::Status::Closed;
            loaded.locked_by = c
                .meta
                .locked_by
                .as_deref()
                .map(|id| store::writers::display_name(&writers, id));
            loaded
        })
        .collect();
    (cassettes, damaged)
}

/// Print the `themes` listing: every selectable theme with a color swatch
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

/// Show the sessions picker and return the chosen session id, or `None`
/// when the human quit without opening one.
///
/// Owns terminal setup and teardown for its own screen. The panic hook is
/// installed here too, not only in the TUI's `run`: a picker that panics
/// with raw mode enabled leaves the user's shell unusable just as surely.
fn run_picker(mut picker: picker::Picker, theme: &theme::Theme) -> io::Result<Option<String>> {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;

    // Everything past `enable_raw_mode` runs inside this closure so that a
    // `?` on any of it — entering the alternate screen, building the
    // terminal, a draw, a read — still reaches `restore_terminal` below.
    // Propagating straight out would print the error onto a shell left in
    // raw mode and the alternate screen. The TUI's `run` already captures
    // its result this way; this path was the hole.
    let result = (|| -> io::Result<Option<String>> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let chosen = loop {
            // Clamp BEFORE drawing, against the height the frame will actually
            // have: the cursor must never point at a row the draw then omits.
            let capacity = picker::Picker::rows_capacity(terminal.size()?.height);
            picker.ensure_cursor_visible(capacity);
            terminal.draw(|f| ui::render_picker(f, &picker, theme))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != event::KeyEventKind::Press {
                continue;
            }
            // While the filter prompt is open it owns the keyboard, so `q` types
            // a `q` rather than quitting. Same rule `Mode::Topic` follows in the
            // main TUI, and the leak it prevents is why that rule exists.
            // Ctrl+C quits from anywhere, including inside the prompt. Raw mode
            // swallows SIGINT, so without this the modal branch below would
            // append a `c` to the query — and Ctrl+C is precisely the key a
            // human reaches for when a modal prompt surprises them.
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break None;
            }
            if picker.filtering {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => picker.end_filter(),
                    KeyCode::Backspace => picker.pop_filter(),
                    // Only plain characters are text. A Ctrl+<letter> that fell
                    // through here would type its letter.
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        picker.push_filter(c)
                    }
                    _ => {}
                }
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break None,
                KeyCode::Char('j') | KeyCode::Down => picker.move_down(),
                KeyCode::Char('k') | KeyCode::Up => picker.move_up(),
                KeyCode::Char('a') => picker.toggle_all(),
                KeyCode::Char('/') => picker.start_filter(),
                // Enter on an empty list is a no-op, not a crash: an empty store
                // and a filter that matches nothing are ordinary states here.
                KeyCode::Enter => {
                    if let Some(e) = picker.selected() {
                        break Some(e.id.clone());
                    }
                }
                _ => {}
            }
        };

        Ok(chosen)
    })();

    restore_terminal();
    let _ = panic::take_hook();
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mut writer: Option<&mut session_writer::SessionWriter>,
    store: Option<&store::Store>,
    theme: &theme::Theme,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();
    let mut last_autosave = Instant::now();
    // Live sync (5b): `ChangeStamp` last seen per cassette id, so an
    // unchanged file is never re-merged — that would reset its cursor and
    // undo stack for no reason. The first sight of a file counts as changed.
    let mut cassette_stamps: HashMap<String, ChangeStamp> = HashMap::new();
    #[cfg(unix)]
    let signals = install_signal_handlers()?;

    loop {
        terminal.draw(|f| ui::render(f, app, theme))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    handle_key(app, key);
                    app.check_goal();
                    follow_focus(app, writer.as_deref_mut());
                }
                Event::Paste(text) => {
                    handle_paste(app, &text);
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

            // Per-tick lock retry runs BEFORE sync: a retry that wins the
            // lock already re-reads that cassette through `acquire`'s
            // `refresh_from_disk`, so sync below then finds it held and
            // skips it — one read this tick, not two. Sync-first would merge
            // the cassette and then immediately re-read it here.
            retry_lock(app, writer.as_deref_mut());

            if let Some(store) = store {
                sync_external_writes(app, store, writer.as_deref(), &mut cassette_stamps);
            }

            last_tick = Instant::now();
        }

        #[cfg(unix)]
        {
            use std::sync::atomic::Ordering;
            // SIGTERM/SIGHUP: quit through the normal path — main() saves the
            // session and restores the terminal on the way out.
            if signals.terminate.load(Ordering::Relaxed) {
                app.should_quit = true;
            }
            if signals.suspend.swap(false, Ordering::Relaxed) || std::mem::take(&mut app.suspend) {
                suspend_session(terminal, app, writer.as_deref_mut())?;
            }
        }
        #[cfg(not(unix))]
        {
            app.suspend = false;
        }

        if std::mem::take(&mut app.bell) {
            let mut out = io::stdout();
            let _ = out.write_all(b"\x07");
            let _ = out.flush();
        }

        // Crash safety: flush the focused cassette every AUTOSAVE_SECS,
        // through the guard this process already holds. It must never call
        // `Store::lock` — `flock` is per-open-file-description, so a second
        // acquire of a lock we hold reports Busy and blames a phantom
        // writer. `flush_focused` is a no-op when nothing is dirty, and only
        // the focused cassette can be: `follow_focus` flushes the one being
        // left before the guard moves.
        if let Some(w) = writer.as_deref_mut() {
            if last_autosave.elapsed() >= Duration::from_secs(AUTOSAVE_SECS) {
                if let Err(e) = w.flush_focused(app) {
                    app.status_msg = Some(format!("autosave failed: {e}"));
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

/// Ctrl+Z / SIGTSTP: flush the session to disk, hand the terminal back to the
/// shell, and stop. When the shell resumes us (`fg` → SIGCONT), re-enter raw
/// mode and the alternate screen and force a full redraw.
#[cfg(unix)]
fn suspend_session(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    writer: Option<&mut session_writer::SessionWriter>,
) -> io::Result<()> {
    if let Some(w) = writer {
        // Through the held guard, which stays held across the stop: the
        // shell may never bring us back.
        if let Err(e) = w.flush_focused(app) {
            app.status_msg = Some(format!("could not flush before suspending: {e}"));
        }
    }
    restore_terminal();
    // SIGSTOP cannot be caught: execution halts here until SIGCONT.
    let _ = signal_hook::low_level::raise(signal_hook::consts::SIGSTOP);
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableBracketedPaste
    )?;
    terminal.clear()?;
    Ok(())
}

/// Restore the two store invariants a keypress can break, in order.
///
/// 1. **Every cassette on screen is a store cassette.** `Ctrl+N` pushes one
///    with no id: the key handlers are pure and cannot reach a `Store`, so
///    the store counterpart is minted here instead of threading one through
///    them.
/// 2. **Focus means held.** When focus has moved, `acquire` flushes the
///    cassette being left *through the guard that is held* and only then
///    drops it — dropping first would lose the text or force a re-acquire
///    `flock` may refuse.
///
/// A cassette another writer holds opens read-only rather than swallowing
/// keystrokes there is no guard to write through.
fn follow_focus(app: &mut App, writer: Option<&mut session_writer::SessionWriter>) {
    let Some(w) = writer else { return };
    if let Err(e) = w.create_missing_cassettes(app) {
        app.status_msg = Some(format!("cannot create cassette: {e}"));
    }
    // A closed cassette is not contended, it is simply not writable, so do
    // not take its lock at all: acquiring would succeed and then claim the
    // cassette is editable. Checked before the held-lock fast path, because
    // an agent may close the very cassette we are holding.
    if app.cassettes.get(app.focus_idx).is_some_and(|c| c.closed) {
        app.read_only = app::ReadOnly::Closed;
        return;
    }
    if app
        .cassettes
        .get(app.focus_idx)
        .is_some_and(|c| Some(c.id.as_str()) == w.held_id())
    {
        // Already ours, but the reason we were read-only may have been a
        // `Closed` that a `queue reopen` has since cleared.
        if matches!(app.read_only, app::ReadOnly::Closed) {
            app.read_only = app::ReadOnly::No;
        }
        return;
    }
    try_acquire(app, w, app.focus_idx);
}

/// Retry the focused cassette's lock once a tick, independent of any
/// keypress — so a human who steps away from a cassette an agent holds
/// finds it editable on their return without typing a character to discover
/// it. A no-op unless the session is currently read-only: an already-held
/// cassette has nothing to retry, and `follow_focus` already retries on
/// every keypress, so this only has work to do between them.
///
/// Runs BEFORE `sync_external_writes` in the tick block: a retry that wins
/// the lock already re-reads this cassette via `acquire`'s
/// `refresh_from_disk`, so sync then finds it newly held and skips it —
/// one read this tick, not two. Sync-first would merge the cassette and
/// then immediately re-read it here.
fn retry_lock(app: &mut App, writer: Option<&mut session_writer::SessionWriter>) {
    let Some(w) = writer else { return };
    // Only CONTENTION is worth retrying. A closed cassette's lock is free,
    // so acquiring it succeeds and clears the read-only state — leaving the
    // human looking at `-- INSERT --` on a cassette every keystroke is
    // dropped from. Closed is cleared by `follow_focus` seeing a reopen, not
    // by winning a lock that was never the obstacle.
    if !matches!(app.read_only, app::ReadOnly::Busy { .. }) {
        return;
    }
    try_acquire(app, w, app.focus_idx);
}

/// Attempt to acquire cassette `idx`'s lock and record the outcome on `app`:
/// `read_only` and `busy_holder` together, so the two can never disagree
/// about whether — and who — is blocking. Shared by `follow_focus` (retries
/// on every keypress) and `retry_lock` (retries on every tick), so there is
/// exactly one place that decides what a failed acquire looks like on screen.
///
/// Those two fields are the *whole* record: this deliberately does not also
/// write `status_msg`. Being busy is a standing condition, not news, and
/// `ui::info_text` already renders it from `read_only`/`busy_holder` as
/// `-- READ ONLY (open by <name>) --` with the rest of the info line — ln/col,
/// char count, cassette position, side — intact. Copying the same fact into
/// `status_msg` would put it on screen through the one branch that outranks
/// the mode line, costing the user that whole line for as long as the lock is
/// held, and it could not use `App::flash` (a standing condition must not
/// auto-expire), so it would need clearing on every path back to editable —
/// which is exactly the stale-message bug this shape cannot have. Leaving
/// `status_msg` alone also means a genuine flash — goal reached, timer
/// expired — still shows for its few seconds while a cassette is busy, and
/// the banner returns underneath it when it expires.
fn try_acquire(app: &mut App, w: &mut session_writer::SessionWriter, idx: usize) {
    // The id, not the index: `acquire`'s `refresh_from_disk` may have
    // adopted a new `priority`/`closed` from disk, and the re-sort below
    // moves cassettes.
    let id = app.cassettes.get(idx).map(|c| c.id.clone());
    match w.acquire(app, idx) {
        Ok(()) => {
            // `refresh_from_disk` is the second place queue metadata enters
            // memory, and unlike `merge_external` it does not sort. Without
            // this, a cassette an agent closed keeps its old slot, the open
            // set stops being a prefix, and everything reading `stack_len()`
            // is reading a lie.
            app.sort_queue();
            let closed = id
                .as_deref()
                .and_then(|id| app.cassettes.iter().find(|c| c.id == id))
                .is_some_and(|c| c.closed);
            // Winning the lock does not make a closed cassette writable: its
            // lock was never the obstacle, so `Ok` alone says nothing here.
            app.read_only = if closed {
                app::ReadOnly::Closed
            } else {
                app::ReadOnly::No
            };
        }
        Err(e) => {
            app.read_only = app::ReadOnly::Busy {
                holder: busy_holder_name(&e),
            };
            // Ordinary contention is the case the banner was built for, and
            // it says everything there is to say. Anything else — an `Io`
            // from `acquire`'s own flush of the OUTGOING cassette (a full
            // disk, a vanished session directory), a `NoSuchCassette` — has
            // no banner of its own and would otherwise show as a bare
            // `-- READ ONLY --` beside a help row blaming a writer who does
            // not exist, while the user's unflushed words sit in memory.
            // `flash` and not `status_msg` directly: this is news, it
            // expires on its own, and each tick's `retry_lock` re-reports it
            // for as long as the condition lasts.
            if !matches!(e, store::lock::LockError::Busy { .. }) {
                app.flash(format!("cannot take this cassette's lock: {e}"));
            }
        }
    }
}

/// The name to show for a busy cassette's holder, straight from the lock
/// anchor's own attribution — the same source `queue write`'s exit-3 message
/// reads (`LockError::Busy`'s `holder`). `None` when there is genuinely
/// nothing to show: no writer at all, or a holder whose anchor line could
/// not be parsed (a crash before it wrote one, or garbled bytes) — `queue
/// write` degrades the same case to "another writer" rather than a name.
fn busy_holder_name(e: &store::lock::LockError) -> Option<String> {
    match e {
        store::lock::LockError::Busy {
            holder: Some(a), ..
        } => Some(a.name.clone()),
        _ => None,
    }
}

/// What the stat-only pass compares to decide a cassette changed. mtime
/// alone misses a second write inside one timestamp tick on a coarse
/// filesystem; every store write goes through `atomic_write`'s rename, so
/// the inode moves on every write and cannot collide. `len` is the
/// fallback signal where there is no inode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChangeStamp {
    modified: SystemTime,
    len: u64,
    #[cfg(unix)]
    ino: u64,
}

fn change_stamp(m: &std::fs::Metadata) -> Option<ChangeStamp> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;
    Some(ChangeStamp {
        modified: m.modified().ok()?,
        len: m.len(),
        #[cfg(unix)]
        ino: m.ino(),
    })
}

/// Notice what other writers have done to this session, once per tick.
///
/// Two passes, cheap-first: this is a freewriting app, and re-reading and
/// re-parsing every cassette's prose every second — the overwhelmingly
/// common case, where nothing changed — would cost real responsiveness at
/// the 36-cassette cap for no benefit.
///
/// 1. **Stat only.** `fs::read_dir` the session's `cassettes/` directory and
///    check each entry's `ChangeStamp` (mtime, length and, on unix, inode)
///    against `stamps`, with no file content read at all. The **held
///    cassette's stamp is still recorded** — this process holds that lock
///    precisely so nobody else can have changed the file, so its stamp only
///    ever moves because of our own flush, and
///    *not* recording it would make the tick after focus moves off it look
///    like a first-sight change — but it is excluded from `changed_ids`
///    unconditionally, so it is never a merge candidate: merging it back in
///    could only replace the human's unsaved keystrokes with whatever was
///    last flushed.
/// 2. **Only if something moved**, call `Store::scan_session` once for the
///    whole session — the one place statuses and priorities are read while
///    running, so it is also what refreshes `App.damaged`. A newcomer needs
///    no insertion index: it carries its own priority and `merge_external`
///    re-sorts. Only the ids the first pass actually flagged are merged;
///    an untouched cassette is left alone, keeping its cursor and undo
///    stack exactly as they were.
///
/// Degrades silently throughout: a vanished or unreadable cassettes
/// directory, a single unreadable directory entry, an unreadable store scan,
/// or one unparseable cassette all just mean nothing is merged this tick.
/// The user is still typing; a hard error over a neighbour's file would be
/// worse than showing something stale. A cassette this process has never
/// seen is also dropped once `MAX_CASSETTES` is already on screen, the same
/// cap `App::add_cassette` enforces.
fn sync_external_writes(
    app: &mut App,
    store: &store::Store,
    writer: Option<&session_writer::SessionWriter>,
    stamps: &mut HashMap<String, ChangeStamp>,
) {
    if app.session.is_empty() {
        return;
    }
    let held_id = writer.and_then(session_writer::SessionWriter::held_id);

    // Pass 1: stat only, no file content touched.
    let Ok(entries) = std::fs::read_dir(store.cassettes_dir(&app.session)) else {
        return;
    };
    let mut changed_ids: Vec<String> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = store::ids::id_from_file_name(file_name) else {
            continue;
        };
        let Some(stamp) = entry.metadata().ok().as_ref().and_then(change_stamp) else {
            continue;
        };
        if Some(id) == held_id {
            // Nobody else can write a cassette whose lock this process
            // holds, so its stamp only ever moves because of our own
            // flush/autosave. Record it anyway: skipping the record (not
            // just the merge) would make the moment focus moves off it look
            // like a first-sight change the instant the lock is released,
            // and `merge_external` would then rebuild it from identical
            // content for no reason. The merge itself is still categorically
            // skipped — this cassette is never a candidate.
            stamps.insert(id.to_string(), stamp);
            continue;
        }
        let changed = stamps.get(id).is_none_or(|prev| *prev != stamp);
        stamps.insert(id.to_string(), stamp);
        if changed {
            changed_ids.push(id.to_string());
        }
    }
    if changed_ids.is_empty() {
        return;
    }

    // Pass 2: something moved, so the whole session's queue order is worth
    // pulling — it is the only place statuses and priorities are read, and
    // cheap at this scale regardless.
    let Ok(scan) = store.scan_session(&app.session) else {
        return;
    };
    // A file may have become damaged (or been repaired) since load, and this
    // is the only place the session is rescanned while running.
    app.damaged = scan
        .damaged
        .iter()
        .map(|d| (d.label(), d.reason.to_string()))
        .collect();
    // Resolved once per tick, not once per changed cassette; a registry read
    // failure degrades to raw writer ids rather than skipping the merge.
    let writers = store.writers().unwrap_or_default();
    for stored in &scan.cassettes {
        if !changed_ids.contains(&stored.meta.id) {
            continue;
        }
        // Cannot happen given pass 1's own check, but the invariant is worth
        // restating rather than trusting the caller two steps back.
        if Some(stored.meta.id.as_str()) == held_id {
            continue;
        }

        let already_known = app.cassettes.iter().any(|c| c.id == stored.meta.id);
        // The cap is on the working set: a newcomer that is already closed
        // is history, not work, so it never counts against it.
        let newcomer_is_open = stored.meta.status != store::meta::Status::Closed;
        if !already_known && newcomer_is_open && app.open_count() >= app::MAX_CASSETTES {
            continue;
        }

        let (side_a, side_b) = queue::json::split_sides(&stored.body);
        let mut incoming = cassette::Cassette::from_sides(
            side_a.trim().to_string(),
            side_b.trim().to_string(),
            stored.meta.topic.clone(),
        );
        incoming.locked_by = stored
            .meta
            .locked_by
            .as_deref()
            .map(|id| store::writers::display_name(&writers, id));
        incoming.priority = stored.meta.priority;
        incoming.closed = stored.meta.status == store::meta::Status::Closed;
        app.merge_external(&stored.meta.id, incoming);
    }
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
        // Raw mode swallows the tty's Ctrl+Z, so suspend is an explicit binding.
        (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
            app.suspend = true;
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

/// Insert pasted text as one edit: line endings normalized to '\n' and a
/// single undo snapshot for the whole chunk. In the topic prompt the paste
/// joins onto one line. Pasting is forward-only insertion, so record mode
/// allows it.
fn handle_paste(app: &mut App, text: &str) {
    app.idle_secs = 0;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if app.mode == Mode::Topic {
        app.topic_input.push_str(&text.replace('\n', " "));
        return;
    }
    app.modify_focused(|c| {
        c.snapshot();
        c.insert_str(&text);
    });
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
                'z' => app.toggle_closed_fold(),
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

/// Today's note filename from the (config-overridable) chrono format string.
/// A malformed format is a config error, not a panic mid-render.
fn daily_note_name(fmt: &str) -> Result<String, String> {
    use chrono::format::{Item, StrftimeItems};
    if StrftimeItems::new(fmt).any(|i| matches!(i, Item::Error)) {
        return Err(format!(
            "invalid daily_format '{fmt}' in config.toml — expected a chrono date format like %Y-%m-%d"
        ));
    }
    Ok(chrono::Local::now().format(fmt).to_string())
}

fn die(msg: &str) -> ! {
    eprintln!("cassette: {msg}");
    eprintln!("try 'cassette --help'");
    std::process::exit(2);
}

/// `cassette writer register|list|whoami`. Rendering lives in `writer.rs` as
/// pure functions over `&Store`; this is the one place that turns their
/// results into exit codes.
///
/// `register` is the only command that can fail with `EnsureError`: `list`
/// and `whoami` only ever see an I/O error reading the registry (exit 1). A
/// `KindMismatch` or `EmptyName` from `register` is a usage error (exit 2) —
/// the caller asked for something the registry cannot honor, not a system
/// failure.
///
/// `json` routes every failure through `exit_with` instead of `die_with`, so
/// `--json` on `writer` gets the same `{"error","code"}` envelope `queue`
/// commands do (spec decision 2 names `session`/`writer` explicitly) — exit
/// codes are unchanged, only the channel and format for a failure.
fn run_writer_cmd(cmd: &cli::WriterCmd, writer_flag: Option<&str>, json: bool) -> ! {
    let store = store::Store::new(store_root(json));
    match cmd {
        cli::WriterCmd::Register { name, kind } => match writer::register(&store, name, *kind) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0)
            }
            Err(e @ store::writers::EnsureError::KindMismatch { .. }) => {
                exit_with(2, &e.to_string(), json)
            }
            Err(e @ store::writers::EnsureError::EmptyName) => exit_with(2, &e.to_string(), json),
            Err(e @ store::writers::EnsureError::Io(_)) => exit_with(1, &e.to_string(), json),
        },
        cli::WriterCmd::List => match writer::list(&store) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0)
            }
            Err(e) => exit_with(1, &e, json),
        },
        cli::WriterCmd::Whoami => {
            // `whoami` only ever looks a name up (`writer::render_whoami`
            // never resolves or creates), but where the name came from
            // decides its authority: a human reached through `$USER` alone
            // acts as an agent on the queue commands, and this says so.
            let (who_name, source) =
                resolve_writer_name(writer_flag).unwrap_or_else(|e| exit_with(2, &e, json));
            match writer::whoami(&store, &who_name, source) {
                Ok(msg) => {
                    println!("{msg}");
                    std::process::exit(0)
                }
                Err(e) => exit_with(1, &e, json),
            }
        }
    }
}

/// `cassette session new|list|alias`. Rendering lives in `session.rs` as pure
/// functions over `&Store`; this is the one place that turns their results
/// into exit codes.
///
/// `new` and `list` fail only on I/O (exit 1) — `session.rs`'s `Result<_,
/// String>` already collapses that to one case. `alias` can also fail on an
/// unknown session id, which is a usage error (exit 2): `session::set_alias`
/// returns `SessionError` so this match can tell the two apart.
///
/// `json` routes every failure through `exit_with` instead of `die_with`, so
/// `--json` on `session` gets the same `{"error","code"}` envelope `queue`
/// commands do (spec decision 2 names `session`/`writer` explicitly) — exit
/// codes are unchanged, only the channel and format for a failure.
fn run_session_cmd(cmd: &cli::SessionCmd, json: bool) -> ! {
    let store = store::Store::new(store_root(json));
    match cmd {
        cli::SessionCmd::New { alias } => match session::new_session(&store, alias.as_deref()) {
            Ok(id) => {
                println!("{id}");
                std::process::exit(0)
            }
            Err(e) => exit_with(1, &e, json),
        },
        cli::SessionCmd::List { all } => match session::list(&store, *all) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0)
            }
            Err(e) => exit_with(1, &e, json),
        },
        cli::SessionCmd::Alias { id, alias } => match session::set_alias(&store, id, alias) {
            Ok(msg) => {
                println!("{msg}");
                std::process::exit(0)
            }
            Err(e @ session::SessionError::Usage(_)) => exit_with(2, &e.to_string(), json),
            Err(e @ session::SessionError::Io(_)) => exit_with(1, &e.to_string(), json),
        },
    }
}

/// The writer to act as, and where that name came from: `--writer`, else
/// `$CASSETTE_WRITER`, else `$USER`. There is deliberately no further
/// fallback — a shared `"unknown"` identity would silently attribute every
/// agent's work to the same writer, in a system whose entire purpose is
/// knowing who wrote what.
///
/// The source travels with the name rather than being flattened away: a
/// command that resolves a writer to act as (`queue write`, `queue new`, and
/// the rest of 4b's mutating commands) must treat an unregistered `--writer`
/// or `$CASSETTE_WRITER` as a usage error — naming a writer explicitly is a
/// claim about identity, so a typo must fail loudly rather than silently
/// spawn a second identity as `human`, the privileged kind — while still
/// bootstrapping an unregistered `$USER` as a new human writer. See
/// `queue::WriterSource`.
fn resolve_writer_name(cli: Option<&str>) -> Result<(String, queue::WriterSource), String> {
    if let Some(name) = cli {
        let name = name.trim();
        if name.is_empty() {
            return Err("--writer cannot be empty".to_string());
        }
        return Ok((name.to_string(), queue::WriterSource::Flag));
    }
    if let Ok(env_writer) = std::env::var("CASSETTE_WRITER") {
        let env_writer = env_writer.trim();
        if !env_writer.is_empty() {
            return Ok((env_writer.to_string(), queue::WriterSource::Flag));
        }
    }
    match std::env::var("USER") {
        Ok(user) if !user.trim().is_empty() => {
            Ok((user.trim().to_string(), queue::WriterSource::Env))
        }
        _ => Err(
            "no writer: $USER is empty or unset, so pass --writer <NAME> or set $CASSETTE_WRITER"
                .to_string(),
        ),
    }
}

/// Exit with an arbitrary code — unlike `die`, which is only ever a CLI usage
/// error (always 2, always suggesting `--help`). `queue::write`'s failures are
/// domain errors (a missing session, a busy lock, a real I/O failure) where
/// "try --help" would not tell the caller anything useful, and they span
/// three different exit codes `die` cannot express.
fn die_with(code: i32, msg: &str) -> ! {
    eprintln!("cassette: {msg}");
    std::process::exit(code);
}

/// Exit successfully for any queue command, printing output first if there
/// is any. Unifies `new`'s `Ok(id) => { println!(...); exit(0) }`, `write`'s
/// (and `close`'s, `reopen`'s, `move`'s) `Ok(()) => exit(0)`, and
/// `list`/`show`/`next`'s `Ok(msg) => { println!(...); exit(0) }` into one
/// path, matching `exit_queue_err`'s single path for the failure side.
fn exit_queue_ok(output: Option<String>) -> ! {
    if let Some(s) = output {
        println!("{s}");
    }
    std::process::exit(0)
}

/// Exit with a message and code, in whichever form the caller asked for.
/// The one place the `{"error","code"}` envelope's shape is built — both
/// `exit_queue_err` and `exit_usage` render through here, so there cannot be
/// a second, subtly different `serde_json::json!` call to drift out of sync.
///
/// The JSON envelope goes to **stdout**, not stderr: an agent that redirects
/// stderr to a log must still receive a parseable failure on the channel it
/// is reading. Prose keeps going to stderr, where it always has.
fn exit_with(code: i32, msg: &str, json: bool) -> ! {
    if json {
        let envelope = serde_json::json!({ "error": msg, "code": code });
        println!("{envelope}");
        std::process::exit(code);
    }
    die_with(code, msg)
}

/// Exit on a failed queue command, in whichever form the caller asked for.
fn exit_queue_err(e: &queue::QueueError, json: bool) -> ! {
    exit_with(queue::exit_code(e), queue::message(e), json)
}

/// Exit 2 for a bad invocation that never reaches a `QueueError` — today
/// only `resolve_writer_name`'s two failures (no `--writer`, no usable
/// `$USER`/`$CASSETTE_WRITER`), at its five call sites. The spec's `--json`
/// contract is unconditional — any command that fails emits the envelope —
/// so these route through `exit_with` exactly like `exit_queue_err` does,
/// rather than `die_with` straight to stderr prose regardless of `--json`.
fn exit_usage(msg: &str, json: bool) -> ! {
    exit_with(2, msg, json)
}

/// Whether `root` sits under a directory a consumer sync client owns.
///
/// A substring match on lowercased path components, not a filesystem probe —
/// "where cheap" is the parent spec's own qualifier. It can false-positive on
/// a directory that merely has one of these words in its name, which is
/// exactly why the caller warns and continues rather than refusing: a tool
/// that will not start because of a folder name would be worse than the risk
/// it names.
///
/// Network filesystems stay undetected. There is no cheap userspace way to
/// tell whether a given mount's `flock` is cross-client safe, and guessing
/// from the mount type would give false confidence rather than protection.
fn looks_synced(root: &Path) -> bool {
    const SYNC_ROOTS: [&str; 7] = [
        "dropbox",
        "mobile documents", // iCloud Drive's on-disk name
        "icloud drive",
        "onedrive",
        "google drive",
        "sync.com",
        // Since macOS 12.3, OneDrive and Google Drive live under
        // `~/Library/CloudStorage/<Provider>-<Account>` — a hyphen, which
        // neither clause below catches. The parent component is the reliable
        // signal there, and it belongs to no other kind of directory.
        "cloudstorage",
    ];
    root.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        // Equality, or a `<name> ` prefix so `Dropbox (Personal)` and
        // `OneDrive - Acme Corp` are caught. `Dropbox Backup` is caught too;
        // that false positive is accepted, because this only ever warns.
        SYNC_ROOTS
            .iter()
            .any(|s| name == *s || name.starts_with(&format!("{s} ")))
    })
}

/// Warn once at startup when the store sits in a syncing folder.
///
/// Locks are local kernel state and do not sync, so two machines editing one
/// session get zero mutual exclusion — the exact guarantee the store exists
/// to provide. Sync clients also interfere with rename-based atomic writes.
fn warn_if_synced(root: &Path) {
    if looks_synced(root) {
        eprintln!(
            "cassette: warning — the store is under a syncing folder ({}).\n\
             cassette: locks do not sync, so two machines editing one session \
             get no mutual exclusion.",
            root.display()
        );
    }
}

/// The store root: `$CASSETTE_DATA_DIR` when set, else the XDG default.
///
/// The environment override exists so tests never touch the real store at
/// `~/.local/share/cassette`. There is no `data_dir` config key — earlier
/// comments here promised one for Phase 6, which closed the redesign without
/// adding it.
///
/// Failing to determine a data dir at all is an I/O failure the caller cannot
/// fix by retrying with different arguments (README's exit-1 rule), not a
/// usage error — and, like every other failure under `--json`, it must still
/// emit the `{"error","code"}` envelope rather than bare stderr prose.
fn store_root(json: bool) -> PathBuf {
    let root = std::env::var_os("CASSETTE_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(store::Store::default_root)
        .unwrap_or_else(|| exit_with(1, "cannot determine a data dir", json));
    warn_if_synced(&root);
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cassette::Side;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[cfg(unix)]
    #[test]
    fn a_same_length_rewrite_inside_one_mtime_tick_still_changes_the_stamp() {
        // Coarse-mtime filesystems give two quick writes the same mtime. The
        // store writes through `atomic_write` (temp file + rename), so the
        // inode moves on every write even when the mtime cannot.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("c.md");
        store::atomic_write(&path, "aaaa").expect("first");
        let first_meta = std::fs::metadata(&path).expect("stat");
        let first = change_stamp(&first_meta).expect("stamp");

        store::atomic_write(&path, "bbbb").expect("second, same length");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(first_meta.modified().expect("mtime"))
            .expect("pin mtime back");
        let second = change_stamp(&std::fs::metadata(&path).expect("stat")).expect("stamp");

        assert_ne!(first, second, "same mtime and length, new inode: a change");
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            handle_key(app, key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    /// A store session, optionally aliased, for the `resolve_session` tests.
    fn seeded_session(store: &store::Store, alias: Option<&str>) -> String {
        store
            .create_session(&store::session::SessionMeta {
                alias: alias.map(|a| a.to_string()),
                created: store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session")
    }

    /// Add a real store cassette (frontmatter + body) and return its id, for
    /// tests that need `sync_external_writes` to see actual files on disk.
    fn store_cassette(store: &store::Store, session: &str, priority: i64, body: &str) -> String {
        let id = store::ids::new_id();
        let m = store::meta::CassetteMeta {
            id: id.clone(),
            topic: None,
            priority,
            status: store::meta::Status::Open,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: store::meta::now_utc(),
        };
        store.add_cassette(session, &m, body).expect("add cassette");
        id
    }

    /// Bump a cassette file's mtime well clear of whatever the filesystem
    /// gave it, so the stat pass must see the change. Without this the test
    /// would be asserting on mtime granularity rather than on sync.
    fn touch_forward(store: &store::Store, session: &str, id: &str) {
        let path = store
            .cassette_path(session, id)
            .expect("cassette path")
            .expect("cassette exists");
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open for touch");
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .expect("set mtime");
    }

    /// The spec's headline behaviour for the fold: focusing a closed
    /// cassette yields `ReadOnly::Closed`, so `modify_focused` drops the
    /// edit through the gate that already exists.
    #[test]
    fn focusing_a_closed_cassette_opens_it_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "## Side A\n\nshut\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id;
        c.priority = 10;
        c.closed = true;
        app.cassettes.push(c);
        app.closed_expanded = true;
        let mut w = session_writer::SessionWriter::open(&store, &session, false, "w", "w");

        follow_focus(&mut app, Some(&mut w));

        assert_eq!(app.read_only, app::ReadOnly::Closed);
        app.modify_focused(|c| c.insert('x'));
        assert!(
            !app.cassettes[0].side_a_text().contains('x'),
            "a closed cassette must not take the keystroke"
        );
    }

    /// And the way back out: a `queue reopen` while the cassette is focused
    /// must clear the banner. Without this the TUI would read `-- CLOSED --`
    /// for the rest of the session on a cassette that is writable again.
    #[test]
    fn a_reopen_clears_the_closed_banner_without_refocusing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "## Side A\n\nshut\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id.clone();
        c.priority = 10;
        app.cassettes.push(c);
        let mut w = session_writer::SessionWriter::open(&store, &session, false, "w", "w");

        // Take the lock first, so the held-lock fast path is the one under
        // test rather than a fresh acquire.
        follow_focus(&mut app, Some(&mut w));
        assert_eq!(app.read_only, app::ReadOnly::No);

        // An agent closes it, then reopens it, while we hold the lock.
        app.cassettes[0].closed = true;
        follow_focus(&mut app, Some(&mut w));
        assert_eq!(app.read_only, app::ReadOnly::Closed, "closed while focused");

        app.cassettes[0].closed = false;
        follow_focus(&mut app, Some(&mut w));
        assert_eq!(
            app.read_only,
            app::ReadOnly::No,
            "a reopen must not leave the banner stuck"
        );
    }

    /// `refresh_from_disk` is a SECOND place `priority`/`closed` enter
    /// memory, and it used to leave the queue unsorted and the banner
    /// saying editable. An agent closes a cassette while we hold nothing;
    /// the next acquire adopts `closed: true` and then reported `No`.
    ///
    /// Three things went wrong at once: the human typed into a closed
    /// cassette and the flush wrote it out; the still-open cassette sorted
    /// behind it became invisible (`render` lays out `0..stack_len()`) and
    /// unreachable (`focus_next` wraps within the stack); and `stack_len()`
    /// stopped describing a prefix, which every other consumer assumes.
    #[test]
    fn acquiring_a_cassette_an_agent_closed_does_not_report_it_editable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let a = store_cassette(&store, &session, 10, "## Side A\n\na\n");
        let b = store_cassette(&store, &session, 20, "## Side A\n\nb\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        for (id, priority) in [(&a, 10), (&b, 20)] {
            let mut c = cassette::Cassette::new();
            c.id = id.clone();
            c.priority = priority;
            app.cassettes.push(c);
        }
        app.focus_idx = 0;

        // An agent closes `a` while this process holds no lock on it.
        let path = store.cassette_path(&session, &a).expect("p").expect("e");
        let raw = std::fs::read_to_string(&path).expect("read");
        std::fs::write(&path, raw.replace("status: open", "status: closed")).expect("write");

        let mut w = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        try_acquire(&mut app, &mut w, 0);

        assert_eq!(
            app.read_only,
            app::ReadOnly::Closed,
            "winning the lock does not make a closed cassette writable"
        );
        assert_eq!(app.open_count(), 1, "`a` left the working set");
        assert!(
            app.cassettes
                .iter()
                .take(app.open_count())
                .all(|c| !c.closed),
            "the open set must still be a prefix: {:?}",
            app.cassettes.iter().map(|c| c.closed).collect::<Vec<_>>()
        );
        assert!(
            app.focus_idx < app.stack_len(),
            "focus must stay inside the drawn stack"
        );
        assert!(
            app.cassettes.iter().any(|c| c.id == b && !c.closed),
            "the still-open cassette must not be stranded"
        );
    }

    /// A closed cassette's lock is FREE, so a tick that retries it succeeds
    /// and reports the cassette editable — while `modify_focused` goes on
    /// dropping every keystroke. Only contention is worth retrying.
    #[test]
    fn retry_lock_does_not_clear_a_closed_cassette() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id;
        c.closed = true;
        app.cassettes.push(c);
        let mut writer = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        app.read_only = app::ReadOnly::Closed;

        retry_lock(&mut app, Some(&mut writer));

        assert_eq!(
            app.read_only,
            app::ReadOnly::Closed,
            "winning a lock that was never the obstacle must not say 'editable'"
        );
    }

    /// A newcomer lands at its queue position because it carries its own
    /// priority, not because the caller computed an index. This is what
    /// retires `merge_external`'s `insert_at` parameter.
    #[test]
    fn sync_places_a_newcomer_by_its_own_priority() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let mine = store_cassette(&store, &session, 20, "## Side A\n\nmine\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = mine.clone();
        c.priority = 20;
        app.cassettes.push(c);

        let mut stamps = HashMap::new();
        sync_external_writes(&mut app, &store, None, &mut stamps);

        // 30 sorts after mine, 10 before it — so a single append would put
        // them both at the tail and only ordering can get this right.
        let later = store_cassette(&store, &session, 30, "## Side A\n\nlater\n");
        let earlier = store_cassette(&store, &session, 10, "## Side A\n\nearlier\n");
        sync_external_writes(&mut app, &store, None, &mut stamps);

        assert_eq!(
            app.cassettes
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec![earlier.as_str(), mine.as_str(), later.as_str()],
            "queue order comes from the cassettes' own priorities"
        );
        assert_eq!(app.cassettes[app.focus_idx].id, mine, "focus unmoved");
    }

    /// A cassette an agent CLOSES while the TUI is running folds away on the
    /// next tick rather than staying in the working set.
    #[test]
    fn sync_notices_a_cassette_being_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let a = store_cassette(&store, &session, 10, "## Side A\n\na\n");
        let b = store_cassette(&store, &session, 20, "## Side A\n\nb\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        for (id, priority) in [(&a, 10), (&b, 20)] {
            let mut c = cassette::Cassette::new();
            c.id = id.clone();
            c.priority = priority;
            app.cassettes.push(c);
        }

        let mut stamps = HashMap::new();
        sync_external_writes(&mut app, &store, None, &mut stamps);
        assert_eq!(app.open_count(), 2);

        // Close `a` the way `queue close` does, then touch it forward.
        let path = store.cassette_path(&session, &a).expect("p").expect("e");
        let raw = std::fs::read_to_string(&path).expect("read");
        std::fs::write(&path, raw.replace("status: open", "status: closed")).expect("write");
        touch_forward(&store, &session, &a);

        sync_external_writes(&mut app, &store, None, &mut stamps);

        assert_eq!(app.open_count(), 1, "the closed one left the working set");
        assert!(app.cassettes[1].closed, "and sorted to the tail");
        assert_eq!(app.cassettes[1].id, a);
    }

    /// The phase's headline feature, end to end in one process: an agent
    /// rewrites a cassette the TUI is NOT holding, and the next tick puts
    /// those words on screen.
    ///
    /// Every part of this was tested in isolation — `merge_external` in
    /// `app.rs`, the store contract in `tests/cli.rs` — but the seam between
    /// them was not: `read_dir` -> mtime compare -> `scan_session` ->
    /// `split_sides` -> merge. Deleting the `merge_external` call at the end
    /// of `sync_external_writes` left the whole suite green.
    #[test]
    fn sync_lands_an_external_write_on_screen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "## Side A\n\nmine\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id.clone();
        c.insert_str("mine");
        app.cassettes.push(c);

        // First sync only seeds the mtime map: nothing has changed yet.
        let mut stamps = HashMap::new();
        sync_external_writes(&mut app, &store, None, &mut stamps);
        assert_eq!(app.cassettes[0].side_a_text(), "mine", "nothing yet");

        // The agent writes, through the same store API `queue write` uses.
        let path = store
            .cassette_path(&session, &id)
            .expect("path")
            .expect("exists");
        let raw = std::fs::read_to_string(&path).expect("read");
        let (fm, _) = raw.split_at(raw.rfind("---\n").expect("frontmatter end") + 4);
        std::fs::write(&path, format!("{fm}\n## Side A\n\nagent words\n")).expect("write");
        touch_forward(&store, &session, &id);

        sync_external_writes(&mut app, &store, None, &mut stamps);

        assert_eq!(
            app.cassettes[0].side_a_text().trim(),
            "agent words",
            "the agent's words must reach the screen"
        );
    }

    /// A cassette an agent creates with `queue new` appears in the stack at
    /// its queue position, and does NOT steal focus from the human.
    #[test]
    fn sync_shows_a_cassette_another_writer_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let mine = store_cassette(&store, &session, 20, "## Side A\n\nmine\n");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = mine.clone();
        app.cassettes.push(c);
        app.focus_idx = 0;

        let mut stamps = HashMap::new();
        sync_external_writes(&mut app, &store, None, &mut stamps);
        assert_eq!(app.cassettes.len(), 1, "only mine so far");

        // Priority 10 sorts ahead of mine (20), so it lands at index 0 —
        // which is exactly the case that could silently move focus.
        let newcomer = store_cassette(&store, &session, 10, "## Side A\n\nfrom the agent\n");

        sync_external_writes(&mut app, &store, None, &mut stamps);

        assert_eq!(app.cassettes.len(), 2, "the newcomer must appear");
        assert_eq!(app.cassettes[0].id, newcomer, "at its queue position");
        assert_eq!(
            app.cassettes[0].side_a_text().trim(),
            "from the agent",
            "with its text"
        );
        assert_eq!(
            app.cassettes[app.focus_idx].id, mine,
            "focus must still be on MY cassette, not dragged by an insertion"
        );
    }

    #[test]
    fn sync_never_inserts_a_newcomer_once_the_cap_is_reached() {
        // Unbounded growth from a busy multi-writer session would be worse
        // than the existing `MAX_CASSETTES` cap `App::add_cassette` already
        // enforces; sync must respect the same cap rather than introduce an
        // unbounded queue of its own.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        for i in 0..app::MAX_CASSETTES {
            let id = store_cassette(&store, &session, (i as i64 + 1) * 10, "");
            let mut c = cassette::Cassette::new();
            c.id = id;
            app.cassettes.push(c);
        }
        assert_eq!(
            app.cassettes.len(),
            app::MAX_CASSETTES,
            "the list is at the cap"
        );
        let known_ids: Vec<String> = app.cassettes.iter().map(|c| c.id.clone()).collect();

        // A newcomer arrives with the lowest priority, so it would land at
        // index 0 if the cap did not stop it.
        let newcomer_id = store_cassette(&store, &session, 1, "## Side A\n\nnewcomer\n");

        let mut stamps = HashMap::new();
        sync_external_writes(&mut app, &store, None, &mut stamps);

        assert_eq!(
            app.cassettes.len(),
            app::MAX_CASSETTES,
            "the cap is not exceeded"
        );
        assert_eq!(
            app.cassettes
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>(),
            known_ids,
            "nothing already present is disturbed"
        );
        assert!(
            !app.cassettes.iter().any(|c| c.id == newcomer_id),
            "the newcomer must not appear once the cap is already reached"
        );
    }

    #[test]
    fn sync_does_not_wipe_undo_when_focus_releases_a_cassette_it_just_flushed() {
        // The regression: typing in cassette 0, then tabbing to cassette 1,
        // flushes and releases 0's lock — moving 0's file mtime with no
        // external writer involved. A sync tick that follows must not treat
        // that self-inflicted mtime move as a reason to rebuild cassette 0:
        // that would discard its undo stack and reset its cursor even
        // though disk and memory already agree.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        for i in 0..2 {
            let id = store_cassette(&store, &session, (i as i64 + 1) * 10, "");
            let mut c = cassette::Cassette::new();
            c.id = id;
            app.cassettes.push(c);
        }
        let mut writer = session_writer::SessionWriter::open(&store, &session, true, "w", "w");
        writer.acquire(&mut app, 0).expect("acquire 0");
        app.focus_idx = 0;
        app.modify_focused(|c| {
            c.snapshot();
            c.insert_str("hello world");
        });
        let cursor_before = app.cassettes[0].cursor_pos();

        let mut stamps = HashMap::new();
        // A tick while 0 is still held: seeds its mtime in the map (the
        // file itself is still whatever `store_cassette` wrote — empty —
        // since nothing has flushed yet).
        sync_external_writes(&mut app, &store, Some(&writer), &mut stamps);

        // Focus moves to 1: `acquire` flushes and drops 0's guard, writing
        // cassette 0's body to disk and moving its mtime.
        writer.acquire(&mut app, 1).expect("focus 1");
        app.focus_idx = 1;

        // The tick that follows: 0 is unheld, and its mtime has moved
        // relative to what was seeded above.
        sync_external_writes(&mut app, &store, Some(&writer), &mut stamps);

        assert_eq!(
            app.cassettes[0].cursor_pos(),
            cursor_before,
            "the flush-induced mtime move must not reset the cursor"
        );
        app.focus_idx = 0;
        app.modify_focused(|c| c.undo());
        assert_eq!(
            app.cassettes[0].text(),
            "",
            "and the undo stack must survive the sync tick that follows a flush"
        );
    }

    /// A heuristic that warns and continues: it can false-positive on a
    /// directory that merely has one of these words in its name, and a tool
    /// that refuses to start over a folder name would be worse than the risk.
    #[test]
    fn a_store_under_a_sync_root_is_flagged() {
        for p in [
            "/home/me/Dropbox/cassette",
            "/Users/me/Library/Mobile Documents/cassette",
            "/home/me/OneDrive/notes/cassette",
            "/home/me/Google Drive/cassette",
            // The `<name> ` prefix clause. Untested until now: deleting it
            // outright left every test green, so the half of the rule with
            // real-world consequences was unexercised.
            "/home/me/Dropbox (Personal)/cassette",
            "/home/me/OneDrive - Acme Corp/cassette",
            // Accepted false positive — this only warns, so over-flagging a
            // folder name costs a line of stderr and nothing else.
            "/home/me/Dropbox Backup/cassette",
            // macOS 12.3+ puts these under CloudStorage with a HYPHEN, which
            // neither the equality nor the prefix clause matches.
            "/Users/me/Library/CloudStorage/OneDrive-Personal/cassette",
            "/Users/me/Library/CloudStorage/GoogleDrive-me@gmail.com/cassette",
        ] {
            assert!(looks_synced(Path::new(p)), "should flag {p}");
        }
    }

    /// An ordinary store, and a path that merely CONTAINS one of the words
    /// inside a longer component, are both left alone — the match is on the
    /// whole component, not a bare substring.
    #[test]
    fn an_ordinary_store_is_not_flagged() {
        for p in [
            "/home/me/.local/share/cassette",
            "/home/me/dropboxes-i-have-known/cassette",
            "/tmp/cassette-test",
        ] {
            assert!(!looks_synced(Path::new(p)), "should not flag {p}");
        }
    }

    /// `busy_holder_name` is what turns a failed acquire into the name
    /// `info_text` shows — pure, so this pins the mapping without needing a
    /// real lock or a second process. `LockError::Busy`'s two shapes (a
    /// readable attribution, and a garbled/absent one) must map to `Some`
    /// and `None` respectively; the other variants never name anyone.
    #[test]
    fn busy_holder_name_reads_the_attribution_queue_writes_exit_three_message_uses() {
        let busy_named = store::lock::LockError::Busy {
            id: "c1".to_string(),
            holder: Some(store::lock::Attribution {
                writer: "01WRITERID0000000000000000".to_string(),
                name: "refactor-agent".to_string(),
                pid: 4242,
                since: "2026-09-18T00:00:00Z".to_string(),
            }),
        };
        assert_eq!(
            busy_holder_name(&busy_named).as_deref(),
            Some("refactor-agent")
        );

        let busy_garbled = store::lock::LockError::Busy {
            id: "c1".to_string(),
            holder: None,
        };
        assert_eq!(busy_holder_name(&busy_garbled), None);

        let no_such = store::lock::LockError::NoSuchCassette {
            session: "s".to_string(),
            id: "c1".to_string(),
        };
        assert_eq!(busy_holder_name(&no_such), None);
    }

    #[test]
    fn retry_lock_does_nothing_while_the_session_is_not_read_only() {
        // Nothing to retry when the last attempt already succeeded — calling
        // `acquire` again for the cassette we already hold would trip its own
        // "already holding this one" fast path, but `retry_lock` shouldn't
        // even try: it's a no-op the moment `read_only` is false.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id;
        app.cassettes.push(c);
        let mut writer = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        writer.acquire(&mut app, 0).expect("acquire");
        assert!(!app.read_only.is_read_only());

        retry_lock(&mut app, Some(&mut writer));
        assert!(
            !app.read_only.is_read_only(),
            "still fine: nothing should have changed"
        );
    }

    #[test]
    fn retry_lock_recovers_read_only_on_a_tick_with_no_keypress() {
        // The point of this task: a human who walks away from a busy
        // cassette and comes back later finds it editable without typing a
        // character to discover it. `app.read_only = true` and a
        // `busy_holder` stand in for an earlier failed `follow_focus`
        // attempt (5a) — together they are the whole on-screen record of
        // being busy, so clearing both is what makes the recovery visible:
        // `info_text` falls back to the ordinary mode line the moment
        // `read_only` goes false. Nothing actually contends for the cassette
        // any more, so the tick's own retry must succeed unaided.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id;
        app.cassettes.push(c);
        let mut writer = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        app.read_only = app::ReadOnly::Busy {
            holder: Some("stale-holder".to_string()),
        };

        retry_lock(&mut app, Some(&mut writer));

        assert!(
            !app.read_only.is_read_only(),
            "the tick's retry must recover on its own"
        );
        assert_eq!(
            app.read_only,
            app::ReadOnly::No,
            "a name left behind would keep claiming someone holds it"
        );
    }

    /// Contention has a banner; nothing else does. An `Io` (`acquire`
    /// propagates `flush_held`'s failure — a full disk, a session directory
    /// pulled out from under us) or a `NoSuchCassette` would otherwise show
    /// as a bare `-- READ ONLY --` beside a help row blaming a writer who
    /// does not exist, while the words that failed to flush sit in memory.
    /// It must reach the screen, and as a flash: news expires, so it cannot
    /// become the standing message the banner has to own.
    ///
    /// The mirror property — that ordinary `Busy` leaves an unrelated flash
    /// alone — needs a genuinely held lock and so lives in
    /// `session_writer`'s cross-process test, not here.
    #[test]
    fn a_non_contention_lock_failure_is_reported_not_silently_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let session = seeded_session(&store, None);
        let id = store_cassette(&store, &session, 10, "");
        let mut app = App::new(None, None, None, session.clone());
        app.cassettes.clear();
        let mut c = cassette::Cassette::new();
        c.id = id.clone();
        app.cassettes.push(c);

        // A second writer in this process cannot contend (HELD is
        // process-global), so drive the `Err` arm through the one input that
        // reaches it without a lock at all: an id no cassette in the session
        // has, which `Store::lock` reports as `NoSuchCassette`.
        app.cassettes[0].id = "01JNOSUCHCASSETTE000000000".to_string();
        let mut writer = session_writer::SessionWriter::open(&store, &session, false, "w", "w");
        try_acquire(&mut app, &mut writer, 0);

        assert!(
            app.read_only.is_read_only(),
            "a failed acquire must open read-only"
        );
        assert_eq!(
            app.read_only,
            app::ReadOnly::Busy { holder: None },
            "there is no holder to name: this was not contention"
        );
        let msg = app
            .status_msg
            .as_deref()
            .expect("a failure with no banner of its own must still be reported");
        assert!(
            msg.contains("01JNOSUCHCASSETTE000000000"),
            "the report must carry the real cause: {msg}"
        );
    }

    #[test]
    fn resume_opens_a_session_by_the_id_find_prints() {
        // `cassette find` lists session ids; an id it printed that `resume`
        // then rejected would be a discovery loop that closes on nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let id = seeded_session(&store, None);

        let args = cli::Args {
            resume: Some(Some(id.clone())),
            ..Default::default()
        };
        let (session, created_here, loaded) = resolve_session(&store, &args, None);

        assert_eq!(session, id, "the printed id opens the session it names");
        assert!(
            !created_here,
            "an opened session is not this run's to delete"
        );
        assert!(loaded.is_some(), "and its cassettes are loaded");
    }

    #[test]
    fn resume_still_prefers_an_alias_over_an_id() {
        // Aliases remain the documented form ("the most recent session, or
        // the one with that alias"); accepting an id adds an opening rather
        // than taking one away.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store::Store::new(dir.path().to_path_buf());
        let _other = seeded_session(&store, None);
        let aliased = seeded_session(&store, Some("monday"));

        let args = cli::Args {
            resume: Some(Some("monday".to_string())),
            ..Default::default()
        };
        let (session, _, _) = resolve_session(&store, &args, None);
        assert_eq!(session, aliased);
    }

    #[test]
    fn ctrl_b_flips_side_and_ctrl_f_does_not() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        handle_key(&mut app, key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::B);
        handle_key(&mut app, key(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::B, "^F is no longer bound");
        handle_key(&mut app, key(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.cassettes[0].side, Side::A);
    }

    #[test]
    fn topic_prompt_sets_edits_and_clears() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
    fn paste_inserts_chunk_normalizes_newlines_one_undo() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        type_str(&mut app, "start ");
        handle_paste(&mut app, "one\r\ntwo\rthree");
        assert_eq!(app.cassettes[0].text(), "start one\ntwo\nthree");
        // One `u` takes back the whole paste.
        handle_key(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        type_str(&mut app, "u");
        assert_eq!(app.cassettes[0].text(), "start ");
    }

    #[test]
    fn paste_into_topic_prompt_stays_one_line() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        handle_key(&mut app, key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        handle_paste(&mut app, "two\nlines");
        handle_key(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("two lines"));
        assert_eq!(app.cassettes[0].text(), "", "paste stays out of the tape");
    }

    #[test]
    fn paste_allowed_in_record_mode() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.record = true;
        handle_paste(&mut app, "quoted material\n");
        assert_eq!(app.cassettes[0].text(), "quoted material\n");
    }

    #[test]
    fn keypress_resets_idle_counter() {
        let mut app = App::new(
            Some(60),
            None,
            None,
            "01JTESTSESSN00000000000000".to_string(),
        );
        for _ in 0..App::IDLE_NUDGE_SECS {
            app.tick_idle();
        }
        assert!(app.idle_nudge());
        type_str(&mut app, "a");
        assert!(!app.idle_nudge(), "typing clears the nudge");
    }

    #[test]
    fn session_summary_counts_only_words_added_after_resume() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.load_cassettes(vec![cassette::Cassette::from_sides(
            "five old words sit here".into(),
            String::new(),
            None,
        )]);
        assert!(
            session_summary(&app).is_none(),
            "resume with nothing new written: no recap"
        );
        type_str(&mut app, " two more");
        let s = session_summary(&app).unwrap();
        assert!(s.starts_with("2 new words in 0:0"), "summary was: {s}");
        assert!(s.contains("(7 total)"), "summary was: {s}");
    }

    #[test]
    fn session_summary_reports_words_and_breakdown() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
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
