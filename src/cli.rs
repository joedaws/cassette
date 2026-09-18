use clap::{Parser, Subcommand};

/// The parsed CLI, in the shape `main()` consumes.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub timer_secs: Option<u32>,
    pub word_goal: Option<usize>,
    pub note_name: Option<String>,
    pub print_stdout: bool,
    pub visible_lines: Option<usize>,
    pub template: Option<String>,
    pub theme: Option<String>,
    pub list_themes: bool,
    pub record: bool,
    pub daily: bool,
    pub stats: bool,
    /// `find` with the query words that followed it; empty = list all.
    pub find: Option<Vec<String>>,
    /// `resume` with an optional note name: `Some(None)` resumes the most
    /// recently modified note.
    pub resume: Option<Option<String>>,
    /// `queue …`, if that's what was invoked.
    pub queue_cmd: Option<QueueCmd>,
    /// registered writer to act as (default: $CASSETTE_WRITER, else $USER — only $USER may register on first use)
    pub writer: Option<String>,
    /// `writer register|list|whoami`, if that's what was invoked.
    pub writer_cmd: Option<WriterCmd>,
    /// `session new|list|alias`, if that's what was invoked.
    pub session_cmd: Option<SessionCmd>,
    /// emit machine-readable JSON instead of prose.
    pub json: bool,
}

/// `queue …` as `main()` consumes it. Every variant carries `session`:
/// there is no active session to fall back to.
#[derive(Debug, PartialEq)]
pub enum QueueCmd {
    New {
        topic: String,
        session: String,
        placement: crate::queue::Placement,
    },
    Write {
        id: String,
        session: String,
        side: crate::queue::Side,
        mode: crate::queue::WriteMode,
    },
    List {
        session: String,
        status: crate::queue::StatusFilter,
        since: Option<String>,
    },
    Show {
        id: String,
        session: String,
    },
    Next {
        session: String,
    },
    Close {
        id: String,
        session: String,
        message: Option<String>,
    },
    Reopen {
        id: String,
        session: String,
    },
    Move {
        id: String,
        session: String,
        anchor: crate::queue::edit::MoveAnchor,
    },
    Lock {
        id: String,
        session: String,
    },
    Unlock {
        id: String,
        session: String,
    },
}

impl QueueCmd {
    /// The `--session` this command was given. Every variant carries one —
    /// there is no active session to fall back to — and `main()` validates
    /// it through this accessor once, before dispatching, so no command can
    /// reach the store with an unchecked session id.
    ///
    /// The exhaustive match is the enforcement: a ninth queue command does
    /// not compile until it says where its session id lives, which is
    /// stronger than a rule saying each command must remember to validate.
    pub fn session(&self) -> &str {
        match self {
            QueueCmd::New { session, .. }
            | QueueCmd::Write { session, .. }
            | QueueCmd::List { session, .. }
            | QueueCmd::Show { session, .. }
            | QueueCmd::Next { session }
            | QueueCmd::Close { session, .. }
            | QueueCmd::Reopen { session, .. }
            | QueueCmd::Move { session, .. }
            | QueueCmd::Lock { session, .. }
            | QueueCmd::Unlock { session, .. } => session,
        }
    }
}

/// `writer …` as `main()` consumes it.
#[derive(Debug, PartialEq)]
pub enum WriterCmd {
    Register {
        name: String,
        kind: crate::store::writers::Kind,
    },
    List,
    Whoami,
}

/// `session …` as `main()` consumes it.
#[derive(Debug, PartialEq)]
pub enum SessionCmd {
    New { alias: Option<String> },
    List { all: bool },
    Alias { id: String, alias: String },
}

#[derive(Parser, Debug)]
#[command(
    name = "cassette",
    version,
    about = "cassette — a freewriting TUI",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// countdown timer in minutes
    // Capped so `timer * 60` in `into_args` cannot overflow u32: a debug
    // build panicked on `-t 100000000`. clap rejects it as a usage error.
    #[arg(short = 't', value_name = "MINUTES", global = true,
          value_parser = clap::value_parser!(u32).range(1..=(u32::MAX / 60) as i64))]
    timer: Option<u32>,

    /// word goal (winds the tape reel)
    #[arg(short = 'w', value_name = "WORDS", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
    word_goal: Option<u32>,

    /// visible text rows per cassette (2-40)
    #[arg(short = 'l', value_name = "LINES", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
    visible_lines: Option<u32>,

    /// start with one cassette per topic from the named [templates] entry
    #[arg(short = 'T', value_name = "TEMPLATE", global = true)]
    template: Option<String>,

    /// color theme for this session (overrides config)
    #[arg(long, value_name = "NAME", global = true)]
    theme: Option<String>,

    /// record mode: no deletions, the tape only rolls forward
    #[arg(short = 'R', long, global = true)]
    record: bool,

    /// print to stdout on quit instead of writing a file
    #[arg(short = 'o', long = "output", global = true)]
    print_stdout: bool,

    /// registered writer to act as (default: $CASSETTE_WRITER, else $USER — only $USER may register on first use)
    #[arg(long, value_name = "NAME", global = true)]
    writer: Option<String>,

    /// emit machine-readable JSON (full data on queue list/next/show;
    /// {"error","code"} on any command that fails)
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// start a session in a named note
    New {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// open today's note, named by date
    Today,
    /// load a saved note back into the TUI (default: most recently modified)
    Resume {
        #[arg(value_name = "FILE")]
        file: Option<String>,
    },
    /// streak, weekly/monthly notes and words, totals
    Stats,
    /// list recent notes newest-first; TEXT filters by name, topic, or content
    Find {
        // NOT `trailing_var_arg = true`: that captures flags after the first
        // query word, so `find foo -t 10` would yield query ["foo","-t","10"].
        #[arg(value_name = "TEXT")]
        query: Vec<String>,
    },
    /// list available themes (built-in and from config.toml)
    Themes,
    /// work with the shared cassette queue
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// register and inspect writers
    Writer {
        #[command(subcommand)]
        action: WriterAction,
    },
    /// create and inspect sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Subcommand, Debug)]
enum QueueAction {
    /// create a cassette and print its id
    New {
        #[arg(value_name = "TOPIC")]
        topic: String,
        /// session to add the cassette to
        #[arg(long, value_name = "ID")]
        session: String,
        /// place at the head of the queue
        #[arg(long, conflicts_with_all = ["last", "priority"])]
        first: bool,
        /// place at the tail of the queue (default)
        #[arg(long, conflicts_with_all = ["first", "priority"])]
        last: bool,
        /// place at an exact, positive sparse priority
        #[arg(long, value_name = "N", conflicts_with_all = ["first", "last"],
              value_parser = clap::value_parser!(i64).range(1..))]
        priority: Option<i64>,
    },
    /// replace (or append to) one side of a cassette's body, read from stdin
    Write {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
        /// which side to write
        #[arg(long, value_enum, default_value = "a")]
        side: SideArg,
        /// append to the side instead of replacing it
        #[arg(long, conflicts_with = "replace")]
        append: bool,
        /// replace the side's content (default)
        #[arg(long, conflicts_with = "append")]
        replace: bool,
    },
    /// list a session's cassettes in queue order
    List {
        /// session to list
        #[arg(long, value_name = "ID")]
        session: String,
        /// which cassettes to show
        #[arg(long, value_enum, default_value = "open")]
        status: StatusArg,
        /// only cassettes updated at or after this RFC3339 timestamp
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
    },
    /// print one cassette's frontmatter and body
    Show {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
    },
    /// print the id of the next open, unlocked cassette in queue order
    Next {
        /// session to search
        #[arg(long, value_name = "ID")]
        session: String,
    },
    /// close a cassette
    Close {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
        /// close-out note, appended to the body as a blockquote
        #[arg(short = 'm', long, value_name = "TEXT")]
        message: Option<String>,
    },
    /// reopen a closed cassette
    Reopen {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
    },
    /// move a cassette to a new position in the queue
    #[command(group(clap::ArgGroup::new("anchor").required(true).args(["before", "after"])))]
    Move {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
        /// place immediately before this cassette
        #[arg(long, value_name = "ID")]
        before: Option<String>,
        /// place immediately after this cassette
        #[arg(long, value_name = "ID")]
        after: Option<String>,
    },
    /// set the sticky lock to the acting writer (human-only)
    Lock {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
    },
    /// clear the sticky lock, whoever holds it (human-only)
    Unlock {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
    },
}

#[derive(Subcommand, Debug)]
enum WriterAction {
    /// register a writer; the kind is fixed at registration
    Register {
        #[arg(long, value_name = "NAME")]
        name: String,
        /// human or agent — what this writer is permitted to do
        #[arg(long, value_name = "KIND")]
        kind: WriterKindArg,
    },
    /// list registered writers
    List,
    /// show the writer this invocation acts as
    Whoami,
}

#[derive(Subcommand, Debug)]
enum SessionAction {
    /// create a session and print its id
    New {
        /// display label shown in `session list`; it never resolves
        #[arg(long, value_name = "NAME")]
        alias: Option<String>,
    },
    /// list sessions, newest first
    List {
        /// show every session instead of the 15 most recent
        #[arg(long)]
        all: bool,
    },
    /// set a session's display label
    Alias {
        #[arg(value_name = "ID")]
        id: String,
        #[arg(value_name = "ALIAS")]
        alias: String,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum WriterKindArg {
    Human,
    Agent,
}

impl From<WriterKindArg> for crate::store::writers::Kind {
    fn from(k: WriterKindArg) -> crate::store::writers::Kind {
        match k {
            WriterKindArg::Human => crate::store::writers::Kind::Human,
            WriterKindArg::Agent => crate::store::writers::Kind::Agent,
        }
    }
}

/// `--status`'s clap-facing type. `queue::StatusFilter` is the domain type
/// and stays clap-free — this mirrors `WriterKindArg` /
/// `store::writers::Kind` below: the only place the command line is read is
/// `cli.rs`, so the `ValueEnum` derive lives here, not on the type `queue`
/// actually works with.
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum StatusArg {
    Open,
    Closed,
    All,
}

impl From<StatusArg> for crate::queue::StatusFilter {
    fn from(s: StatusArg) -> crate::queue::StatusFilter {
        match s {
            StatusArg::Open => crate::queue::StatusFilter::Open,
            StatusArg::Closed => crate::queue::StatusFilter::Closed,
            StatusArg::All => crate::queue::StatusFilter::All,
        }
    }
}

/// `--side`'s clap-facing type, same shape as `StatusArg`/`WriterKindArg`
/// above: `queue::Side` stays clap-free, and `cli.rs` alone reads the
/// command line. `a`/`b` are the value-enum's default kebab-case spellings
/// of the variant names, so no explicit `#[value(name = ...)]` is needed.
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum SideArg {
    A,
    B,
}

impl From<SideArg> for crate::queue::Side {
    fn from(s: SideArg) -> crate::queue::Side {
        match s {
            SideArg::A => crate::queue::Side::A,
            SideArg::B => crate::queue::Side::B,
        }
    }
}

impl Cli {
    fn into_args(self) -> Args {
        let mut args = Args {
            // The CLI takes minutes; the app works in seconds.
            timer_secs: self.timer.map(|m| m * 60),
            word_goal: self.word_goal.map(|w| w as usize),
            visible_lines: self.visible_lines.map(|l| l as usize),
            template: self.template,
            theme: self.theme,
            record: self.record,
            print_stdout: self.print_stdout,
            writer: self.writer,
            json: self.json,
            ..Args::default()
        };
        match self.command {
            None => {}
            Some(Command::New { name }) => args.note_name = Some(name),
            Some(Command::Today) => args.daily = true,
            Some(Command::Resume { file }) => args.resume = Some(file),
            Some(Command::Stats) => args.stats = true,
            Some(Command::Find { query }) => args.find = Some(query),
            Some(Command::Themes) => args.list_themes = true,
            Some(Command::Queue { action }) => {
                args.queue_cmd = Some(match action {
                    QueueAction::New {
                        topic,
                        session,
                        first,
                        last: _,
                        priority,
                    } => {
                        // `--priority`'s clap `range(1..)` already rejects a
                        // non-positive value before this ever runs; `first`
                        // wins any (impossible, thanks to `conflicts_with_all`)
                        // ambiguity, and no flag at all means the default:
                        // tail placement, so an agent adding work cannot jump
                        // the human's line.
                        let placement = if first {
                            crate::queue::Placement::First
                        } else if let Some(p) = priority {
                            crate::queue::Placement::Explicit(p)
                        } else {
                            crate::queue::Placement::Last
                        };
                        QueueCmd::New {
                            topic,
                            session,
                            placement,
                        }
                    }
                    QueueAction::Write {
                        id,
                        session,
                        side,
                        append,
                        replace: _,
                    } => QueueCmd::Write {
                        id,
                        session,
                        side: side.into(),
                        // No flag at all means the default: replace, which is
                        // `queue write`'s behaviour from before `--side` and
                        // `--append` existed — see the module doc on why that
                        // default must never change under `tests/lock.rs`.
                        mode: if append {
                            crate::queue::WriteMode::Append
                        } else {
                            crate::queue::WriteMode::Replace
                        },
                    },
                    QueueAction::List {
                        session,
                        status,
                        since,
                    } => QueueCmd::List {
                        session,
                        status: status.into(),
                        since,
                    },
                    QueueAction::Show { id, session } => QueueCmd::Show { id, session },
                    QueueAction::Next { session } => QueueCmd::Next { session },
                    QueueAction::Close {
                        id,
                        session,
                        message,
                    } => QueueCmd::Close {
                        id,
                        session,
                        message,
                    },
                    QueueAction::Reopen { id, session } => QueueCmd::Reopen { id, session },
                    QueueAction::Lock { id, session } => QueueCmd::Lock { id, session },
                    QueueAction::Unlock { id, session } => QueueCmd::Unlock { id, session },
                    QueueAction::Move {
                        id,
                        session,
                        before,
                        after,
                    } => {
                        // The `anchor` ArgGroup (`required(true)`, `multiple`
                        // defaulted to `false`) already guarantees exactly
                        // one of `before`/`after` is `Some` by the time clap
                        // hands this back.
                        let anchor = match (before, after) {
                            (Some(b), None) => crate::queue::edit::MoveAnchor::Before(b),
                            (None, Some(a)) => crate::queue::edit::MoveAnchor::After(a),
                            _ => unreachable!(
                                "the 'anchor' ArgGroup enforces exactly one of before/after"
                            ),
                        };
                        QueueCmd::Move {
                            id,
                            session,
                            anchor,
                        }
                    }
                });
            }
            Some(Command::Writer { action }) => {
                args.writer_cmd = Some(match action {
                    WriterAction::Register { name, kind } => WriterCmd::Register {
                        name,
                        kind: kind.into(),
                    },
                    WriterAction::List => WriterCmd::List,
                    WriterAction::Whoami => WriterCmd::Whoami,
                });
            }
            Some(Command::Session { action }) => {
                args.session_cmd = Some(match action {
                    SessionAction::New { alias } => SessionCmd::New { alias },
                    SessionAction::List { all } => SessionCmd::List { all },
                    SessionAction::Alias { id, alias } => SessionCmd::Alias { id, alias },
                });
            }
        }
        args
    }
}

/// Parse the process arguments, exiting with clap's usage error (code 2) on
/// bad input. No hand-written validation: with no top-level positional there
/// is nothing ambiguous left for clap to need help with.
pub fn parse() -> Args {
    Cli::parse().into_args()
}

#[cfg(test)]
fn parse_args_from(args: &[String]) -> Args {
    let mut argv = vec!["cassette".to_string()];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv)
        .expect("test invocation should parse")
        .into_args()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_bare_invocation() {
        assert_eq!(parse_args_from(&argv(&[])), Args::default());
    }

    #[test]
    fn new_sets_the_note_name() {
        let a = parse_args_from(&argv(&["new", "mynote"]));
        assert_eq!(a.note_name, Some("mynote".to_string()));
        assert!(a.resume.is_none());
    }

    #[test]
    fn timer_is_converted_from_minutes_to_seconds() {
        assert_eq!(parse_args_from(&argv(&["-t", "10"])).timer_secs, Some(600));
    }

    #[test]
    fn parses_word_goal_and_visible_lines() {
        let a = parse_args_from(&argv(&["-w", "500", "-l", "8"]));
        assert_eq!(a.word_goal, Some(500));
        assert_eq!(a.visible_lines, Some(8));
    }

    #[test]
    fn parses_template_and_theme() {
        let a = parse_args_from(&argv(&["-T", "morning", "--theme", "gruvbox"]));
        assert_eq!(a.template, Some("morning".to_string()));
        assert_eq!(a.theme, Some("gruvbox".to_string()));
    }

    #[test]
    fn parses_record_and_output_in_both_spellings() {
        assert!(parse_args_from(&argv(&["-R"])).record);
        assert!(parse_args_from(&argv(&["--record"])).record);
        assert!(parse_args_from(&argv(&["-o"])).print_stdout);
        assert!(parse_args_from(&argv(&["--output"])).print_stdout);
    }

    #[test]
    fn bare_resume_means_newest_note() {
        assert_eq!(parse_args_from(&argv(&["resume"])).resume, Some(None));
    }

    #[test]
    fn resume_takes_an_optional_file_name() {
        assert_eq!(
            parse_args_from(&argv(&["resume", "note.md"])).resume,
            Some(Some("note.md".to_string()))
        );
    }

    #[test]
    fn resume_accepts_a_global_flag_without_consuming_it_as_a_file() {
        let a = parse_args_from(&argv(&["resume", "-R"]));
        assert_eq!(a.resume, Some(None));
        assert!(a.record);
    }

    #[test]
    fn parses_action_subcommands() {
        assert!(parse_args_from(&argv(&["today"])).daily);
        assert!(parse_args_from(&argv(&["stats"])).stats);
        assert!(parse_args_from(&argv(&["themes"])).list_themes);
    }

    #[test]
    fn find_collects_trailing_words_as_one_query() {
        assert_eq!(
            parse_args_from(&argv(&["find", "some", "words"])).find,
            Some(vec!["some".to_string(), "words".to_string()])
        );
    }

    #[test]
    fn bare_find_lists_everything() {
        assert_eq!(parse_args_from(&argv(&["find"])).find, Some(Vec::new()));
    }

    #[test]
    fn find_treats_a_later_flag_as_a_flag_not_a_query_word() {
        let a = parse_args_from(&argv(&["find", "foo", "-t", "10"]));
        assert_eq!(a.find, Some(vec!["foo".to_string()]));
        assert_eq!(a.timer_secs, Some(600));
    }

    #[test]
    fn find_with_a_leading_flag_keeps_an_empty_query() {
        let a = parse_args_from(&argv(&["find", "-t", "10"]));
        assert_eq!(a.find, Some(Vec::new()));
        assert_eq!(a.timer_secs, Some(600));
    }

    #[test]
    fn global_flags_work_before_and_after_a_subcommand() {
        assert_eq!(
            parse_args_from(&argv(&["-t", "10", "today"])).timer_secs,
            Some(600)
        );
        let a = parse_args_from(&argv(&["today", "-t", "10"]));
        assert_eq!(a.timer_secs, Some(600));
        assert!(a.daily);
        assert_eq!(
            parse_args_from(&argv(&["stats", "-t", "10"])).timer_secs,
            Some(600)
        );
        assert_eq!(
            parse_args_from(&argv(&["themes", "-t", "10"])).timer_secs,
            Some(600)
        );
    }

    #[test]
    fn global_flags_work_on_either_side_of_new() {
        let a = parse_args_from(&argv(&["new", "mynote", "-t", "10"]));
        assert_eq!(a.note_name, Some("mynote".to_string()));
        assert_eq!(a.timer_secs, Some(600));
        let b = parse_args_from(&argv(&["-t", "10", "new", "mynote"]));
        assert_eq!(b.note_name, Some("mynote".to_string()));
        assert_eq!(b.timer_secs, Some(600));
    }
}
