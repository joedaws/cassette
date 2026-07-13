# `cassette find` Implementation Plan (issue #55)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `cassette find [TEXT…]` action that lists recent notes from the notes dir — newest first, with date, word count, draft marker, topics, and a first-line preview — optionally filtered by a case-insensitive substring.

**Architecture:** New `src/find.rs` mirroring `src/stats.rs`: pure `parse_entry`/`render` functions with unit tests, one thin I/O scanner. `main.rs` gains the `find` action in `parse_args` (remaining positionals join into the query) and an early-return branch beside `stats`.

**Tech Stack:** Rust, chrono (already a dependency). No new crates.

**Spec:** `docs/superpowers/specs/2026-07-13-find-command-design.md` — read it first; output formats there are normative.

## Global Constraints

- Plain text output, no color, exit 0 always (even for no matches).
- Non-recursive scan of `.md` files only, like `stats::scan_notes_dir`.
- Cap listing at 10 entries; overflow becomes one `… N more` line.
- Empty notes dir message must be exactly the stats one: `no notes yet — the first session starts the count`.
- Pure functions take data, not paths; only the scanner touches the filesystem.

---

### Task 1: `find.rs` — `NoteEntry` + `parse_entry`

**Files:**
- Create: `src/find.rs`
- Modify: `src/main.rs` (add `mod find;` next to `mod stats;`)

**Interfaces:**
- Consumes: `output::is_draft(content: &str) -> bool` (already `pub`).
- Produces:
  ```rust
  pub struct NoteEntry {
      pub name: String,            // file name including .md
      pub date: chrono::NaiveDateTime,
      pub words: usize,
      pub topics: Vec<String>,
      pub preview: String,         // "" when the note has no body text line
      pub draft: bool,
      haystack: String,            // lowercased name + content, for filtering
  }
  pub fn parse_entry(name: &str, content: &str, fallback: chrono::NaiveDateTime) -> NoteEntry
  ```

- [ ] **Step 1: Write the failing tests** — `src/find.rs` with the struct stubbed out is fine, but tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    const NOTE: &str = "---\n\
        date: 2026-07-13T09:12:00\n\
        word_count: 412\n\
        cassettes: 2\n\
        ---\n\
        # Cassette 1 — gratitude\n\n\
        ## Side A\n\n\
        woke up thinking about the demo\n\n\
        # Cassette 2 — priorities\n\n\
        ## Side A\n\n\
        ship the find command\n";

    #[test]
    fn parse_entry_reads_frontmatter_topics_and_preview() {
        let e = parse_entry("2026-07-13.md", NOTE, dt("2000-01-01T00:00:00"));
        assert_eq!(e.name, "2026-07-13.md");
        assert_eq!(e.date, dt("2026-07-13T09:12:00"));
        assert_eq!(e.words, 412);
        assert_eq!(e.topics, vec!["gratitude", "priorities"]);
        assert_eq!(e.preview, "woke up thinking about the demo");
        assert!(!e.draft);
    }

    #[test]
    fn parse_entry_draft_and_date_only() {
        let note = "---\ndate: 2026-07-11\ndraft: true\nword_count: 188\n---\n\
                    # Cassette 1\n\n## Side A\n\ncan't sleep again\n";
        let e = parse_entry("late-night.md", note, dt("2000-01-01T00:00:00"));
        assert!(e.draft);
        assert_eq!(e.date, dt("2026-07-11T00:00:00"), "date-only lands on midnight");
        assert!(e.topics.is_empty());
    }

    #[test]
    fn parse_entry_falls_back_to_mtime_and_skips_headings() {
        let e = parse_entry("loose.md", "# just a heading\n\nreal first line\n", dt("2026-07-01T08:00:00"));
        assert_eq!(e.date, dt("2026-07-01T08:00:00"), "no frontmatter date → fallback");
        assert_eq!(e.words, 0);
        assert_eq!(e.preview, "real first line", "headings never become previews");
    }

    #[test]
    fn parse_entry_truncates_long_previews() {
        let long = format!("---\ndate: 2026-07-13T09:12:00\n---\n{}\n", "x".repeat(100));
        let e = parse_entry("n.md", &long, dt("2000-01-01T00:00:00"));
        assert_eq!(e.preview.chars().count(), 73, "72 chars + ellipsis");
        assert!(e.preview.ends_with('…'));
    }

    #[test]
    fn parse_entry_no_body_text_means_no_preview() {
        let e = parse_entry("n.md", "---\ndate: 2026-07-13T09:12:00\n---\n# Cassette 1\n\n## Side A\n\n\n", dt("2000-01-01T00:00:00"));
        assert_eq!(e.preview, "");
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test find::` — expect compile errors (`parse_entry` not defined).

- [ ] **Step 3: Implement** `parse_entry`:

```rust
use chrono::NaiveDateTime;

use crate::output;

/// One saved note as `cassette find` shows it.
pub struct NoteEntry {
    pub name: String,
    pub date: NaiveDateTime,
    pub words: usize,
    pub topics: Vec<String>,
    pub preview: String,
    pub draft: bool,
    haystack: String,
}

const PREVIEW_CHARS: usize = 72;

/// Frontmatter `date:` with or without a time part; anything else is None.
fn parse_date(v: &str) -> Option<NaiveDateTime> {
    let v = v.trim();
    NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(v.get(..10)?, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
        })
}

/// Build a note's listing entry from its content; `fallback` (the file's
/// mtime) stands in when the frontmatter has no parseable date, so notes the
/// writer didn't produce still browse.
pub fn parse_entry(name: &str, content: &str, fallback: NaiveDateTime) -> NoteEntry {
    let mut date = None;
    let mut words = 0;
    let mut in_frontmatter = content.lines().next() == Some("---");
    let mut past_opening = false;
    let mut topics = Vec::new();
    let mut preview = String::new();
    for line in content.lines() {
        if in_frontmatter {
            if !past_opening {
                past_opening = true;
                continue;
            }
            if line == "---" {
                in_frontmatter = false;
            } else if let Some(v) = line.strip_prefix("date:") {
                date = parse_date(v);
            } else if let Some(v) = line.strip_prefix("word_count:") {
                words = v.trim().parse().unwrap_or(0);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("# Cassette") {
            if let Some((_, topic)) = rest.split_once(" — ") {
                topics.push(topic.trim().to_string());
            }
        } else if preview.is_empty() && !line.trim().is_empty() && !line.starts_with('#') {
            preview = truncate(line.trim(), PREVIEW_CHARS);
        }
    }
    NoteEntry {
        name: name.to_string(),
        date: date.unwrap_or(fallback),
        words,
        topics,
        preview,
        draft: output::is_draft(content),
        haystack: format!("{}\n{}", name, content).to_lowercase(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
```

Add `mod find;` in `main.rs` beside the other module declarations.

- [ ] **Step 4: Run** `cargo test find::` — expect all 5 tests PASS (plus full `cargo test` still green).

- [ ] **Step 5: Commit** — `git add src/find.rs src/main.rs && git commit -m "feat: find.rs note entry parsing"`.

---

### Task 2: `find.rs` — `render`

**Files:**
- Modify: `src/find.rs`

**Interfaces:**
- Consumes: `NoteEntry`, `parse_entry` from Task 1.
- Produces: `pub fn render(entries: &[NoteEntry], query: Option<&str>) -> String`.

- [ ] **Step 1: Write the failing tests** (append to the tests module; `entry` is a test helper):

```rust
    fn entry(name: &str, date: &str, words: usize) -> NoteEntry {
        parse_entry(
            name,
            &format!("---\ndate: {date}\nword_count: {words}\n---\n# Cassette 1\n\n## Side A\n\nbody of {name}\n"),
            dt("2000-01-01T00:00:00"),
        )
    }

    #[test]
    fn render_sorts_newest_first_with_footer() {
        let entries = [
            entry("old.md", "2026-07-01T08:00:00", 10),
            entry("new.md", "2026-07-13T09:12:00", 412),
        ];
        let out = render(&entries, None);
        let new_pos = out.find("new.md").unwrap();
        let old_pos = out.find("old.md").unwrap();
        assert!(new_pos < old_pos, "{out}");
        assert!(out.contains("2026-07-13 09:12    412 words  new.md"), "{out}");
        assert!(out.contains("    body of new.md"), "{out}");
        assert!(out.ends_with("resume one: cassette --resume <name>\n") || out.ends_with("resume one: cassette --resume <name>"), "{out}");
    }

    #[test]
    fn render_marks_drafts_and_topics() {
        let e = parse_entry(
            "d.md",
            "---\ndate: 2026-07-13T09:12:00\ndraft: true\nword_count: 5\n---\n# Cassette 1 — gratitude\n\n## Side A\n\nhi\n",
            dt("2000-01-01T00:00:00"),
        );
        let out = render(std::slice::from_ref(&e), None);
        assert!(out.contains("d.md (draft) — gratitude"), "{out}");
    }

    #[test]
    fn render_filters_case_insensitively() {
        let entries = [
            entry("morning.md", "2026-07-13T09:12:00", 10),
            entry("evening.md", "2026-07-12T21:00:00", 10),
        ];
        let out = render(&entries, Some("MORNING"));
        assert!(out.contains("morning.md"), "{out}");
        assert!(!out.contains("evening.md"), "{out}");
        assert_eq!(render(&entries, Some("zzz")), "no notes match 'zzz'");
    }

    #[test]
    fn render_caps_at_ten_with_more_line() {
        let entries: Vec<NoteEntry> = (1..=12)
            .map(|i| entry(&format!("n{i:02}.md"), &format!("2026-07-{i:02}T08:00:00"), 1))
            .collect();
        let out = render(&entries, None);
        assert!(out.contains("n12.md") && out.contains("n03.md"), "{out}");
        assert!(!out.contains("n02.md"), "{out}");
        assert!(out.contains("… 2 more — 'cassette find <text>' narrows the list"), "{out}");
    }

    #[test]
    fn render_empty_dir_message() {
        assert_eq!(render(&[], None), "no notes yet — the first session starts the count");
        assert_eq!(render(&[], Some("x")), "no notes yet — the first session starts the count");
    }
```

- [ ] **Step 2: Run** `cargo test find::` — expect FAIL (`render` not defined).

- [ ] **Step 3: Implement:**

```rust
const MAX_LISTED: usize = 10;

/// The plain-text `cassette find` listing: newest first, optionally filtered,
/// capped at `MAX_LISTED` with a "… N more" hint.
pub fn render(entries: &[NoteEntry], query: Option<&str>) -> String {
    if entries.is_empty() {
        return "no notes yet — the first session starts the count".into();
    }
    let mut matched: Vec<&NoteEntry> = match query {
        Some(q) => {
            let q = q.to_lowercase();
            entries.iter().filter(|e| e.haystack.contains(&q)).collect()
        }
        None => entries.iter().collect(),
    };
    if matched.is_empty() {
        return format!("no notes match '{}'", query.unwrap_or_default());
    }
    matched.sort_by(|a, b| b.date.cmp(&a.date));

    let mut out = String::new();
    for e in matched.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "{}  {:>5} words  {}",
            e.date.format("%Y-%m-%d %H:%M"),
            e.words,
            e.name
        ));
        if e.draft {
            out.push_str(" (draft)");
        }
        if !e.topics.is_empty() {
            out.push_str(&format!(" — {}", e.topics.join(", ")));
        }
        out.push('\n');
        if !e.preview.is_empty() {
            out.push_str(&format!("    {}\n", e.preview));
        }
    }
    if matched.len() > MAX_LISTED {
        out.push_str(&format!(
            "… {} more — 'cassette find <text>' narrows the list\n",
            matched.len() - MAX_LISTED
        ));
    }
    out.push_str("\nresume one: cassette --resume <name>");
    out
}
```

- [ ] **Step 4: Run** `cargo test find::` — all PASS.

- [ ] **Step 5: Commit** — `git commit -am "feat: find.rs listing renderer"`.

---

### Task 3: scanner + `main.rs` wiring + USAGE

**Files:**
- Modify: `src/find.rs` (add `scan_notes_dir`)
- Modify: `src/main.rs` (`Args`, `parse_args`, `main`, `USAGE`)

**Interfaces:**
- Consumes: `find::parse_entry`, `find::render`.
- Produces: `pub fn scan_notes_dir(dir: &Path) -> Vec<NoteEntry>`; `Args.find: Option<Vec<String>>` (`Some(vec![])` = unfiltered listing).

- [ ] **Step 1: Add the scanner** (I/O-thin, mirrors `stats::scan_notes_dir`; no unit test — its parts are covered):

```rust
use std::path::Path;

/// Read every `.md` note in the notes dir (non-recursive, like the writer),
/// using each file's mtime as the date fallback.
pub fn scan_notes_dir(dir: &Path) -> Vec<NoteEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| {
            let content = std::fs::read_to_string(e.path()).ok()?;
            let mtime: chrono::DateTime<chrono::Local> =
                e.metadata().and_then(|m| m.modified()).ok()?.into();
            let name = e.file_name().to_string_lossy().into_owned();
            Some(parse_entry(&name, &content, mtime.naive_local()))
        })
        .collect()
}
```

- [ ] **Step 2: Wire `parse_args`.** Add `find: Option<Vec<String>>` to `Args` (doc comment: `/// \`find\` with the query words that followed it; empty = list all.`), init `let mut find = None;`, and:

```rust
            "find" if !daily && !stats && find.is_none() && note_name.is_none() => {
                find = Some(Vec::new());
                i += 1;
            }
```

placed beside the `"stats"` arm. In the trailing positional arm, route extra args into the query instead of dying:

```rust
            arg => {
                if let Some(words) = &mut find {
                    words.push(arg.to_string());
                } else if note_name.is_some() || daily || stats {
                    die(&format!("unexpected extra argument '{arg}'"));
                } else {
                    note_name = Some(arg.to_string());
                }
                i += 1;
            }
```

Also guard the `"today"`/`"stats"` arms with `&& find.is_none()` so `cassette find today` treats `today` as a query word, and add `find` to the `Args { … }` literal.

- [ ] **Step 3: Wire `main`.** After the `args.stats` block:

```rust
    if let Some(words) = &args.find {
        let entries = notes_dir
            .as_deref()
            .map(find::scan_notes_dir)
            .unwrap_or_default();
        let query = (!words.is_empty()).then(|| words.join(" "));
        println!("{}", find::render(&entries, query.as_deref()));
        return Ok(());
    }
```

- [ ] **Step 4: USAGE.** Under `Actions:`, after the `stats` lines:

```
  find [TEXT]    list recent notes newest-first (date, words, topics,
                 first line); TEXT filters by name, topic, or content
```

- [ ] **Step 5: Verify.** `cargo test` all green; then against real notes:
  - `cargo run -- find` → listing, newest first, footer hint.
  - `cargo run -- find zzzznope` → `no notes match 'zzzznope'`.
  - `cargo run -- find some words` → multi-word query accepted.
  - `cargo run -- stats find` → dies with `unexpected extra argument 'find'` (order still matters, as with `today`).

- [ ] **Step 6: Commit** — `git commit -am "feat: cassette find lists recent notes"`.

---

### Task 4: docs + close-out

**Files:**
- Modify: `CLAUDE.md` (add `cargo run -- find` to Commands; one sentence about `find.rs` in the stats.rs paragraph area of Source layout)
- Modify: `README.md` (mirror the USAGE block near line 328; add a short `find` mention beside the `--resume` section around line 182)
- Delete: `docs/plans/issue-55-find-command.md` (this file — per CLAUDE.md plan lifecycle)

- [ ] **Step 1:** Update CLAUDE.md and README as above.
- [ ] **Step 2:** `cargo test && cargo clippy --all-targets -- -D warnings` — both clean.
- [ ] **Step 3:** `chainlink close 55` with a closing comment noting spec/plan locations and any deviations; delete this plan file in the same commit.
- [ ] **Step 4:** Commit — `git commit -am "docs: cassette find in README/CLAUDE.md; close issue 55"`.
