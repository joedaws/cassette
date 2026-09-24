# `queue topic` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `cassette queue topic <ID> --session <ID> "<TOPIC>"`, so any writer can set, change or clear a cassette's topic without the TUI.

**Architecture:** One new function, `queue::edit::retopic`, shaped exactly like `queue::edit::close` (resolve writer → `Store::lock` → read through the guard → sticky check → write through the guard). A new `QueueAction::Topic`/`QueueCmd::Topic` pair in `cli.rs`, dispatched in `main.rs` next to `Close`. No TUI change: the existing re-read-on-acquire and `sync_external_writes` already give the right precedence, and Task 2 pins that with a test.

**Tech Stack:** Rust, clap v4 derive, the crate's own `store` module; tests via `cargo test` (unit tests in-module, CLI tests in `tests/cli.rs` spawning the built binary).

**Spec:** `docs/superpowers/specs/2026-09-24-queue-topic-design.md`

## Global Constraints

- Exit codes: 0 ok, 2 usage, 3 busy, 4 sticky, 1 I/O — via the existing `QueueError` variants; add none.
- Blank / whitespace-only topic clears it (`topic: None`); the value is trimmed.
- A topic containing `\n` is `QueueError::Usage` and nothing is written.
- `last_writer` is **not** modified; `updated_at` **is**, except on a no-op.
- The file name (slug) is never changed.
- Sticky rule is `close_permitted`'s: agent + any `locked_by` → `Sticky`; humans never blocked.
- `cargo clippy --all-targets -- -D warnings` must pass at the end of every task — so the function and its CLI caller land in the same task.

## Review Focus

- **A topic beginning with `-`** (e.g. `"-draft"`) — clap will read it as a flag. Expect it to be accepted as a topic when given after `--` (`queue topic ID --session S -- -draft`); Task 1 adds a CLI test for the `--` form so the escape hatch is known to work.
- **Unicode / emoji topics** — must round-trip exactly through frontmatter. Task 1's unit test uses `"café ☕"`.
- **Retitling a closed cassette** — must succeed and keep `status: closed`. Task 1 unit test.
- **Same topic twice** — must not bump `updated_at` (would wake every TUI's sync). Task 1 unit test.
- **Unfocused cassette retitled while the TUI runs, then focused and typed in** — the CLI's topic must survive the TUI's next flush. Task 2.

---

### Task 1: `queue topic` end to end

**Files:**
- Modify: `src/queue/edit.rs` (new fn `retopic` + `topic_value` after `close`, ~line 308; tests in the `mod tests` block)
- Modify: `src/cli.rs` (`QueueCmd` enum ~line 69, `QueueCmd::session` ~line 101, `QueueAction` ~line 307, `into_args` lowering ~line 540)
- Modify: `src/main.rs` (dispatch next to `cli::QueueCmd::Close`, ~line 264)
- Test: `src/queue/edit.rs` `mod tests`, `tests/cli.rs`

**Interfaces:**
- Produces: `pub fn retopic(store: &Store, session: &str, id: &str, topic: &str, who_name: &str, source: WriterSource) -> Result<(), QueueError>`; `cli::QueueCmd::Topic { id: String, session: String, topic: String }`.

- [ ] **Step 1: Write the failing unit tests** (append inside `mod tests` in `src/queue/edit.rs`; `meta`, `new_session` helpers already exist there)

```rust
    #[test]
    fn topic_value_trims_and_blank_clears() {
        assert_eq!(topic_value("  morning  ").unwrap(), Some("morning".to_string()));
        assert_eq!(topic_value("   ").unwrap(), None);
        assert_eq!(topic_value("").unwrap(), None);
        assert!(matches!(topic_value("a\nb"), Err(QueueError::Usage(_))));
    }

    #[test]
    fn retopic_sets_the_topic_and_touches_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let id = "aaa00000000000000000000000";
        store
            .add_cassette(&sid, &meta(id, 10, Status::Closed), "## Side A\n\nhello\n")
            .expect("add");
        let before = store.scan_session(&sid).expect("scan").cassettes.remove(0);

        retopic(&store, &sid, id, "café ☕", "tester", WriterSource::Env).expect("retopic");

        let after = store.scan_session(&sid).expect("scan").cassettes.remove(0);
        assert_eq!(after.meta.topic.as_deref(), Some("café ☕"));
        assert_eq!(after.meta.status, Status::Closed, "a closed cassette stays closed");
        assert_eq!(after.meta.last_writer, before.meta.last_writer, "retitling is not a turn");
        assert_eq!(after.body, before.body);
        assert_eq!(after.path, before.path, "the file is never renamed");
    }

    #[test]
    fn retopic_with_a_blank_topic_clears_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let id = "aaa00000000000000000000000";
        store.add_cassette(&sid, &meta(id, 10, Status::Open), "").expect("add");

        retopic(&store, &sid, id, "  ", "tester", WriterSource::Env).expect("retopic");

        let c = store.scan_session(&sid).expect("scan").cassettes.remove(0);
        assert_eq!(c.meta.topic, None);
    }

    #[test]
    fn retopic_to_the_same_topic_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let id = "aaa00000000000000000000000";
        // meta() stamps updated_at 2026-09-15T09:00:00Z and topic "topic-<id>".
        store.add_cassette(&sid, &meta(id, 10, Status::Open), "").expect("add");

        retopic(&store, &sid, id, &format!("topic-{id}"), "tester", WriterSource::Env)
            .expect("retopic");

        let c = store.scan_session(&sid).expect("scan").cassettes.remove(0);
        assert_eq!(c.meta.updated_at, "2026-09-15T09:00:00Z", "a no-op must not bump updated_at");
    }

    #[test]
    fn retopic_refuses_a_busy_cassette() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let id = "aaa00000000000000000000000";
        store.add_cassette(&sid, &meta(id, 10, Status::Open), "").expect("add");
        let _held = store
            .lock(&sid, id, &Attribution::for_now("writer-1", "joseph"))
            .expect("hold it");

        match retopic(&store, &sid, id, "x", "tester", WriterSource::Env) {
            Err(QueueError::Busy(_)) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn retopic_denies_an_agent_over_a_sticky_lock_but_allows_a_human() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = new_session(&store);
        let id = "aaa00000000000000000000000";
        let mut m = meta(id, 10, Status::Open);
        m.locked_by = Some("01WRITER0000000000000000AB".to_string());
        store.add_cassette(&sid, &m, "").expect("add");
        store.ensure_writer("bot", Kind::Agent).expect("agent");
        store.ensure_writer("joseph", Kind::Human).expect("human");

        match retopic(&store, &sid, id, "x", "bot", WriterSource::Flag) {
            Err(QueueError::Sticky(_)) => {}
            other => panic!("expected Sticky, got {other:?}"),
        }
        retopic(&store, &sid, id, "x", "joseph", WriterSource::Flag).expect("human may");
    }
```

Note: if `StoredCassette.path` is not visible to tests in this module, drop that one assertion — the CLI test in Step 5 covers the file name.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --bin cassette retopic topic_value`
Expected: compile error — `cannot find function 'retopic'` / `'topic_value'`.

- [ ] **Step 3: Implement** (in `src/queue/edit.rs`, directly after `close`)

```rust
/// Normalise `queue topic`'s argument the way the TUI's topic prompt does:
/// trimmed, and blank means "clear". A newline is rejected rather than
/// flattened — `meta::one_line` would otherwise rewrite it on the way to
/// disk and the caller would never learn its title was altered.
fn topic_value(topic: &str) -> Result<Option<String>, QueueError> {
    if topic.contains('\n') {
        return Err(QueueError::Usage(
            "topic must not contain a newline".to_string(),
        ));
    }
    let trimmed = topic.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

/// `cassette queue topic <ID> --session <ID> <TOPIC>`: set, change or clear
/// a cassette's topic.
///
/// `close`'s order of operations, for `close`'s reasons: validate, resolve
/// the writer (keeping its `Kind`), take the lock (`Busy` for everyone),
/// read through the guard, then `close_permitted` — a sticky claim covers
/// the cassette's title as much as its body.
///
/// Deliberately leaves `last_writer` alone: it is the "whose turn ended
/// last" hint `waiting_on` derives from, and retitling is housekeeping,
/// not a turn. Setting the topic it already has writes nothing, so a
/// repeated call does not bump `updated_at` and wake every reader's sync.
/// The file name's slug is frozen at creation and is not renamed.
pub fn retopic(
    store: &Store,
    session: &str,
    id: &str,
    topic: &str,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> {
    let topic = topic_value(topic)?;

    let (writer, kind) = match source {
        WriterSource::Env => store
            .resolve_writer(who_name)
            .map_err(resolve_error_to_queue_error)?,
        WriterSource::Flag => store
            .require_writer(who_name)
            .map_err(require_error_to_queue_error)?,
    };
    let who = Attribution::for_now(&writer, who_name);

    let guard = store
        .lock(session, id, &who)
        .map_err(|e| lock_error_to_queue_error(id, e))?;

    let current = guard
        .read()
        .map_err(|e| QueueError::Io(format!("cannot read '{id}': {e}")))?;
    close_permitted(kind, current.meta.locked_by.as_deref())?;

    if current.meta.topic == topic {
        return Ok(());
    }

    let mut m = current.meta;
    m.topic = topic;
    m.updated_at = crate::store::meta::now_utc();

    guard
        .write(&m, &current.body)
        .map_err(|e| QueueError::Io(format!("cannot write '{id}': {e}")))
}
```

`close_permitted`'s error text says "only a human may close it". Generalise it rather than duplicating the function: change its message to `"cassette is locked by '{holder}' — only a human may change it"` and update any test asserting the old wording (`grep -n "may close it" src tests`).

- [ ] **Step 4: Wire the CLI.** In `src/cli.rs`:

`QueueCmd` (after `Reopen`):
```rust
    Topic {
        id: String,
        session: String,
        topic: String,
    },
```
`QueueCmd::session`: add `| QueueCmd::Topic { session, .. }` to the or-pattern.

`QueueAction` (after `Reopen`):
```rust
    /// set a cassette's topic (a blank topic clears it)
    Topic {
        #[arg(value_name = "ID")]
        id: String,
        /// session the cassette lives in
        #[arg(long, value_name = "ID")]
        session: String,
        /// the new topic; "" clears it
        #[arg(value_name = "TOPIC")]
        topic: String,
    },
```
`into_args` lowering (after the `Reopen` arm):
```rust
                    QueueAction::Topic { id, session, topic } => {
                        QueueCmd::Topic { id, session, topic }
                    }
```

In `src/main.rs`, after the `cli::QueueCmd::Reopen` arm:
```rust
            // `topic` attributes the change and, like `close`, needs the
            // writer's `Kind` for the sticky-lock boundary.
            cli::QueueCmd::Topic { id, session, topic } => {
                let (who_name, writer_source) = match resolve_writer_name(args.writer.as_deref()) {
                    Ok(w) => w,
                    Err(msg) => exit_usage(&msg, args.json),
                };
                match queue::edit::retopic(&store, session, id, topic, &who_name, writer_source) {
                    Ok(()) => exit_queue_ok(None),
                    Err(e) => exit_queue_err(&e, args.json),
                }
            }
```
If `cli.rs` has a unit test enumerating queue subcommands for `into_args`, add a `Topic` case there in the same style.

- [ ] **Step 5: Add CLI tests** (`tests/cli.rs`, beside the `queue_close_*` tests; `bin()`/`stderr()` helpers exist)

```rust
#[test]
fn queue_topic_sets_changes_and_clears_a_topic_without_renaming_the_file() {
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
    let sid = String::from_utf8_lossy(&cmd(&["session", "new"]).stdout).trim().to_string();
    let cid = String::from_utf8_lossy(&cmd(&["queue", "new", "draft", "--session", &sid]).stdout)
        .trim()
        .to_string();
    let files = || -> Vec<_> {
        std::fs::read_dir(root.join("sessions").join(&sid).join("cassettes"))
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .filter(|n| n.to_string_lossy().ends_with(".md"))
            .collect()
    };
    let names_before = files();

    let set = cmd(&["queue", "topic", &cid, "--session", &sid, "what it is now about"]);
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    let shown = cmd(&["queue", "show", &cid, "--session", &sid]);
    assert!(String::from_utf8_lossy(&shown.stdout).contains("what it is now about"));
    assert_eq!(files(), names_before, "the slug is frozen at creation");

    let dash = cmd(&["queue", "topic", &cid, "--session", &sid, "--", "-draft"]);
    assert_eq!(dash.status.code(), Some(0), "{}", stderr(&dash));

    let cleared = cmd(&["queue", "topic", &cid, "--session", &sid, ""]);
    assert_eq!(cleared.status.code(), Some(0), "{}", stderr(&cleared));
    let json = cmd(&["--json", "queue", "show", &cid, "--session", &sid]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
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
    let sid = String::from_utf8_lossy(&cmd(&["session", "new"]).stdout).trim().to_string();
    let cid = String::from_utf8_lossy(&cmd(&["queue", "new", "draft", "--session", &sid]).stdout)
        .trim()
        .to_string();
    let out = cmd(&["queue", "topic", &cid, "--session", &sid, "two\nlines"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}
```

Check the `--json` flag's position and `CassetteView`'s topic key against `queue_show`'s existing JSON test before relying on `v["topic"]`; adjust the key if it differs.

- [ ] **Step 6: Run everything**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/queue/edit.rs src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat: queue topic sets or clears a cassette's topic from the CLI"
```

---

### Task 2: Pin TUI precedence for a CLI retitle

**Files:**
- Test: `src/session_writer.rs` `mod tests` (beside `regaining_focus_re_reads_a_cassette_another_writer_changed`, ~line 965)

**Interfaces:**
- Consumes: the `queue topic` subcommand from Task 1 (via `bin_path()`, which rebuilds the binary); existing helpers `fixture(&store, n)`, `SessionWriter::open`, `acquire`, `flush_focused`.

- [ ] **Step 1: Write the test**

```rust
    #[test]
    fn a_topic_set_from_the_cli_survives_the_tui_refocusing_and_flushing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let store = Store::new(root.clone());
        let (mut app, session) = fixture(&store, 2);
        let mut w = SessionWriter::open(&store, &session, true, "w", "w");

        w.acquire(&mut app, 0).expect("acquire 0");
        app.focus_idx = 0;
        app.modify_focused(|c| c.topic = Some("old title".to_string()));
        let zero_id = app.cassettes[0].id.clone();
        w.acquire(&mut app, 1).expect("focus 1 — flushes 0 with 'old title'");
        app.focus_idx = 1;

        let out = std::process::Command::new(bin_path())
            .args(["queue", "topic", &zero_id, "--session", &session, "new title"])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", "agent")
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{out:?}");

        w.acquire(&mut app, 0).expect("focus 0 again");
        app.focus_idx = 0;
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("new title"), "acquire re-reads");
        app.modify_focused(|c| c.insert_str("!"));
        w.flush_focused(&mut app).expect("flush");

        let scan = store.scan_session(&session).expect("scan");
        let zero = scan.cassettes.iter().find(|c| c.meta.id == zero_id).expect("0");
        assert_eq!(zero.meta.topic.as_deref(), Some("new title"));
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test --bin cassette a_topic_set_from_the_cli_survives`
Expected: PASS without production changes (the spec's claim). If it fails, stop: the spec's "Precedence with the TUI" section is wrong, and `refresh_from_disk`'s topic comparison is where to look — fix there and update the spec in the same commit.

- [ ] **Step 3: Commit**

```bash
git add src/session_writer.rs
git commit -m "test: a CLI retitle survives the TUI refocusing and flushing"
```

---

### Task 3: Docs

**Files:**
- Modify: `CLAUDE.md` (Commands block; the `src/queue/{mod,view,write,edit}.rs` paragraph)
- Modify: `.claude/skills/cassette-session/SKILL.md`
- Modify: `docs/follow-up.md` (remove the "`queue topic`" section)

- [ ] **Step 1:** In `CLAUDE.md`'s Commands block, after the `queue lock` line add:
```
cargo run -- queue topic <ID> --session <ID> "new title"  # set a cassette's topic ("" clears it)
```
In the `edit.rs holds …` sentence add: "`retopic` (`queue topic`: `close`'s lock-then-`close_permitted` shape; blank clears, a newline is `Usage`, `last_writer` untouched because a retitle is not a turn, and setting the current topic writes nothing)".

- [ ] **Step 2:** In `SKILL.md`, next to the `queue move` guidance, add: "**Retitle** a cassette whose subject has drifted — including an untitled one the human created — with `cassette queue topic <id> --session <ID> \"<topic>\"`. It does not change whose turn it is."

- [ ] **Step 3:** Delete the `## \`queue topic\` — set a cassette's topic from the CLI` section from `docs/follow-up.md`.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md .claude/skills/cassette-session/SKILL.md docs/follow-up.md
git commit -m "docs: document queue topic and retire its follow-up entry"
```
