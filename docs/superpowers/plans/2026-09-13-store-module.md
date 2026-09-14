# Store Module and Data Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the on-disk session store — session directories, per-cassette
markdown files with frontmatter, a writer registry, ULID ids, slug naming, and
priority insertion — as pure functions plus a thin I/O layer, with no TUI
integration and no locking.

**Architecture:** A new `src/store/` module directory. Pure data and math
(`ids`, `meta`, `priority`) sit underneath a thin I/O layer (`session`,
`writers`, and `Store` in `mod.rs`) in the shape `stats.rs` and `find.rs`
already use. Every write goes through one `atomic_write` helper
(write-temp-then-`rename()` in the same directory). Nothing in `main.rs`,
`app.rs`, or `ui.rs` changes: the module is reachable and fully tested but not
yet wired into the running app. Phase 3 wraps the write calls in locks; Phase 4
puts a CLI on top; Phase 5 connects the TUI.

**Tech Stack:** Rust 2021, `ulid` 3.0 (ids), `serde` + `toml` 0.8 (session.toml,
writers.toml — both already dependencies), `chrono` (timestamps, already a
dependency), `tempfile` 3 (dev-only, store tests).

**Spec:** `docs/superpowers/specs/2026-09-13-session-store-design.md`

## Global Constraints

Copied verbatim from the spec. Every task's requirements implicitly include this
section.

- **Invariant 4:** "All writes are write-temp-then-`rename()` within the same
  directory." No task may call `fs::write` on a live store path directly.
- **The id is a tiebreak only.** "It is never load-bearing for ordering and must
  not be relied on to resolve creation order correctly 100% of the time." Sort
  by `priority` first; the id only settles what priority leaves equal.
- **Priority lives in frontmatter and nowhere else.** Never encode order in a
  filename.
- **The slug is derived from the topic at creation and never renamed
  afterward**; `topic` in frontmatter remains the source of truth for display.
- **Sparse priorities:** 10, 20, 30 … `--last` (the default) is max + 10;
  `--first` is min - 10, or half the minimum when that would reach zero.
- **No normalize-on-open pass.** Only a run with no integer gap left is
  renumbered, and only that run.
- Data dir is created **`0700`**.
- All dependencies are **pure Rust**; no C toolchain may be introduced.
- The `## Side A` / `## Side B` body format is unchanged.
- Open-cassette cap is **36** (`MAX_CASSETTES`), overridable later by `max_open`.

### Decisions taken during planning (not in the spec)

- **No legacy support and no migration path.** The flat `notes_dir` store is not
  carried forward, dual-read, or migrated by this plan. The user is the only
  user and will do a one-time migration separately if they want the old notes.
  Phase 6 deletes the old machinery outright.
- **Real 26-character ULIDs.** The spec's example ids (`01K5GR7T2M9WPD`) are
  14 characters, which is not a ULID — they are illustrative shorthand. This
  plan mints real `ulid` 3.0 values, 26 Crockford base32 characters.
- **`0700` is pulled forward from Phase 6 to this phase.** Creating the data dir
  world-readable now and tightening it later leaves a window where a private
  journal is readable by other local users. It costs one line here.
- **Phase 2 stores bodies as opaque strings.** Converting between a body and
  `Vec<Cassette>` is Phase 5's job; this phase must not depend on `app.rs` or
  `cassette.rs`.

## File Structure

| File | Responsibility |
|---|---|
| `src/store/mod.rs` (create) | `Store` (root + layout paths), `atomic_write`, `create_session`, `add_cassette`, `write_cassette`, `scan_session`; re-exports the submodules |
| `src/store/ids.rs` (create) | ULID minting, slug generation, cassette file naming and id recovery |
| `src/store/meta.rs` (create) | `CassetteMeta` + `Status`; frontmatter build/parse, body split |
| `src/store/priority.rs` (create) | Queue ordering and sparse priority insertion math |
| `src/store/session.rs` (create) | `SessionMeta` (`session.toml`) and the `active` pointer file |
| `src/store/writers.rs` (create) | `Writer`/`Kind`/`Writers` (`writers.toml`), `ensure_writer` |
| `src/main.rs` (modify) | One line: `mod store;` |
| `Cargo.toml` (modify) | Add `ulid = "3"`, `tempfile = "3"` (dev) |

Every submodule keeps its tests in a `#[cfg(test)] mod tests` block at the
bottom, matching `stats.rs`, `find.rs`, and `theme.rs`.

---

### Task 1: Dependencies, module skeleton, ids and slugs

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs` (add `mod store;` beside the existing `mod` lines)
- Create: `src/store/mod.rs`
- Create: `src/store/ids.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `store::ids::new_id() -> String` — a fresh 26-char ULID.
  - `store::ids::slug(topic: Option<&str>) -> String`
  - `store::ids::file_name(topic: Option<&str>, id: &str) -> String`
  - `store::ids::id_from_file_name(name: &str) -> Option<&str>`
  - `store::ids::SLUG_MAX: usize` = 32

- [ ] **Step 1: Add the dependencies**

```bash
cd /home/jozzef/Atelier/software/cassette
cargo add ulid@3
cargo add tempfile@3 --dev
```

Confirm `Cargo.toml` now has `ulid = "3"` under `[dependencies]` and
`tempfile = "3"` under `[dev-dependencies]`. Both are pure Rust.

- [ ] **Step 2: Create the module skeleton**

Create `src/store/mod.rs`:

```rust
//! The session store: session directories of per-cassette markdown files.
//!
//! Layout under the data dir (created `0700`):
//!
//! ```text
//! ~/.local/share/cassette/
//!   writers.toml
//!   active                       # single line: active session id
//!   sessions/
//!     <session ulid>/
//!       session.toml
//!       cassettes/
//!         <slug>-<cassette ulid>.md
//!       .locks/
//!         <cassette ulid>        # empty flock anchor (Phase 3)
//! ```
//!
//! Pure data and math live in `ids`, `meta` and `priority`; the thin I/O layer
//! is `session`, `writers`, and `Store` here. No locking yet — Phase 3 wraps
//! the write calls.

pub mod ids;
```

Add `mod store;` to `src/main.rs` alongside the other `mod` declarations.

- [ ] **Step 3: Write the failing tests for ids and slugs**

Create `src/store/ids.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_twenty_six_char_ulids_and_unique() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 26, "ULIDs are 26 Crockford base32 chars: {a}");
        assert!(
            a.chars().all(|c| c.is_ascii_alphanumeric()),
            "no separators in a ULID: {a}"
        );
        assert_ne!(a, b, "two mints must differ");
    }

    #[test]
    fn slug_lowercases_and_dashes_the_topic() {
        assert_eq!(slug(Some("Gratitude")), "gratitude");
        assert_eq!(slug(Some("loose thoughts")), "loose-thoughts");
        assert_eq!(slug(Some("What's next?!")), "what-s-next");
    }

    #[test]
    fn slug_collapses_runs_and_trims_edges() {
        assert_eq!(slug(Some("  --a   b--  ")), "a-b");
        assert_eq!(slug(Some("a///b")), "a-b");
    }

    #[test]
    fn slug_falls_back_when_nothing_survives() {
        // No topic, empty topic, and topics with no ASCII alphanumerics all
        // need a name a filesystem accepts.
        assert_eq!(slug(None), "cassette");
        assert_eq!(slug(Some("")), "cassette");
        assert_eq!(slug(Some("   ")), "cassette");
        assert_eq!(slug(Some("日本語")), "cassette");
        assert_eq!(slug(Some("!!!")), "cassette");
    }

    #[test]
    fn slug_is_capped_without_a_trailing_dash() {
        let long = "a".repeat(100);
        assert_eq!(slug(Some(&long)).len(), SLUG_MAX);
        // Cutting mid-run must not leave a dangling separator.
        let words = "aaaaaaaaaa ".repeat(10);
        let s = slug(Some(&words));
        assert!(s.len() <= SLUG_MAX, "{s}");
        assert!(!s.ends_with('-'), "{s}");
    }

    #[test]
    fn slug_cap_holds_when_a_word_boundary_straddles_it() {
        // Regression: a separator plus the char after it can step the length
        // from 31 to 33 in one iteration. An `== SLUG_MAX` check placed after
        // the push misses that and never fires again, uncapping the rest of
        // the topic. Any topic whose alnum run reaches 31 just before a
        // boundary reproduces it.
        let topic = format!("{} {}", "a".repeat(31), "c".repeat(100));
        let s = slug(Some(&topic));
        assert!(s.len() <= SLUG_MAX, "cap bypassed: {} chars — {s}", s.len());
        assert!(!s.ends_with('-'), "{s}");
    }

    #[test]
    fn file_name_joins_slug_and_id() {
        assert_eq!(
            file_name(Some("gratitude"), "01K5GR7T2M9WPD0000000000"),
            "gratitude-01K5GR7T2M9WPD0000000000.md"
        );
    }

    #[test]
    fn id_round_trips_through_the_file_name() {
        let id = new_id();
        let name = file_name(Some("loose thoughts"), &id);
        assert_eq!(id_from_file_name(&name), Some(id.as_str()));
    }

    #[test]
    fn id_recovery_handles_dashed_slugs_and_rejects_junk() {
        // The slug itself contains dashes, so recovery must take the LAST one.
        let id = "01K5GR7T2M9WPD0000000000AB";
        let name = format!("loose-thoughts-{id}.md");
        assert_eq!(id_from_file_name(&name), Some(id));
        assert_eq!(id_from_file_name("no-extension"), None);
        assert_eq!(id_from_file_name("nodash.md"), None);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test store::ids`
Expected: FAIL to compile — `cannot find function 'new_id' in this scope`.

- [ ] **Step 5: Implement ids and slugs**

Put this above the test module in `src/store/ids.rs`:

```rust
//! Cassette and session identity: ULIDs, and the slug half of a file name.

/// Longest slug allowed in a file name; the ULID and `.md` follow it.
pub const SLUG_MAX: usize = 32;

/// Used when a topic yields no usable slug characters.
const SLUG_FALLBACK: &str = "cassette";

/// A fresh ULID: 26 Crockford base32 characters, roughly sortable by
/// creation time. Roughly is enough — the id is only ever a tiebreak, never
/// an ordering guarantee (see the spec's "Identity and file naming").
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

/// The filename-safe half of a cassette file name, derived from its topic at
/// creation and never recomputed afterward: `topic` in frontmatter stays the
/// source of truth for display, so a renamed topic leaves the file where it
/// is. ASCII alphanumerics are kept lowercased, every other run becomes a
/// single `-`, and a topic that leaves nothing behind (absent, blank, or
/// entirely non-ASCII) falls back to `cassette`.
pub fn slug(topic: Option<&str>) -> String {
    let mut out = String::with_capacity(SLUG_MAX);
    let mut pending_dash = false;
    for ch in topic.unwrap_or_default().chars() {
        if !ch.is_ascii_alphanumeric() {
            pending_dash = true;
            continue;
        }
        // Check the budget BEFORE pushing, counting the separator this char
        // would drag in with it. Checking afterwards for an exact `== SLUG_MAX`
        // lets a dash+char pair step from 31 straight to 33, and since the
        // length only grows the cap can then never be hit again.
        let needed = if pending_dash && !out.is_empty() { 2 } else { 1 };
        if out.len() + needed > SLUG_MAX {
            break;
        }
        if pending_dash && !out.is_empty() {
            out.push('-');
        }
        pending_dash = false;
        out.push(ch.to_ascii_lowercase());
    }
    if out.is_empty() {
        return SLUG_FALLBACK.to_string();
    }
    out
}

/// `<slug>-<id>.md`. Priority is deliberately absent: encoding order in the
/// name would make every reprioritization a rename, and renames desynchronize
/// an flock from the path other writers resolve.
pub fn file_name(topic: Option<&str>, id: &str) -> String {
    format!("{}-{}.md", slug(topic), id)
}

/// Recover a cassette id from its file name — the segment after the LAST
/// dash, since slugs contain dashes of their own. `None` when the name is not
/// a `<slug>-<id>.md` pair.
pub fn id_from_file_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (_, id) = stem.rsplit_once('-')?;
    (!id.is_empty()).then_some(id)
}
```

Add `pub mod ids;` — already done in Step 2.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test store::ids`
Expected: PASS, 9 tests.

Then confirm nothing else broke:

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green; the existing 153 tests still pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/store/
git commit -m "feat: store module skeleton with ULID ids and topic slugs"
```

---

### Task 2: Cassette frontmatter

**Files:**
- Create: `src/store/meta.rs`
- Modify: `src/store/mod.rs` (add `pub mod meta;`)

**Interfaces:**
- Consumes: `store::ids::new_id` (tests only).
- Produces:
  - `store::meta::CassetteMeta { id, topic, priority, status, locked_by, created_by, last_writer, updated_at }`
  - `store::meta::Status::{Open, Closed}` with `as_str()` and `parse()`
  - `store::meta::build_frontmatter(&CassetteMeta) -> String`
  - `store::meta::parse_frontmatter(&str) -> Option<CassetteMeta>`
  - `store::meta::split(&str) -> (Option<CassetteMeta>, &str)` — meta plus body
  - `store::meta::now_utc() -> String` — RFC3339 seconds precision, `Z`

- [ ] **Step 1: Write the failing tests**

Create `src/store/meta.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> CassetteMeta {
        CassetteMeta {
            id: "01K5GR7T2M9WPD0000000000AB".to_string(),
            topic: Some("gratitude".to_string()),
            priority: 20,
            status: Status::Open,
            locked_by: None,
            created_by: "01K5H2WRITERID000000000000".to_string(),
            last_writer: "01K5H2WRITERID000000000000".to_string(),
            updated_at: "2026-09-13T14:02:11Z".to_string(),
        }
    }

    #[test]
    fn frontmatter_round_trips() {
        let m = meta();
        let parsed = parse_frontmatter(&build_frontmatter(&m)).expect("parses");
        assert_eq!(parsed, m);
    }

    #[test]
    fn frontmatter_is_delimited_and_ordered() {
        let text = build_frontmatter(&meta());
        assert!(text.starts_with("---\n"), "{text}");
        assert!(text.ends_with("---\n"), "{text}");
        assert!(text.contains("\nid: 01K5GR7T2M9WPD0000000000AB\n"), "{text}");
        assert!(text.contains("\npriority: 20\n"), "{text}");
        assert!(text.contains("\nstatus: open\n"), "{text}");
    }

    #[test]
    fn an_empty_lock_round_trips_as_none() {
        // The spec writes `locked_by:` with nothing after it when unlocked.
        let m = meta();
        let text = build_frontmatter(&m);
        assert!(text.contains("\nlocked_by:\n"), "{text}");
        assert_eq!(parse_frontmatter(&text).unwrap().locked_by, None);
    }

    #[test]
    fn a_held_lock_round_trips() {
        let mut m = meta();
        m.locked_by = Some("01K5H3AGENTID00000000000000".to_string());
        let parsed = parse_frontmatter(&build_frontmatter(&m)).unwrap();
        assert_eq!(parsed.locked_by.as_deref(), Some("01K5H3AGENTID00000000000000"));
    }

    #[test]
    fn a_missing_topic_round_trips_as_none() {
        let mut m = meta();
        m.topic = None;
        let parsed = parse_frontmatter(&build_frontmatter(&m)).unwrap();
        assert_eq!(parsed.topic, None);
    }

    #[test]
    fn a_topic_with_a_colon_survives() {
        // Topics are free user text; everything after the first `topic:` is
        // the value, so an embedded colon must not truncate it.
        let mut m = meta();
        m.topic = Some("re: yesterday".to_string());
        let parsed = parse_frontmatter(&build_frontmatter(&m)).unwrap();
        assert_eq!(parsed.topic.as_deref(), Some("re: yesterday"));
    }

    #[test]
    fn closed_status_round_trips_and_unknown_reads_open() {
        let mut m = meta();
        m.status = Status::Closed;
        assert_eq!(parse_frontmatter(&build_frontmatter(&m)).unwrap().status, Status::Closed);
        // A hand-edited file must not vanish from the queue.
        assert_eq!(Status::parse("nonsense"), Status::Open);
    }

    #[test]
    fn split_returns_meta_and_the_untouched_body() {
        let body = "## Side A\n\nhello\n\n## Side B\n\nscratch\n";
        let content = format!("{}\n{}", build_frontmatter(&meta()), body);
        let (parsed, rest) = split(&content);
        assert_eq!(parsed.unwrap().id, meta().id);
        assert_eq!(rest, body, "body must survive byte-for-byte");
    }

    #[test]
    fn content_without_frontmatter_is_all_body() {
        let (parsed, rest) = split("## Side A\n\nhello\n");
        assert!(parsed.is_none());
        assert_eq!(rest, "## Side A\n\nhello\n");
    }

    #[test]
    fn frontmatter_missing_required_fields_is_rejected() {
        // A file we cannot identify must not silently become a cassette
        // with an empty id that later collides.
        assert!(parse_frontmatter("---\ntopic: x\n---\n").is_none());
    }

    #[test]
    fn now_utc_is_rfc3339_zulu_seconds() {
        let t = now_utc();
        assert_eq!(t.len(), 20, "YYYY-MM-DDTHH:MM:SSZ — got {t}");
        assert!(t.ends_with('Z'), "{t}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::meta`
Expected: FAIL to compile — `cannot find type 'CassetteMeta' in this scope`.

- [ ] **Step 3: Implement the frontmatter**

Put this above the test module in `src/store/meta.rs`:

```rust
//! A cassette file's YAML frontmatter.
//!
//! Hand-rolled rather than serde_yaml: the repo already hand-parses
//! frontmatter in `output.rs` and `find.rs`, the field set is fixed and
//! small, and it keeps a YAML crate out of the dependency tree.

/// Whether a cassette is still in the queue or has been retired to the
/// collapsed closed row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Closed => "closed",
        }
    }

    /// Anything unrecognized reads as `Open`: a hand-edited typo should leave
    /// the cassette visible in the queue rather than silently hiding it.
    pub fn parse(s: &str) -> Status {
        match s.trim() {
            "closed" => Status::Closed,
            _ => Status::Open,
        }
    }
}

/// Everything about a cassette except its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CassetteMeta {
    pub id: String,
    /// Display name, and the source of truth for it — the file's slug is
    /// frozen at creation and may disagree after a retopic.
    pub topic: Option<String>,
    /// Sparse queue position (10, 20, 30 …). The only ordering mechanism.
    pub priority: i64,
    pub status: Status,
    /// Sticky lock: the writer id holding it, or `None`.
    pub locked_by: Option<String>,
    pub created_by: String,
    pub last_writer: String,
    /// RFC3339 UTC, seconds precision.
    pub updated_at: String,
}

/// `---` block in the spec's field order, always ending with a newline so a
/// body can be concatenated straight onto it.
pub fn build_frontmatter(m: &CassetteMeta) -> String {
    let mut s = String::with_capacity(256);
    s.push_str("---\n");
    s.push_str(&format!("id: {}\n", m.id));
    if let Some(topic) = &m.topic {
        s.push_str(&format!("topic: {topic}\n"));
    } else {
        s.push_str("topic:\n");
    }
    s.push_str(&format!("priority: {}\n", m.priority));
    s.push_str(&format!("status: {}\n", m.status.as_str()));
    s.push_str(&format!(
        "locked_by:{}\n",
        m.locked_by.as_deref().map(|w| format!(" {w}")).unwrap_or_default()
    ));
    s.push_str(&format!("created_by: {}\n", m.created_by));
    s.push_str(&format!("last_writer: {}\n", m.last_writer));
    s.push_str(&format!("updated_at: {}\n", m.updated_at));
    s.push_str("---\n");
    s
}

/// Parse the leading `---` block. `None` when there is no frontmatter or when
/// `id` is missing — an unidentifiable file must not become a cassette with an
/// empty id that collides with the next one.
pub fn parse_frontmatter(content: &str) -> Option<CassetteMeta> {
    let rest = content.strip_prefix("---\n")?;
    let (block, _) = rest.split_once("\n---")?;

    let mut id = None;
    let mut topic = None;
    let mut priority = 0i64;
    let mut status = Status::Open;
    let mut locked_by = None;
    let mut created_by = String::new();
    let mut last_writer = String::new();
    let mut updated_at = String::new();

    for line in block.lines() {
        // Split on the FIRST colon only: topics are free text and may contain
        // their own.
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "id" => id = (!value.is_empty()).then(|| value.to_string()),
            "topic" => topic = (!value.is_empty()).then(|| value.to_string()),
            "priority" => priority = value.parse().unwrap_or(0),
            "status" => status = Status::parse(value),
            "locked_by" => locked_by = (!value.is_empty()).then(|| value.to_string()),
            "created_by" => created_by = value.to_string(),
            "last_writer" => last_writer = value.to_string(),
            "updated_at" => updated_at = value.to_string(),
            _ => {}
        }
    }

    Some(CassetteMeta {
        id: id?,
        topic,
        priority,
        status,
        locked_by,
        created_by,
        last_writer,
        updated_at,
    })
}

/// Frontmatter plus the body after it, with the body byte-for-byte intact.
/// `build_frontmatter` ends in `---\n` and writers add one blank line, so that
/// blank line is consumed here and re-added on write.
pub fn split(content: &str) -> (Option<CassetteMeta>, &str) {
    let Some(meta) = parse_frontmatter(content) else {
        return (None, content);
    };
    let body = content
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(_, body)| body.strip_prefix('\n').unwrap_or(body))
        .unwrap_or("");
    (Some(meta), body)
}

/// `2026-09-13T14:02:11Z` — RFC3339, UTC, seconds precision.
pub fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
```

Add `pub mod meta;` to `src/store/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test store::meta`
Expected: PASS, 11 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "feat: cassette frontmatter model with round-trip parsing"
```

---

### Task 3: Queue ordering and priority insertion

**Files:**
- Create: `src/store/priority.rs`
- Modify: `src/store/mod.rs` (add `pub mod priority;`)

**Interfaces:**
- Consumes: `store::meta::{CassetteMeta, Status}`.
- Produces:
  - `store::priority::STEP: i64` = 10
  - `store::priority::last(existing: &[i64]) -> i64`
  - `store::priority::first(existing: &[i64]) -> Option<i64>`
  - `store::priority::between(lo: i64, hi: i64) -> Option<i64>`
  - `store::priority::renumber(count: usize) -> Vec<i64>`
  - `store::priority::queue_order(metas: &mut [CassetteMeta])`

`first` and `between` return `None` to mean "no integer gap left — renumber
this run"; Phase 4 is where that turns into taking locks and rewriting.

- [ ] **Step 1: Write the failing tests**

Create `src/store/priority.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};

    fn m(id: &str, priority: i64, status: Status) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: None,
            priority,
            status,
            locked_by: None,
            created_by: String::new(),
            last_writer: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn tail_placement_is_max_plus_a_step() {
        assert_eq!(last(&[10, 20, 30]), 40);
        // Out-of-order input must still append after the true maximum.
        assert_eq!(last(&[30, 10, 20]), 40);
    }

    #[test]
    fn tail_placement_on_an_empty_queue_is_the_first_step() {
        assert_eq!(last(&[]), STEP);
    }

    #[test]
    fn head_placement_is_min_minus_a_step() {
        assert_eq!(first(&[20, 30]), Some(10));
        assert_eq!(first(&[]), Some(STEP));
    }

    #[test]
    fn head_placement_halves_instead_of_reaching_zero() {
        // min - STEP would be <= 0, so halve the minimum instead.
        assert_eq!(first(&[10]), Some(5));
        assert_eq!(first(&[4]), Some(2));
        assert_eq!(first(&[3]), Some(1));
    }

    #[test]
    fn head_placement_gives_up_when_there_is_no_room_below() {
        assert_eq!(first(&[1]), None, "nothing fits below 1");
    }

    #[test]
    fn between_takes_the_midpoint() {
        assert_eq!(between(10, 20), Some(15));
        assert_eq!(between(15, 20), Some(17));
    }

    #[test]
    fn between_gives_up_on_adjacent_values() {
        assert_eq!(between(15, 16), None);
        assert_eq!(between(15, 15), None);
    }

    #[test]
    fn renumber_produces_a_sparse_run() {
        assert_eq!(renumber(3), vec![10, 20, 30]);
        assert_eq!(renumber(0), Vec::<i64>::new());
    }

    #[test]
    fn queue_order_puts_open_before_closed() {
        let mut v = vec![
            m("b", 10, Status::Closed),
            m("a", 20, Status::Open),
        ];
        queue_order(&mut v);
        assert_eq!(
            v.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
            "an open cassette outranks a closed one with a better priority"
        );
    }

    #[test]
    fn queue_order_sorts_by_priority_then_id() {
        let mut v = vec![
            m("z", 20, Status::Open),
            m("a", 20, Status::Open),
            m("m", 10, Status::Open),
        ];
        queue_order(&mut v);
        assert_eq!(
            v.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["m", "a", "z"],
            "priority first; the id only settles the tie"
        );
    }

    #[test]
    fn queue_order_is_stable_across_repeated_calls() {
        // The queue must not jitter between reads.
        let mut v = vec![
            m("b", 10, Status::Open),
            m("a", 10, Status::Open),
        ];
        queue_order(&mut v);
        let once: Vec<String> = v.iter().map(|c| c.id.clone()).collect();
        queue_order(&mut v);
        let twice: Vec<String> = v.iter().map(|c| c.id.clone()).collect();
        assert_eq!(once, twice);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::priority`
Expected: FAIL to compile — `cannot find function 'last' in this scope`.

- [ ] **Step 3: Implement ordering and insertion**

Put this above the test module in `src/store/priority.rs`:

```rust
//! Queue ordering and sparse priority insertion.
//!
//! Priorities are sparse (10, 20, 30 …) so most insertions are a single
//! frontmatter edit rather than a renumbering of the whole queue. `first` and
//! `between` return `None` when a run has no integer gap left; the caller
//! renumbers just that run, taking just those locks. There is no
//! normalize-on-open pass — it would rewrite every cassette.

use crate::store::meta::{CassetteMeta, Status};

/// The gap left between adjacent priorities.
pub const STEP: i64 = 10;

/// Tail placement — the default for a new cassette. An agent adding work
/// cannot jump the human's line.
pub fn last(existing: &[i64]) -> i64 {
    existing.iter().copied().max().unwrap_or(0) + STEP
}

/// Head placement. `min - STEP` normally; when that would reach zero, half
/// the minimum instead. `None` when even that leaves no room.
pub fn first(existing: &[i64]) -> Option<i64> {
    let Some(min) = existing.iter().copied().min() else {
        return Some(STEP);
    };
    if min - STEP > 0 {
        return Some(min - STEP);
    }
    let halved = min / 2;
    (halved > 0).then_some(halved)
}

/// Midpoint of two neighbours. `None` when they are adjacent or equal, which
/// means this run must be renumbered.
pub fn between(lo: i64, hi: i64) -> Option<i64> {
    let mid = lo + (hi - lo) / 2;
    (mid > lo && mid < hi).then_some(mid)
}

/// Fresh sparse priorities for a run that ran out of gaps.
pub fn renumber(count: usize) -> Vec<i64> {
    (1..=count as i64).map(|i| i * STEP).collect()
}

/// Queue order: open cassettes by priority, closed ones last, ties by id.
///
/// The id tiebreak is for stability, not truth — two cassettes minted in the
/// same millisecond sort arbitrarily with respect to each other, but they
/// sort the *same way* on every read, so the queue does not jitter.
pub fn queue_order(metas: &mut [CassetteMeta]) {
    metas.sort_by(|a, b| {
        let closed = |m: &CassetteMeta| m.status == Status::Closed;
        closed(a)
            .cmp(&closed(b))
            .then(a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });
}
```

Add `pub mod priority;` to `src/store/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test store::priority`
Expected: PASS, 11 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "feat: queue ordering and sparse priority insertion"
```

---

### Task 4: `session.toml` and the active-session pointer

**Files:**
- Create: `src/store/session.rs`
- Modify: `src/store/mod.rs` (add `pub mod session;`)

**Interfaces:**
- Consumes: `store::atomic_write` — **not yet written** (Task 6). Until then
  this task uses `std::fs::write` and Task 6 swaps it. That swap is an explicit
  step in Task 6, not an oversight here.
- Produces:
  - `store::session::SessionMeta { alias, created, timer_secs, word_goal }`
  - `store::session::read(path: &Path) -> io::Result<SessionMeta>`
  - `store::session::write(path: &Path, m: &SessionMeta) -> io::Result<()>`
  - `store::session::read_active(root: &Path) -> Option<String>`
  - `store::session::write_active(root: &Path, id: &str) -> io::Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `src/store/session.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_toml_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        let m = SessionMeta {
            alias: Some("freewriting-2026-09-13".to_string()),
            created: "2026-09-13T09:25:57Z".to_string(),
            timer_secs: Some(600),
            word_goal: Some(500),
        };
        write(&path, &m).expect("write");
        assert_eq!(read(&path).expect("read"), m);
    }

    #[test]
    fn optional_fields_are_omitted_not_nulled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        let m = SessionMeta {
            alias: None,
            created: "2026-09-13T09:25:57Z".to_string(),
            timer_secs: None,
            word_goal: None,
        };
        write(&path, &m).expect("write");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(!text.contains("alias"), "absent keys stay absent: {text}");
        assert!(!text.contains("timer_secs"), "{text}");
        assert_eq!(read(&path).expect("read"), m);
    }

    #[test]
    fn a_session_with_only_created_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "created = \"2026-09-13T09:25:57Z\"\n").expect("write");
        let m = read(&path).expect("read");
        assert_eq!(m.created, "2026-09-13T09:25:57Z");
        assert_eq!(m.alias, None);
    }

    #[test]
    fn active_pointer_round_trips_and_ignores_whitespace() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_active(dir.path()), None, "no pointer yet");
        write_active(dir.path(), "01K5GQ2R8V3XQZ0000000000AB").expect("write");
        assert_eq!(
            read_active(dir.path()).as_deref(),
            Some("01K5GQ2R8V3XQZ0000000000AB")
        );
        // A hand-edited file with a trailing newline or spaces still resolves.
        std::fs::write(dir.path().join("active"), "  01K5ZZ  \n\n").expect("write");
        assert_eq!(read_active(dir.path()).as_deref(), Some("01K5ZZ"));
    }

    #[test]
    fn an_empty_active_file_is_no_active_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("active"), "\n").expect("write");
        assert_eq!(read_active(dir.path()), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::session`
Expected: FAIL to compile — `cannot find type 'SessionMeta' in this scope`.

- [ ] **Step 3: Implement the session metadata**

Put this above the test module in `src/store/session.rs`:

```rust
//! `session.toml` and the `active` pointer.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name of the pointer holding the active session id.
pub const ACTIVE_FILE: &str = "active";

/// A session's own metadata. The id is the directory name, not a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMeta {
    /// Human-facing name; the id is displayed when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// RFC3339 UTC.
    pub created: String,
    /// Session defaults, mirroring the `-t` and `-w` flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_goal: Option<usize>,
}

pub fn read(path: &Path) -> io::Result<SessionMeta> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(path: &Path, m: &SessionMeta) -> io::Result<()> {
    let text =
        toml::to_string(m).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, text)
}

fn active_path(root: &Path) -> PathBuf {
    root.join(ACTIVE_FILE)
}

/// The active session id, or `None` when the pointer is missing or blank.
pub fn read_active(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(active_path(root)).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

pub fn write_active(root: &Path, id: &str) -> io::Result<()> {
    std::fs::write(active_path(root), format!("{id}\n"))
}
```

Add `pub mod session;` to `src/store/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test store::session`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "feat: session.toml and the active-session pointer"
```

---

### Task 5: The writer registry

**Files:**
- Create: `src/store/writers.rs`
- Modify: `src/store/mod.rs` (add `pub mod writers;`)

**Interfaces:**
- Consumes: `store::ids::new_id`, `store::meta::now_utc`.
- Produces:
  - `store::writers::Kind::{Human, Agent}`
  - `store::writers::Writer { name, kind, created }`
  - `store::writers::Writers { writers: BTreeMap<String, Writer> }` with
    `find_by_name(&str) -> Option<&str>`
  - `store::writers::read(root: &Path) -> io::Result<Writers>`
  - `store::writers::write(root: &Path, w: &Writers) -> io::Result<()>`
  - `store::writers::ensure(root: &Path, name: &str, kind: Kind) -> io::Result<String>`

`BTreeMap` rather than `HashMap`: the file is rewritten in full on every
change, and a stable key order keeps the diff readable and the tests
deterministic.

- [ ] **Step 1: Write the failing tests**

Create `src/store/writers.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writers_toml_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = Writers::default();
        w.writers.insert(
            "01K5H2WRITERID000000000000".to_string(),
            Writer {
                name: "joseph".to_string(),
                kind: Kind::Human,
                created: "2026-09-13T09:20:00Z".to_string(),
            },
        );
        write(dir.path(), &w).expect("write");
        assert_eq!(read(dir.path()).expect("read"), w);
    }

    #[test]
    fn a_missing_registry_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read(dir.path()).expect("read").writers.is_empty());
    }

    #[test]
    fn an_unreadable_registry_errors_rather_than_reading_as_empty() {
        // Only NotFound may mean "empty". Any other read failure must
        // propagate: reporting an empty registry would let the next `ensure`
        // overwrite a real one, losing every writer id in the store. A
        // directory where the file should be is the portable way to make
        // `read_to_string` fail for a reason other than NotFound.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(WRITERS_FILE)).expect("mkdir");
        assert!(
            read(dir.path()).is_err(),
            "an unreadable registry must not read as empty"
        );
    }

    #[test]
    fn kind_serializes_lowercase() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = Writers::default();
        w.writers.insert(
            "01K5H3AGENTID00000000000000".to_string(),
            Writer {
                name: "refactor-agent".to_string(),
                kind: Kind::Agent,
                created: "2026-09-13T09:22:00Z".to_string(),
            },
        );
        write(dir.path(), &w).expect("write");
        let text = std::fs::read_to_string(dir.path().join(WRITERS_FILE)).expect("read");
        assert!(text.contains("kind = \"agent\""), "{text}");
    }

    #[test]
    fn ensure_creates_once_and_then_reuses_the_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        let again = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        assert_eq!(first, again, "the same name must not mint a second id");
        assert_eq!(read(dir.path()).expect("read").writers.len(), 1);
    }

    #[test]
    fn ensure_distinguishes_different_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let human = ensure(dir.path(), "joseph", Kind::Human).expect("ensure");
        let agent = ensure(dir.path(), "refactor-agent", Kind::Agent).expect("ensure");
        assert_ne!(human, agent);
        let all = read(dir.path()).expect("read");
        assert_eq!(all.writers.len(), 2);
        assert_eq!(all.writers[&agent].kind, Kind::Agent);
    }

    #[test]
    fn find_by_name_locates_an_existing_writer() {
        let mut w = Writers::default();
        w.writers.insert(
            "id-1".to_string(),
            Writer {
                name: "joseph".to_string(),
                kind: Kind::Human,
                created: String::new(),
            },
        );
        assert_eq!(w.find_by_name("joseph"), Some("id-1"));
        assert_eq!(w.find_by_name("nobody"), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::writers`
Expected: FAIL to compile — `cannot find type 'Writers' in this scope`.

- [ ] **Step 3: Implement the registry**

Put this above the test module in `src/store/writers.rs`:

```rust
//! `writers.toml` — who is allowed to appear in `created_by` / `last_writer`.
//!
//! Attribution is cooperative: this registry names writers, it does not
//! authenticate them. OS-level enforcement is explicitly deferred in the spec.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::ids;
use crate::store::meta;

/// File name of the registry, at the store root.
pub const WRITERS_FILE: &str = "writers.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writer {
    pub name: String,
    pub kind: Kind,
    /// RFC3339 UTC.
    pub created: String,
}

/// The whole registry: writer id → writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Writers {
    #[serde(default)]
    pub writers: BTreeMap<String, Writer>,
}

impl Writers {
    /// The id registered under `name`, if any.
    pub fn find_by_name(&self, name: &str) -> Option<&str> {
        self.writers
            .iter()
            .find(|(_, w)| w.name == name)
            .map(|(id, _)| id.as_str())
    }
}

/// A missing registry is an empty one — the first writer creates it. Every
/// OTHER read failure propagates: `read_to_string` also errors on
/// permission-denied, on a directory, and on non-UTF-8 content, and treating
/// those as "empty" is a data-loss path — the next `ensure` would write a
/// fresh single-entry registry over a file that was merely unreadable,
/// destroying every existing writer id.
pub fn read(root: &Path) -> io::Result<Writers> {
    let path = root.join(WRITERS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Writers::default()),
        Err(e) => return Err(e),
    };
    toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(root: &Path, w: &Writers) -> io::Result<()> {
    let text =
        toml::to_string(w).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(root.join(WRITERS_FILE), text)
}

/// The id for `name`, registering it on first sight. Idempotent: calling it
/// twice with the same name returns the same id rather than minting a second.
pub fn ensure(root: &Path, name: &str, kind: Kind) -> io::Result<String> {
    let mut all = read(root)?;
    if let Some(id) = all.find_by_name(name) {
        return Ok(id.to_string());
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

Add `pub mod writers;` to `src/store/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test store::writers`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add src/store/
git commit -m "feat: writers.toml registry with idempotent registration"
```

---

### Task 6: `Store` — layout, atomic writes, create and scan

**Files:**
- Modify: `src/store/mod.rs` (the `Store` type and its I/O)
- Modify: `src/store/session.rs` (swap `fs::write` for `atomic_write`)
- Modify: `src/store/writers.rs` (swap `fs::write` for `atomic_write`)

**Interfaces:**
- Consumes: every earlier task.
- Produces:
  - `store::atomic_write(path: &Path, contents: &str) -> io::Result<()>`
  - `store::Store { root: PathBuf }` with `new`, `default_root`,
    `sessions_dir`, `session_dir`, `cassettes_dir`, `locks_dir`
  - `store::Store::create_session(&self, &SessionMeta) -> io::Result<String>`
  - `store::Store::add_cassette(&self, session: &str, &CassetteMeta, body: &str) -> io::Result<PathBuf>`
  - `store::Store::write_cassette(&self, path: &Path, &CassetteMeta, body: &str) -> io::Result<()>`
  - `store::Store::scan_session(&self, session: &str) -> io::Result<Vec<StoredCassette>>`
  - `store::StoredCassette { path, meta, body }`

- [ ] **Step 1: Write the failing tests**

Add this test module at the bottom of `src/store/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::session::SessionMeta;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn cassette_meta(id: &str, priority: i64) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: Some("gratitude".to_string()),
            priority,
            status: Status::Open,
            locked_by: None,
            created_by: "writer-1".to_string(),
            last_writer: "writer-1".to_string(),
            updated_at: meta::now_utc(),
        }
    }

    fn session_meta() -> SessionMeta {
        SessionMeta {
            alias: None,
            created: meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        }
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let (dir, _s) = store();
        let path = dir.path().join("f.txt");
        atomic_write(&path, "hello").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "f.txt")
            .collect();
        assert!(strays.is_empty(), "temp files left behind: {strays:?}");
    }

    #[test]
    fn atomic_write_replaces_existing_content_whole() {
        let (dir, _s) = store();
        let path = dir.path().join("f.txt");
        atomic_write(&path, "first").expect("write");
        atomic_write(&path, "second").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn create_session_builds_the_directory_layout() {
        let (_dir, s) = store();
        let id = s.create_session(&session_meta()).expect("create");
        assert_eq!(id.len(), 26);
        assert!(s.session_dir(&id).is_dir());
        assert!(s.cassettes_dir(&id).is_dir());
        assert!(s.locks_dir(&id).is_dir(), ".locks must exist before Phase 3");
        assert!(s.session_dir(&id).join("session.toml").is_file());
    }

    #[test]
    fn create_session_reads_its_metadata_back() {
        let (_dir, s) = store();
        let m = SessionMeta {
            alias: Some("morning".to_string()),
            ..session_meta()
        };
        let id = s.create_session(&m).expect("create");
        let read_back = session::read(&s.session_dir(&id).join("session.toml")).expect("read");
        assert_eq!(read_back.alias.as_deref(), Some("morning"));
    }

    #[test]
    fn add_cassette_names_the_file_by_slug_and_id() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let path = s.add_cassette(&sid, &m, "## Side A\n\nhello\n").expect("add");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "gratitude-01K5GR7T2M9WPD0000000000AB.md"
        );
    }

    #[test]
    fn a_cassette_round_trips_through_the_store() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let body = "## Side A\n\nhello\n\n## Side B\n\nscratch\n";
        s.add_cassette(&sid, &m, body).expect("add");

        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].meta, m);
        assert_eq!(found[0].body, body, "body must survive byte-for-byte");
    }

    #[test]
    fn scan_returns_cassettes_in_queue_order() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut closed = cassette_meta("aaa00000000000000000000000", 5);
        closed.status = Status::Closed;
        s.add_cassette(&sid, &cassette_meta("ccc00000000000000000000000", 30), "")
            .expect("add");
        s.add_cassette(&sid, &closed, "").expect("add");
        s.add_cassette(&sid, &cassette_meta("bbb00000000000000000000000", 10), "")
            .expect("add");

        let ids: Vec<String> = s
            .scan_session(&sid)
            .expect("scan")
            .into_iter()
            .map(|c| c.meta.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "bbb00000000000000000000000",
                "ccc00000000000000000000000",
                "aaa00000000000000000000000"
            ],
            "open by priority, closed last — even with the best priority"
        );
    }

    #[test]
    fn scan_ignores_non_cassette_files() {
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        s.add_cassette(&sid, &cassette_meta("aaa00000000000000000000000", 10), "")
            .expect("add");
        // An editor swap file and a file with no frontmatter must not appear.
        std::fs::write(s.cassettes_dir(&sid).join("notes.txt"), "stray").expect("write");
        std::fs::write(s.cassettes_dir(&sid).join("broken.md"), "no frontmatter\n")
            .expect("write");
        assert_eq!(s.scan_session(&sid).expect("scan").len(), 1);
    }

    #[test]
    fn scanning_a_missing_session_is_empty_not_an_error() {
        let (_dir, s) = store();
        assert!(s.scan_session("nope").expect("scan").is_empty());
    }

    #[test]
    fn write_cassette_updates_in_place_without_renaming() {
        // The slug is frozen at creation: retopicking must not move the file,
        // because an flock is held on the inode another writer resolved.
        let (_dir, s) = store();
        let sid = s.create_session(&session_meta()).expect("create");
        let mut m = cassette_meta("01K5GR7T2M9WPD0000000000AB", 10);
        let path = s.add_cassette(&sid, &m, "old\n").expect("add");

        m.topic = Some("completely different".to_string());
        s.write_cassette(&path, &m, "new\n").expect("write");

        assert!(path.is_file(), "the file must not have been renamed");
        let found = s.scan_session(&sid).expect("scan");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].meta.topic.as_deref(), Some("completely different"));
        assert_eq!(found[0].body, "new\n");
        assert_eq!(
            found[0].path.file_name().unwrap().to_string_lossy(),
            "gratitude-01K5GR7T2M9WPD0000000000AB.md",
            "the slug stays as minted"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_data_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("store");
        let s = Store::new(root.clone());
        s.create_session(&session_meta()).expect("create");
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "a private journal is not world-readable");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test store::tests`
Expected: FAIL to compile — `cannot find type 'Store' in this scope`.

- [ ] **Step 3: Implement `atomic_write` and `Store`**

Add to `src/store/mod.rs`, above the test module and below the existing `pub mod` lines:

```rust
use std::io;
use std::path::{Path, PathBuf};

use crate::store::meta::CassetteMeta;
use crate::store::session::SessionMeta;

/// Directory holding session directories, under the store root.
pub const SESSIONS_DIR: &str = "sessions";
/// Per-session directory of cassette files.
pub const CASSETTES_DIR: &str = "cassettes";
/// Per-session directory of flock anchors (Phase 3 opens these; this phase
/// only creates the directory so the layout is complete).
pub const LOCKS_DIR: &str = ".locks";

/// Write via a temp file in the same directory, then `rename()` over the
/// target. Spec invariant 4: a reader either sees the old file whole or the
/// new one whole, never a half-written mix. Same directory matters — `rename`
/// is only atomic within a filesystem.
pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // The temp name carries a ULID so two writers never collide on it.
    let tmp = dir.join(format!(".tmp-{}", ids::new_id()));
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Never leave a stray temp file behind on failure.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// One cassette as it exists on disk.
#[derive(Debug, Clone)]
pub struct StoredCassette {
    pub path: PathBuf,
    pub meta: CassetteMeta,
    /// Everything after the frontmatter, byte-for-byte.
    pub body: String,
}

/// The store rooted at a data dir. Holds no state beyond the path: every
/// method reads or writes the filesystem directly, which is what makes
/// concurrent writers possible.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    /// `~/.local/share/cassette` — the `data_dir` config key overrides it.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("cassette"))
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join(SESSIONS_DIR)
    }

    pub fn session_dir(&self, session: &str) -> PathBuf {
        self.sessions_dir().join(session)
    }

    pub fn cassettes_dir(&self, session: &str) -> PathBuf {
        self.session_dir(session).join(CASSETTES_DIR)
    }

    pub fn locks_dir(&self, session: &str) -> PathBuf {
        self.session_dir(session).join(LOCKS_DIR)
    }

    /// Create the store root `0700` if it is not there yet. A freewriting
    /// journal is private by default; tightening it later would leave a
    /// window where other local users can read it.
    fn ensure_root(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&self.root)?.permissions();
            if perms.mode() & 0o777 != 0o700 {
                perms.set_mode(0o700);
                std::fs::set_permissions(&self.root, perms)?;
            }
        }
        Ok(())
    }

    /// Mint a session id, build its directory layout, write `session.toml`,
    /// and return the id.
    pub fn create_session(&self, m: &SessionMeta) -> io::Result<String> {
        self.ensure_root()?;
        let id = ids::new_id();
        std::fs::create_dir_all(self.cassettes_dir(&id))?;
        std::fs::create_dir_all(self.locks_dir(&id))?;
        session::write(&self.session_dir(&id).join("session.toml"), m)?;
        Ok(id)
    }

    /// Create a new cassette file, named `<slug>-<id>.md` from the topic at
    /// creation. Returns the path, which callers keep: the name is never
    /// recomputed, even when the topic changes.
    pub fn add_cassette(
        &self,
        session: &str,
        m: &CassetteMeta,
        body: &str,
    ) -> io::Result<PathBuf> {
        let path = self
            .cassettes_dir(session)
            .join(ids::file_name(m.topic.as_deref(), &m.id));
        self.write_cassette(&path, m, body)?;
        Ok(path)
    }

    /// Overwrite a cassette in place. Deliberately takes the path rather than
    /// deriving it: the file keeps the slug it was minted with, so a retopic
    /// updates frontmatter without a rename.
    pub fn write_cassette(
        &self,
        path: &Path,
        m: &CassetteMeta,
        body: &str,
    ) -> io::Result<()> {
        atomic_write(path, &format!("{}\n{}", meta::build_frontmatter(m), body))
    }

    /// Every cassette in a session, in queue order. A missing session, files
    /// that are not `.md`, and `.md` files without parseable frontmatter are
    /// all skipped rather than erroring — the store shares a directory with
    /// editors and their swap files.
    pub fn scan_session(&self, session: &str) -> io::Result<Vec<StoredCassette>> {
        let dir = self.cassettes_dir(session);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (Some(meta), body) = meta::split(&content) else {
                continue;
            };
            found.push(StoredCassette {
                path,
                meta,
                body: body.to_string(),
            });
        }
        let mut metas: Vec<CassetteMeta> = found.iter().map(|c| c.meta.clone()).collect();
        priority::queue_order(&mut metas);
        let order: Vec<&str> = metas.iter().map(|m| m.id.as_str()).collect();
        found.sort_by_key(|c| {
            order
                .iter()
                .position(|id| *id == c.meta.id)
                .unwrap_or(usize::MAX)
        });
        Ok(found)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test store::tests`
Expected: PASS, 11 tests.

- [ ] **Step 5: Route the remaining writes through `atomic_write`**

Task 4 and Task 5 used `std::fs::write` because `atomic_write` did not exist
yet. Swap them now so invariant 4 holds everywhere.

In `src/store/session.rs`, replace the body of `write`:

```rust
pub fn write(path: &Path, m: &SessionMeta) -> io::Result<()> {
    let text =
        toml::to_string(m).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::atomic_write(path, &text)
}
```

and `write_active`:

```rust
pub fn write_active(root: &Path, id: &str) -> io::Result<()> {
    crate::store::atomic_write(&active_path(root), &format!("{id}\n"))
}
```

In `src/store/writers.rs`, replace the body of `write`:

```rust
pub fn write(root: &Path, w: &Writers) -> io::Result<()> {
    let text =
        toml::to_string(w).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::atomic_write(&root.join(WRITERS_FILE), &text)
}
```

- [ ] **Step 6: Verify no direct writes remain in the store**

Test modules legitimately write fixture files directly, so a plain
`grep -rn 'fs::write' src/store/` is the wrong check — it returns the fixture
writes too and will never reach one hit. Check the production side only, by
reading each file up to its `#[cfg(test)]` marker:

```bash
for f in src/store/*.rs; do
  awk '/#\[cfg\(test\)\]/{exit} /fs::write/{print FILENAME":"FNR": "$0}' "$f"
done
```

Expected: exactly one line — the `std::fs::write(&tmp, contents)` inside
`atomic_write` in `src/store/mod.rs`. Any other line is a production write
bypassing the atomic path; route it through `atomic_write`. Before Step 5 this
same command prints the two `session.rs` writes and the one in `writers.rs`,
which is how you know it is looking in the right place.

- [ ] **Step 7: Run the whole suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green. The store adds 54 tests; the existing 153 still pass.

- [ ] **Step 8: Commit**

```bash
git add src/store/
git commit -m "feat: store layout, atomic writes, session creation and scanning"
```

---

## Verification

After Task 6, confirm the phase's own acceptance criteria:

- [ ] `cargo test` — 207 tests pass (153 existing + 54 new).
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --check` — clean.
- [ ] The production-write check from Task 6 Step 6 prints exactly one line —
      the `std::fs::write(&tmp, contents)` inside `atomic_write`. (A plain
      `grep -rn 'fs::write' src/store/` still shows the test-fixture writes;
      those are expected and correct.)
- [ ] `grep -rn 'store::' src/main.rs src/app.rs src/ui.rs` returns only the
      `mod store;` declaration — the phase must not have wired itself into the
      running app.
- [ ] Running `cargo run -- new /tmp/smoke.md` still behaves exactly as it did
      before this phase: the store is dormant.

## What this phase deliberately does not do

Each of these belongs to a later phase; a reviewer should reject them if they
appear here.

- **No locking.** `.locks/<id>` directories are created but never opened.
  `locked_by` is carried in frontmatter as data only.
- **No CLI.** No `session`, `queue`, or `writer` subcommands.
- **No TUI integration**, no `data_dir` config key, no `Cassette` conversion.
- **No legacy migration.** The flat notes store is untouched and unread.
