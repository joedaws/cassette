# Phase 4c — JSON and the Sticky Lock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the queue machine-drivable with a `--json` contract, and make the sticky lock actually bind.

**Architecture:** A new `src/queue/json.rs` holds the contract types and their `Serialize` impls as pure data — no I/O — so the frozen shape is unit-testable without a store. `view.rs` gains JSON-building siblings to its prose functions, sharing one `build_view`. `edit.rs` gains `lock`/`unlock` beside `close`/`reopen`, which they resemble. The five verbatim `QueueError`→exit matches in `main.rs` collapse into one helper that emits prose or the JSON error envelope.

**Tech Stack:** Rust 2021, clap v4 derive, serde + serde_json, `fs4` advisory locks, `tempfile` in tests.

**Spec:** `docs/superpowers/specs/2026-09-16-json-and-sticky-lock-design.md` (authoritative for this sub-phase), which argues from `docs/superpowers/specs/2026-09-13-session-store-design.md`.

## Global Constraints

- **The brief may be wrong.** Across Phases 2–4b the plans were wrong more than a dozen times — boundary expectations simply incorrect, a suggested test technique that would have reintroduced known flakiness, a defect list missing an instance. Verify every claim here against the code and the spec before acting. Deviating is allowed and expected; say so in your report when you do.
- **`LockGuard::write` is the only code in the crate that writes a cassette file.** No new path may write a `.md` under `cassettes/`.
- **`src/queue/view.rs` performs no writes and holds no locks.** It may *probe* (`Store::is_free`), which acquires and releases inside one call and constructs no `LockGuard`.
- **`busy` must be derived with `store::lock::probe`, never by acquiring and dropping a `LockGuard`.** `lock::acquire` stamps the holder's attribution into the anchor *after* acquiring, so acquiring to answer a question would make a read-only command write to every cassette it reports on, and create anchor files that never existed.
- **Sessions are named by ULID only** — no alias resolution, no id-prefix matching.
- **Only `human` writers may set or clear a sticky lock.** An agent invoking `queue lock`/`unlock` is exit **2**, not 4.
- **Exit codes:** 1 I/O, 2 usage, 3 busy, 4 sticky-locked, 5 nothing available, 6 queue full.
- `src/cli.rs` is the only place the command line is read; clap enums live there with a `From` into the domain type, following the existing `StatusArg` and `WriterKindArg`.
- **No sleeps and no polling in tests.** In `tests/lock.rs`, a child's lock acquisition is proven by output it writes AFTER acquiring — never by the parent's write to its stdin, which proves only that a pipe buffer accepted bytes.
- **This crate is binary-only — there is no `[lib]` target.** `cargo test --lib <name>` does not work. Use `cargo test <name>`.
- `cargo fmt` clean and `cargo clippy --all-targets -- -D warnings` clean at every commit.

## File Structure

| File | Responsibility |
|---|---|
| `src/queue/json.rs` (create) | Contract types, `Serialize` impls, the body splitter, word counting. Pure data — no `Store`, no `std::fs`. |
| `src/queue/view.rs` (modify) | Gains `list_view` / `next_view` / `show_view` building contract types from the store, sharing one `build_view`. Still no writes. |
| `src/queue/edit.rs` (modify) | Gains `lock` and `unlock`. |
| `src/queue/write.rs` (modify) | Gains the sticky-lock check. |
| `src/queue/mod.rs` (modify) | `pub mod json;`, and `exit_code(&QueueError) -> i32`. |
| `src/cli.rs` (modify) | `--json` global flag; `Lock` / `Unlock` variants on `QueueCmd` and `QueueAction`. |
| `src/main.rs` (modify) | One exit helper replacing five copies; `--json` dispatch. |
| `Cargo.toml` (modify) | Add `serde_json`. |

---

### Task 1: One exit path, `--json`, and the error envelope

**Files:**
- Modify: `Cargo.toml`, `src/queue/mod.rs`, `src/cli.rs`, `src/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces: `queue::exit_code(e: &QueueError) -> i32`, `queue::message(e: &QueueError) -> &str`
- Produces: `Args.json: bool` (set by a global `--json` flag)
- Produces in `main.rs`: `fn exit_queue_err(e: &queue::QueueError, json: bool) -> !` and `fn exit_queue_ok(output: Option<String>) -> !`

**Why first:** every later task emits either prose or JSON on failure. Building that once means Tasks 2–6 wire into it rather than each growing their own branch — and `main.rs` currently carries **five verbatim copies** of the six-arm `QueueError`→exit match, which `--json` would otherwise turn into five copies of the envelope too.

- [ ] **Step 1: Write the failing test**

In `tests/cli.rs`:

```rust
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
        v["error"].as_str().expect("error string").contains("not-a-ulid"),
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
```

`tests/cli.rs` needs `serde_json` as a dev-dependency for this; adding it to `[dependencies]` (Step 3) covers integration tests too.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test json_errors`
Expected: FAIL — `--json` is not a known flag, so clap exits 2 with a usage error and stdout is empty.

- [ ] **Step 3: Add serde_json**

```bash
cargo add serde_json
```

Pin whatever current major version resolves; `serde` is already present.

- [ ] **Step 4: Add `exit_code` and `message` to `queue/mod.rs`**

```rust
/// The process exit code this failure deserves. The single source of truth
/// for the spec's exit table: `main.rs` renders, this decides.
///
/// Kept here rather than in `main.rs` because `--json` puts the number in a
/// machine-readable field — a caller branches on it — so it is contract, not
/// presentation.
pub fn exit_code(e: &QueueError) -> i32 {
    match e {
        QueueError::Io(_) => 1,
        QueueError::Usage(_) => 2,
        QueueError::Busy(_) => 3,
        QueueError::Sticky(_) => 4,
        QueueError::Empty(_) => 5,
        QueueError::Full(_) => 6,
    }
}

/// The human-readable message, whatever the variant.
pub fn message(e: &QueueError) -> &str {
    match e {
        QueueError::Io(m)
        | QueueError::Usage(m)
        | QueueError::Busy(m)
        | QueueError::Sticky(m)
        | QueueError::Empty(m)
        | QueueError::Full(m) => m,
    }
}
```

- [ ] **Step 5: Add the `--json` global flag**

In `src/cli.rs`, on the `Cli` struct beside `--writer`:

```rust
    /// emit machine-readable JSON (full data on queue list/next/show;
    /// {"error","code"} on any command that fails)
    #[arg(long, global = true)]
    json: bool,
```

and `pub json: bool` on `Args`, set in `into_args`.

- [ ] **Step 6: Replace the five exit matches with one helper**

In `src/main.rs`:

```rust
/// Exit on a failed queue command, in whichever form the caller asked for.
///
/// The JSON envelope goes to **stdout**, not stderr: an agent that redirects
/// stderr to a log must still receive a parseable failure on the channel it
/// is reading. Prose keeps going to stderr, where it always has.
fn exit_queue_err(e: &queue::QueueError, json: bool) -> ! {
    let code = queue::exit_code(e);
    if json {
        let envelope = serde_json::json!({ "error": queue::message(e), "code": code });
        println!("{envelope}");
        std::process::exit(code);
    }
    die_with(code, queue::message(e))
}
```

Replace all five copies of the six-arm match (in the `New`, `Write`, `List`, `Show`, `Next`, `Close`, `Reopen`, `Move` arms and in `exit_on_queue_result`) with calls to it. Grep for `QueueError::Full(m)) => die_with(6` to find them all; expect zero matches afterwards.

Also route `require_session`'s failure through it, so a bad `--session` produces an envelope under `--json` like every other failure.

- [ ] **Step 7: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add --json and collapse five exit matches into one helper

The exit code moves into queue::exit_code because --json puts it in a field
a caller branches on: it is contract now, not presentation. The envelope goes
to stdout so an agent redirecting stderr still gets a parseable failure."
```

---

### Task 2: The contract types and the body splitter

**Files:**
- Create: `src/queue/json.rs`
- Modify: `src/queue/mod.rs` (`pub mod json;`)

**Interfaces:**
- Produces:
  - `json::SessionRef { id: String, alias: Option<String> }`
  - `json::WriterRef { name: String, kind: &'static str }`
  - `json::CassetteView { id, topic: Option<String>, priority: i64, status: &'static str, words: usize, busy: bool, sticky_lock: Option<WriterRef>, created_by: Option<WriterRef>, last_writer: Option<WriterRef>, waiting_on: Option<&'static str>, updated_at: String, side_a: String, side_b: String }`
  - `json::Listing { session: SessionRef, cassettes: Vec<CassetteView> }`
  - `json::split_sides(body: &str) -> (String, String)`
  - `json::count_words(side_a: &str, side_b: &str) -> usize`
  - `json::waiting_on(last_writer: Option<&WriterRef>) -> Option<&'static str>`

All of it derives `serde::Serialize`. **No `Store`, no `std::fs`, no locking in this module** — it is pure data, so the frozen contract shape is testable without a store.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_with_side_headings_splits_on_them() {
        let (a, b) = split_sides("## Side A\n\nfront words\n\n## Side B\n\nback words\n");
        assert_eq!(a.trim(), "front words");
        assert_eq!(b.trim(), "back words");
    }

    #[test]
    fn a_body_with_no_headings_is_all_side_a() {
        // Nothing writes sides yet — `queue write` replaces the whole body
        // with flat text — so this is the common case today, and the one
        // that must not silently lose the text.
        let (a, b) = split_sides("just prose an agent wrote\n");
        assert_eq!(a.trim(), "just prose an agent wrote");
        assert_eq!(b, "");
    }

    #[test]
    fn side_a_only_leaves_side_b_empty() {
        let (a, b) = split_sides("## Side A\n\nwords\n");
        assert_eq!(a.trim(), "words");
        assert_eq!(b, "");
    }

    #[test]
    fn words_counts_both_sides() {
        // Matches Cassette::word_count in src/cassette.rs, which is
        // split_whitespace().count() over both sides. The TUI and the JSON
        // must never disagree about how long a cassette is.
        assert_eq!(count_words("one two", "three"), 3);
        assert_eq!(count_words("", ""), 0);
        assert_eq!(count_words("  spaced   out  ", ""), 2);
    }

    #[test]
    fn waiting_on_is_the_inverse_of_who_wrote_last() {
        let human = WriterRef { name: "joseph".into(), kind: "human" };
        let agent = WriterRef { name: "bot".into(), kind: "agent" };
        assert_eq!(waiting_on(Some(&human)), Some("agent"));
        assert_eq!(waiting_on(Some(&agent)), Some("human"));
        // An unresolvable writer yields null rather than a guess: a writer
        // missing from writers.toml is a damaged store, and inventing a turn
        // would tell an agent it is up when nobody knows whose turn it is.
        assert_eq!(waiting_on(None), None);
    }

    #[test]
    fn a_cassette_serializes_to_the_contract_shape() {
        let v = CassetteView {
            id: "01K5GR7T2M9WPD0000000000AB".into(),
            topic: Some("refactor notes".into()),
            priority: 10,
            status: "open",
            words: 2,
            busy: false,
            sticky_lock: None,
            created_by: Some(WriterRef { name: "joseph".into(), kind: "human" }),
            last_writer: Some(WriterRef { name: "joseph".into(), kind: "human" }),
            waiting_on: Some("agent"),
            updated_at: "2026-09-13T14:02:11Z".into(),
            side_a: "two words".into(),
            side_b: String::new(),
        };
        let j: serde_json::Value = serde_json::to_value(&v).expect("serialize");
        assert_eq!(j["id"], "01K5GR7T2M9WPD0000000000AB");
        assert_eq!(j["status"], "open");
        assert_eq!(j["sticky_lock"], serde_json::Value::Null);
        assert_eq!(j["created_by"]["kind"], "human");
        assert_eq!(j["waiting_on"], "agent");
        assert_eq!(j["side_b"], "");
    }

    #[test]
    fn prose_with_quotes_and_newlines_round_trips() {
        // The reason serde_json is a dependency rather than hand-rolled
        // escaping: bodies are arbitrary user prose.
        let body = "she said \"no\"\\ever\n\ttabbed\n";
        let (a, _) = split_sides(body);
        let j = serde_json::to_string(&a).expect("serialize");
        let back: String = serde_json::from_str(&j).expect("round trip");
        assert_eq!(back, a);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test queue::json`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement `src/queue/json.rs`**

`split_sides` looks for lines equal to `## Side A` and `## Side B` after trimming trailing whitespace — the format `src/output.rs` writes (see its `build_body`, which emits `## Side A\n\n` and `## Side B\n\n`). With no `## Side A` heading anywhere, the entire body is `side_a` and `side_b` is empty. Text before a first heading, when headings do exist, belongs to `side_a`.

`count_words` is `side_a.split_whitespace().count() + side_b.split_whitespace().count()`.

`WriterRef.kind` is `&'static str` (`"human"` / `"agent"`) rather than the `Kind` enum, so the serialized spelling is fixed here and cannot drift with a `Kind` rename.

Register with `pub mod json;` in `src/queue/mod.rs`.

- [ ] **Step 4: Run the tests and commit**

Run: `cargo test queue::json && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add the JSON contract types and body splitter

Pure data with no Store and no fs, so the frozen contract shape is testable
without a store. side_a/side_b fall back to the whole body, so the contract
does not change when Phase 5 makes sides real."
```

---

### Task 3: Build views from the store; wire `--json` into list/next/show

**Files:**
- Modify: `src/queue/view.rs`, `src/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: everything from Task 2.
- Produces:
  - `view::build_view(store: &Store, session: &str, c: &StoredCassette, writers: &writers::Writers) -> Result<json::CassetteView, QueueError>`
  - `view::list_view(store, session, status, since) -> Result<json::Listing, QueueError>`
  - `view::next_view(store, session) -> Result<json::CassetteView, QueueError>`
  - `view::show_view(store, session, id) -> Result<json::CassetteView, QueueError>`

**Keep the prose functions as they are.** `list`, `next` and `show` keep their current signatures and output; the `_view` functions are siblings for the JSON path. In particular `show`'s prose output is the raw file text — do **not** reimplement it by re-rendering frontmatter from a `CassetteView`, which would reorder fields and drop any the parser does not model.

The writer registry is read **once per command** and passed into `build_view`, not read per cassette: a 36-cassette listing must not re-read `writers.toml` 36 times.

- [ ] **Step 1: Write the failing test**

In `tests/cli.rs`:

```rust
#[test]
fn queue_list_json_emits_the_contract_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let sid = {
        let o = Command::new(bin()).args(["session", "new", "--alias", "monday"])
            .env("CASSETTE_DATA_DIR", &root).output().expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let cid = {
        let o = Command::new(bin()).args(["queue", "new", "gratitude", "--session", &sid])
            .env("CASSETTE_DATA_DIR", &root).env("USER", "joseph").output().expect("spawn");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let mut child = Command::new(bin())
        .args(["queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root).env("USER", "joseph")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
        .spawn().expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"three whole words\n").expect("write");
    assert!(child.wait().expect("wait").success());

    let out = Command::new(bin())
        .args(["queue", "list", "--session", &sid, "--json"])
        .env("CASSETTE_DATA_DIR", &root).env("USER", "joseph")
        .output().expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["session"]["id"], sid.as_str());
    assert_eq!(v["session"]["alias"], "monday");
    let c = &v["cassettes"][0];
    assert_eq!(c["id"], cid.as_str());
    assert_eq!(c["status"], "open");
    assert_eq!(c["words"], 3, "words counts the body: {c}");
    assert_eq!(c["busy"], false, "nobody holds it: {c}");
    assert_eq!(c["sticky_lock"], serde_json::Value::Null);
    assert_eq!(c["last_writer"]["name"], "joseph");
    assert_eq!(c["last_writer"]["kind"], "human");
    assert_eq!(c["waiting_on"], "agent", "a human wrote last, so the agent is up");
    assert_eq!(c["side_a"].as_str().expect("side_a").trim(), "three whole words");
    assert_eq!(c["side_b"], "");
}
```

`tests/cli.rs` needs `use std::io::Write;` for the stdin write if it is not already imported.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test queue_list_json`
Expected: FAIL — `--json` is accepted but `queue list` still prints prose, so `serde_json::from_slice` errors.

- [ ] **Step 3: Implement the builders**

`build_view` assembles a `CassetteView` from a `StoredCassette`:
- `busy` from `store.is_free(session, id)` **negated** — and `is_free` is the non-stamping probe. Never `store.lock(...)`.
- `created_by` / `last_writer` by looking the frontmatter's writer **id** up in `writers.writers` (a `BTreeMap<String, Writer>` keyed by id), mapping to `WriterRef`. A missing id yields `None`.
- `sticky_lock` the same way from `meta.locked_by`.
- `waiting_on` from `json::waiting_on(view.last_writer.as_ref())`.
- `side_a` / `side_b` from `json::split_sides(&c.body)`, `words` from `json::count_words`.

`list_view` applies the same `StatusFilter` and `--since` filtering as `list`, in the same `queue_order`. Factor the filter so the two cannot disagree about which cassettes are in scope — a listing whose prose and JSON forms disagree is worse than either being wrong alone.

- [ ] **Step 4: Dispatch on `args.json` in `main.rs`**

For `List`, `Show` and `Next`: when `args.json`, call the `_view` sibling, serialize with `serde_json::to_string`, print to stdout, exit 0. Otherwise the existing prose path. Failures go through `exit_queue_err(e, args.json)` from Task 1.

`next --json` emits the single `CassetteView` object, not a `Listing`.

- [ ] **Step 5: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: emit the JSON contract from queue list, next and show

The prose paths are untouched: show's output is the raw file, and
re-rendering it from a view would reorder frontmatter and drop unmodelled
fields. The writer registry is read once per command, not once per cassette."
```

---

### Task 4: `queue lock` and `queue unlock`

**Files:**
- Modify: `src/queue/edit.rs`, `src/cli.rs`, `src/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces:
  - `edit::lock(store, session, id, who_name, source) -> Result<(), QueueError>`
  - `edit::unlock(store, session, id, who_name, source) -> Result<(), QueueError>`
  - `cli::QueueCmd::Lock { id, session }` and `cli::QueueCmd::Unlock { id, session }`

**Shape them on `close`/`reopen`,** which they resemble exactly: acquire the advisory lock, read through the guard, check permission, write one frontmatter field. Reuse `lock_error_to_queue_error` for the acquisition.

**`QueueCmd::session()` is an exhaustive match** — adding two variants will not compile until they declare where their session id lives. That is deliberate; fill it in rather than working around it.

**The permission rules, exactly:**

| Case | Result |
|---|---|
| agent invokes `lock` or `unlock` | **exit 2** — a usage error about the caller's kind |
| human `lock` on an unlocked cassette | sets `locked_by` to the acting writer's id, exit 0 |
| human `lock` on a cassette **they already hold** | exit 0, **no write** |
| human `lock` on a cassette **another writer holds** | **exit 4** |
| human `unlock` on a cassette **anyone** holds | clears it, exit 0 |
| human `unlock` on an unlocked cassette | exit 0, **no write** |
| either, on a cassette another process is writing | **exit 3** (busy) |

Exit 2 for an agent, not 4: exit 4 means "you met a durable claim, escalate to a human". An agent calling a human-only command has met no claim — it misused the CLI, which is what 2 is for. Telling it to escalate would send a human to look at a bug in the agent's own invocation.

The no-write no-ops are deliberate. Phase 4b's final review found no-op transitions handled inconsistently — `queue close` on an already-closed cassette appends a *second* close-out blockquote — so these two are settled explicitly rather than left to fall out of the implementation.

- [ ] **Step 1: Write the failing tests**

In `src/queue/edit.rs`'s test module, following the shape of the existing `close_end_to_end_denies_an_agent_over_a_sticky_lock_but_allows_a_human`:

```rust
#[test]
fn an_agent_may_not_set_or_clear_a_sticky_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = new_session(&store);
    store.add_cassette(&sid, &meta("aaa00000000000000000000000", 10, Status::Open), "")
        .expect("add");
    store.ensure_writer("bot", Kind::Agent).expect("register agent");

    // Exit 2, NOT 4: the agent has met no claim — the cassette is unlocked —
    // it has called a command its kind may not call.
    match lock(&store, &sid, "aaa00000000000000000000000", "bot", WriterSource::Flag) {
        Err(QueueError::Usage(_)) => {}
        other => panic!("expected Usage, got {other:?}"),
    }
    match unlock(&store, &sid, "aaa00000000000000000000000", "bot", WriterSource::Flag) {
        Err(QueueError::Usage(_)) => {}
        other => panic!("expected Usage, got {other:?}"),
    }
}

#[test]
fn locking_twice_is_a_no_op_that_does_not_rewrite_the_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = new_session(&store);
    store.add_cassette(&sid, &meta("aaa00000000000000000000000", 10, Status::Open), "body\n")
        .expect("add");
    store.ensure_writer("joseph", Kind::Human).expect("register human");

    lock(&store, &sid, "aaa00000000000000000000000", "joseph", WriterSource::Flag)
        .expect("first lock");
    let after_first = store.scan_session(&sid).expect("scan").cassettes[0].meta.updated_at.clone();

    lock(&store, &sid, "aaa00000000000000000000000", "joseph", WriterSource::Flag)
        .expect("locking your own lock again succeeds");
    let after_second = store.scan_session(&sid).expect("scan").cassettes[0].meta.updated_at.clone();

    assert_eq!(
        after_first, after_second,
        "a no-op must not rewrite the cassette — 4b's close appends a second \
         blockquote on a re-close, and this is the pattern not to repeat"
    );
}

#[test]
fn a_human_may_not_steal_another_writers_sticky_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = new_session(&store);
    let mut m = meta("aaa00000000000000000000000", 10, Status::Open);
    m.locked_by = Some("01OTHERWRITER00000000000AB".to_string());
    store.add_cassette(&sid, &m, "").expect("add");
    store.ensure_writer("joseph", Kind::Human).expect("register human");

    match lock(&store, &sid, "aaa00000000000000000000000", "joseph", WriterSource::Flag) {
        Err(QueueError::Sticky(_)) => {}
        other => panic!("expected Sticky, got {other:?}"),
    }

    // ...but any human may CLEAR any sticky lock: the spec's capability rule
    // is that humans hold the escape hatch.
    unlock(&store, &sid, "aaa00000000000000000000000", "joseph", WriterSource::Flag)
        .expect("a human may clear another writer's lock");
    assert!(store.scan_session(&sid).expect("scan").cassettes[0].meta.locked_by.is_none());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test queue::edit`
Expected: FAIL — `lock` and `unlock` do not exist.

- [ ] **Step 3: Implement `lock` and `unlock`**

Resolve the writer first (keeping the `Kind`, as `close` does), reject a non-human with `QueueError::Usage` **before** acquiring anything — an agent should not take a lock only to be refused. Then acquire, read through the guard, apply the table above, and write only when the field actually changes.

- [ ] **Step 4: Wire the CLI**

`queue lock <ID> --session <ID>` and `queue unlock <ID> --session <ID>`. Add both to `QueueAction`, `QueueCmd`, `into_args`, `QueueCmd::session()`, and the `main.rs` dispatch, routing failures through `exit_queue_err(e, args.json)`.

- [ ] **Step 5: Add a CLI test**

```rust
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
        .args(["queue", "lock", &cid, "--session", &sid])
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

    // An agent may not WRITE it either — exit 4, a durable claim (Task 5).
    let mut child = Command::new(bin())
        .args(["--writer", "bot", "queue", "write", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"agent prose\n").expect("write");
    let blocked = child.wait_with_output().expect("wait");
    assert_eq!(blocked.status.code(), Some(4), "{}", String::from_utf8_lossy(&blocked.stderr));

    let cleared = Command::new(bin())
        .args(["queue", "unlock", &cid, "--session", &sid])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "joseph")
        .output()
        .expect("spawn");
    assert_eq!(cleared.status.code(), Some(0), "{}", stderr(&cleared));
}
```

The `queue write` assertion in the middle depends on Task 5. If you are running Tasks 4 and 5 in order, write the whole test now and expect that one assertion to fail until Task 5 lands; if you would rather keep every commit green, split that block into its own test and add it in Task 5. Say which you did.

- [ ] **Step 6: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat: add queue lock and unlock

An agent invoking either is exit 2, not 4: exit 4 means 'you met a durable
claim, escalate', and an agent calling a human-only command has met no claim.
Both no-ops are writeless — 4b's close-on-closed appends a second blockquote,
and that is the pattern not to repeat."
```

---

### Task 5: Make the sticky lock bind

**Files:**
- Modify: `src/queue/write.rs`, `src/queue/view.rs`
- Test: `src/queue/write.rs`, `src/queue/view.rs`, `tests/lock.rs`

**Interfaces:**
- Consumes: `edit::lock` from Task 4, `QueueError::Sticky`, `Store::is_free`.

**The two gaps this closes**, both inherited and both recorded in the spec:

1. **`queue write` ignores `locked_by` entirely.** An agent writing a sticky-locked cassette must get **exit 4**. Today the field is read nowhere in `write.rs` — its only mention is a comment. A human writing one is permitted.
2. **`queue next` ignores `locked_by`.** The parent spec defines `next` as "the highest-priority open cassette carrying **no sticky lock** and no live flock", but the implementation filters only on `Status::Open` and the advisory lock. It must skip sticky-locked cassettes.

`write.rs` already resolves the writer's `Kind` and binds it as `_kind` with a comment saying a later phase is where it matters. This is that phase.

**Ordering inside `write`:** the check must happen **after** acquiring the guard and reading through it, like `close` does — reading `locked_by` from an unlocked read would race a concurrent `queue lock`.

**In `next`:** filter on `locked_by.is_none()` alongside `Status::Open`, **before** probing the advisory lock. A sticky-locked cassette should never cost a probe syscall, and more importantly the two empty-handed outcomes must keep their meanings — if every open cassette is sticky-locked, that is `Empty` (exit 5, "nothing to wait for; a human must clear these"), not `Busy` (exit 3, "retry shortly"). A sticky lock is cleared by a human, not by waiting.

- [ ] **Step 1: Write the failing tests**

**`src/queue/write.rs` has no `#[cfg(test)] mod tests` today** — you are creating one. It needs the same imports the other queue modules' test modules use (`super::*`, `crate::store::meta::Status`, `crate::store::writers::Kind`, `crate::store::Store`).

`queue::write::write` reads its body from **stdin**, which a unit test cannot supply. Extract the body-writing core into a function taking `body: &str`:

```rust
/// The lock-read-modify-write core, with the body already in hand.
///
/// Split out from `write` so the sticky-lock rule is unit-testable: `write`
/// itself reads stdin, which a test cannot supply. Keeping the split here
/// also makes the lock-before-stdin ordering explicit — `write` acquires,
/// then reads stdin, then calls this.
fn write_body(
    store: &Store,
    session: &str,
    id: &str,
    body: &str,
    who_name: &str,
    source: WriterSource,
) -> Result<(), QueueError> { ... }
```

then test it directly:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::writers::Kind;
    use crate::store::Store;

    fn sticky_cassette(store: &Store) -> (String, String) {
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let id = "aaa00000000000000000000000".to_string();
        let m = CassetteMeta {
            id: id.clone(),
            topic: Some("claimed".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: Some("01OTHERWRITER00000000000AB".to_string()),
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: "2026-09-16T09:00:00Z".to_string(),
        };
        store.add_cassette(&sid, &m, "original\n").expect("add");
        (sid, id)
    }

    #[test]
    fn an_agent_may_not_write_a_sticky_locked_cassette() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (sid, id) = sticky_cassette(&store);
        store.ensure_writer("bot", Kind::Agent).expect("agent");

        match write_body(&store, &sid, &id, "agent prose\n", "bot", WriterSource::Flag) {
            Err(QueueError::Sticky(_)) => {}
            other => panic!("expected Sticky, got {other:?}"),
        }
        let scan = store.scan_session(&sid).expect("scan");
        assert!(
            scan.cassettes[0].body.contains("original"),
            "the refused write must not have touched the body: {}",
            scan.cassettes[0].body
        );
    }

    #[test]
    fn a_human_may_write_a_sticky_locked_cassette() {
        // The lock keeps agents out; a human blocked by one can always
        // `queue unlock` and proceed, so blocking humans would only let one
        // terminal lock the same person out of their own work.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let (sid, id) = sticky_cassette(&store);
        store.ensure_writer("joseph", Kind::Human).expect("human");

        write_body(&store, &sid, &id, "human prose\n", "joseph", WriterSource::Flag)
            .expect("a human may write over a sticky lock");
        let scan = store.scan_session(&sid).expect("scan");
        assert!(scan.cassettes[0].body.contains("human prose"));
    }
}
```

In `src/queue/view.rs`'s tests:

```rust
#[test]
fn next_skips_a_sticky_locked_cassette() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = new_session(&store);

    // The HIGHER-priority cassette is the claimed one, so a `next` that
    // ignored locked_by would return it — this test fails loudly rather
    // than passing by luck of ordering.
    let mut claimed = meta("aaa00000000000000000000000", 10, Status::Open);
    claimed.locked_by = Some("01OTHERWRITER00000000000AB".to_string());
    store.add_cassette(&sid, &claimed, "").expect("add");
    store
        .add_cassette(&sid, &meta("bbb00000000000000000000000", 20, Status::Open), "")
        .expect("add");

    assert_eq!(
        next(&store, &sid).expect("a free cassette exists"),
        "bbb00000000000000000000000"
    );
}

#[test]
fn a_queue_of_only_sticky_locked_cassettes_is_empty_not_busy() {
    // Exit 5, not 3. Busy means "another process is writing it right now —
    // retry shortly". A sticky lock is cleared by a human, never by waiting,
    // so reporting Busy would spin an agent's retry loop forever.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path().to_path_buf());
    let sid = new_session(&store);
    let mut only = meta("aaa00000000000000000000000", 10, Status::Open);
    only.locked_by = Some("01OTHERWRITER00000000000AB".to_string());
    store.add_cassette(&sid, &only, "").expect("add");

    match next(&store, &sid) {
        Err(QueueError::Empty(_)) => {}
        other => panic!("expected Empty, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test queue`
Expected: FAIL — `next` returns the sticky-locked cassette, and `write` succeeds for the agent.

- [ ] **Step 3: Implement both**

- [ ] **Step 4: Add the cross-process test for `busy`**

In `tests/lock.rs`, assert that `queue list --json` reports `"busy": true` for a cassette a live second process is holding.

**Follow the file's discipline exactly** — read its module doc and the comment on `contend` first. **No sleeps, no polling.** A child's lock acquisition is proven by output the child writes *after* acquiring, never by the parent's successful write to the child's stdin, which proves only that a pipe buffer accepted bytes. `queue write` emits nothing after acquiring, so a bare `spawn_write` does **not** prove acquisition — Phase 4b's Task 9 rejected exactly that technique and used the existing `contend()` two-racer elimination instead. Do the same, and stress-run the result at least 20 times before reporting.

- [ ] **Step 5: Run the tests and commit**

Run: `cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add -A
git commit -m "fix: make the sticky lock bind on write and on next

Two gaps inherited from earlier phases: queue write ignored locked_by
entirely, and queue next ignored it despite the parent spec defining next as
the highest-priority open cassette carrying no sticky lock. A queue of only
sticky-locked cassettes is Empty (5), not Busy (3) — waiting will not clear
a sticky lock, so telling an agent to retry would spin it forever."
```

---

### Task 6: Settle the unreadable-session.toml exit code; docs

**Files:**
- Modify: `src/store/mod.rs` (`require_session`), `src/queue/mod.rs` if the error type needs widening
- Modify: `README.md`, `CLAUDE.md`
- Test: `src/store/mod.rs`

**The decision to implement:** `Store::require_session` currently collapses *every* `session_meta` failure into "no session" → exit 2, including a permissions error or a corrupt `session.toml`. The spec's table reserves exit 1 for I/O failures. This was cosmetic while only a human read the message; `--json` now puts `code` in a field an agent branches on, and a store it cannot read is not the same situation as a session id it typed wrong.

Split it: a **missing** session directory or `session.toml` stays exit 2; any other I/O error (permissions, a corrupt file) becomes exit 1. Test the distinction with a directory whose `session.toml` exists but is unreadable — on unix, create it and `set_permissions` to `0o000`, guarded `#[cfg(unix)]`, and skip the test when running as root, where the mode is not enforced.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(unix)]
#[test]
fn an_unreadable_session_toml_is_an_io_error_not_a_missing_session() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, s) = store();
    let sid = s.create_session(&session_meta()).expect("create");
    let toml = s.session_dir(&sid).join("session.toml");

    let mut perms = std::fs::metadata(&toml).expect("metadata").permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&toml, perms).expect("chmod");

    // Root ignores the mode bits, so the chmod proves nothing there. Probe
    // the actual effect rather than guessing from $USER, which can be unset
    // or lie in a container.
    if std::fs::read_to_string(&toml).is_ok() {
        return; // running with privileges that defeat the test's premise
    }

    let err = s.require_session(&sid).expect_err("must not read as present");
    assert!(
        !err.contains("no session"),
        "an unreadable store is an I/O failure, not a typo: {err}"
    );
}
```

The root check probes the real effect — attempting the read — rather than comparing `$USER` against `"root"`, which is unset in some containers and does not track effective uid anyway.

Note the assertion is on the message rather than a variant because `Store::require_session` returns `Result<(), String>` today. If you change its error type to distinguish the two cases (which is the cleaner fix), assert on the variant instead and say so in your report.

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test session_toml`

- [ ] **Step 3: Update the docs**

In `README.md`: document `--json` with a real example of the contract (copy a genuine emitted object, do not hand-write one), `queue lock`/`unlock`, and that a sticky lock stops an agent from writing or closing a cassette and removes it from `queue next`. State plainly that only a human can set or clear one.

In `CLAUDE.md`: add `src/queue/json.rs` to the source layout in the style and density of the neighbouring entries, and bring the Commands block up to date.

**Verify every factual claim against the binary before committing.** Phase 4b shipped a README `--help` transcript that had quietly dropped a clause; if you include command output, generate it rather than typing it.

- [ ] **Step 4: Full verification**

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Then confirm the invariants still hold:

```bash
# Only LockGuard::write, session.toml and writers.toml write via atomic_write
grep -rn 'atomic_write(' src/ | grep -v test
# view.rs holds no locks and performs no writes
grep -n 'guard\|LockGuard' src/queue/view.rs
# busy is derived by probing, never by acquiring
grep -n 'is_free\|store.lock(' src/queue/view.rs
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "fix: distinguish an unreadable session from a missing one; document 4c

--json puts the exit code in a field an agent branches on, so 'I cannot read
your store' and 'you typed the id wrong' stop being interchangeable."
```

---

## Plan Self-Review

**Spec coverage.** Decision 1 (sticky binds on write) → Task 5; decision 2 (`--json` scope and the error envelope) → Tasks 1 and 3; decision 3 (`side_a`/`side_b` with fallback) → Task 2; decision 4 (serde_json) → Task 1; decision 5 (idempotent lock) → Task 4. The contract's derived-field table → Tasks 2 and 3. Inherited gaps 1 and 2 → Task 5; gap 3 is explicitly out of scope. The open item the spec promoted to "4c should settle" → Task 6. Testing section → every task, cross-process in Task 5.

**Ordering checks.** Task 1 precedes all others (they all emit through its exit helper). Task 2 precedes Task 3 (which builds its types). Task 4 precedes Task 5 only for convenience — Task 5's tests set `locked_by` directly via `add_cassette`, as 4b's tests already do, so it does not hard-depend on the commands existing.

**Type consistency.** `CassetteView`, `Listing`, `WriterRef` and `SessionRef` are defined in Task 2 and used under those names in Task 3. `exit_code`/`message` are defined in Task 1 and used in Tasks 3–6. `edit::lock`/`edit::unlock` are defined in Task 4 and referenced in Task 5's commit message only.

**Two places this plan deliberately leaves a choice to the implementer,** each with an instruction to report which was taken: how to test `write`'s sticky rule given that `write` reads stdin (extract a core, or drive the CLI), and how to detect root in Task 6's permission test. Both are genuine judgment calls where either answer is defensible; neither is a gap in the requirements.

**Known risk.** Task 3 is the largest and touches the most call sites. If its reviewer finds the prose and JSON paths disagreeing about which cassettes are in scope, that is the filter not having been factored as Step 3 instructs — fix the factoring rather than patching one path.
