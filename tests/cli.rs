use std::io::Write;
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

/// Both of these exit inside clap, before `main` reaches any store code, so
/// they are safe without `CASSETTE_DATA_DIR` — every test that DOES reach
/// the store sets it, because otherwise it writes the user's real notes.
/// Both rejection paths exit 2 with their own message: a malformed id and a
/// well-formed one naming nothing are different mistakes. Neither touches
/// the store, so these need no CASSETTE_DATA_DIR.
#[test]
fn export_rejects_a_malformed_session_id() {
    let out = run(&["export", "not-a-ulid"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(stderr(&out).contains("malformed"), "{}", stderr(&out));
}

#[test]
fn export_help_parses() {
    let out = run(&["export", "--help"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("flat markdown"), "{text}");
    assert!(
        text.contains("--out"),
        "the path flag is documented: {text}"
    );
}

#[test]
fn sessions_help_parses() {
    let out = run(&["sessions", "--help"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("pick a session"),
        "the subcommand describes itself: {text}"
    );
}

#[test]
fn sessions_rejects_an_unexpected_argument() {
    let out = run(&["sessions", "nope"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "bad input exits 2 through clap: {out:?}"
    );
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
    //
    // Runs against a real session: `--session` is validated before any
    // command resolves a writer, so a made-up session id would fail first
    // and this test would pass for the wrong reason.
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
            "write",
            "cas_01K5GR7T2M9WPD0000000000AB",
            "--session",
            &sid,
        ])
        .env_remove("USER")
        .env("CASSETTE_DATA_DIR", &root)
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
    const SESSION: &str = "ses_01K5GQ2R8V3XQZ0000000000AB";
    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
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
             created_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
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
        .args(["queue", "write", "cas_01K5GR7T2M9WPD0000000000AB"])
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
    assert_eq!(
        id.len(),
        30,
        "a ses_ id is printed bare for scripting: {id:?}"
    );

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
        .args(["session", "alias", "ses_01K5GQ2R8V3XQZ0000000000AB", "x"])
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
             created_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
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
        cassettes.join("closed-cas_01K5GR7T2M9WPD0000000000CD.md"),
        "---\nid: cas_01K5GR7T2M9WPD0000000000CD\ntopic: closed\npriority: 10\nstatus: closed\n\
         locked_by:\ncreated_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
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
    let root = dir.path().join("store");
    // A real session: an unknown one is exit 2 for every queue command, so
    // listing a made-up id would no longer test writer identity at all.
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();

    let out = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn queue_list_json_emits_the_contract_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new", "--alias", "monday"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let mut child = Command::new(bin())
        .args(["queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"three whole words\n")
        .expect("write");
    assert!(child.wait().expect("wait").success());

    let out = Command::new(bin())
        .args(["queue", "list", "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["session"]["id"], sid.as_str());
    assert_eq!(v["session"]["alias"], "monday");
    assert_eq!(v["unreadable"], 0, "nothing damaged in this store: {v}");
    let c = &v["cassettes"][0];
    assert_eq!(c["id"], cid.as_str());
    assert_eq!(c["status"], "open");
    assert_eq!(c["words"], 3, "words counts the body: {c}");
    assert_eq!(c["busy"], false, "nobody holds it: {c}");
    assert_eq!(c["sticky_lock"], serde_json::Value::Null);
    assert_eq!(c["last_writer"]["name"], "joseph");
    assert_eq!(c["last_writer"]["kind"], "human");
    assert_eq!(
        c["waiting_on"], "agent",
        "a human wrote last, so the agent is up"
    );
    assert_eq!(
        c["side_a"].as_str().expect("side_a").trim(),
        "three whole words"
    );
    assert_eq!(c["side_b"], "");
}

#[test]
fn queue_list_json_counts_unreadable_cassettes_like_the_prose_listing_does() {
    // A damaged cassette must not silently vanish from either form: prose
    // `queue list` already counts it into a trailing "N unreadable" line
    // rather than hiding it, and `--json` must report the same count from
    // the same scan rather than going quiet about store damage.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();
    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    // A second file in the same cassettes dir with no parseable frontmatter
    // at all — the store counts this as unreadable rather than skipping it
    // silently.
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::write(cassettes.join("garbled.md"), "not a cassette file\n").expect("garbled file");

    let prose = Command::new(bin())
        .args(["queue", "list", "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(prose.status.code(), Some(0), "{}", stderr(&prose));
    let prose_text = String::from_utf8_lossy(&prose.stdout).to_string();
    assert!(prose_text.contains("1 unreadable"), "prose: {prose_text}");

    let json_out = Command::new(bin())
        .args(["queue", "list", "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(json_out.status.code(), Some(0), "{}", stderr(&json_out));
    let v: serde_json::Value = serde_json::from_slice(&json_out.stdout).expect("valid JSON");
    assert_eq!(v["unreadable"], 1, "same count as the prose listing: {v}");
    assert_eq!(
        v["cassettes"].as_array().expect("cassettes").len(),
        1,
        "the one valid cassette is still listed: {v}"
    );
}

#[test]
fn queue_show_json_emits_the_same_cassette_as_show() {
    // `show --json` reads and parses the same file `show`'s prose reads —
    // the two must never disagree about which cassette they describe.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();
    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let out = Command::new(bin())
        .args(["queue", "show", ID, "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["id"], ID);
    assert_eq!(v["topic"], "gratitude");
    assert_eq!(v["status"], "open");
    assert_eq!(
        v["side_a"].as_str().expect("side_a").trim(),
        "hello from gratitude"
    );
    // The fixture's writer id ("w") is not in writers.toml, so both
    // attributions must resolve to null rather than a guessed name.
    assert_eq!(v["created_by"], serde_json::Value::Null);
    assert_eq!(v["last_writer"], serde_json::Value::Null);
    assert_eq!(v["waiting_on"], serde_json::Value::Null);
}

#[test]
fn queue_next_json_emits_a_single_cassette_view_not_a_listing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let new = Command::new(bin())
        .args(["session", "new"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let sid = String::from_utf8_lossy(&new.stdout).trim().to_string();
    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    write_fixture_cassette(&root, &sid, ID, "gratitude");

    let out = Command::new(bin())
        .args(["queue", "next", "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .env_remove("USER")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(
        v["id"], ID,
        "a bare CassetteView, not {{\"cassettes\": [...]}}: {v}"
    );
    assert_eq!(v["busy"], false);
}

#[test]
fn a_traversal_session_id_exits_two_and_writes_nothing_outside_the_store() {
    // `--session` is joined straight onto the store root, so an unvalidated
    // `../../escaped` used to exit 0 and write the cassette outside the
    // store entirely. Shape is checked before any command touches disk.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let out = Command::new(bin())
        .args(["queue", "new", "oops", "--session", "../../escaped"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("malformed session id"),
        "must say the id is malformed, not that the session is missing: {}",
        stderr(&out)
    );
    // Nothing anywhere: not in the store, not beside it, not above it.
    assert!(!root.join("escaped").exists());
    assert!(!dir.path().join("escaped").exists());
    assert!(!dir.path().parent().unwrap().join("escaped").exists());
}

#[test]
fn a_well_formed_but_unknown_session_exits_two_and_creates_nothing() {
    // The phantom-session case: a well-formed id nobody created used to
    // exit 0, minting a session directory with no session.toml — so the
    // cassette it wrote could never appear in `session list` again. Only
    // `session new` creates sessions.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    const GHOST: &str = "ses_01M2N4PCZZQC4J9B0DVAK40GJM";
    let out = Command::new(bin())
        .args(["queue", "new", "ghost", "--session", GHOST])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no session"),
        "must distinguish a missing session from a malformed id: {}",
        stderr(&out)
    );
    assert!(
        !root.join("sessions").join(GHOST).exists(),
        "an unknown session must never be created by side effect"
    );
}

#[test]
fn every_queue_command_rejects_an_unknown_session_with_exit_two() {
    // The spec's exit table: unknown session is 2, raised by any command.
    // Before the shared gate, `list` exited 0 with "no cassettes" and `next`
    // exited 5 — telling an agent loop to idle when the truth was a typo.
    // One case per `QueueCmd` variant, so a ninth command has a row to add.
    const GHOST: &str = "ses_01M2N4PCZZQC4J9B0DVAK40GJM";
    const CID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    let commands: [&[&str]; 8] = [
        &["queue", "list", "--session", GHOST],
        &["queue", "next", "--session", GHOST],
        &["queue", "new", "a topic", "--session", GHOST],
        &["queue", "show", CID, "--session", GHOST],
        &["queue", "close", CID, "--session", GHOST],
        &["queue", "reopen", CID, "--session", GHOST],
        &["queue", "move", CID, "--session", GHOST, "--before", CID],
        &["queue", "write", CID, "--session", GHOST],
    ];
    for args in commands {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = Command::new(bin())
            .args(args)
            .env("CASSETTE_DATA_DIR", dir.path().join("store"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("spawn");
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?} must exit 2 on an unknown session: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("no session"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn session_alias_rejects_a_malformed_id_through_the_same_gate() {
    // `session alias` is the other command that takes a typed session id,
    // and it shares `Store::require_session` so the two cannot drift.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["session", "alias", "../../escaped", "x"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("malformed session id"),
        "{}",
        stderr(&out)
    );
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\n\
             locked_by: wri_01K5GQ00000000000000000002\ncreated_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\n\
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

    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::write(
        cassettes.join(format!("closed-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: closed\npriority: 9999\nstatus: closed\nlocked_by:\n\
             created_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
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

#[test]
fn queue_lock_then_an_agent_is_refused_and_a_human_clears_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");

    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let reg = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(reg.status.code(), Some(0), "{}", stderr(&reg));

    let locked = Command::new(bin())
        .args([
            "--writer",
            "joseph",
            "queue",
            "lock",
            &cid,
            "--session",
            &sid,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(locked.status.code(), Some(0), "{}", stderr(&locked));

    // An agent invoking a human-only command is exit 2, not 4.
    let refused = Command::new(bin())
        .args(["--writer", "bot", "queue", "lock", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));

    let cleared = Command::new(bin())
        .args([
            "--writer",
            "joseph",
            "queue",
            "unlock",
            &cid,
            "--session",
            &sid,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(cleared.status.code(), Some(0), "{}", stderr(&cleared));
}

#[test]
fn queue_write_is_blocked_by_a_sticky_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");

    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let reg = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(reg.status.code(), Some(0), "{}", stderr(&reg));

    let locked = Command::new(bin())
        .args([
            "--writer",
            "joseph",
            "queue",
            "lock",
            &cid,
            "--session",
            &sid,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(locked.status.code(), Some(0), "{}", stderr(&locked));

    // An agent writing over a sticky lock is exit 4, not 3: nobody is
    // actively holding the advisory flock, but the sticky claim still binds.
    let mut child = Command::new(bin())
        .args(["--writer", "bot", "queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"agent prose\n")
        .expect("write");
    let refused = child.wait_with_output().expect("wait");
    assert_eq!(refused.status.code(), Some(4), "{}", stderr(&refused));

    // A human may still write over the same sticky lock — named
    // explicitly, since a writer taken from `$USER` carries no human
    // authority.
    let mut child = Command::new(bin())
        .args([
            "--writer",
            "joseph",
            "queue",
            "write",
            &cid,
            "--session",
            &sid,
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"human prose\n")
        .expect("write");
    let allowed = child.wait_with_output().expect("wait");
    assert_eq!(allowed.status.code(), Some(0), "{}", stderr(&allowed));
}

#[test]
fn json_errors_carry_the_exit_code_in_the_envelope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    // A malformed session id is exit 2 on every queue command.
    let out = Command::new(bin())
        .args(["queue", "list", "--session", "not-a-ulid", "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));

    // The envelope goes to stdout as parseable JSON, not to stderr as prose:
    // an agent redirecting stderr must still get a machine-readable failure.
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(v["code"], 2, "{v}");
    assert!(
        v["error"]
            .as_str()
            .expect("error string")
            .contains("not-a-ulid"),
        "the message must name what was wrong: {v}"
    );
}

#[test]
fn without_json_errors_stay_prose_on_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["queue", "list", "--session", "not-a-ulid"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .env("USER", "tester")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "no JSON without --json");
    assert!(stderr(&out).contains("cassette:"), "{}", stderr(&out));
}

#[test]
fn json_covers_a_missing_writer_identity_before_any_queue_error_exists() {
    // `resolve_writer_name`'s own failure (no `--writer`, no usable $USER or
    // $CASSETTE_WRITER) is not a `QueueError` — it happens in `main.rs`
    // before any queue command runs — so `exit_queue_err` alone cannot
    // reach it. Pinned here so it doesn't regress back to prose-only under
    // `--json`; see `a_writer_name_is_required_when_user_is_unset` for the
    // non-JSON version of the same gap.
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
            "write",
            "cas_01K5GR7T2M9WPD0000000000AB",
            "--session",
            &sid,
            "--json",
        ])
        .env_remove("USER")
        .env("CASSETTE_DATA_DIR", &root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no prose on stderr under --json: {}",
        stderr(&out)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(v["code"], 2, "{v}");
    assert!(
        v["error"]
            .as_str()
            .expect("error string")
            .contains("--writer"),
        "the message must name the fix: {v}"
    );
}

#[test]
fn session_alias_on_an_unknown_id_emits_the_json_envelope() {
    // Spec decision 2 names `session`/`writer` explicitly alongside the
    // queue commands: --json must change every command's failure output,
    // not only QueueError's. This pins `session alias` specifically since
    // it is the one session command with its own usage-error case.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args([
            "session",
            "alias",
            "ses_01K5GQ2R8V3XQZ0000000000AB",
            "x",
            "--json",
        ])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no prose on stderr under --json: {}",
        stderr(&out)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(v["code"], 2, "{v}");
    assert!(
        v["error"]
            .as_str()
            .expect("error string")
            .contains("ses_01K5GQ2R8V3XQZ0000000000AB"),
        "{v}"
    );
}

#[test]
fn session_alias_on_an_unknown_id_without_json_still_prints_prose() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["session", "alias", "ses_01K5GQ2R8V3XQZ0000000000AB", "x"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "no JSON without --json");
    assert!(stderr(&out).contains("cassette:"), "{}", stderr(&out));
}

#[test]
fn writer_register_kind_mismatch_emits_the_json_envelope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let first = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));

    let out = Command::new(bin())
        .args([
            "writer", "register", "--name", "bot", "--kind", "human", "--json",
        ])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no prose on stderr under --json: {}",
        stderr(&out)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(v["code"], 2, "{v}");
    assert!(
        v["error"].as_str().expect("error string").contains("bot"),
        "{v}"
    );
}

#[test]
fn writer_register_kind_mismatch_without_json_still_prints_prose() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let first = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "agent"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));

    let out = Command::new(bin())
        .args(["writer", "register", "--name", "bot", "--kind", "human"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(out.stdout.is_empty(), "no JSON without --json");
    assert!(stderr(&out).contains("cassette:"), "{}", stderr(&out));
}

#[test]
fn malformed_config_toml_emits_the_json_envelope_on_every_command() {
    // `config::load_config()` runs before any subcommand dispatch, so a
    // broken config.toml used to print TOML-parser prose (plus "try
    // 'cassette --help'") to stderr and exit 2 with an empty stdout — even
    // under --json, on every command, not just `queue`. Spec decision 2
    // ("Any command invoked with --json emits {"error","code"} on failure")
    // and the README's identical claim make no exception for config load.
    let dir = tempfile::tempdir().expect("tempdir");
    let config_home = dir.path().join("xdg-config");
    std::fs::create_dir_all(config_home.join("cassette")).expect("mkdir");
    std::fs::write(
        config_home.join("cassette").join("config.toml"),
        "this is not valid toml [[[\n",
    )
    .expect("write config");
    let root = dir.path().join("store");

    let out = Command::new(bin())
        .args(["session", "new", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).is_empty(),
        "no prose on stderr under --json: {}",
        stderr(&out)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(v["code"], 2, "{v}");
    assert!(
        v["error"]
            .as_str()
            .expect("error string")
            .contains("invalid config"),
        "{v}"
    );
}

#[test]
fn malformed_config_toml_without_json_still_prints_prose() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_home = dir.path().join("xdg-config");
    std::fs::create_dir_all(config_home.join("cassette")).expect("mkdir");
    std::fs::write(
        config_home.join("cassette").join("config.toml"),
        "this is not valid toml [[[\n",
    )
    .expect("write config");
    let root = dir.path().join("store");

    let out = Command::new(bin())
        .args(["session", "new"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(out.stdout.is_empty(), "no JSON without --json");
    assert!(stderr(&out).contains("invalid config"), "{}", stderr(&out));
}

/// Spawn `queue write` with `body` piped to stdin, waiting for it to finish.
/// Shared by the `--side`/`--append`/`--replace` wiring tests below.
fn write_stdin(args: &[&str], root: &std::path::Path, body: &[u8]) -> Output {
    let mut child = Command::new(bin())
        .args(args)
        .env("CASSETTE_DATA_DIR", root)
        .env("USER", "joseph")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(body)
        .expect("write");
    child.wait_with_output().expect("wait")
}

#[test]
fn queue_write_defaults_to_side_a() {
    // `--side` defaults to `a` — the behaviour every caller had before
    // `--side` existed, and what `tests/lock.rs`'s cross-process cases rely
    // on staying true.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };

    let out = write_stdin(
        &["queue", "write", &cid, "--session", &sid],
        &root,
        b"front text\n",
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["queue", "show", &cid, "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let v: serde_json::Value = serde_json::from_slice(&show.stdout).expect("valid JSON");
    assert_eq!(v["side_a"].as_str().expect("side_a").trim(), "front text");
    assert_eq!(v["side_b"], "");
}

#[test]
fn queue_write_side_b_targets_side_b_and_leaves_side_a_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let first = write_stdin(
        &["queue", "write", &cid, "--session", &sid],
        &root,
        b"front text\n",
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let out = write_stdin(
        &["queue", "write", &cid, "--session", &sid, "--side", "b"],
        &root,
        b"back text\n",
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["queue", "show", &cid, "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    let v: serde_json::Value = serde_json::from_slice(&show.stdout).expect("valid JSON");
    assert_eq!(
        v["side_a"].as_str().expect("side_a").trim(),
        "front text",
        "writing --side b must not touch side A: {v}"
    );
    assert_eq!(v["side_b"].as_str().expect("side_b").trim(), "back text");
}

#[test]
fn queue_write_append_and_replace_are_mutually_exclusive() {
    // Neither a real session nor a real cassette is needed: clap's
    // `conflicts_with` rejects the combination during argument parsing,
    // before the command ever touches the store or reads stdin.
    let out = run(&[
        "queue",
        "write",
        "cas_000000000000000000000000NS",
        "--session",
        "ses_01K5GQ2R8V3XQZ0000000000AB",
        "--append",
        "--replace",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn an_unknown_side_value_exits_two() {
    // Same reasoning as the conflict test above: clap's `value_enum` rejects
    // an unrecognized `--side` before any store I/O happens.
    let out = run(&[
        "queue",
        "write",
        "cas_000000000000000000000000NS",
        "--session",
        "ses_01K5GQ2R8V3XQZ0000000000AB",
        "--side",
        "z",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn stats_and_find_read_a_session_written_through_the_store() {
    // Phase 5a: `stats` and `find` read only the session store, not the
    // legacy notes dir. A session created and written through the same
    // primitives the TUI now uses must show up in both commands.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let write = write_stdin(
        &["queue", "write", &cid, "--session", &sid],
        &root,
        b"five whole words right here\n",
    );
    assert_eq!(
        write.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );

    let stats = Command::new(bin())
        .args(["stats"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(stats.status.code(), Some(0), "{}", stderr(&stats));
    let stats_out = String::from_utf8_lossy(&stats.stdout);
    assert!(
        stats_out.contains("1 note") && stats_out.contains("5 words"),
        "{stats_out}"
    );

    let find = Command::new(bin())
        .args(["find"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(find.status.code(), Some(0), "{}", stderr(&find));
    let find_out = String::from_utf8_lossy(&find.stdout);
    assert!(find_out.contains(&sid), "{find_out}");
    assert!(find_out.contains("gratitude"), "{find_out}");
    assert!(
        find_out.contains("five whole words right here"),
        "{find_out}"
    );
}

/// The discovery→reopen loop, end to end: whatever `find` prints and
/// whatever it lets you search by must both lead back into the session.
///
/// `find`'s query used to be matched against the session id and the cassette
/// bodies only, so a row that printed `— gratitude` was missed by `cassette
/// find gratitude`, and the id every row printed was rejected by `resume`,
/// which matched aliases alone.
#[test]
fn find_matches_the_alias_and_topic_it_prints_and_resume_takes_the_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let aliased = Command::new(bin())
        .args(["session", "alias", &sid, "morning-pages"])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(aliased.status.code(), Some(0), "{}", stderr(&aliased));
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let write = write_stdin(
        &["queue", "write", &cid, "--session", &sid],
        &root,
        b"nothing here repeats the topic\n",
    );
    assert_eq!(write.status.code(), Some(0), "{}", stderr(&write));

    for query in ["gratitude", "morning-pages", sid.as_str()] {
        let out = Command::new(bin())
            .args(["find", query])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(&sid),
            "'cassette find {query}' must find the session whose row shows it: {text}"
        );
    }

    // And the id that listing prints is openable. Driving the TUI itself
    // needs a pty (`.claude/skills/verify`), so this asserts on the one
    // thing that can be seen without one: resolution happens before the
    // terminal is touched, so a rejected name exits 2 with "no session
    // named" and an accepted one gets as far as failing to enter raw mode
    // on a pipe.
    let opened = Command::new(bin())
        .args(["resume", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert!(
        !stderr(&opened).contains("no session named"),
        "the id `find` printed must resolve: {}",
        stderr(&opened)
    );
    assert_ne!(
        opened.status.code(),
        Some(2),
        "and it must not be a usage error: {}",
        stderr(&opened)
    );

    let unknown = Command::new(bin())
        .args(["resume", "ses_01K5GQ2R8V3XQZ0000000000AB"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(unknown.status.code(), Some(2), "{}", stderr(&unknown));
    assert!(
        stderr(&unknown).contains("no session named"),
        "{}",
        stderr(&unknown)
    );
}

/// `-o` prints this sitting to stdout and persists nothing, so it names no
/// store session — which used to mean `cassette -o resume` opened a blank
/// editor and silently dropped the subcommand the user typed.
#[test]
fn print_stdout_refuses_the_subcommands_that_name_a_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    for args in [
        vec!["-o", "resume"],
        vec!["-o", "today"],
        vec!["-o", "new", "somename"],
    ] {
        let out = Command::new(bin())
            .args(&args)
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("persists nothing"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    assert!(
        !root.exists(),
        "a refused combination must not create a store"
    );
}

/// `-T` seeds one cassette per topic into a *new* session. On a day that
/// already has one, `load_cassettes` replaces everything `apply_topics`
/// built, so the topics were silently discarded; `resume` + `-T` already
/// refuses rather than discard them, and `today` now matches.
#[test]
fn today_refuses_a_template_once_the_day_already_has_a_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let xdg = dir.path().join("xdg");
    std::fs::create_dir_all(xdg.join("cassette")).expect("mkdir");
    std::fs::write(
        xdg.join("cassette").join("config.toml"),
        "[templates]\nmorning = [\"gratitude\", \"priorities\"]\n",
    )
    .expect("config");

    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let aliased = Command::new(bin())
        .args(["session", "alias", &sid, &today])
        .env("CASSETTE_DATA_DIR", &root)
        .output()
        .expect("spawn");
    assert_eq!(aliased.status.code(), Some(0), "{}", stderr(&aliased));

    let out = Command::new(bin())
        .args(["-T", "morning", "today"])
        .env("CASSETTE_DATA_DIR", &root)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("already exists"),
        "the discarded topics must be reported, not dropped: {}",
        stderr(&out)
    );
}

#[cfg(unix)]
#[test]
fn a_cassette_written_behind_the_tuis_back_is_visible_to_the_next_reader() {
    // Not a TUI test: this pins the store-side contract live sync depends on
    // — that an external write is observable by re-reading, with the mtime
    // moving. The TUI-side merge is unit-tested in app.rs, and the two
    // together are what Task 6 drives through a pty.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin())
            .args(["session", "new"])
            .env("CASSETTE_DATA_DIR", &root)
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin())
            .args(["queue", "new", "shared", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "joseph")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let path = std::fs::read_dir(root.join("sessions").join(&sid).join("cassettes"))
        .expect("read cassettes dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains(&cid))
        .expect("the cassette file");
    let stamp = |p: &std::path::Path| {
        use std::os::unix::fs::MetadataExt as _;
        let m = std::fs::metadata(p).expect("stat");
        (m.ino(), m.len(), m.modified().expect("mtime"))
    };
    let before = stamp(&path);

    let mut child = Command::new(bin())
        .args(["queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"agent words\n")
        .expect("write");
    assert!(child.wait().expect("wait").success());

    let after = stamp(&path);
    assert_ne!(
        after, before,
        "the write must change what live sync watches (inode, length, mtime)"
    );
    let body = std::fs::read_to_string(&path).expect("read");
    assert!(body.contains("agent words"), "{body}");
}

#[test]
fn an_implicit_user_cannot_write_over_a_sticky_lock_but_an_explicit_one_can() {
    // An agent runs in the human's shell and inherits `$USER`, which names
    // the human's own registered identity. Only a name given explicitly —
    // `--writer` or `$CASSETTE_WRITER` — carries human authority.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let run_as = |args: &[&str], env: &[(&str, &str)], stdin: &[u8]| {
        let mut c = Command::new(bin());
        c.args(args)
            .env("CASSETTE_DATA_DIR", &root)
            .env_remove("CASSETTE_WRITER")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().expect("spawn");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin)
            .expect("stdin write");
        child.wait_with_output().expect("wait")
    };
    let sid = String::from_utf8_lossy(&run_as(&["session", "new"], &[], b"").stdout)
        .trim()
        .to_string();
    const ID: &str = "cas_01K5GR7T2M9WPD0000000000AB";
    let cassettes = root.join("sessions").join(&sid).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\n\
             locked_by: wri_01K5GQ00000000000000000002\ncreated_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\n\
             updated_at: 2026-09-14T09:25:57Z\n---\n\n## Side A\n\nhello\n"
        ),
    )
    .expect("cassette");
    let reg = run_as(
        &["writer", "register", "--name", "joseph", "--kind", "human"],
        &[],
        b"",
    );
    assert_eq!(reg.status.code(), Some(0), "{}", stderr(&reg));

    let implicit = run_as(
        &["queue", "write", ID, "--session", &sid],
        &[("USER", "joseph")],
        b"x\n",
    );
    assert_eq!(implicit.status.code(), Some(4), "{}", stderr(&implicit));
    assert!(
        stderr(&implicit).contains("--writer joseph"),
        "{}",
        stderr(&implicit)
    );

    let via_env = run_as(
        &["queue", "write", ID, "--session", &sid],
        &[("USER", "joseph"), ("CASSETTE_WRITER", "joseph")],
        b"y\n",
    );
    assert_eq!(
        via_env.status.code(),
        Some(0),
        "$CASSETTE_WRITER is explicit: {}",
        stderr(&via_env)
    );

    let explicit = run_as(
        &[
            "--writer",
            "joseph",
            "queue",
            "write",
            ID,
            "--session",
            &sid,
        ],
        &[("USER", "joseph")],
        b"z\n",
    );
    assert_eq!(explicit.status.code(), Some(0), "{}", stderr(&explicit));

    let unlock_implicit = run_as(
        &["queue", "unlock", ID, "--session", &sid],
        &[("USER", "joseph")],
        b"",
    );
    assert_eq!(
        unlock_implicit.status.code(),
        Some(2),
        "{}",
        stderr(&unlock_implicit)
    );
}

#[test]
fn queue_topic_sets_changes_and_clears_a_topic_without_renaming_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let cmd = |args: &[&str]| {
        Command::new(bin())
            .args(args)
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .env_remove("CASSETTE_WRITER")
            .output()
            .expect("spawn")
    };
    let sid = String::from_utf8_lossy(&cmd(&["session", "new"]).stdout)
        .trim()
        .to_string();
    let cid = String::from_utf8_lossy(&cmd(&["queue", "new", "draft", "--session", &sid]).stdout)
        .trim()
        .to_string();
    let files = || -> Vec<_> {
        let mut v: Vec<_> = std::fs::read_dir(root.join("sessions").join(&sid).join("cassettes"))
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .filter(|n| n.to_string_lossy().ends_with(".md"))
            .collect();
        v.sort();
        v
    };
    let names_before = files();

    let set = cmd(&[
        "queue",
        "topic",
        &cid,
        "--session",
        &sid,
        "what it is now about",
    ]);
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    let shown = cmd(&["queue", "show", &cid, "--session", &sid, "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("json");
    assert_eq!(v["topic"], "what it is now about");
    assert_eq!(files(), names_before, "the slug is frozen at creation");

    let dash = cmd(&["queue", "topic", &cid, "--session", &sid, "--", "-draft"]);
    assert_eq!(dash.status.code(), Some(0), "{}", stderr(&dash));
    let shown = cmd(&["queue", "show", &cid, "--session", &sid, "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("json");
    assert_eq!(v["topic"], "-draft", "`--` lets a topic start with a dash");

    let cleared = cmd(&["queue", "topic", &cid, "--session", &sid, ""]);
    assert_eq!(cleared.status.code(), Some(0), "{}", stderr(&cleared));
    let shown = cmd(&["queue", "show", &cid, "--session", &sid, "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("json");
    assert!(v["topic"].is_null(), "{v}");
}

#[test]
fn queue_topic_rejects_a_newline_with_exit_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let cmd = |args: &[&str]| {
        Command::new(bin())
            .args(args)
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn")
    };
    let sid = String::from_utf8_lossy(&cmd(&["session", "new"]).stdout)
        .trim()
        .to_string();
    let cid = String::from_utf8_lossy(&cmd(&["queue", "new", "draft", "--session", &sid]).stdout)
        .trim()
        .to_string();
    let out = cmd(&["queue", "topic", &cid, "--session", &sid, "two\nlines"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn completions_cover_the_queue_surface_for_each_shell() {
    for shell in ["bash", "zsh", "fish"] {
        let out = Command::new(bin())
            .args(["completions", shell])
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{shell}: {}", stderr(&out));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("queue") && text.contains("write"), "{shell}");
    }
}

#[test]
fn completions_reject_an_unknown_shell() {
    let out = Command::new(bin())
        .args(["completions", "nope"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn generators_ignore_config_and_never_create_the_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = dir.path().join("cfg");
    std::fs::create_dir_all(cfg.join("cassette")).expect("mkdir");
    std::fs::write(cfg.join("cassette/config.toml"), "this is = = not toml").expect("write");
    let store = dir.path().join("never");
    for args in [&["completions", "zsh"][..], &["man"][..]] {
        let out = Command::new(bin())
            .args(args)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("CASSETTE_DATA_DIR", &store)
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    }
    assert!(!store.exists(), "no data dir is created");
}

#[test]
fn man_prints_the_top_page_and_out_dir_writes_one_per_subcommand() {
    let out = Command::new(bin()).arg("man").output().expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let page = String::from_utf8_lossy(&out.stdout);
    assert!(
        page.lines()
            .any(|l| l.starts_with(".TH") && l.contains("cassette")),
        "{page}"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("man1");
    let out = Command::new(bin())
        .args(["man", "--out-dir"])
        .arg(&target)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(target.join("cassette.1").is_file());
    assert!(target.join("cassette-queue-write.1").is_file());
}

#[test]
fn resume_with_a_cassette_id_is_simply_no_such_session() {
    // `resume` takes free text (alias first), so a wrong-kind id is not a
    // session: the ordinary "no session named" exit, no WrongKind message.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .args(["resume", "cas_01K5GQ2R8VXM3T0000000000AB"])
        .env("CASSETTE_DATA_DIR", dir.path().join("store"))
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no session named"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_session_id_where_a_cassette_id_belongs_is_a_usage_error_naming_both() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let cmd = |args: &[&str]| {
        Command::new(bin())
            .args(args)
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "tester")
            .output()
            .expect("spawn")
    };
    let sid = String::from_utf8_lossy(&cmd(&["session", "new"]).stdout)
        .trim()
        .to_string();
    let cid = String::from_utf8_lossy(&cmd(&["queue", "new", "x", "--session", &sid]).stdout)
        .trim()
        .to_string();

    let out = cmd(&["queue", "show", &sid, "--session", &sid]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("is a session id; <ID> takes a cassette id (cas_…)"),
        "{}",
        stderr(&out)
    );

    let out = cmd(&["queue", "move", &cid, "--session", &sid, "--before", &sid]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--before takes a cassette id"),
        "{}",
        stderr(&out)
    );

    let out = cmd(&["queue", "list", "--session", &cid]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("is a cassette id; --session takes a session id (ses_…)"),
        "{}",
        stderr(&out)
    );
}
