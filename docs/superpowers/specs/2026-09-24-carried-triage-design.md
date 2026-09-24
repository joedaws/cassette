# Carried triage — the small items, bundled

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s "Carried triage" list.
> The `$USER` bootstrap item has its own spec (`2026-09-24-implicit-writer-authority-design.md`).
> Each item below was re-verified against the source on `follow-up-specs` @ `3f839b7`.
> Judgement calls are under **Decisions**.

Six items: four are fixes, two are measure-or-note only. One fix turned out to be a real bug
hiding behind a weak test assertion (item 2).

## 1. Remove `src/store/mod.rs`'s module-wide `#![allow(dead_code)]`

The comment says "remove when Phase 4 lands"; Phase 4 landed three phases ago. Removing it and
running `cargo clippy --all-targets -- -D warnings` today reports exactly three items, all
verified:

| Item | Non-test readers | Test readers | Resolution |
|---|---|---|---|
| `LockGuard::path()` (`store/lock.rs:298`) | none | none | **delete** |
| `meta::parse_frontmatter` (`store/meta.rs:155`) | none | 9 asserts in `meta.rs` tests | **delete**; tests call `meta::split(..).0`, the function production uses |
| `StoredCassette.path` (`store/mod.rs:203`) | none | 1 assert in `store/mod.rs` tests (slug stays as minted) | **delete**; that test uses `Store::cassette_path` |

`DamagedCassette.path` is **not** dead (`label()` and the damage sort read it) and stays.

The follow-up called `StoredCassette.path` "a design call, since it is a public store field".
It is public only within a binary crate with no `lib.rs`, so nothing outside the crate can read
it, and nothing inside does. `Store::cassette_path(session, id)` already answers "where is this
cassette's file" from the one place that knows the frozen-slug rule. Keeping a second answer
cached on every scanned cassette is exactly the unfinished deletion Phase 6 said an `allow`
hides.

## 2. Live sync can miss a write: change detection keys on mtime alone

**This is the real bug behind the follow-up's "`assert!(after >= before)` passes when the mtime
did not move".**

`main.rs` `sync_external_writes` decides a cassette changed iff its `SystemTime` mtime differs
from the one recorded last tick (`mtimes: HashMap<String, SystemTime>`). On a filesystem with
coarse timestamps (ext4 on some kernels, many network and FUSE mounts, HFS+ at 1 s), two writes
inside one timestamp tick leave the mtime identical, and the second write **is never merged**
until some later write happens to move the mtime. The reader keeps looking at stale text with no
indication.

Every store write goes through `atomic_write`, which writes a temp file and `rename`s it into
place, so **every write gives the file a new inode**. That is a change signal that cannot
collide:

- Replace `SystemTime` in the map with a `ChangeStamp { modified: SystemTime, len: u64,
  #[cfg(unix)] ino: u64 }`, built from the same `entry.metadata()` call (no extra syscall).
- `changed` compares whole stamps.
- On non-unix builds (the crate already carries `#[cfg(not(unix))]` paths in `store/mod.rs`)
  the stamp is `(modified, len)` — strictly better than today, not perfect.

The test in `tests/cli.rs` (~line 2346) asserts `(ino, len, mtime)` changed as a tuple, rather
than `mtime >=`, so it actually tests what its message claims.

## 3. `merge_external`'s inactive-side cursor

The follow-up says side A's cursor is "hardcoded to `0`" after a side-B merge "while the mirror
case keeps side B's stored cursor". Half right. Verified in `app.rs` ~line 426 and
`Cassette::from_sides_with_cursor`:

- Reader on **side A**: the inactive side B is seeded into `back_left` — its cursor lands at
  **the end** of side B, not at its stored position.
- Reader on **side B**: the inactive side A is seeded with `cursor_for_a = 0` — **the start**.

So the asymmetry is end-vs-start, and neither preserves the old position. Fix: the inactive side
lands **at its end** in both cases (`cursor_for_a = side_a_len` when the reader is on side B).
End is what `from_sides` does for resume and what a writer flipping sides expects — you flip to
keep writing, not to reread from the top.

## 4. `write_permitted`'s inline id→name lookup

`queue::write::write_permitted` resolves `locked_by` to a name with its own
`writers.get(holder).map(|w| w.name.clone()).unwrap_or_else(|| holder.to_string())`, the exact
body of `store::writers::display_name`, whose own doc comment says it exists so this lookup is
not reimplemented. Replace the inline chain with `display_name(&w, holder)`; `store.writers()`
failing still falls back to the raw id.

## 5. `cassette sessions` scans every cassette before the first frame — measure only

Verified: `picker` is fed by `find::scan_store`, which `scan_session`s every session and reads
every cassette body (for word counts, topics, previews, the haystack). No change is designed —
there is no evidence it is slow yet. The plan's last task produces the evidence: a scripted
store of 100 / 1 000 / 5 000 sessions × 5 cassettes, timing `cassette find` (the same scan
without a terminal), recorded in `docs/follow-up.md`. **Threshold for acting:** first frame over
~150 ms at a store size a year of daily use reaches (~400 sessions). If it is crossed, the likely
fix is a cached per-session summary, which is a design of its own.

## 6. A newcomer above a scrolled viewport shifts the visible set — note only

Verified: `App::merge_external` inserts and re-sorts, then restores **focus** by id, but
`cassette_scroll` is a bare index, so a newcomer sorting above it shifts every visible cassette
down one row. Cosmetic, as the follow-up says. The fix, recorded for when it is built: capture
the id at `cassette_scroll` before the re-sort and restore the index by that id after — the same
by-id rule focus already follows — then `ensure_focus_visible`. Not planned now: it needs a
render-level test to be worth anything, and the user's triage marked it cosmetic.

## Decisions

1. **Delete all three dead items** rather than `#[cfg(test)]`-gate them. A test-only
   `parse_frontmatter` would be a second parser entry point that production never exercises.
2. **Item 2 is in scope** even though the follow-up listed only the test. The weak assertion was
   masking a production miss; fixing the test alone would have made it fail intermittently on
   coarse filesystems with no production fix to point to.
3. **Inode, not content hashing**, for item 2. Hashing would mean reading every file every tick,
   which is exactly what the stat-only first pass exists to avoid.
4. **Inactive side lands at its end** (item 3), not at a preserved position. Preserving it needs
   a new `Cassette` accessor for the back cursor; nothing has asked for that.
5. **Items 5 and 6 stay in `docs/follow-up.md`** with the measurements / fix sketch added; items
   1–4 are removed from it when built.

## Interaction with the other overnight plans

- `2026-09-24-implicit-writer-authority.md` rewrites `write_permitted`'s signature. Whichever
  lands second rebases item 4 onto the other; the change is one line either way.
- `2026-09-24-queue-topic.md` Task 1 asserts `after.path == before.path` on a `StoredCassette`
  and already says to drop that assertion if the field is gone — it will be, if this lands first.

## Acceptance criteria

- `src/store/mod.rs` has no `allow(dead_code)`; `grep -rn "allow(dead_code)" src` is empty.
- `LockGuard::path`, `meta::parse_frontmatter`, `StoredCassette.path` no longer exist.
- A unit test drives `sync_external_writes`' change check with two stamps that share an mtime but
  differ in inode (or length) and sees a change.
- `tests/cli.rs`'s mtime test asserts the stamp tuple changed.
- A side-B merge leaves side A's cursor at its end (unit test in `app.rs`).
- `write_permitted` calls `writers::display_name`.
- `docs/follow-up.md` carries measured timings for item 5 and the fix sketch for item 6.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` green.
