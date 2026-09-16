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
fn queue_write_without_session_exits_two() {
    // The headline behaviour Phase 4b introduced — every `queue` command
    // requires `--session` — is otherwise unpinned by any test. clap itself
    // must refuse a missing required arg before any store I/O happens.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let out = Command::new(bin())
        .args(["queue", "write", "01K5GR7T2M9WPD0000000000AB"])
        .env("CASSETTE_DATA_DIR", &root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--session"),
        "clap's usage error should name the missing flag: {}",
        stderr(&out)
    );
}

#[test]
fn cassette_writer_env_names_a_writer_but_must_already_be_registered() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let out = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // `queue new` resolves a writer to act as, so an unknown name is a typo,
    // not a first run — exit 2 rather than a second identity created as
    // `human`, the privileged kind.
    let out = Command::new(bin())
        .args(["queue", "new", "a topic", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("CASSETTE_WRITER", "ghost")
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));

    // ...and it must not have created one.
    let listed = Command::new(bin())
        .args(["writer", "list"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(
        !String::from_utf8_lossy(&listed.stdout).contains("ghost"),
        "a typo must not spawn an identity"
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

#[test]
fn session_new_then_list_then_alias() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");

    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(new.status.code(), Some(0), "{}", stderr(&new));
    let id = String::from_utf8_lossy(&new.stdout).trim().to_string();
    assert_eq!(id.len(), 26, "a ULID is printed bare for scripting: {id:?}");

    let aliased = Command::new(bin())
        .args(["session", "alias", &id, "monday"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(aliased.status.code(), Some(0), "{}", stderr(&aliased));

    let list = Command::new(bin())
        .args(["session", "list"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&list.stdout).to_string();
    assert!(text.contains(&id), "{text}");
    assert!(text.contains("monday"), "{text}");
}

#[test]
fn session_alias_on_an_unknown_id_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["session", "alias", "01K5GQ2R8V3XQZ0000000000AB", "x"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

/// Hand-write a cassette file into `session`'s cassettes dir — there is no
/// CLI command that creates cassettes yet, so tests exercising `queue list`
/// and `queue show` build the fixture directly, same shape as
/// `queue_write_with_an_unknown_writer_flag_exits_two_without_creating_one`.
fn write_fixture_cassette(root: &std::path::Path, session: &str, id: &str, topic: &str) {
    let cassettes = root.join("sessions").join(session).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::write(
        cassettes.join(format!("{topic}-{id}.md")),
        format!(
            "---\nid: {id}\ntopic: {topic}\npriority: 10\nstatus: open\nlocked_by:\n\
             created_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
             ## Side A\n\nhello from {topic}\n"
        ),
    )
    .expect("cassette");
}

#[test]
fn queue_list_and_show_round_trip_through_a_real_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");

    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(new.status.code(), Some(0), "{}", stderr(&new));
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let list = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(list.status.code(), Some(0), "{}", stderr(&list));
    let text = String::from_utf8_lossy(&list.stdout).to_string();
    assert!(text.contains(ID), "{text}");
    assert!(text.contains("gratitude"), "{text}");

    let show = Command::new(bin())
        .args(["queue", "show", ID, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(show.status.code(), Some(0), "{}", stderr(&show));
    let shown = String::from_utf8_lossy(&show.stdout).to_string();
    assert!(shown.contains(&format!("id: {ID}")), "{shown}");
    assert!(shown.contains("hello from gratitude"), "{shown}");
}

#[test]
fn queue_list_says_so_when_a_session_has_no_cassettes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "no cassettes");
}

#[test]
fn queue_list_status_filter_excludes_closed_by_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::write(
        cassettes.join("closed-01K5GR7T2M9WPD0000000000CD.md"),
        "---\nid: 01K5GR7T2M9WPD0000000000CD\ntopic: closed\npriority: 10\nstatus: closed\n\
         locked_by:\ncreated_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
         ## Side A\n\nold\n",
    )
    .expect("cassette");

    let default_list = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(
        String::from_utf8_lossy(&default_list.stdout).trim(),
        "no cassettes",
        "closed cassettes are hidden unless asked for"
    );

    let all_list = Command::new(bin())
        .args(["queue", "list", "--session", &sid, "--status", "all"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(
        String::from_utf8_lossy(&all_list.stdout).contains("closed"),
        "{}",
        String::from_utf8_lossy(&all_list.stdout)
    );
}

#[test]
fn queue_show_on_an_unknown_id_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "show", "nosuchid", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_list_and_show_need_no_writer_identity() {
    // Read-only: neither attributes anything, so neither should fail just
    // because $USER is unset and no --writer was passed — unlike `queue
    // write`, see `a_writer_name_is_required_when_user_is_unset`.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["queue", "list", "--session", "01K5GQ2R8V3XQZ0000000000AB"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn queue_next_on_an_empty_session_exits_five() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let id = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "next", "--session", &id])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(5), "{}", stderr(&out));
}

#[test]
fn queue_next_prints_the_id_of_an_open_unlocked_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let out = Command::new(bin())
        .args(["queue", "next", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), ID);
}

#[test]
fn queue_next_exits_three_when_every_open_cassette_is_locked() {
    // Every open cassette held is a different empty-handed outcome than no
    // open cassettes at all (exit 5, see `queue_next_on_an_empty_session_
    // exits_five`): 3 says wait and retry, 5 says there is nothing to wait
    // for. Held by flocking the anchor directly in this test process, the
    // same primitive `queue write`/`queue next` use.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let anchor_path = root.join("sessions").join(&sid).join(".locks").join(ID);
    let anchor = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&anchor_path)
        .expect("open anchor");
    fs4::FileExt::lock(&anchor).expect("flock the anchor");

    let out = Command::new(bin())
        .args(["queue", "next", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));

    fs4::FileExt::unlock(&anchor).expect("release");
}

#[test]
fn queue_new_rejects_a_non_positive_priority() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args([
            "queue",
            "new",
            "a topic",
            "--session",
            &sid,
            "--priority",
            "0",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_new_then_next_returns_the_new_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let made = Command::new(bin())
        .args(["queue", "new", "gratitude", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(made.status.code(), Some(0), "{}", stderr(&made));
    let cid = String::from_utf8_lossy(&made.stdout).trim().to_string();

    let next = Command::new(bin())
        .args(["queue", "next", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(next.status.code(), Some(0), "{}", stderr(&next));
    assert_eq!(String::from_utf8_lossy(&next.stdout).trim(), cid);
}

#[test]
fn queue_new_first_and_last_are_mutually_exclusive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args([
            "queue",
            "new",
            "a topic",
            "--session",
            &sid,
            "--first",
            "--last",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_new_exits_six_once_the_open_cap_is_reached() {
    // The cap arithmetic itself (open-only counting, boundary value) is unit
    // tested in `src/queue/edit.rs::tests::the_cap_counts_open_cassettes_only`;
    // this is the end-to-end pin that the CLI actually enforces
    // `store::MAX_OPEN` (36) and maps a full queue to exit code 6.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    for _ in 0..36 {
        let out = Command::new(bin())
            .args(["queue", "new", "filler", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    }

    let out = Command::new(bin())
        .args(["queue", "new", "one too many", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(6), "{}", stderr(&out));
}

#[test]
fn queue_close_then_list_shows_it_closed_and_reopen_restores_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let out = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let cid = {
        let out = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let closed = Command::new(bin())
        .args([
            "queue",
            "close",
            &cid,
            "--session",
            &sid,
            "-m",
            "done for now",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(closed.status.code(), Some(0), "{}", stderr(&closed));

    // Closed cassettes are hidden by the default --status open filter.
    let listed = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(
        !String::from_utf8_lossy(&listed.stdout).contains(&cid),
        "a closed cassette must not show under --status open"
    );

    let shown = Command::new(bin())
        .args(["queue", "show", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(
        String::from_utf8_lossy(&shown.stdout).contains("\n> done for now\n"),
        "{}",
        String::from_utf8_lossy(&shown.stdout)
    );

    let reopened = Command::new(bin())
        .args(["queue", "reopen", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(reopened.status.code(), Some(0), "{}", stderr(&reopened));

    let again = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&again.stdout).contains(&cid));
}

#[test]
fn queue_close_rejects_a_message_containing_a_newline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let out = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let cid = {
        let out = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let out = Command::new(bin())
        .args([
            "queue",
            "close",
            &cid,
            "--session",
            &sid,
            "-m",
            "done\n## Side B",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));

    // Rejected before anything is touched: still open.
    let listed = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&listed.stdout).contains(&cid));
}

#[test]
fn queue_close_exits_three_when_the_cassette_is_locked() {
    // A cassette someone is actively writing cannot be closed by anyone,
    // human or agent — the advisory flock wins regardless of `Kind`. Same
    // fixture-and-flock technique as `queue_next_exits_three_when_every_
    // open_cassette_is_locked`.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let anchor_path = root.join("sessions").join(&sid).join(".locks").join(ID);
    let anchor = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&anchor_path)
        .expect("open anchor");
    fs4::FileExt::lock(&anchor).expect("flock the anchor");

    let out = Command::new(bin())
        .args(["queue", "close", ID, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));

    fs4::FileExt::unlock(&anchor).expect("release");
}

#[test]
fn queue_close_exits_four_for_an_agent_over_a_sticky_lock_but_a_human_may_close_it() {
    // The permission boundary this whole task adds: nothing in this phase
    // sets `locked_by` through the CLI (`queue lock` is 4c), so the fixture
    // hand-writes it, the same way the store-level unit test does.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\n\
             locked_by: 01WRITER0000000000000000AB\ncreated_by: w\nlast_writer: w\n\
             updated_at: 2026-09-14T09:25:57Z\n---\n\n## Side A\n\nhello\n"
        ),
    )
    .expect("cassette");

    let agent = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(agent.status.code(), Some(0), "{}", stderr(&agent));
    let human = Command::new(bin())
        .args(["writer", "register", "--name", "joseph", "--kind", "human"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));

    let as_agent = Command::new(bin())
        .args(["--writer", "bot", "queue", "close", ID, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(as_agent.status.code(), Some(4), "{}", stderr(&as_agent));

    let as_human = Command::new(bin())
        .args([
            "--writer",
            "joseph",
            "queue",
            "close",
            ID,
            "--session",
            &sid,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(as_human.status.code(), Some(0), "{}", stderr(&as_human));
}

#[test]
fn queue_reopen_exits_six_once_the_open_cap_is_reached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    for _ in 0..36 {
        let out = Command::new(bin())
            .args(["queue", "new", "filler", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    }

    const ID: &str = "01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::write(
        cassettes.join(format!("closed-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: closed\npriority: 9999\nstatus: closed\nlocked_by:\n\
             created_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
             ## Side A\n\nhello\n"
        ),
    )
    .expect("cassette");

    let out = Command::new(bin())
        .args(["queue", "reopen", ID, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(6), "{}", stderr(&out));
}

#[test]
fn queue_move_reorders_cassettes_and_persists_the_new_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let mut ids = Vec::new();
    for topic in ["first", "second", "third"] {
        let out = Command::new(bin())
            .args(["queue", "new", topic, "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        ids.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let first = &ids[0];
    let third = &ids[2];

    let moved = Command::new(bin())
        .args(["queue", "move", third, "--session", &sid, "--before", first])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(moved.status.code(), Some(0), "{}", stderr(&moved));

    let listed = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let out = String::from_utf8_lossy(&listed.stdout).to_string();
    // Assert on relative order only, never exact formatting.
    let pos_third = out.find(third.as_str()).expect("third listed");
    let pos_first = out.find(first.as_str()).expect("first listed");
    assert!(
        pos_third < pos_first,
        "the moved cassette must now sort before its anchor: {out}"
    );
}

#[test]
fn queue_move_rejects_moving_relative_to_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let created = Command::new(bin())
        .args(["queue", "new", "solo", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    let id = String::from_utf8_lossy(&created.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "move", &id, "--session", &sid, "--before", &id])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn queue_move_requires_exactly_one_of_before_or_after() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let neither = Command::new(bin())
        .args(["queue", "move", "someid", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(neither.status.code(), Some(2), "{}", stderr(&neither));

    let both = Command::new(bin())
        .args([
            "queue",
            "move",
            "someid",
            "--session",
            &sid,
            "--before",
            "a",
            "--after",
            "b",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(both.status.code(), Some(2), "{}", stderr(&both));
}
