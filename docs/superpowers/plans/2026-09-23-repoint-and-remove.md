# Phase 6 — Repoint and Remove Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the flat-note machinery the store replaced, add `cassette export`, and finish the deferred packaging items — the last phase of the session-store redesign.

**Architecture:** Mostly deletion. `output.rs` keeps exactly one item (`cassette_body`, the live store-body builder) and loses its module-wide `#![allow(dead_code)]` in the same change, so the compiler proves the removal was complete. `export` is a read-only projection over `Store::scan_session`, reusing `queue::write::build_body` so an export cannot drift from what the writer produced.

**Tech Stack:** Rust; `clap_mangen` and `clap_complete` as build-dependencies (build-only, not in the shipped binary). No runtime dependencies added.

**Spec:** `docs/superpowers/specs/2026-09-23-repoint-and-remove-design.md`

## Global Constraints

- **The `#![allow(dead_code)]` at `src/output.rs:7` must be gone by the end of Task 1**, not narrowed. It is the proof that the deletion was complete: anything left dead in that module fails the build.
- **`Config.notes_dir` keeps parsing.** Deleting the field breaks every command for any user whose `config.toml` still sets it, because `load_config` exits 2 on a parse error. The field stays; only its resolution helpers go.
- Every task ends green: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Any command that spawns the `cassette` binary sets `CASSETTE_DATA_DIR` to a temp path. Without it, tests read and write the user's real store.
- **Sequence tasks by producer-and-consumer, not by file** (CLAUDE.md): clippy's `-D warnings` fails a task whose API has no caller yet.

## File Structure

| File | Responsibility |
|---|---|
| `src/output.rs` | Shrinks to `cassette_body` alone; the allow goes |
| `src/config.rs` | Loses `default_notes_dir`, `find_available_path`, `resolve_output_path` if dead; keeps the `notes_dir` field |
| `src/export.rs` | **new** — the `cassette export` projection |
| `src/cli.rs` | the `Export` subcommand |
| `src/main.rs` | dispatch, the `0700` store root, the sync-root warning |
| `build.rs` | **new** — man page and completions from the clap tree |

---

### Task 1: Delete the flat-note machinery

**Files:** Modify `src/output.rs`, `src/config.rs`, `src/queue/edit.rs` (one doc comment)

**Interfaces:** Produces nothing new. `output::cassette_body` survives unchanged.

**Audit before deleting.** The spec's table was built by grepping the tree, but re-verify — a doc-comment mention is not a caller, and a caller in a test still counts:

```bash
for f in write_markdown write_markdown_appended parse_append_base is_draft \
         frontmatter_date parse_markdown cassette_body; do
  printf '%-26s ' "$f"; grep -rn "$f" src/ tests/ | grep -v '^src/output.rs' | wc -l
done
```

- [ ] **Step 1: Write the failing test**

The one test that stands between an upgrade and a broken config. In `src/config.rs`'s `mod tests`:

```rust
/// `notes_dir` has no readers since 5a, but the FIELD must keep parsing:
/// `load_config` exits 2 on a parse error, so removing it would break every
/// command for anyone whose config still sets it and who has not edited it
/// since. Deprecated, not deleted.
#[test]
fn a_config_that_still_sets_notes_dir_keeps_parsing() {
    let toml = r#"
        notes_dir = "/home/someone/notes"
        visible_lines = 8
    "#;
    let cfg: Config = toml::from_str(toml).expect("an old config must still load");
    assert_eq!(cfg.visible_lines, Some(8), "and the rest of it still applies");
}
```

- [ ] **Step 2: Run it, then delete**

Run: `cargo test --bin cassette a_config_that_still_sets_notes_dir` — it should PASS already; it is a regression guard, not a red test. Say so in the report rather than pretending it failed first.

Delete from `output.rs`: `write_markdown`, `write_markdown_appended`, `parse_append_base`, `AppendBase`, `is_draft`, `frontmatter_date`, `parse_markdown`, and every helper and test that exists only for them. Keep `cassette_body` and its tests. **Remove `#![allow(dead_code)]`.**

Delete from `config.rs`: `default_notes_dir`, `find_available_path`, and `resolve_output_path` if the audit shows it dead. Keep the `notes_dir` field, and change its doc comment to say it is deprecated and read by nothing.

Fix `src/queue/edit.rs`'s doc comment that refers to `output::parse_markdown`.

- [ ] **Step 3: Verify the deletion is complete and commit**

```bash
grep -n 'allow(dead_code)' src/output.rs && echo "STILL THERE — not done" || echo "allow removed"
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "refactor: delete the flat-note machinery the store replaced"
```

---

### Task 2: `cassette export`

**Files:** Create `src/export.rs`; modify `src/cli.rs`, `src/main.rs`; test in `tests/cli.rs`

**Interfaces:**
- Consumes: `Store::scan_session`, `queue::write::build_body`
- Produces: `export::render(scan: &SessionScan) -> String`, `cassette export <SESSION> [--out <PATH>]`

**This task carries its own consumer** — the CLI dispatch lands with the module, so clippy has no dead code to object to.

- [ ] **Step 1: Write the failing test**

In `src/export.rs`'s `mod tests`:

```rust
/// An export is the archive, so it is not lossy: closed cassettes are
/// included and marked, and a damaged one becomes a visible heading rather
/// than a silent omission — the same reason 5c gave damaged files rows.
#[test]
fn export_renders_every_cassette_in_queue_order_and_marks_the_unusual_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let session = store.create_session(&session_meta()).expect("session");
    add(&store, &session, 10, Status::Open, Some("morning"), "first words\n");
    add(&store, &session, 20, Status::Closed, Some("done"), "archived words\n");
    std::fs::write(
        store.cassettes_dir(&session).join("01M3600000000000000000BAD.md"),
        "no frontmatter\n",
    )
    .expect("write");

    let scan = store.scan_session(&session).expect("scan");
    let out = render(&scan);

    let first = out.find("first words").expect("open cassette");
    let second = out.find("archived words").expect("closed cassette");
    assert!(first < second, "queue order: open before closed");
    assert!(out.contains("# Cassette 1 — morning"), "{out}");
    assert!(out.contains("(closed)"), "closed cassettes are marked: {out}");
    assert!(
        out.contains("01M3600000000000000000BAD") && out.contains("unreadable"),
        "a damaged cassette is named, not dropped: {out}"
    );
}

/// The export must not drift from what the writer produced: assert against
/// `build_body` rather than a hand-written string, so a change to one shows
/// up here instead of silently diverging.
#[test]
fn an_exported_body_is_what_the_writer_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let session = store.create_session(&session_meta()).expect("session");
    add(&store, &session, 10, Status::Open, None, "side a text\n");

    let scan = store.scan_session(&session).expect("scan");
    let out = render(&scan);

    assert!(
        out.contains(scan.cassettes[0].body.trim()),
        "the stored body appears verbatim:\n{out}"
    );
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

`export::render` walks `scan.cassettes` (already in queue order from `scan_session`) emitting `# Cassette N — <topic>` (`— <topic> (closed)` when closed, `# Cassette N` bare when no topic) followed by the stored body verbatim, then one heading per `scan.damaged` entry naming its id and reason.

`cli.rs` gains `Export { session: String, out: Option<PathBuf> }`; `main.rs` dispatches: scan, render, then write to `--out` or stdout. An unknown session id is a usage error (exit 2), via the same `require_session` path the queue commands use. **No locking** — a read-only projection, and `atomic_write`'s rename means a reader cannot see a torn file.

- [ ] **Step 3: Add the CLI test and commit**

```rust
#[test]
fn export_of_an_unknown_session_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = run_in(&dir, &["export", "01M3NOSUCHSESSION00000000A"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}
```

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: cassette export renders a session to flat markdown"
```

---

### Task 3: `0700` store root and the sync-root warning

**Files:** Modify `src/main.rs` (or `src/store/mod.rs` where the root is created)

**Interfaces:** Produces `fn warn_if_synced(root: &Path)`; no new public API.

- [ ] **Step 1: Write the failing tests**

```rust
/// Freewriting is private by nature; the default 0755 would make every
/// session world-readable on a shared machine.
#[cfg(unix)]
#[test]
fn a_new_store_root_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("fresh");
    let store = Store::new(root.clone());
    store.create_session(&session_meta()).expect("session");

    let mode = std::fs::metadata(&root).expect("root").permissions().mode();
    assert_eq!(mode & 0o777, 0o700, "got {:o}", mode & 0o777);
}

/// A directory the user already has is left alone: re-chmodding it is a
/// surprise they did not ask for, and it fights anyone who set it on purpose.
#[cfg(unix)]
#[test]
fn an_existing_store_root_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("existing");
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let store = Store::new(root.clone());
    store.create_session(&session_meta()).expect("session");

    let mode = std::fs::metadata(&root).expect("root").permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "left as the user set it");
}

/// A heuristic that warns and continues. It can false-positive on a
/// directory that merely has one of these words in its name, and a tool that
/// refuses to start over a substring match is worse than the risk it names.
#[test]
fn a_store_under_a_sync_root_is_flagged() {
    assert!(looks_synced(Path::new("/home/me/Dropbox/cassette")));
    assert!(looks_synced(Path::new("/Users/me/Library/Mobile Documents/x")));
    assert!(!looks_synced(Path::new("/home/me/.local/share/cassette")));
}
```

- [ ] **Step 2: Run to verify they fail, then implement**

Create the root with `0700` **only when creating it** — check existence first, so an existing directory is untouched. `#[cfg(unix)]`; Windows is out of CI by the Phase 3 scope decision and there is no portable equivalent worth faking.

`looks_synced` is a lowercase path-component match against `dropbox`, `mobile documents`, `icloud drive`, `onedrive`, `google drive`, `sync.com`. `main.rs` calls it once at startup and prints one line to stderr when it hits. It never blocks.

- [ ] **Step 3: Run the tests and commit**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
git add -A && git commit -m "feat: create the store 0700, and warn under a sync root"
```

---

### Task 4: Man page and shell completions — DEFERRED

**Not done in this phase.** `build.rs` cannot reach the clap tree: `include!("src/cli.rs")`
fails because `cli.rs` uses nine `crate::` paths in its derive. The plan's own known-risk note
said to stop and report rather than restructure the crate mid-task, which is what happened —
the user chose to defer man page, completions and release packaging to their own design.

Phase 6 adds no dependencies as a result. See the spec section of the same name.

---

### Task 5: Documentation and end-to-end verification

**Files:** Modify `README.md`, `CLAUDE.md`, `docs/distribution.md`

**Generate every piece of command output rather than typing it.** 5b's docs task documented an accident; 5c's and 5d's pty drives each caught a live bug.

- [ ] **Step 1: Update the docs**

`README.md`: `cassette export` with real generated output; a note that `notes_dir` is deprecated and read by nothing; the `0700` and sync-root behaviour. `CLAUDE.md`: shrink the `src/output.rs` entry to what remains, add `src/export.rs`, and drop the now-false claims about the flat-note writer being kept. `docs/distribution.md`: where the generated man page and completions land.

- [ ] **Step 2: Drive it end to end**

Under an isolated `CASSETTE_DATA_DIR`: create a session with an open, a closed and a damaged cassette, `cassette export <id>` to stdout, then `--out` to a file, and diff the two. Confirm the TUI still opens and writes the session afterwards — the deletion touched the module `session_writer` calls.

- [ ] **Step 3: Full verification**

```bash
cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings
grep -n 'allow(dead_code)' src/ -r || echo "no module-wide allows left"
git diff main...HEAD --stat
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "docs: document export, the private store root, and the sync warning"
```

---

## Plan Self-Review

**Spec coverage.** The deletions and the `notes_dir` compatibility guard → T1; `export` → T2; the sync warning → T3 (the `0700` root turned out to be done already, in `ee29073`, and the spec was corrected to the code); man page and completions → deferred; docs and end-to-end → T5.

**Ordering.** T1 first: it is the one whose failure mode is "something still had a caller", and finding that out before new code lands keeps the cause unambiguous. T2–T4 are mutually independent. T5 last.

**Type consistency.** `export::render(&SessionScan) -> String` is defined in T2 and called only from `main.rs`'s dispatch in the same task. `looks_synced(&Path) -> bool` is defined and used in T3.

**One judgment call left to the implementer**, with an instruction to report which was taken: whether `resolve_output_path` in `config.rs` is dead. It is adjacent to the deleted helpers but may still serve `-o`; the audit command in T1 Step 1 settles it, and deleting a live function is exactly the failure this phase risks.

**Known risk.** T4's `include!("src/cli.rs")` depends on `cli.rs` being free of `crate::` paths. If it is not, the task stops and reports rather than restructuring the crate — a packaging step is not worth a lib-target refactor decided mid-flight.
