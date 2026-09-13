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
    /// `--resume` with an optional note name: `Some(None)` resumes the most
    /// recently modified note.
    pub resume: Option<Option<String>>,
}

#[derive(Parser, Debug)]
#[command(
    name = "cassette",
    version,
    about = "cassette — a freewriting TUI",
    disable_help_subcommand = true
)]
struct Cli {
    /// output note name or path; an existing note is resumed
    #[arg(value_name = "NAME")]
    name: Option<String>,

    /// countdown timer in minutes
    #[arg(short = 't', value_name = "MINUTES", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
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

    /// load a saved note back into the TUI and keep writing
    #[arg(long, value_name = "FILE", num_args = 0..=1, global = true)]
    resume: Option<Option<String>>,

    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Subcommand, Debug)]
enum Action {
    /// open today's note (named by date)
    Today,
    /// streak, weekly/monthly notes and words, totals
    Stats,
    /// list recent notes newest-first; TEXT filters by name, topic, or content
    Find {
        // NOTE: deliberately NOT `trailing_var_arg = true`. That attribute would
        // capture flags after the first query word, so `find foo -t 10` would
        // parse as query ["foo", "-t", "10"] with no timer. The hand-rolled
        // parser matches `-t` as a flag there, and behavior is frozen.
        #[arg(value_name = "TEXT")]
        query: Vec<String>,
    },
    /// list available themes (built-in and from config.toml)
    #[command(name = "+themes")]
    Themes,
}

impl Cli {
    fn into_args(self) -> Args {
        let (daily, stats, list_themes, find) = match self.action {
            Some(Action::Today) => (true, false, false, None),
            Some(Action::Stats) => (false, true, false, None),
            Some(Action::Themes) => (false, false, true, None),
            Some(Action::Find { query }) => (false, false, false, Some(query)),
            None => (false, false, false, None),
        };
        Args {
            // The CLI takes minutes; the app works in seconds.
            timer_secs: self.timer.map(|m| m * 60),
            word_goal: self.word_goal.map(|w| w as usize),
            note_name: self.name,
            print_stdout: self.print_stdout,
            visible_lines: self.visible_lines.map(|l| l as usize),
            template: self.template,
            theme: self.theme,
            list_themes,
            record: self.record,
            daily,
            stats,
            find,
            resume: self.resume,
        }
    }
}

/// Parse the process arguments, exiting with clap's usage error (code 2) on
/// bad input.
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

    /// clap's own structural validation: catches conflicting arg
    /// definitions that would otherwise only surface at runtime.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// Build the argument slice the parser expects from string literals.
    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// Every successful invocation shape, pinned. These pass against the
    /// hand-rolled parser and must keep passing against clap.
    #[test]
    fn parses_bare_invocation() {
        assert_eq!(parse_args_from(&argv(&[])), Args::default());
    }

    #[test]
    fn parses_positional_note_name() {
        let a = parse_args_from(&argv(&["mynote"]));
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
        assert_eq!(parse_args_from(&argv(&["--resume"])).resume, Some(None));
    }

    #[test]
    fn resume_takes_an_optional_file_name() {
        assert_eq!(
            parse_args_from(&argv(&["--resume", "note.md"])).resume,
            Some(Some("note.md".to_string()))
        );
    }

    #[test]
    fn resume_does_not_swallow_a_following_flag() {
        let a = parse_args_from(&argv(&["--resume", "-R"]));
        assert_eq!(a.resume, Some(None));
        assert!(a.record);
    }

    #[test]
    fn parses_action_words() {
        assert!(parse_args_from(&argv(&["today"])).daily);
        assert!(parse_args_from(&argv(&["stats"])).stats);
        assert!(parse_args_from(&argv(&["+themes"])).list_themes);
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
    fn flags_work_before_and_after_an_action_word() {
        assert_eq!(
            parse_args_from(&argv(&["-t", "10", "today"])).timer_secs,
            Some(600)
        );
        let a = parse_args_from(&argv(&["today", "-t", "10"]));
        assert_eq!(a.timer_secs, Some(600));
        assert!(a.daily);
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
    fn flags_work_on_either_side_of_stats_and_themes() {
        assert_eq!(
            parse_args_from(&argv(&["stats", "-t", "10"])).timer_secs,
            Some(600)
        );
        assert_eq!(
            parse_args_from(&argv(&["-t", "10", "stats"])).timer_secs,
            Some(600)
        );
        assert_eq!(
            parse_args_from(&argv(&["+themes", "-t", "10"])).timer_secs,
            Some(600)
        );
        assert_eq!(
            parse_args_from(&argv(&["-t", "10", "+themes"])).timer_secs,
            Some(600)
        );
    }

    #[test]
    fn flags_work_on_either_side_of_a_positional_note_name() {
        let a = parse_args_from(&argv(&["mynote", "-t", "10"]));
        assert_eq!(a.note_name, Some("mynote".to_string()));
        assert_eq!(a.timer_secs, Some(600));
        let b = parse_args_from(&argv(&["-t", "10", "mynote"]));
        assert_eq!(b.note_name, Some("mynote".to_string()));
        assert_eq!(b.timer_secs, Some(600));
    }
}
