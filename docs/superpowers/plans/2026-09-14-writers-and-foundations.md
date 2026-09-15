# Phase 4a — Foundations and Writers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make writer identity correct and declarable before any queue command writes
records that depend on it, and close the arithmetic and error-typing defects Phases 2 and
3 carried forward.

**Architecture:** Three defect fixes in `src/store/` (priority overflow, a `WriterError`
that rejects a `kind` mismatch, a cassette id on `LockError::Busy`), then a `--writer`
global flag and a `writer` subcommand group, then `queue write` lifts out of `main.rs`
into `src/queue.rs` to match the per-command shape `stats.rs` and `find.rs` already use.

**Tech Stack:** Rust 2021, existing dependencies only — `clap` 4 derive, `fs4`, `serde` +
`toml`, `chrono`. **No new dependencies.**

**Spec:** `docs/superpowers/specs/2026-09-13-session-store-design.md` (Phase 4a in the
"Implementation phases" section)

## Global Constraints

- **`kind` is a registered property, never a per-call claim.** `--kind` exists only on
  `writer register`. Every other command resolves a writer's kind from the registry. An
  agent must not be able to pass `--kind human` on a write to gain permission — `kind`
  is what the spec's permission boundary rests on (an `agent` refuses to close a cassette
  whose `locked_by` is set; a `human` may).
- **Registering an existing name with a different kind is an error**, not a silent
  update. Nothing may change a writer's identity implicitly.
- **No `"unknown"` writer.** If neither `--writer` nor `$USER` yields a name, that is a
  usage error, not a shared fallback identity.
- Exit codes: 0 ok, 1 I/O failure, 2 usage error, 3 busy. **4, 5 and 6 stay unused** —
  they belong to 4b and 4c.
- **No `--json`** — that is 4c.
- Invariant 1 holds: `LockGuard::write` remains the only code that writes a cassette file.
- Every directory under the data dir is created `0700` via `ensure_private_dir`.
- `fs4`'s trait is `fs4::FileExt` at the crate root, and its methods are called through
  **UFCS** (`FileExt::try_lock(&file)`), because rustc 1.97's `std::fs::File` has inherent
  `lock`/`try_lock` that otherwise shadow them and yield the wrong error type.

### Facts verified against the current tree (2026-09-14)

- Suite is **249 tests** (228 unit + 16 CLI + 5 cross-process lock) and must stay green.
- `priority::last` and `priority::between` are **not called anywhere outside tests** — the
  store is behind a module-wide `#![allow(dead_code)]`. Changing `last`'s signature is
  therefore free right now and will not be later.
- `writers::ensure(root, name, kind) -> io::Result<String>` returns the existing id on a
  name hit **without looking at `kind`**.
- `Store::ensure_writer` holds the registry lock across the call; the binding is
  `let _registry`, and a bare `let _` would silently drop it — there is a comment saying so.
- `LockError` is `{ Busy(Option<Attribution>), NoSuchCassette { session, id }, Io }`.
- `main.rs` is ~1196 lines and contains `queue_write`, `whoami`, `die_with`, `store_root`.
- `src/cli.rs` has no top-level positional; `Command::Queue { action: QueueAction }` is
  the existing nested-subcommand shape to copy.

## File Structure

| File | Responsibility |
|---|---|
| `src/store/priority.rs` (modify) | overflow-safe `last`/`between` |
| `src/store/writers.rs` (modify) | `WriterError`; `ensure` rejects a `kind` mismatch |
| `src/store/mod.rs` (modify) | `ensure_writer` returns `Result<String, WriterError>` |
| `src/store/lock.rs` (modify) | `LockError::Busy` carries the cassette id |
| `src/cli.rs` (modify) | `--writer` global flag; `writer` subcommand group |
| `src/queue.rs` (create) | `queue write`, lifted out of `main.rs` |
| `src/writer.rs` (create) | `writer register` / `list` / `whoami` |
| `src/main.rs` (modify) | dispatch; `whoami` resolution; shrink |

---

### Task 1: Overflow-safe priority arithmetic

**Files:**
- Modify: `src/store/priority.rs`

**Interfaces:**
- Produces: `last(&[i64]) -> Option<i64>` (signature change), `between(i64, i64) -> Option<i64>` (behaviour change only)

`last` changes from returning a bare `i64` to `Option<i64>`, matching `first` and
`between`: `None` means "no room, renumber this run". This is free today because nothing
outside tests calls it, and will not be free once 4b's `queue new` does.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `src/store/priority.rs`:

```rust
    #[test]
    fn tail_placement_refuses_to_overflow() {
        // Reachable from a hand-edited frontmatter priority: `parse_frontmatter`
        // accepts any i64-parseable string. A debug build panicked here.
        assert_eq!(last(&[i64::MAX]), None, "no room above i64::MAX");
        assert_eq!(last(&[i64::MAX - 1]), None, "nor within one STEP of it");
        assert_eq!(last(&[i64::MAX - STEP]), Some(i64::MAX), "exactly one step fits");
    }

    #[test]
    fn between_refuses_to_overflow() {
        // `hi - lo` overflows before the midpoint guard ever runs.
        assert_eq!(between(i64::MIN, i64::MAX), None, "the span is not representable");
        // Also unrepresentable: `0 - i64::MIN` is `i64::MAX + 1`. Returning
        // None here is correct — it reads as "renumber this run", and real
        // priorities are positive by construction anyway.
        assert_eq!(between(i64::MIN, 0), None, "this span overflows too");
        // A span that IS representable must still produce a midpoint.
        assert_eq!(between(-10, 10), Some(0), "a representable span still works");
    }
```

Update the existing `tail_placement_is_max_plus_a_step`,
`tail_placement_on_an_empty_queue_is_the_first_step` and any other caller of `last` to
unwrap the `Option` — for example `assert_eq!(last(&[10, 20, 30]), Some(40));`. Do not
weaken any existing assertion; only adjust for the new return type.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test store::priority`
Expected: compile failure on the new `Option` comparisons, or a panic with
"attempt to add with overflow" on the `i64::MAX` case.

- [ ] **Step 3: Implement**

```rust
/// Tail placement — the default for a new cassette. `None` when there is no
/// room left above the maximum, which means this run must be renumbered.
///
/// Returns `Option` rather than a bare `i64` so an unrepresentable result is a
/// value the caller must handle, not a debug-build panic. Priorities come from
/// frontmatter, which `parse_frontmatter` will accept as any i64-parseable
/// string, so `i64::MAX` is reachable by hand-editing a file.
pub fn last(existing: &[i64]) -> Option<i64> {
    existing
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(STEP)
}

/// Midpoint of two neighbours. `None` when they are adjacent or equal, when
/// they are reversed, or when the span between them is not representable —
/// all of which mean the same thing to a caller: this run must be renumbered.
pub fn between(lo: i64, hi: i64) -> Option<i64> {
    let span = hi.checked_sub(lo)?;
    let mid = lo.checked_add(span / 2)?;
    (mid > lo && mid < hi).then_some(mid)
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test store::priority`
Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/priority.rs
git commit -m "fix: priority arithmetic cannot overflow"
```

---

### Task 2: `WriterError` and rejecting a `kind` mismatch

**Files:**
- Modify: `src/store/writers.rs`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Produces:
  - `store::writers::WriterError { KindMismatch { name: String, registered: Kind, requested: Kind }, Io(io::Error) }` with `Display` and `From<io::Error>`
  - `writers::ensure(root, name, kind) -> Result<String, WriterError>`
  - `Store::ensure_writer(name, kind) -> Result<String, WriterError>`

A dedicated variant rather than `io::Error::new(InvalidInput, …)`, for the same reason
`LockError::NoSuchCassette` exists: the caller must render a usage error (exit 2) without
matching on `io::ErrorKind`, a heuristic that silently depends on nothing else in the call
graph producing that kind.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `src/store/writers.rs`:

```rust
    #[test]
    fn re_registering_with_a_different_kind_is_rejected() {
        // `kind` is what the permission boundary rests on: an agent refuses to
        // close a cassette whose `locked_by` is set, a human may. Letting a
        // second registration silently flip it would let any caller change an
        // identity — including an agent re-registering itself as human.
        let dir = tempfile::tempdir().expect("tempdir");
        let id = ensure(dir.path(), "bot", Kind::Agent).expect("first");
        match ensure(dir.path(), "bot", Kind::Human) {
            Err(WriterError::KindMismatch { name, registered, requested }) => {
                assert_eq!(name, "bot");
                assert_eq!(registered, Kind::Agent);
                assert_eq!(requested, Kind::Human);
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
        // And the record must be untouched.
        let all = read(dir.path()).expect("read");
        assert_eq!(all.writers[&id].kind, Kind::Agent, "the registered kind stands");
        assert_eq!(all.writers.len(), 1, "no second id was minted");
    }

    #[test]
    fn re_registering_with_the_same_kind_is_still_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure(dir.path(), "joseph", Kind::Human).expect("first");
        let again = ensure(dir.path(), "joseph", Kind::Human).expect("again");
        assert_eq!(first, again);
        assert_eq!(read(dir.path()).expect("read").writers.len(), 1);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test store::writers`
Expected: FAIL to compile — `cannot find type 'WriterError' in this scope`.

- [ ] **Step 3: Implement**

Add to `src/store/writers.rs`:

```rust
/// Why a writer could not be resolved.
#[derive(Debug)]
pub enum WriterError {
    /// This name is registered with a different `kind`. A distinct variant
    /// rather than an `Io(InvalidInput)` so a caller renders a usage error
    /// without matching on `io::ErrorKind`.
    KindMismatch {
        name: String,
        registered: Kind,
        requested: Kind,
    },
    Io(io::Error),
}

impl std::fmt::Display for WriterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriterError::KindMismatch {
                name,
                registered,
                requested,
            } => write!(
                f,
                "'{name}' is already registered as {} — cannot register as {}",
                registered.as_str(),
                requested.as_str()
            ),
            WriterError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for WriterError {
    fn from(e: io::Error) -> WriterError {
        WriterError::Io(e)
    }
}
```

`Kind` needs an `as_str` for those messages, and `Display` on `Kind` would be a second
way to say the same thing — add only `as_str`:

```rust
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Human => "human",
            Kind::Agent => "agent",
        }
    }
}
```

Rewrite `ensure`:

```rust
/// The id for `name`, registering it on first sight. Idempotent for a matching
/// `kind`; a mismatch is rejected rather than silently updated — see
/// `WriterError::KindMismatch`.
pub(crate) fn ensure(root: &Path, name: &str, kind: Kind) -> Result<String, WriterError> {
    let mut all = read(root)?;
    if let Some((id, existing)) = all
        .writers
        .iter()
        .find(|(_, w)| w.name == name)
        .map(|(id, w)| (id.clone(), w.kind))
    {
        if existing != kind {
            return Err(WriterError::KindMismatch {
                name: name.to_string(),
                registered: existing,
                requested: kind,
            });
        }
        return Ok(id);
    }
    let id = ids::new_id();
    all.writers.insert(
        id.clone(),
        Writer {
            name: name.to_string(),
            kind,
            created: meta::now_utc(),
        },
    );
    write(root, &all)?;
    Ok(id)
}
```

`Kind` must derive `PartialEq` for that comparison — check whether it already does and add
it only if missing.

In `src/store/mod.rs`, change `ensure_writer`'s return type to
`Result<String, writers::WriterError>` and keep the `let _registry` binding and its
comment exactly as they are.

- [ ] **Step 4: Fix the callers**

`main.rs`'s `queue_write` calls `ensure_writer` and maps its error. Update it so a
`KindMismatch` exits 2 and an `Io` exits 1. Do not collapse them.

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test` — all green, plus clippy and fmt.

- [ ] **Step 6: Commit**

```bash
git add src/store/ src/main.rs
git commit -m "fix: reject a writer kind mismatch instead of ignoring it"
```

---

### Task 3: `LockError::Busy` carries the cassette id

**Files:**
- Modify: `src/store/lock.rs`
- Modify: `src/store/mod.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `LockError::Busy { id: String, holder: Option<Attribution> }`

Today `Busy` carries only the holder. `queue write` gets away with it because `main.rs`
already knows the single id it asked for — but 4b's `queue move` backs onto `lock_many`,
and a user would otherwise see "held by joseph" with no way to learn *which* of several
cassettes blocked.

- [ ] **Step 1: Write the failing test**

Add to `src/store/lock.rs`'s test module:

```rust
    #[test]
    fn busy_names_the_cassette_as_well_as_the_holder() {
        // With `lock_many` behind `queue move`, "held by joseph" is not
        // actionable unless it says which cassette.
        let (_d, s) = store();
        let sid = s.create_session(&session_meta()).expect("session");
        s.add_cassette(&sid, &cassette_meta(ID), "").expect("add");
        let who = Attribution::for_now("writer-1", "joseph");
        let _held = s.lock(&sid, ID, &who).expect("first");
        match s.lock(&sid, ID, &Attribution::for_now("writer-2", "agent")) {
            Err(LockError::Busy { id, holder }) => {
                assert_eq!(id, ID, "the blocked caller must learn which cassette");
                assert_eq!(holder.expect("holder").name, "joseph");
            }
            other => panic!("expected Busy with an id, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test store::lock::tests::busy_names_the_cassette`
Expected: FAIL to compile — `Busy` is a tuple variant, not a struct variant.

- [ ] **Step 3: Implement**

Change the variant in `src/store/lock.rs`:

```rust
    /// Another live writer holds it. `holder` is `None` when they left no
    /// readable attribution — a crash before writing their line, or garbled
    /// bytes. It still blocks us; we just cannot name them. `id` is always
    /// present, so a caller locking several cassettes can say which one.
    Busy {
        id: String,
        holder: Option<Attribution>,
    },
```

Update `Display`:

```rust
            LockError::Busy { id, holder: Some(a) } => {
                write!(f, "'{id}' is held by {} (since {})", a.name, a.since)
            }
            LockError::Busy { id, holder: None } => {
                write!(f, "'{id}' is held by another writer")
            }
```

`acquire` constructs the variant, so it needs the id — it already takes `id: &str`. Update
the construction site and the `From<LockError> for io::Error` arm (the `busy` catch-all
binding will no longer match a tuple variant; restructure it explicitly rather than
leaving a wildcard that could swallow a future variant).

Update `main.rs`'s `Busy` arm to use the struct fields. The user-facing message must not
regress — `tests/lock.rs` asserts on `"is open by"` and `"try again later"`, so keep that
wording in `main.rs` even though `Display` phrases it differently.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: green. `tests/lock.rs` must pass unchanged — if it does not, the CLI message
regressed and the fix is in `main.rs`, not in the test.

- [ ] **Step 5: Commit**

```bash
git add src/store/ src/main.rs
git commit -m "feat: LockError::Busy names the cassette it could not take"
```

---

### Task 4: `--writer` and the end of the `"unknown"` fallback

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: global `--writer <NAME>`; `resolve_writer_name(cli: Option<&str>) -> Result<String, String>`

`whoami()` currently falls back to `"unknown"`, so every caller without `$USER` collapses
into one shared identity — in a system whose whole point is per-writer attribution. After
this task there is no fallback: a name comes from `--writer`, else `$USER`, else it is a
usage error naming the fix.

- [ ] **Step 1: Write the failing tests**

Add to `tests/cli.rs`:

```rust
#[test]
fn a_writer_name_is_required_when_user_is_unset() {
    // No shared "unknown" identity: attribution is the point of the system.
    let out = Command::new(bin())
        .args(["queue", "write", "01K5GR7T2M9WPD0000000000AB"])
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
```

Check `tests/cli.rs`'s existing helpers — it already has `run` and `stderr`; reuse them,
and add a `bin()` only if one is not already present.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test cli`
Expected: the first fails (today it proceeds as `"unknown"`), the second fails (no such flag).

- [ ] **Step 3: Implement**

In `src/cli.rs`, add to the `Cli` struct beside the other globals:

```rust
    /// writer to act as (default: $USER)
    #[arg(long, value_name = "NAME", global = true)]
    writer: Option<String>,
```

and carry it into `Args` as `pub writer: Option<String>`.

In `src/main.rs`, replace `whoami`:

```rust
/// The writer to act as: `--writer`, else `$USER`. There is deliberately no
/// fallback — a shared `"unknown"` identity would silently attribute every
/// agent's work to the same writer, in a system whose entire purpose is
/// knowing who wrote what.
fn resolve_writer_name(cli: Option<&str>) -> Result<String, String> {
    if let Some(name) = cli {
        let name = name.trim();
        if name.is_empty() {
            return Err("--writer cannot be empty".to_string());
        }
        return Ok(name.to_string());
    }
    match std::env::var("USER") {
        Ok(user) if !user.trim().is_empty() => Ok(user.trim().to_string()),
        _ => Err("no writer: $USER is unset, so pass --writer <NAME>".to_string()),
    }
}
```

and call it where `whoami()` was, mapping its `Err` to `die_with(2, …)`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test` plus clippy and fmt.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat: --writer, and no shared unknown identity"
```

---

### Task 5: The `writer` subcommand group

**Files:**
- Modify: `src/cli.rs`
- Create: `src/writer.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `cassette writer register --name <N> --kind human|agent`, `writer list`, `writer whoami`
- Produces: `writer::register`, `writer::list`, `writer::whoami` — each taking a `&Store` and returning `Result<String, String>` (rendered output, or a message), so the module stays testable and `main.rs` owns exit codes

`--kind` appears **only** on `register`. Every other command resolves kind from the
registry, so an agent cannot claim to be human per-invocation.

- [ ] **Step 1: Write the failing tests**

Add to `tests/cli.rs`:

```rust
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
    assert!(text.contains("agent"), "kind comes from the registry: {text}");
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
    assert!(stderr(&second).contains("already registered"), "{}", stderr(&second));
}
```

`tempfile` is already a dev-dependency.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test cli`
Expected: `unrecognized subcommand 'writer'`.

- [ ] **Step 3: Add the CLI shape**

In `src/cli.rs`, add to `Command`:

```rust
    /// register and inspect writers
    Writer {
        #[command(subcommand)]
        action: WriterAction,
    },
```

and beside `QueueAction`:

```rust
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
```

Carry the action into `Args` as a small enum of its own rather than a tuple — the existing
`queue_write: Option<(String, Option<String>)>` tuple is already at the edge of readable,
and this one has three shapes:

```rust
/// `writer …` as `main()` consumes it.
#[derive(Debug, PartialEq)]
pub enum WriterCmd {
    Register { name: String, kind: crate::store::writers::Kind },
    List,
    Whoami,
}
```

with `pub writer_cmd: Option<WriterCmd>` on `Args`.

- [ ] **Step 4: Implement the command module**

Create `src/writer.rs`. Rendering is split from I/O so it can be unit-tested, matching
`stats.rs` and `find.rs`. This module never calls `process::exit` — `main.rs` owns exit
codes.

```rust
//! The `cassette writer` commands: register, list, whoami.
//!
//! `kind` is fixed at registration and resolved from the registry everywhere
//! else, so a caller cannot claim a kind per-invocation — that is the property
//! the spec's permission boundary rests on.

use crate::store::writers::{Kind, WriterError, Writers};
use crate::store::Store;

/// Register `name`, or return its existing id when it is already registered
/// with this same kind. A kind mismatch is rejected — see `WriterError`.
pub fn register(store: &Store, name: &str, kind: Kind) -> Result<String, String> {
    match store.ensure_writer(name, kind) {
        Ok(id) => Ok(format!("{id}  {name} ({})", kind.as_str())),
        Err(WriterError::KindMismatch { .. }) => Err(store
            .ensure_writer(name, kind)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()),
        Err(e) => Err(e.to_string()),
    }
}

/// One line per writer, sorted by name. The registry is a `BTreeMap` keyed by
/// id, so id order is not name order — sort explicitly.
pub fn render_list(all: &Writers) -> String {
    if all.writers.is_empty() {
        return "no writers registered".to_string();
    }
    let mut rows: Vec<(&str, &str, &str)> = all
        .writers
        .iter()
        .map(|(id, w)| (w.name.as_str(), w.kind.as_str(), id.as_str()))
        .collect();
    rows.sort_unstable();
    rows.iter()
        .map(|(name, kind, id)| format!("{name}  {kind}  {id}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn list(store: &Store) -> Result<String, String> {
    store
        .writers()
        .map(|all| render_list(&all))
        .map_err(|e| e.to_string())
}

/// What `name` resolves to. Reports plainly when the name is not registered
/// yet rather than inventing a kind for it.
pub fn render_whoami(all: &Writers, name: &str) -> String {
    match all.writers.iter().find(|(_, w)| w.name == name) {
        Some((id, w)) => format!("{name}  {}  {id}", w.kind.as_str()),
        None => format!("{name}  (not registered — 'cassette writer register' first)"),
    }
}

pub fn whoami(store: &Store, name: &str) -> Result<String, String> {
    store
        .writers()
        .map(|all| render_whoami(&all, name))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::writers::Writer;

    fn writers() -> Writers {
        let mut all = Writers::default();
        all.writers.insert(
            "zzz-id".to_string(),
            Writer {
                name: "alice".to_string(),
                kind: Kind::Human,
                created: "2026-09-14T09:00:00Z".to_string(),
            },
        );
        all.writers.insert(
            "aaa-id".to_string(),
            Writer {
                name: "bot".to_string(),
                kind: Kind::Agent,
                created: "2026-09-14T09:01:00Z".to_string(),
            },
        );
        all
    }

    #[test]
    fn list_sorts_by_name_not_by_id() {
        // The registry is keyed by id, and "aaa-id" (bot) sorts before
        // "zzz-id" (alice) — so relying on map order would list them backwards.
        let out = render_list(&writers());
        let alice = out.find("alice").expect("alice");
        let bot = out.find("bot").expect("bot");
        assert!(alice < bot, "sorted by name, not id: {out}");
    }

    #[test]
    fn list_says_so_when_empty() {
        assert_eq!(render_list(&Writers::default()), "no writers registered");
    }

    #[test]
    fn whoami_reports_the_registered_kind() {
        let out = render_whoami(&writers(), "bot");
        assert!(out.contains("agent"), "{out}");
        assert!(out.contains("aaa-id"), "{out}");
    }

    #[test]
    fn whoami_is_plain_about_an_unregistered_name() {
        let out = render_whoami(&writers(), "nobody");
        assert!(out.contains("not registered"), "{out}");
        assert!(!out.contains("human") && !out.contains("agent"), "must not invent a kind: {out}");
    }
}
```

Note the `register` function above calls `ensure_writer` twice on the mismatch path, which
is wasteful and slightly racy. Restructure it to capture the error once — the shape is
shown for the message and the branches, not as a literal to transcribe. If you find a
cleaner formulation, use it and say so in your report.

- [ ] **Step 5: Dispatch from `main.rs`**

Beside the other pre-terminal commands, before the terminal is touched.

- [ ] **Step 6: Run the suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/writer.rs src/main.rs tests/cli.rs
git commit -m "feat: cassette writer register, list and whoami"
```

---

### Task 6: Lift `queue write` out of `main.rs`

**Files:**
- Create: `src/queue.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `queue::write(store: &Store, id: &str, session: Option<&str>, writer: &str) -> Result<(), QueueError>` and `queue::QueueError` carrying enough for `main.rs` to choose an exit code

`main.rs` is ~1196 lines and this codebase's own pattern is a thin per-command module —
`stats.rs`, `find.rs`. 4b adds seven more `queue` subcommands; if they all land in
`main.rs` it becomes a dumping ground.

**This is a pure move. No behaviour may change.** In particular:

- The lock is acquired **before** stdin is read. A child spawned with an open stdin pipe
  is then provably holding the lock, which is the entire basis of `tests/lock.rs`'s
  determinism. Reversing it would make five cross-process tests racy.
- `ensure_writer` runs **before** `Store::lock`. `registering_writers_concurrently_keeps_both`
  asserts both writers reach the registry even though one loses the lock with exit 3.
- Exit codes stay 0/1/2/3 with the same stderr wording — `tests/lock.rs` asserts on
  `"is open by"` and `"try again later"`.

- [ ] **Step 1: Move the code**

Create `src/queue.rs` and move `queue_write`'s body into `queue::write`, returning an
error instead of calling `die_with`. The body is the existing function in `main.rs` —
move it verbatim, changing only the exits into returns. Keep every comment that explains
*why* an ordering is what it is; those comments are the guard against a future refactor
breaking the cross-process tests.

The error type, so `main.rs` can choose an exit code without matching on strings:

```rust
/// Why a queue command failed, in the shape `main.rs` maps to an exit code.
pub enum QueueError {
    /// Bad invocation, unknown session, unknown cassette, unregistered
    /// writer, or a kind mismatch. Exit 2.
    Usage(String),
    /// Another writer holds the cassette. Exit 3. Carries the rendered
    /// message so the wording lives next to the logic that produces it.
    Busy(String),
    /// Anything else. Exit 1.
    Io(String),
}
```

`main.rs`'s dispatch becomes:

```rust
    if let Some((id, session)) = &args.queue_write {
        let store = store::Store::new(store_root());
        let writer = match resolve_writer_name(args.writer.as_deref()) {
            Ok(w) => w,
            Err(msg) => die_with(2, &msg),
        };
        match queue::write(&store, id, session.as_deref(), &writer) {
            Ok(()) => std::process::exit(0),
            Err(queue::QueueError::Usage(m)) => die_with(2, &m),
            Err(queue::QueueError::Busy(m)) => die_with(3, &m),
            Err(queue::QueueError::Io(m)) => die_with(1, &m),
        }
    }
```

- [ ] **Step 2: Reduce `main.rs` to dispatch**

`main.rs` keeps `store_root`, `resolve_writer_name`, `die_with`, and the match that turns
a `QueueError` into an exit code.

- [ ] **Step 3: Verify nothing changed**

Run: `cargo test` — **all 5 `tests/lock.rs` tests must pass unmodified.** If any needs
changing, the move was not behaviour-preserving; fix the move, not the test.

Then run the cross-process tests repeatedly, since they are what would catch a reordering:

```bash
for i in $(seq 10); do cargo test --test lock || break; done
```

Report the result.

- [ ] **Step 4: Confirm the line count moved**

Run: `wc -l src/main.rs src/queue.rs`
Expected: `main.rs` meaningfully smaller; `queue.rs` holding the command.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/queue.rs
git commit -m "refactor: queue write moves to its own module"
```

---

## Verification

- [ ] `cargo test` — green; ~265 tests (249 + ~16 new).
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --check` — clean.
- [ ] `cargo test --test lock` passes 10 consecutive runs — the cross-process tests are
      what a bad refactor in Task 6 would break.
- [ ] `grep -rn 'unknown' src/main.rs src/queue.rs` shows no writer fallback.
- [ ] `grep -rnw 'write_cassette' src/` still shows only the test name — invariant 1 intact.
- [ ] Exit codes 4, 5 and 6 appear nowhere: `grep -rn 'die_with(4\|die_with(5\|die_with(6' src/`.
- [ ] An agent cannot claim a kind per-invocation: `--kind` appears only under
      `writer register` in `cassette --help` and `cassette writer register --help`.

## What this phase deliberately does not do

- **No `session` or `queue` commands beyond the existing `queue write`** — that is 4b.
- **No `--json`** — 4c, with the full contract and its derived fields.
- **No sticky lock**, no `queue lock`/`unlock`, no exit code 4 — 4c.
- **No `Store::holds` / reentrancy work** — Phase 5, where the TUI holds guards across
  time. A CLI command is a one-shot process holding nothing before it starts.
- **No interactive session picker** — Phase 5.
- **No repair path for an already-mis-registered writer.** Rejecting a mismatch is this
  phase; a `writer set-kind` command, if it proves needed, is a later decision. Nobody has
  a populated registry yet.
