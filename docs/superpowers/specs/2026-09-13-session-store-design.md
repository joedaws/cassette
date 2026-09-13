# Session store — per-cassette files, writers, and turn-taking locks

**Date:** 2026-09-13
**Status:** draft — pending review

## Purpose

Today a session is one markdown file rewritten wholesale every 30 seconds. That makes
concurrent writers impossible: two processes saving the same file clobber each other,
and the last writer wins silently.

The goal is a queue of cassettes that a human and one or more agents work through
together, taking turns. A cassette the human is editing must be off-limits to agents;
everything else is fair game. Cassettes carry a priority so either side can reorder the
queue, and a status so finished ones drop out of the working set without being deleted.

This replaces the storage model outright. The project is pre-1.0 with a single user, so
there is no migration path: existing flat notes in `notes/` are simply no longer read.

## Approaches considered

1. **Per-cassette markdown files under a session directory (chosen)** — writes are
   per-cassette and atomic, so two writers on different cassettes cannot conflict.
   Keeps plain text as the artifact, keeps `stats`/`find` as pure functions over
   frontmatter, adds no C toolchain to the build.
2. **SQLite session store** — real transactions and race-free lock acquisition. Rejected:
   the contention here is two-to-three cooperating writers mediated by a single binary,
   which does not need MVCC; live sync is polling either way (SQLite has no cross-process
   push); it contradicts the project's "notes dir is the database" premise and would force
   a markdown export pipeline anyway; and `rusqlite` drags a C build into a crate being
   shipped to crates.io, AUR, and macOS.
3. **Hybrid — markdown bodies plus a separate index for priority/status/lock** — cheap
   queue-wide reads, but two sources of truth that can drift. Not warranted at this scale.

Note on a premise raised during design: SQLite does not "periodically write to disk" — it
writes on commit, and that durability is the point. An in-memory database with periodic
snapshots would be strictly less safe than the current file-per-session model.

The tripwire for revisiting SQLite: cross-cassette transactions that must not tear, more
than ~3 concurrent writers, thousands of cassettes, or full revision history. Files →
SQLite is a clean import later; the reverse is not.

## Storage layout

```
~/.local/share/cassette/            # data_dir, created 0700
  writers.toml
  active                            # single line: active session id
  sessions/
    01K5GQ2R8V3XQZ/                 # session id (ULID)
      session.toml
      cassettes/
        gratitude-01K5GR7T2M9WPD.md
      .locks/
        01K5GR7T2M9WPD              # empty; flock anchor; never renamed or deleted
```

### Identity and file naming

Cassette files are named `<slug>-<ulid>.md`. The slug is derived from the topic at
creation and **never renamed afterward**; `topic` in frontmatter remains the source of
truth for display. Stability beats accuracy — see "Why priority never touches the
filename" below.

ULIDs are used rather than random UUIDs because they sort by creation time, which makes
the priority tiebreak meaningful: equal priorities resolve to creation order, i.e. FIFO,
which is what a queue should do. The CLI accepts unambiguous id prefixes (git shortsha
style) so nobody types 26 characters.

### `session.toml`

```toml
alias = "freewriting-2026-09-13"   # optional; id is displayed when absent
created = "2026-09-13T09:25:57Z"
timer_secs = 600                   # optional session defaults
word_goal = 500
```

### Cassette frontmatter

```yaml
---
id: 01K5GR7T2M9WPD
topic: gratitude
priority: 20            # sparse: 10, 20, 30 …
status: open            # open | closed
locked_by:              # sticky lock: writer id, or empty
created_by: 01K5H2WRITERID
last_writer: 01K5H2WRITERID
updated_at: 2026-09-13T14:02:11Z
---
## Side A

…

## Side B

…
```

The `## Side A` / `## Side B` body format is unchanged, so `output::parse_markdown` and
`build_body` carry over nearly intact.

### `writers.toml`

```toml
[writers.01K5H2WRITERID]
name = "joseph"
kind = "human"
created = "2026-09-13T09:20:00Z"

[writers.01K5H3AGENTID]
name = "refactor-agent"
kind = "agent"
created = "2026-09-13T09:22:00Z"
```

## Config

`config.toml` keys change as follows:

| Key | Change |
|---|---|
| `notes_dir` | renamed `data_dir`; now the store root, not a flat notes folder |
| `daily_format` | retained — now the **alias** format used by `cassette today` |
| `writer` | new — the default human writer name for the TUI |
| `max_open` | new, optional — overrides the 36-open cap |
| `theme`, `[themes.*]`, `templates`, `visible_lines` | unchanged |

There is no deprecation shim for `notes_dir`; pre-1.0 with a single user, a renamed key
that fails loudly beats one that silently points at a store the code no longer reads.

## Writers

A writer is `{id: ULID, name: String, kind: human | agent}`, selected per-invocation via
`--writer <name>` or `CASSETTE_WRITER`. The TUI defaults to the human writer named in
config.toml, auto-registered from `$USER` on first run. An unknown name is a usage error
(exit 2) rather than an auto-create, so a typo fails loudly instead of silently spawning
a third identity.

Capability follows `kind`:

- Only `human` writers may clear a sticky lock, and only `human` writers may set one.
- Closing writes frontmatter, so **closing requires holding the implicit lock**, for every
  writer regardless of kind. A busy cassette cannot be closed by anyone until its holder
  releases (exit 3). This follows from Invariant 1 below, not from a permission rule.
- The asymmetry is the sticky lock: an `agent` refuses to close a cassette whose
  `locked_by` is set (exit 4); a `human` may close it.

**This is identity, not authentication.** Any writer that can read the session directory
can read `writers.toml` and pass a different `--writer`. A token would not change that —
it would sit in the same directory the agent already reads. This layer prevents mistakes
and ambiguity, not a determined impersonator. Real enforcement would mean the OS doing it
(separate Unix users with group-write on the session dir, or a mediating daemon); both
remain available later without changing this data model, because the writer registry is
exactly what either would authenticate against.

## Locking protocol

Two locks with deliberately different lifetimes:

| | Mechanism | Released by | Meaning to a blocked writer |
|---|---|---|---|
| Implicit | `flock(LOCK_EX\|LOCK_NB)` on `.locks/<id>` | Kernel, on unfocus or process death | Transient — retry (exit 3) |
| Sticky | `locked_by` in frontmatter | A human, explicitly | Durable — escalate (exit 4) |

The implicit lock is held by the TUI for exactly as long as a cassette is **focused**.
Agents may write any other cassette in a live session — that is the collaborative case.

### Why the lock is a sidecar and not the cassette file

`flock` attaches to an **inode**. Atomic writes replace the file with a **new inode** via
`rename()`. Locking the `.md` directly is therefore broken: writer A flocks `foo.md`
(inode 1), renames its temp over it, and `foo.md` is now inode 2 — unlocked. Writer B
flocks `foo.md`, gets inode 2, and both believe they hold the lock. Atomic writes and
inode locks are individually correct and quietly incompatible.

The `.locks/<id>` anchor is never renamed and never deleted, so its inode is stable. Its
*existence* carries no meaning — lockedness is kernel state, tested by attempting
acquisition. This is deliberately not the create-to-lock/delete-to-unlock pattern, which
is the stale-lock trap: a crashed holder leaves the file behind, requiring a pid, a
heartbeat, a reaper, and a `--force-unlock` escape hatch. With `flock` the kernel releases
on process death for any reason including `SIGKILL`. Nothing to reap.

A second dividend: on Windows `LockFileEx` is *mandatory*, so a locked file can refuse
reads from other processes. Because only the sidecar is ever locked, cassette files are
never locked and reads always succeed on both platforms.

### Attribution

`flock` is anonymous — `EWOULDBLOCK` says "someone" and nothing more. So the holder writes
`writer=<id> name=<name> pid=<pid> since=<ts>` into the anchor file *after* acquiring,
which the lock itself serializes. A blocked writer reads it for the message:

```
cassette: 'refactor notes' is open by joseph (since 14:02) — try again later
```

Stale contents after a crash are harmless: lockedness is decided by kernel flock state and
never by the file's bytes. The contents are display-only.

### Invariants

1. **You may only write a cassette whose lock you hold. Every cassette you do not hold,
   you re-read from disk.** Clobbering becomes structurally impossible rather than
   something to remember.
2. **Acquire-and-write is one operation.** Advisory listings may test-acquire to display
   busy state, but nothing in the write path may check-then-act.
3. **Flush on blur.** The TUI holds only the focused cassette's lock, so moving focus must
   flush the outgoing cassette before releasing. Autosave is per-cassette; focus change is
   a save point.
4. All writes are write-temp-then-`rename()` within the same directory.

### Why priority never touches the filename

Encoding order in the name (`001-`, `002-`) makes every reprioritization a rename, and
renames break this design: an `flock` is held on an inode, so renaming mid-edit
desynchronizes the lock from the path another writer resolves; git history fragments on
every reorder; concurrent renumbering collides; cached paths go stale. Priority lives in
frontmatter and nowhere else.

### Priority insertion

Priorities are sparse (10, 20, 30). Inserting between 10 and 20 yields 15; between 15 and
20 yields 17. `--last` (the default for `queue new`) is max + 10; `--first` is min - 10,
or half the minimum when that would reach zero. Only when a run has no integer gap left is *that run* renumbered, taking
just those locks. There is no normalize-on-open pass — it would rewrite every cassette,
and the TUI does not hold all those locks.

Two writers can compute the same insertion point and collide on a value. This is benign:
ties break by ULID, i.e. creation order.

## Live sync

Each event-loop tick, stat the cassettes dir; for any file whose mtime moved and whose
lock we do not hold, re-read and merge. Polling ~10 files at 500ms is negligible and
avoids an inotify dependency.

Per the project convention that `App` performs no I/O, the stat-and-read lives in
`main.rs` and hands `App` a parsed `Cassette` via `app.merge_external(id, cassette)`.

## CLI surface

Migrating from the hand-rolled 112-line `parse_args` to **clap v4 (derive)**. This also
advances the man-page and shell-completion work: `clap_mangen` and `clap_complete`
generate both from the same structs.

There is **one set of verbs**, not a separate agent namespace; permission comes from the
invoking writer's kind. Two namespaces would mean two code paths drifting apart, and a
human's manual fixup of an agent's mess would be a different command than the agent's own.

```
# Sessions
cassette sessions                            # interactive picker: 15 recent, 'a' = all
cassette session new [--alias <name>]        # prints id, becomes active
cassette session list [--all]
cassette session use <id>
cassette session alias <id> <alias>

# Queue — acts on the active session unless --session <id>
cassette queue list [--status open|closed|all] [--since <ts>]
cassette queue next
cassette queue new <topic> [--first|--last|--priority N]
cassette queue write <id> [--side a|b] [--append|--replace]
cassette queue show <id>
cassette queue close <id> [-m "close-out sentence"]
cassette queue reopen <id>
cassette queue lock <id> | unlock <id>       # human writers only
cassette queue move <id> --before <id> | --after <id>

# Writers
cassette writer register --name <n> --kind human|agent
cassette writer list
cassette writer whoami

# Retained
cassette [OPTIONS]        # TUI; -t -w -l -T -R --theme -o
cassette today            # session whose alias is today's date
cassette stats | find | export <session> | +themes
cassette --resume         # open the most recent session (by session.toml created)
```

Global flags: `--writer <name>`, `--session <id>`, `--json`.

`queue write` takes its body from stdin (or `-m` for one line); multi-paragraph prose
through shell argument quoting is a bug farm. It acquires the lock **before** reading
stdin, so the lock covers the whole read-modify-write.

`queue next` is the turn-taking primitive: the highest-priority open cassette carrying no
sticky lock and no live flock. Priority ordering is the scheduler and the locks are the
mutex, without the agent needing to model either.

### Sequencing

Migrate the existing CLI to clap **first**, holding the current surface constant and
tested, then layer the new commands and change the storage underneath. Most flags survive
verbatim, so this is close to mechanical — and it keeps a parser regression
distinguishable from a storage bug.

### Reconsidered decision

`2026-07-13-find-command-design.md` explicitly rejected a ratatui browser screen for
`find` as more code than the problem warranted, and `find` keeps that plain-stdout,
pipe-friendly shape. `cassette sessions` is interactive anyway because the calculus
changed: notes had human-typed names, sessions have ULIDs. There is no name to type, so
selection has to be a picker. `sessions` follows the `find.rs` structure — a thin
`scan_sessions_dir` plus pure sort/render functions.

Keys: `j/k` and arrows, `Enter` opens the session in the TUI and marks it active, `a`
expands to all sessions, `/` filters, `q` quits. Alias shown where set, id otherwise.

## JSON contract

`--json` on `list`/`next`/`show` emits full cassette bodies — cassettes are short by
design, and a second round trip to fetch 400 words is not worth saving.

```json
{
  "session": { "id": "01K5GQ2R8V3XQZ", "alias": "freewriting-2026-09-13" },
  "cassettes": [{
    "id": "01K5GR7T2M9WPD",
    "topic": "refactor notes",
    "priority": 10,
    "status": "open",
    "words": 412,
    "busy": false,
    "sticky_lock": null,
    "created_by": { "name": "joseph", "kind": "human" },
    "last_writer": { "name": "joseph", "kind": "human" },
    "waiting_on": "agent",
    "updated_at": "2026-09-13T14:02:11Z",
    "side_a": "…full text…",
    "side_b": ""
  }]
}
```

`waiting_on` is **derived at read time** as the inverse of `last_writer.kind`, not stored.
The human wrote last, so the agent is up. This is why there is no separate `turn` field —
one fewer piece of state that can disagree with itself.

`created_by` exists so a human opening a session that grew overnight can tell which
cassettes they raised from which an agent injected.

Frontmatter stores writer **ids**; JSON emits them resolved against `writers.toml` as
`{name, kind}`, since an agent should not have to join two files. `sticky_lock` is the
resolved form of frontmatter's `locked_by`, and is `null` when unset.

Caveats: a polling agent re-ingests every body on each call — `--since` and `queue next`
exist to scope that, and default `list` is open-only so bodies stay bounded (`--status
all` grows without limit as closed cassettes accumulate). Reading N files is not a
point-in-time snapshot: each file is atomic, so text is never torn, but cassette 1 may be
staler than cassette 9.

## Exit codes

| Code | Meaning | Caller's move |
|---|---|---|
| 0 | ok | continue |
| 2 | usage error (clap), unknown writer | fix the invocation |
| 3 | busy — a live writer holds it | retry later |
| 4 | sticky-locked | stop; a human must clear it |
| 5 | nothing available (`queue next`) | idle or enqueue |
| 6 | queue full — 36 open cassettes | stop; escalate |

3 and 4 are distinct because one says wait and the other says escalate. With `--json`,
failures also emit `{"error": "...", "code": 3}` so an agent gets structure on either
channel.

## TUI changes

- Cassettes sort by priority; closed ones collapse into a single `▸ 12 closed` row at the
  bottom, expandable, greyed.
- `MAX_CASSETTES` (36) applies to **open** cassettes only, so a long-lived daily session
  hits the cap on working set rather than on retained history.
- Focusing a busy cassette degrades to **read-only** with a banner (`open by
  refactor-agent`), retrying acquisition each tick so it becomes editable when released.
  Refusing to focus it would be worse — you could not see what an agent is doing to your
  queue.
- Sticky-locked cassettes show the holder in the separator.
- Autosave becomes per-cassette; focus change flushes.

## Deletions

The unified model removes more than it adds:

- `output::AppendBase`, `parse_append_base`, `write_markdown_appended`, and the
  `## Session N — HH:MM` machinery. `cassette today` becomes "the session whose alias is
  today's date", so the append-and-re-sum path and its double-counting bug class dissolve.
- `is_draft`, `clear_draft_flag`, the `draft: true` marker, and the `[y/N]` crash-recovery
  prompt. Under per-cassette atomic writes and kernel-released locks, the most that can be
  lost is edits to one focused cassette since its last autosave.
- `config::find_available_path` and the `_1.md` conflict-rename dance — ULIDs do not
  collide.
- `newest_note`.
- Reading of the legacy flat `notes/` directory.

`cassette export <session>` renders a session to a single flat markdown file (stdout or a
path), reusing `build_body`. Not a second store — a one-way projection. It matters because
the reason to use this tool is that the writing is plain text you own, and without it the
words live in a directory tree with sidecar lock files.

## Error handling

- **The TUI never dies on I/O.** A failed save flashes a status message, keeps the text
  dirty in memory, and retries next tick. The existing panic hook and `catch_unwind` stay.
- **One bad file must not take down the queue.** A cassette with unparseable frontmatter
  renders as an error row and is skipped, not fatal.
- A cassette deleted underneath a live session drops from view — except that if we hold
  its lock with dirty content, the flush recreates it. The human's unsaved words win.
- Leftover `.tmp-<ulid>` files from a crashed write are swept on session open.
- Timestamps are used only for display and `--since`, never for correctness. Ordering
  comes from priority and sortable ids, so clock skew cannot corrupt it.

## Platform support

`fs4` rather than raw `libc::flock`, so Windows (`LockFileEx`) is supported rather than
silently excluded. CI runs a Linux + Windows matrix, because the locking primitive is
genuinely different underneath and a Linux-only suite proves little about the Windows path.

Windows note: replacing a file via rename can fail with a sharing violation if another
process holds it open. The design mitigates this by construction — `.md` files are opened
only for the duration of one read or one write, never held across event-loop ticks; the
only long-lived descriptor is the `.locks/<id>` anchor. Rust's exact share-mode defaults
should be verified on a Windows runner rather than assumed. Antivirus can also transiently
hold files open; the retry path covers it.

### Permissions

The design requires no new privileges. Lock files live in the app's own data directory
alongside the cassettes, and `flock` is unprivileged — no capabilities, no setuid, no
group membership. This is a consequence of deliberately *not* using a system-wide lock
location: `/var/lock` or `/run/lock` would require the `lock` group or root, and therefore
install-time setup. No daemon, no systemd unit, no post-install script; `cargo install`,
AUR, and Homebrew are unaffected.

`~/Library/Application Support` and `~/.local/share` are not TCC-protected, so macOS
prompts for nothing. Pointing `data_dir` at `~/Documents` or `~/Desktop` would trigger a
macOS folder-access prompt — for the whole store, not the sidecars.

**The data directory is created `0700`.** Freewriting content is private by nature, and
the default `0755`/`0644` would make every session world-readable on a shared machine.

### Unsupported: cloud-backed storage

**Pointing `data_dir` at Dropbox, iCloud Drive, OneDrive, Google Drive, or any syncing
folder is unsupported.** Locks are local kernel state and do not sync, so two machines
editing the same session get **zero** mutual exclusion — the exact guarantee this design
exists to provide. Sync clients also interfere with rename-based atomic writes and
produce conflicted copies. NFS is excluded for the same underlying reason: flock there is
emulated or unreliable.

Where cheap, warn at startup when `data_dir` resolves under a known sync root.

## Testing

Unit tests over pure functions, in the project's existing style: priority gap-finding,
sort order (open by priority, closed last, ties by id), frontmatter round-trips,
`waiting_on` derivation, slug generation. Store-level tests use `tempfile`.

Concurrency is the part most likely to be quietly wrong, and the tests must be
**deterministic rather than sleep-based** — flaky lock tests get deleted within a month.
Two techniques:

1. **The test process holds the lock**, then spawns the real binary
   (`env!("CARGO_BIN_EXE_cassette")`, no extra dependency) and asserts exit 3. No race:
   the lock is held before the child starts.
2. **Stdin is the synchronization primitive for the reverse direction.** `queue write`
   acquires the lock and *then* reads its body from stdin, so a child spawned with an open
   stdin pipe is deterministically holding the lock. The test asserts contention, then
   closes the pipe to release. The correct design and the testable design agree here.

**Crash release gets an explicit test** — spawn a holder, `SIGKILL` it, assert the next
acquisition succeeds immediately. That is the property the no-reaper decision rests on, so
it should fail loudly if a future refactor swaps in a lockfile-existence scheme.

Because `flock` is per open file description, two threads in one process genuinely
contend, so in-process tests are meaningful. Under `fcntl` locks they would silently pass
by sharing the lock — small evidence the primitive is right.

`flock` itself is not tested; that is the kernel's job.

## Dependencies added

| Crate | Why | Notes |
|---|---|---|
| `clap` (derive) | CLI surface | pure Rust |
| `clap_mangen`, `clap_complete` | man page + completions | build-deps |
| `fs4` | cross-platform file locking | pure Rust |
| `ulid` | sortable ids | pure Rust |
| `serde_json` | `--json` output | pure Rust |
| `tempfile` | store tests | dev-dep |

All pure Rust; no C toolchain is introduced, preserving the distribution story for
crates.io, AUR, and macOS packaging.

## Implementation phases

This is well past the 500-line threshold for decomposition, so it ships as sequenced
phases, each independently testable and each leaving the tool working.

1. **clap migration.** Replace `parse_args` with clap derive, holding the current surface
   constant and covered by tests. No behavior change. Isolates parser regressions from
   everything that follows.
2. **Store module and data model.** `session.toml`, cassette frontmatter, `writers.toml`,
   ULID ids, slug naming, priority insertion. Pure functions plus a thin scan layer, in
   the shape `stats.rs`/`find.rs` already use. Tested against `tempfile` dirs; no TUI and
   no locking yet.
3. **Lock protocol.** `.locks/<id>` anchors via `fs4`, acquire/release, attribution
   contents, the deterministic concurrency tests, and the SIGKILL crash-release test.
   Windows added to CI here — the phase that introduces the platform-divergent primitive
   is the phase that should prove it.
4. **CLI commands.** `session`, `queue`, `writer`, `--json`, exit codes. At the end of
   this phase an agent can drive the queue end-to-end with no TUI involvement.
5. **TUI integration.** Per-cassette autosave, flush-on-blur, live-sync polling and
   `merge_external`, read-only banner for busy cassettes, priority ordering, the collapsed
   closed row, sticky-lock indicators.
6. **Repoint and remove.** `stats`, `find`, `today`, `--resume`, `export`; delete the
   append/draft/conflict-rename machinery listed under Deletions; `0700` data dir; the
   cloud-sync warning; man page and completions.

Phase 1 is mechanical, 3 and 5 carry the real risk. Phases 2–3 are worth reviewing before
4–6 build on them, since the lock protocol is the part that is expensive to change later.

## Deferred

- **Multi-agent contention on one cassette.** Two agents can both target the same unlocked
  cassette; flock serializes them so nothing corrupts, but they could interleave
  incoherently. `last_writer` plus priority ordering is assumed sufficient for now. If it
  is not, the fix is an agent-settable claim (a sticky lock an agent is allowed to set on
  itself).
- **Agent flooding the queue.** The 36-open cap plus tail-by-default placement means agent
  additions cannot jump the human's line. A TUI marker distinguishing agent-created
  cassettes is available via `created_by` if it proves necessary.
- **Revision history.** `last_writer` + `updated_at` give turn-taking attribution without a
  revision log. Full history is a reason to revisit SQLite, not to add a log here.
- **OS-level enforcement** of writer identity (Unix users or a mediating daemon).
