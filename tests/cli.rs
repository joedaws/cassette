use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cassette"))
        .args(args)
        .output()
        .expect("failed to run the cassette binary")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn version_flag_prints_name_and_version() {
    let out = run(&["-V"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("cassette {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_flag_exits_zero_and_documents_the_surface() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8_lossy(&out.stdout);
    // Assert on content that survives the move to clap-generated help,
    // never on exact formatting.
    for expected in ["cassette", "--resume", "--theme", "today", "stats", "find"] {
        assert!(
            help.contains(expected),
            "help is missing {expected:?}:\n{help}"
        );
    }
}

#[test]
fn short_help_flag_also_exits_zero_and_prints_help() {
    let out = run(&["-h"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("cassette"));
}

#[test]
fn zero_is_rejected_for_numeric_options() {
    for flag in ["-t", "-w", "-l"] {
        let out = run(&[flag, "0"]);
        assert_eq!(out.status.code(), Some(2), "{flag} 0 should exit 2");
    }
}

#[test]
fn non_numeric_values_are_rejected() {
    let out = run(&["-t", "abc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !stderr(&out).is_empty(),
        "an error message should reach stderr"
    );
}

#[test]
fn unknown_option_exits_two() {
    let out = run(&["-x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).is_empty());
}

#[test]
fn extra_positional_after_an_action_exits_two() {
    assert_eq!(run(&["today", "extra"]).status.code(), Some(2));
}

#[test]
fn missing_value_for_an_option_exits_two() {
    assert_eq!(run(&["-T"]).status.code(), Some(2));
    assert_eq!(run(&["--theme"]).status.code(), Some(2));
}

#[test]
fn unknown_template_name_exits_two() {
    let out = run(&["-T", "nosuchtemplate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).is_empty());
}

#[test]
fn unknown_theme_name_exits_two() {
    let out = run(&["--theme", "nosuchtheme"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).is_empty());
}

#[test]
fn note_name_before_an_action_word_exits_two() {
    assert_eq!(run(&["mynote", "stats"]).status.code(), Some(2));
    assert_eq!(run(&["mynote", "find", "foo"]).status.code(), Some(2));
    assert_eq!(run(&["mynote", "today"]).status.code(), Some(2));
}

#[test]
fn note_name_with_themes_action_is_still_allowed() {
    let out = run(&["mynote", "+themes"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("themes"));
}

#[test]
fn version_flag_works_after_an_action_word() {
    let out = run(&["stats", "--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("cassette {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(run(&["today", "-V"]).status.code(), Some(0));
}
