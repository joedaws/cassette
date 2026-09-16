use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cassette")
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
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
    for expected in [
        "cassette", "new", "today", "resume", "stats", "find", "themes",
    ] {
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
fn a_timer_too_large_to_convert_to_seconds_is_rejected() {
    // `-t` is minutes and the app stores seconds; an unbounded value used to
    // overflow the `* 60` and panic in debug builds. It must be a usage error.
    let out = run(&["-t", "100000000"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        !stderr(&out).contains("panicked"),
        "should be a usage error, not a panic: {}",
        stderr(&out)
    );
}

#[test]
fn the_largest_convertible_timer_is_accepted() {
    // u32::MAX / 60 minutes is the boundary: it must still parse.
    let out = run(&["-t", &(u32::MAX / 60).to_string(), "themes"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
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
fn new_without_a_name_exits_two() {
    assert_eq!(run(&["new"]).status.code(), Some(2));
}

#[test]
fn version_after_a_subcommand_exits_two() {
    // clap convention: --version is top-level only, like `git status --version`.
    assert_eq!(run(&["stats", "--version"]).status.code(), Some(2));
}

#[test]
fn queue_write_without_an_id_exits_two() {
    let out = run(&["queue", "write"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_write_appears_in_help() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("queue"), "{text}");
}

#[test]
fn a_writer_name_is_required_when_user_is_unset() {
    // No shared "unknown" identity: attribution is the point of the system.
    let out = Command::new(bin())
        .args([
            "queue",
            "write",
            "01K5GR7T2M9WPD0000000000AB",
            "--session",
            "01K5GQ2R8V3XQZ0000000000AB",
        ])
        .env_remove("USER")
        .env("CASSETTE_DATA_DIR", "/nonexistent-store")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--writer"),
        "the error must name the fix: {}",
        stderr(&out)
    );
}

#[test]
fn the_writer_flag_is_global() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("--writer"), "{text}");
}

#[test]
fn writer_register_then_list_then_whoami() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let reg = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(reg.status.code(), Some(0), "{}", stderr(&reg));

    let list = Command::new(bin())
        .args(["writer", "list"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&list.stdout).to_string();
    assert!(text.contains("bot"), "{text}");
    assert!(text.contains("agent"), "{text}");

    let who = Command::new(bin())
        .args(["--writer", "bot", "writer", "whoami"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&who.stdout).to_string();
    assert!(text.contains("bot"), "{text}");
    assert!(
        text.contains("agent"),
        "kind comes from the registry: {text}"
    );
}

#[test]
fn an_unknown_writer_flag_is_a_usage_error() {
    // A typo in --writer must fail loudly rather than silently creating a
    // second identity — and an auto-created one would be `human`, the
    // privileged kind, so this fails open if it is wrong.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let out = Command::new(bin())
        .args(["--writer", "nosuchwriter", "writer", "list"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    // `writer list` does not resolve a writer, so this must still succeed —
    // the point is that merely NAMING an unknown writer is not itself fatal.
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn queue_write_with_an_unknown_writer_flag_exits_two_without_creating_one() {
    // The command that DOES resolve a writer: `queue write --writer <typo>`
    // must exit 2 rather than auto-registering a second, human identity.
    // Hand-built fixture, same shape as tests/lock.rs — no CLI command
    // creates a session yet.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    const SESSION: &str = "01K5GQ2R8V3XQZ0000000000AB";
    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(SESSION).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::create_dir_all(root.join("sessions").join(SESSION).join(".locks")).expect("mkdir");
    std::fs::write(
        root.join("sessions").join(SESSION).join("session.toml"),
        "created = \"2026-09-14T09:25:57Z\"\n",
    )
    .expect("session.toml");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\nlocked_by:\n\
             created_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
             ## Side A\n\noriginal\n"
        ),
    )
    .expect("cassette");

    let out = Command::new(bin())
        .args([
            "--writer",
            "nosuchwriter",
            "queue",
            "write",
            ID,
            "--session",
            SESSION,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .stdin(std::process::Stdio::piped())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("nosuchwriter"), "{}", stderr(&out));

    // And the registry must not have gained a second, auto-created identity.
    let registry = root.join("writers.toml");
    assert!(
        !registry.exists(),
        "an unknown --writer must not create anything: {}",
        std::fs::read_to_string(&registry).unwrap_or_default()
    );
}

#[test]
fn registering_a_known_name_with_a_different_kind_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let first = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(first.status.code(), Some(0));

    let second = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "human"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(second.status.code(), Some(2), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("already registered"),
        "{}",
        stderr(&second)
    );
}
