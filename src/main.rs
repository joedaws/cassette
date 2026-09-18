use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
mod find;
mod output;
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
    let args = cli::parse();
    let cfg = config::load_config().unwrap_or_else(|e| exit_with(2, &e, args.json));

    if args.list_themes {
        print_themes(&cfg);
        return Ok(());
    }

    // `stats` and `find` read only the session store — the legacy notes dir
    // (`cfg.notes_dir` / `config::default_notes_dir`) is deliberately not
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
    if let Some(cassettes) = loaded {
        app.load_cassettes(cassettes);
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
        let focus = app.focus_idx;
        if let Err(e) = w.acquire(&mut app, focus) {
            app.read_only = true;
            eprintln!("cassette: {e} — opening read-only");
        }
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
        run(&mut terminal, &mut app, writer.as_mut(), &theme)
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
fn resolve_session(
    store: &store::Store,
    args: &cli::Args,
    daily_name: Option<&str>,
) -> (String, bool, Option<Vec<cassette::Cassette>>) {
    let by_alias = |alias: &str| -> Option<String> {
        store
            .list_sessions()
            .unwrap_or_else(|e| die_with(1, &format!("cannot list sessions: {e}")))
            .into_iter()
            .find(|(_, m)| m.alias.as_deref() == Some(alias))
            .map(|(id, _)| id)
    };
    let open = |id: String| -> (String, bool, Option<Vec<cassette::Cassette>>) {
        let cassettes = load_session_cassettes(store, &id);
        (id, false, Some(cassettes))
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
fn load_session_cassettes(store: &store::Store, session: &str) -> Vec<cassette::Cassette> {
    let scan = store
        .scan_session(session)
        .unwrap_or_else(|e| die_with(1, &format!("cannot read session '{session}': {e}")));
    if scan.unreadable > 0 {
        eprintln!(
            "cassette: {} file(s) in this session could not be read and were skipped",
            scan.unreadable
        );
    }
    scan.cassettes
        .into_iter()
        .map(|c| {
            let (side_a, side_b) = queue::json::split_sides(&c.body);
            let mut loaded = cassette::Cassette::from_sides(
                side_a.trim().to_string(),
                side_b.trim().to_string(),
                c.meta.topic,
            );
            loaded.id = c.meta.id;
            loaded
        })
        .collect()
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

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    mut writer: Option<&mut session_writer::SessionWriter>,
    theme: &theme::Theme,
) -> io::Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();
    let mut last_autosave = Instant::now();
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
    if app
        .cassettes
        .get(app.focus_idx)
        .is_some_and(|c| Some(c.id.as_str()) == w.held_id())
    {
        return;
    }
    match w.acquire(app, app.focus_idx) {
        Ok(()) => app.read_only = false,
        Err(e) => {
            app.read_only = true;
            app.status_msg = Some(e.to_string());
        }
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
            // never resolves or creates), so where the name came from makes
            // no difference here — `.0` drops the `WriterSource`.
            let (who_name, _source) =
                resolve_writer_name(writer_flag).unwrap_or_else(|e| exit_with(2, &e, json));
            match writer::whoami(&store, &who_name) {
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

/// The store root: `$CASSETTE_DATA_DIR` when set, else the XDG default.
///
/// The environment override exists so tests never touch the real store at
/// `~/.local/share/cassette`. Phase 6 adds a `data_dir` config key beside it;
/// the existing `notes_dir` key points at the old flat notes folder and is
/// deliberately NOT consulted here.
///
/// Failing to determine a data dir at all is an I/O failure the caller cannot
/// fix by retrying with different arguments (README's exit-1 rule), not a
/// usage error — and, like every other failure under `--json`, it must still
/// emit the `{"error","code"}` envelope rather than bare stderr prose.
fn store_root(json: bool) -> PathBuf {
    std::env::var_os("CASSETTE_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(store::Store::default_root)
        .unwrap_or_else(|| exit_with(1, "cannot determine a data dir", json))
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
