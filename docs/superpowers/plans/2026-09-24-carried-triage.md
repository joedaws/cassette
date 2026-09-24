# Carried Triage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear four small carried-triage items (a stale `allow(dead_code)`, an mtime-only sync check that can miss writes, an inactive-side cursor asymmetry, a duplicated name lookup) and measure one (picker scan cost).

**Architecture:** Independent, small changes, one per task, each ending green. No new modules. Task 2 is the only behaviour change a user could notice: live sync stops missing writes that land inside one mtime tick.

**Tech Stack:** Rust; `cargo test`, `cargo clippy --all-targets -- -D warnings`.

**Spec:** `docs/superpowers/specs/2026-09-24-carried-triage-design.md`

## Global Constraints

- No `#[allow(dead_code)]` anywhere in `src/` when done.
- The stat pass in `sync_external_writes` must stay stat-only: no file content read to decide "changed".
- Clippy `-D warnings` green after every task.
- Do not remove items 5 and 6 from `docs/follow-up.md`; add findings to them.

## Review Focus

- **Two writes inside one mtime tick with the same length** — the inode must still differ. Task 2's unit test forces exactly this (same-length rewrite, mtime set back).
- **The held cassette's own autosave** — its stamp changes every flush now (new inode each `atomic_write`); it must still be recorded and skipped, not merged. Task 2 keeps the held-id branch unchanged and the existing sync tests cover it.
- **Non-unix build** — `ChangeStamp` must compile without `ino`. Task 2 gates the field with `#[cfg(unix)]`; run `cargo check --target x86_64-pc-windows-gnu` if that target is installed, otherwise note it as unverified.
- **Side-B merge with the side-A cursor mid-text before the merge** — lands at the end of the new side A. Task 3 test.
- **`store.writers()` failing inside `write_permitted`** — message still shows the raw id. Task 4 keeps the `.ok()` fallback.

---

### Task 1: Remove the store's `allow(dead_code)` and the three items it hid

**Files:**
- Modify: `src/store/mod.rs` (lines 1–5: the comment and `#![allow(dead_code)]`; `StoredCassette.path` ~line 203 and wherever `scan_session` constructs it; the slug test ~line 1093)
- Modify: `src/store/lock.rs` (delete `pub fn path` ~line 298)
- Modify: `src/store/meta.rs` (delete `parse_frontmatter` ~line 153–157; rewrite its test call sites)
- Modify: `src/store/priority.rs` (doc comments at ~lines 20 and 112 name `parse_frontmatter`: say `meta::split`)

- [ ] **Step 1: Remove the allow and see the failure**

Delete the first five lines of `src/store/mod.rs` (the four-line `// Nothing in the non-test build…` comment and `#![allow(dead_code)]`).

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | grep -A1 '^error'`
Expected: exactly three errors — `field 'path' is never read` (`store/mod.rs`), `method 'path' is never used` (`store/lock.rs`), `function 'parse_frontmatter' is never used` (`store/meta.rs`). If there are others, the code moved since the spec; handle each the same way (delete if nothing reads it) and note it in the commit message.

- [ ] **Step 2: Delete `LockGuard::path`**

Remove from `src/store/lock.rs`:
```rust
    pub fn path(&self) -> &Path {
        &self.path
    }
```
Keep the `path` *field* — `read`/`write` use it. Drop a now-unused `Path` import if clippy flags it.

- [ ] **Step 3: Delete `parse_frontmatter`; tests use `split`**

Remove from `src/store/meta.rs`:
```rust
/// Parse the leading `---` block. `None` when there is no well-formed
/// frontmatter or when `id` is missing.
pub fn parse_frontmatter(content: &str) -> Option<CassetteMeta> {
    parse_block(split_parts(content)?.0)
}
```
In `meta.rs`'s tests, replace every `parse_frontmatter(X)` with `split(X).0`. E.g.
```rust
        let parsed = split(&build_frontmatter(&m)).0.expect("parses");
        assert_eq!(split(&text).0.unwrap().locked_by, None);
        assert!(split("---\ntopic: x\n---\n").0.is_none());
```
`split` returns `(None, content)` in exactly the cases `parse_frontmatter` returned `None` (no block, or `parse_block` fails), so every assertion keeps its meaning.

- [ ] **Step 4: Delete `StoredCassette.path`**

Remove `pub path: PathBuf,` from `StoredCassette` and the `path` initialiser in `scan_session`'s construction of it (`grep -n "StoredCassette {" src/store`). Rewrite the slug assertion (~line 1093) to ask the store:
```rust
        assert_eq!(
            s.cassette_path(&sid, "01K5GR7T2M9WPD0000000000AB")
                .expect("io")
                .expect("exists")
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "gratitude-01K5GR7T2M9WPD0000000000AB.md",
            "the slug stays as minted"
        );
```
Any other construction site of `StoredCassette` (`grep -rn "StoredCassette {" src`) loses its `path:` line too.

- [ ] **Step 5: Fix the doc references** — in `src/store/priority.rs`, replace `parse_frontmatter` with `meta::split` in both comments.

- [ ] **Step 6: Verify**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && grep -rn "allow(dead_code)" src`
Expected: tests and clippy green; grep prints nothing.

- [ ] **Step 7: Commit**

```bash
git add src/store
git commit -m "refactor: drop the store's Phase 2 dead_code allow and the three items it hid"
```

---

### Task 2: Live sync detects every write, not every mtime change

**Files:**
- Modify: `src/main.rs` (`sync_external_writes` ~line 1257–1305; its doc comment ~1233; `cassette_mtimes` declaration ~line 959; test call sites ~2139–2256 declare `HashMap::new()` — type inference carries them)
- Modify: `tests/cli.rs` (~lines 2321–2349)

**Interfaces:**
- Produces (private to `main.rs`): `#[derive(Clone, Copy, PartialEq, Eq, Debug)] struct ChangeStamp`, `fn change_stamp(m: &std::fs::Metadata) -> Option<ChangeStamp>`; `sync_external_writes(.., stamps: &mut HashMap<String, ChangeStamp>)`.

- [ ] **Step 1: Failing unit test** (in `main.rs`'s `mod tests`)

```rust
    #[cfg(unix)]
    #[test]
    fn a_same_length_rewrite_inside_one_mtime_tick_still_changes_the_stamp() {
        // Coarse-mtime filesystems give two quick writes the same mtime. The
        // store writes through `atomic_write` (temp file + rename), so the
        // inode moves on every write even when the mtime cannot.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("c.md");
        store::atomic_write(&path, "aaaa").expect("first");
        let first_meta = std::fs::metadata(&path).expect("stat");
        let first = change_stamp(&first_meta).expect("stamp");

        store::atomic_write(&path, "bbbb").expect("second, same length");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(first_meta.modified().expect("mtime"))
            .expect("pin mtime back");
        let second = change_stamp(&std::fs::metadata(&path).expect("stat")).expect("stamp");

        assert_ne!(first, second, "same mtime and length, new inode: a change");
    }
```
Check that `store::atomic_write` is reachable from `main.rs` tests (`pub fn atomic_write` in `store/mod.rs` line ~71). If it is not `pub` at that path, use `crate::store::atomic_write`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --bin cassette a_same_length_rewrite`
Expected: compile error — `change_stamp` not found.

- [ ] **Step 3: Implement** (above `sync_external_writes`)

```rust
/// What the stat-only pass compares to decide a cassette changed. mtime
/// alone misses a second write inside one timestamp tick on a coarse
/// filesystem; every store write goes through `atomic_write`'s rename, so
/// the inode moves on every write and cannot collide. `len` is the
/// fallback signal where there is no inode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChangeStamp {
    modified: SystemTime,
    len: u64,
    #[cfg(unix)]
    ino: u64,
}

fn change_stamp(m: &std::fs::Metadata) -> Option<ChangeStamp> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;
    Some(ChangeStamp {
        modified: m.modified().ok()?,
        len: m.len(),
        #[cfg(unix)]
        ino: m.ino(),
    })
}
```
In `sync_external_writes`: parameter `mtimes: &mut HashMap<String, SystemTime>` → `stamps: &mut HashMap<String, ChangeStamp>`; replace
```rust
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
```
with
```rust
        let Some(stamp) = entry.metadata().ok().as_ref().and_then(change_stamp) else {
            continue;
        };
```
and every later `modified` in the loop with `stamp` (`stamps.insert(id.to_string(), stamp)`, `stamps.get(id).is_none_or(|prev| *prev != stamp)`). Rename `cassette_mtimes` at ~line 959 to `cassette_stamps` with type `HashMap<String, ChangeStamp>`. Update the doc comment's "check each entry's mtime" to "check each entry's `ChangeStamp` (mtime, length, inode)". Test call sites that pass `&mut mtimes` compile unchanged via inference; rename the locals to `stamps` for honesty.

- [ ] **Step 4: Tighten the CLI test** (`tests/cli.rs` ~2321 and ~2342): capture a tuple instead of the mtime:

```rust
    let stamp = |p: &std::path::Path| {
        use std::os::unix::fs::MetadataExt as _;
        let m = std::fs::metadata(p).expect("stat");
        (m.ino(), m.len(), m.modified().expect("mtime"))
    };
    let before = stamp(&path);
    // … unchanged spawn/write …
    let after = stamp(&path);
    assert_ne!(after, before, "the write must change what live sync watches");
```
If `tests/cli.rs` must build off-unix, gate this test with `#[cfg(unix)]`.

- [ ] **Step 5: Verify**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -m "fix: live sync keys on (mtime, len, inode) so a write inside one mtime tick is not missed"
```

---

### Task 3: The inactive side lands at its end after a side-B merge

**Files:**
- Modify: `src/app.rs` (`merge_external` ~line 419–432; tests beside `merging_follows_the_new_text_on_side_b_when_the_cursor_was_at_the_end` ~line 1530)

- [ ] **Step 1: Failing test**

```rust
    #[test]
    fn after_a_side_b_merge_side_a_s_cursor_is_at_its_end_like_the_mirror_case() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| {
            c.insert_str("old a");
            c.flip();
            c.insert_str("scratch");
        });
        app.clear_dirty(0);

        let incoming = Cassette::from_sides("new side a".to_string(), "scratch".to_string(), None);
        // Differ from the stored text so the no-op short circuit does not fire.
        app.merge_external("aaa00000000000000000000000", incoming);

        app.cassettes[0].flip();
        assert_eq!(app.cassettes[0].side, Side::A);
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            "new side a".chars().count(),
            "the inactive side lands at its end, as side B does in the mirror case"
        );
    }
```

- [ ] **Step 2: Run** `cargo test --bin cassette after_a_side_b_merge` — expected FAIL, cursor `0`.

- [ ] **Step 3: Implement.** In `merge_external`:
```rust
            let cursor_for_a = if existing_on_side_b || was_at_end {
                side_a_len
            } else {
                existing_cursor
            };
```
and rewrite the comment above it: "Side A is seeded via `from_sides_with_cursor`. When the reader is on side B, side A is the inactive side and lands at its end — the same place `from_sides` puts inactive side B in the mirror case, and where a writer flipping over to keep writing expects to be."

- [ ] **Step 4: Verify** `cargo test && cargo clippy --all-targets -- -D warnings` — green.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "fix: a side-B merge leaves side A's cursor at its end, matching the mirror case"
```

---

### Task 4: `write_permitted` uses `writers::display_name`

**Files:**
- Modify: `src/queue/write.rs` (`write_permitted` ~line 48–62)
- Modify: `src/store/writers.rs` (`display_name` doc comment ~line 172–178, which says `write_permitted` "already applies [this] inline")

- [ ] **Step 1: Implement** (no new test: behaviour is identical and the existing sticky-lock tests in `tests/cli.rs` and `queue/write.rs` assert the message)

```rust
        (Kind::Agent, Some(holder)) => {
            let name = store
                .writers()
                .map(|w| crate::store::writers::display_name(&w, holder))
                .unwrap_or_else(|_| holder.to_string());
```
If the implicit-writer-authority plan has already landed, the match is on `acting.authority` and the message carries `acting.hint(..)`; change only the `name` computation.

Update `display_name`'s doc: drop "The same fallback `queue::write::write_permitted`'s sticky-lock message already applies inline;" — it now *is* that fallback's only implementation.

- [ ] **Step 2: Verify** `cargo test && cargo clippy --all-targets -- -D warnings` — green.

- [ ] **Step 3: Commit**

```bash
git add src/queue/write.rs src/store/writers.rs
git commit -m "refactor: write_permitted resolves the sticky holder through display_name"
```

---

### Task 5: Measure the picker's scan; record findings; retire done items

**Files:**
- Create: `scripts/bench-store.sh` — only if the user wants it kept; otherwise run it from the scratchpad and do not commit it.
- Modify: `docs/follow-up.md`

- [ ] **Step 1: Build a store and time the scan** (release build; `find` runs the same `scan_store` the picker does, without a terminal)

```bash
cargo build --release
BIN=$PWD/target/release/cassette
for N in 100 1000 5000; do
  export CASSETTE_DATA_DIR=$(mktemp -d)/store USER=bench
  for i in $(seq $N); do
    S=$($BIN session new)
    for t in a b c d e; do
      C=$($BIN queue new "$t" --session "$S")
      printf 'some words about %s in session %s\n' "$t" "$i" | $BIN queue write "$C" --session "$S" >/dev/null
    done
  done
  echo "N=$N"; for r in 1 2 3; do /usr/bin/time -f '%e s  %M KB' $BIN find >/dev/null; done
done
```
Store creation for N=5000 spawns ~55 000 processes; if that takes over ~20 minutes, stop at 1000 and say so.

- [ ] **Step 2: Record.** In `docs/follow-up.md`'s Carried triage, under the "`cassette sessions` scans every cassette…" bullet, add the three medians and the verdict against the spec's threshold (150 ms at ~400 sessions, interpolated). Under the "newcomer above a scrolled viewport" bullet add: "Fix when built: capture the id at `cassette_scroll` before `merge_external`'s re-sort, restore the index by that id after, then `ensure_focus_visible` — the by-id rule focus already follows."

- [ ] **Step 3: Remove** the four fixed bullets (store `dead_code` allow, `after >= before`, `cursor_for_a`, `write_permitted` inline lookup) from `docs/follow-up.md`.

- [ ] **Step 4: Update CLAUDE.md** — in the `main.rs` paragraph, "a cheap stat-only pass over the session's `cassettes/` directory decides what changed since the last tick" → add "by `ChangeStamp` (mtime, length and — on unix — inode, since every store write renames a new file into place; mtime alone misses a second write inside one timestamp tick)".

- [ ] **Step 5: Commit**

```bash
git add docs/follow-up.md CLAUDE.md
git commit -m "docs: record picker scan timings and retire the fixed triage items"
```
