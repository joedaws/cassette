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
    /// `queue write`: the cassette id and the session it lives in.
    pub queue_write: Option<(String, Option<String>)>,
    /// registered writer to act as (default: $USER, registered on first use)
    pub writer: Option<String>,
    /// `writer register|list|whoami`, if that's what was invoked.
    pub writer_cmd: Option<WriterCmd>,
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

    /// registered writer to act as (default: $USER, registered on first use)
    #[arg(long, value_name = "NAME", global = true)]
    writer: Option<String>,
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
}

#[derive(Subcommand, Debug)]
enum QueueAction {
    /// replace a cassette's body, read from stdin
    Write {
        #[arg(value_name = "ID")]
        id: String,
        /// session to write in (default: the active session)
        #[arg(long, value_name = "ID")]
        session: Option<String>,
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
            Some(Command::Queue { action }) => match action {
                QueueAction::Write { id, session } => args.queue_write = Some((id, session)),
            },
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
