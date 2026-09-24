# Implicit Writer Authority Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A queue command whose writer came from `$USER` is attributed to that writer but treated as `Kind::Agent` for every permission check, so an agent that forgets `--writer` cannot use human authority.

**Architecture:** A single `queue::resolve_acting` returns an `Acting { id, authority, registered, source }` and replaces the seven copied `match source { Env => resolve_writer, Flag => require_writer }` blocks in `src/queue/{write,edit}.rs`. The permission helpers (`write_permitted`, `close_permitted`, the `lock`/`unlock` human-only gates) read `acting.authority` and append `acting.hint(name)` to their refusal text. `writer whoami` reports the downgrade. The TUI (`main.rs` `resolve_tui_writer`) is deliberately untouched.

**Tech Stack:** Rust; tests in-module and in `tests/cli.rs` (spawn the built binary with `CASSETTE_DATA_DIR` and `USER` set).

**Spec:** `docs/superpowers/specs/2026-09-24-implicit-writer-authority-design.md`

## Global Constraints

- `WriterSource::Env` ⇒ `authority = Kind::Agent`, always. `WriterSource::Flag` ⇒ `authority` = registered kind.
- Attribution (`id`, lock `Attribution`, `last_writer`) always uses the registered identity.
- No change to exit codes, to `--json` wire shapes, or to the TUI.
- Hint text, verbatim: ` — '$USER' alone does not carry human authority; pass --writer <name> to act as yourself` (with `<name>` substituted), appended only when `source == Env` and the registered kind is `Human`.
- `cargo clippy --all-targets -- -D warnings` green at the end of every task.
- If the `queue topic` plan (`2026-09-24-queue-topic.md`) has already landed, `retopic` is an eighth call site and is converted in Task 1 too.

## Review Focus

- **Human at their own shell, own sticky lock, no `--writer`** — now exits 4. The message must tell them to pass `--writer <their name>`; Task 1's CLI test asserts the hint text.
- **Registered agent whose name equals `$USER`** (unusual but possible) — `Env` with registered `Agent`: must behave exactly as before, with no hint. Task 1 unit test.
- **Unregistered `$USER` on a fresh store** — still bootstraps as human (attribution) but has agent authority. Task 1 unit test.
- **`$CASSETTE_WRITER` set** — remains `Flag`, keeps human authority. Task 1 CLI test.
- **TUI launched with only `$USER`** — still attributes the session to `$USER`. Covered by existing `session_writer` tests; Task 1 Step 6 runs the full suite.

---

### Task 1: `resolve_acting` and the downgrade

**Files:**
- Modify: `src/queue/mod.rs` (add `Acting`, `resolve_acting`; near `WriterSource`, ~line 58–74)
- Modify: `src/queue/write.rs` (delete private `resolve` ~line 104–119; `write_permitted` ~line 48; its call ~line 187)
- Modify: `src/queue/edit.rs` (`new` ~80, `close_permitted` ~222 and `close` ~275, `reopen` ~327, `lock` ~393, `unlock` ~452, `move_cassette` ~744)
- Test: `src/queue/mod.rs` `#[cfg(test)]`, `tests/cli.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Acting { pub id: String, pub authority: Kind, pub registered: Kind, pub source: WriterSource }
  impl Acting { pub fn hint(&self, name: &str) -> String }
  pub fn resolve_acting(store: &Store, name: &str, source: WriterSource) -> Result<Acting, QueueError>
  ```

- [ ] **Step 1: Failing unit tests** (add a `#[cfg(test)] mod tests` to `src/queue/mod.rs` if none exists)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::writers::Kind;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = Store::new(dir.path().to_path_buf());
        (dir, s)
    }

    #[test]
    fn an_implicit_human_is_attributed_as_itself_but_has_agent_authority() {
        let (_d, s) = store();
        let id = s.ensure_writer("joseph", Kind::Human).expect("register");
        let a = resolve_acting(&s, "joseph", WriterSource::Env).expect("resolve");
        assert_eq!(a.id, id, "attribution is the registered identity");
        assert_eq!(a.registered, Kind::Human);
        assert_eq!(a.authority, Kind::Agent, "implicit identity carries no human authority");
        assert!(a.hint("joseph").contains("--writer joseph"), "{}", a.hint("joseph"));
    }

    #[test]
    fn an_explicit_human_keeps_human_authority_and_no_hint() {
        let (_d, s) = store();
        s.ensure_writer("joseph", Kind::Human).expect("register");
        let a = resolve_acting(&s, "joseph", WriterSource::Flag).expect("resolve");
        assert_eq!(a.authority, Kind::Human);
        assert_eq!(a.hint("joseph"), "");
    }

    #[test]
    fn an_implicit_agent_is_unchanged_and_gets_no_hint() {
        let (_d, s) = store();
        s.ensure_writer("bot", Kind::Agent).expect("register");
        let a = resolve_acting(&s, "bot", WriterSource::Env).expect("resolve");
        assert_eq!((a.registered, a.authority), (Kind::Agent, Kind::Agent));
        assert_eq!(a.hint("bot"), "");
    }

    #[test]
    fn an_unregistered_user_still_bootstraps_as_human_but_acts_as_agent() {
        let (_d, s) = store();
        let a = resolve_acting(&s, "newcomer", WriterSource::Env).expect("resolve");
        assert_eq!(a.registered, Kind::Human);
        assert_eq!(a.authority, Kind::Agent);
    }

    #[test]
    fn an_unknown_explicit_writer_is_still_a_usage_error() {
        let (_d, s) = store();
        assert!(matches!(
            resolve_acting(&s, "typo", WriterSource::Flag),
            Err(QueueError::Usage(_))
        ));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --bin cassette queue::tests`
Expected: compile error, `resolve_acting` not found.

- [ ] **Step 3: Implement in `src/queue/mod.rs`** (below `WriterSource`; add `use crate::store::writers::Kind;` if not imported)

```rust
/// Who is acting, split into the two things the old `(id, kind)` pair
/// conflated: **attribution** (`id` — always the registered identity) and
/// **authority** (`authority` — the kind permission checks read).
///
/// Identity may be implicit; authority may not. A name that came from
/// `$USER` is attributed as itself but acts as an agent, because an agent
/// runs in the human's shell and inherits the human's `$USER` — the one
/// thing that differs between the two is whether the caller *said* who it
/// is. See `docs/superpowers/specs/2026-09-24-implicit-writer-authority-design.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acting {
    pub id: String,
    pub authority: Kind,
    pub registered: Kind,
    pub source: WriterSource,
}

impl Acting {
    /// Suffix for a refusal the downgrade caused, so the human who hits it
    /// at their own shell learns the fix. Empty when nothing was downgraded.
    pub fn hint(&self, name: &str) -> String {
        if self.source == WriterSource::Env && self.registered == Kind::Human {
            format!(
                " — '$USER' alone does not carry human authority; pass --writer {name} to act as yourself"
            )
        } else {
            String::new()
        }
    }
}

/// The one place a queue command turns a writer name into an `Acting`.
/// Every mutating command calls this; none re-derives the `Env`/`Flag`
/// split, so a new command cannot forget the downgrade.
pub fn resolve_acting(
    store: &Store,
    name: &str,
    source: WriterSource,
) -> Result<Acting, QueueError> {
    let (id, registered) = match source {
        WriterSource::Env => store
            .resolve_writer(name)
            .map_err(resolve_error_to_queue_error)?,
        WriterSource::Flag => store
            .require_writer(name)
            .map_err(require_error_to_queue_error)?,
    };
    let authority = match source {
        WriterSource::Env => Kind::Agent,
        WriterSource::Flag => registered,
    };
    Ok(Acting { id, authority, registered, source })
}
```

- [ ] **Step 4: Convert every call site.** Each of the seven blocks

```rust
    let (writer, kind) = match source {
        WriterSource::Env => store.resolve_writer(who_name).map_err(resolve_error_to_queue_error)?,
        WriterSource::Flag => store.require_writer(who_name).map_err(require_error_to_queue_error)?,
    };
```

becomes

```rust
    let acting = resolve_acting(store, who_name, source)?;
    let writer = acting.id.clone();
```

and every later use of `kind` becomes `acting.authority`. Per site:

- `write.rs`: delete the private `fn resolve`; its callers (`write`, `write_body`) use `resolve_acting`. Change `write_permitted`'s signature to `fn write_permitted(store: &Store, acting: &Acting, who_name: &str, locked_by: Option<&str>)`; match on `(acting.authority, locked_by)` and append `acting.hint(who_name)` to the `Sticky` message.
- `edit.rs` `close_permitted`: signature `fn close_permitted(acting: &Acting, who_name: &str, locked_by: Option<&str>)`, same treatment. Update its unit test (~line 1049) to build `Acting` values:
  ```rust
  let agent = Acting { id: "a".into(), authority: Kind::Agent, registered: Kind::Agent, source: WriterSource::Flag };
  let human = Acting { id: "h".into(), authority: Kind::Human, registered: Kind::Human, source: WriterSource::Flag };
  assert!(matches!(close_permitted(&agent, "bot", Some("01WRITER")), Err(QueueError::Sticky(_))));
  assert!(close_permitted(&human, "joseph", Some("01WRITER")).is_ok());
  assert!(close_permitted(&agent, "bot", None).is_ok());
  ```
- `lock`/`unlock`: `if acting.authority != Kind::Human { return Err(QueueError::Usage(format!("queue lock is human-only — an agent may not set a sticky lock{}", acting.hint(who_name)))); }` (and the `unlock` wording).
- `new`, `reopen`, `move_cassette`: only `writer` is used; drop the `_kind` binding.
- `use` lines: import `resolve_acting`/`Acting` from `super`; drop `resolve_error_to_queue_error`/`require_error_to_queue_error` imports that become unused (clippy will say which).

Then check: `grep -n "WriterSource::Env =>" src/queue` must print only the line inside `resolve_acting`.

- [ ] **Step 5: CLI tests** (`tests/cli.rs`, beside `queue_close_exits_four_for_an_agent_over_a_sticky_lock_but_a_human_may_close_it`, reusing its hand-written sticky fixture)

```rust
#[test]
fn an_implicit_user_cannot_write_over_a_sticky_lock_but_an_explicit_one_can() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let run_as = |args: &[&str], env: &[(&str, &str)], stdin: &[u8]| {
        use std::io::Write as _;
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
        child.stdin.take().expect("stdin").write_all(stdin).expect("stdin write");
        child.wait_with_output().expect("wait")
    };
    let sid = String::from_utf8_lossy(&run_as(&["session", "new"], &[], b"").stdout)
        .trim()
        .to_string();
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
    let reg = run_as(&["writer", "register", "--name", "joseph", "--kind", "human"], &[], b"");
    assert_eq!(reg.status.code(), Some(0), "{}", stderr(&reg));

    let implicit = run_as(&["queue", "write", ID, "--session", &sid], &[("USER", "joseph")], b"x\n");
    assert_eq!(implicit.status.code(), Some(4), "{}", stderr(&implicit));
    assert!(stderr(&implicit).contains("--writer joseph"), "{}", stderr(&implicit));

    let via_env = run_as(
        &["queue", "write", ID, "--session", &sid],
        &[("USER", "joseph"), ("CASSETTE_WRITER", "joseph")],
        b"y\n",
    );
    assert_eq!(via_env.status.code(), Some(0), "$CASSETTE_WRITER is explicit: {}", stderr(&via_env));

    let explicit = run_as(
        &["--writer", "joseph", "queue", "write", ID, "--session", &sid],
        &[("USER", "joseph")],
        b"z\n",
    );
    assert_eq!(explicit.status.code(), Some(0), "{}", stderr(&explicit));

    let lock_implicit = run_as(&["queue", "unlock", ID, "--session", &sid], &[("USER", "joseph")], b"");
    assert_eq!(lock_implicit.status.code(), Some(2), "{}", stderr(&lock_implicit));
}
```

Also update any existing CLI test that relied on `$USER` alone to write over, close over, `lock` or `unlock` a sticky cassette: `grep -n '"lock"\|"unlock"' tests/cli.rs tests/lock.rs` and give each human-acting invocation `--writer <name>` after registering that name as human. Those edits are the behaviour change landing; note each in the commit message.

- [ ] **Step 6: Run the suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add src/queue tests
git commit -m "fix: a writer taken from \$USER is attributed but carries no human authority"
```

---

### Task 2: `writer whoami` reports the downgrade

**Files:**
- Modify: `src/writer.rs` (`render_whoami` ~line 52, `whoami` ~59, tests)
- Modify: `src/main.rs` (`WriterCmd::Whoami` arm ~line 1686: pass the `WriterSource` instead of discarding it)

**Interfaces:**
- Consumes: `queue::WriterSource`.
- Produces: `pub fn render_whoami(all: &Writers, name: &str, source: WriterSource) -> String`, `pub fn whoami(store: &Store, name: &str, source: WriterSource) -> Result<String, String>`.

- [ ] **Step 1: Failing tests** (in `src/writer.rs` tests; `writers()` fixture has `alice` human, `bot` agent)

```rust
    #[test]
    fn whoami_flags_an_implicit_human() {
        let out = render_whoami(&writers(), "alice", WriterSource::Env);
        assert!(out.contains("implicit via $USER"), "{out}");
        assert!(out.contains("--writer"), "{out}");
    }

    #[test]
    fn whoami_is_plain_for_an_explicit_human_and_for_any_agent() {
        assert!(!render_whoami(&writers(), "alice", WriterSource::Flag).contains("implicit"));
        assert!(!render_whoami(&writers(), "bot", WriterSource::Env).contains("implicit"));
    }
```
Update the existing `whoami_*` tests to pass `WriterSource::Flag`.

- [ ] **Step 2: Run** `cargo test --bin cassette writer::tests` — expect compile failure.

- [ ] **Step 3: Implement**

```rust
pub fn render_whoami(all: &Writers, name: &str, source: WriterSource) -> String {
    match lookup_by_name(all, name) {
        Some((id, kind)) => {
            let note = if source == WriterSource::Env && kind == Kind::Human {
                "   (implicit via $USER: acts as agent — pass --writer for human authority)"
            } else {
                ""
            };
            format!("{name}  {}  {id}{note}", kind.as_str())
        }
        None => format!("{name}  (not registered — 'cassette writer register' first)"),
    }
}
```
Thread `source` through `whoami` and the `main.rs` arm (rename `_source` → `source`, update the comment above it, which currently says the source makes no difference).

- [ ] **Step 4:** `cargo test && cargo clippy --all-targets -- -D warnings` — green.

- [ ] **Step 5: Commit**

```bash
git add src/writer.rs src/main.rs
git commit -m "feat: writer whoami says when \$USER carries no human authority"
```

---

### Task 3: Docs

**Files:** `CLAUDE.md`, `.claude/skills/cassette-session/SKILL.md`, `docs/follow-up.md`

- [ ] **Step 1: CLAUDE.md** — in the `queue/{mod,…}` paragraph, replace the `WriterSource` parenthetical with: "`WriterSource` (`Flag` — from `--writer` or `$CASSETTE_WRITER`, where an unknown name is a usage error — versus `Env`, from `$USER`, which bootstraps a new writer on first use) feeds `resolve_acting`, the one resolver every mutating queue command calls: it returns `Acting`, splitting attribution (`id`, always the registered writer) from authority (`authority`, forced to `Agent` for `Env`). Identity may be implicit; authority may not — an agent inherits the human's `$USER`, so only a name given on the command line carries human privileges. Do not export `$CASSETTE_WRITER` globally in a shell an agent uses."

- [ ] **Step 2: SKILL.md** — replace the "Pass `--writer bot` on every command" paragraph with: "**Pass `--writer bot` on every command.** Without it the CLI falls back to `$USER` — the human's name — and your writes are *attributed to the human*, which misroutes `waiting_on`. Sticky locks still bind you (an identity taken from `$USER` never carries human authority), so forgetting is no longer a way to trample a claim, but it is still wrong. `cassette writer whoami` shows what you are acting as."

- [ ] **Step 3:** Remove the `$USER` bullet from `docs/follow-up.md`'s "Carried triage".

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md .claude/skills/cassette-session/SKILL.md docs/follow-up.md
git commit -m "docs: implicit writers carry no human authority"
```
