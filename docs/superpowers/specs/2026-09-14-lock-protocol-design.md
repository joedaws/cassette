# Lock protocol — Phase 3 of the session store

**Status:** approved 2026-09-14
**Parent spec:** `docs/superpowers/specs/2026-09-13-session-store-design.md`

This elaborates Phase 3. The parent spec remains the authority on everything it already
settles — the sidecar-versus-inode argument, the attribution format, the four invariants,
the exit-code table. This document covers only what Phase 3 adds, and the questions the
parent left open.

## What Phase 3 delivers

- `LockGuard` as the only way to write a cassette, with `Store::lock` and
  `Store::lock_many`, released on drop or process death.
- `.locks/<id>` anchors: created by `add_cassette` and on demand by `lock`, never deleted.
- Attribution written into the anchor after acquisition; display-only.
- `.locks/writers` guarding the writer registry's read-modify-write.
- An acquisition order for multi-lock operations, for liveness only.
- `cassette queue write`, pulled forward from Phase 4 because it is the surface the
  concurrency tests need.
- Deterministic two-writer concurrency tests and a `SIGKILL` crash-release test.

## The lock primitive

`Store::write_cassette` is removed as a public method and moves onto the guard. Invariant 1
— "you may only write a cassette whose lock you hold" — stops being a rule a reviewer has
to remember and becomes a signature that cannot be bypassed.

```rust
pub struct LockGuard<'s> {
    store: &'s Store,
    session: String,
    id: String,
    path: PathBuf,
    _file: File,   // the flock lives on this file's open *description*
}

impl Store {
    /// Acquire a cassette's lock. Never blocks — a human may hold this one
    /// indefinitely, so `Err(LockError::Busy)` and pick another cassette.
    pub fn lock(&self, session: &str, id: &str) -> Result<LockGuard<'_>, LockError>;

    /// Acquire several at once, in acquisition order (see below). All or
    /// nothing: on contention, every guard already taken is dropped.
    pub fn lock_many(&self, session: &str, ids: &[&str])
        -> Result<Vec<LockGuard<'_>>, LockError>;

    /// Acquire the writer registry's lock. BLOCKS — no human can hold this
    /// one, its critical section is a single read-modify-write, and a caller
    /// that cannot register has no fallback. Private: `ensure_writer` is the
    /// only caller, so the blocking behaviour cannot leak into a code path
    /// where a human could wedge it.
    fn lock_registry(&self) -> io::Result<LockGuard<'_>>;
}

impl LockGuard<'_> {
    /// The cassette's current state, read under the lock.
    pub fn read(&self) -> io::Result<StoredCassette>;
    /// Replace it. The only write path for an existing cassette.
    pub fn write(&self, meta: &CassetteMeta, body: &str) -> io::Result<()>;
    /// Who holds it — this guard's own attribution.
    pub fn holder(&self) -> &Attribution;
}
```

### Never block on a lock a human can hold

Cassette locks are acquired `LOCK_EX | LOCK_NB` — non-blocking. A cassette lock is held
for as long as a person keeps that cassette focused, which may be minutes or hours, and
the holder may have walked away entirely. Blocking on one would let a single idle editor
stall every agent. The queue's premise is that an agent finding a cassette busy writes a
*different* one, so there is always somewhere else to go.

The registry lock (Gap 1) is the opposite kind of critical section and is acquired
**blocking** (`LOCK_EX`, no `LOCK_NB`):

| | Cassette lock | Registry lock |
|---|---|---|
| Held for | As long as a cassette stays focused — minutes to hours | One read, insert and write — microseconds |
| Held by | A person who may have walked away | A process actively finishing |
| On contention | Write a different cassette | Nothing else to do; must wait |
| Acquisition | `LOCK_EX \| LOCK_NB` | `LOCK_EX`, blocking |

The rule is therefore not "never block" but **never block on a lock a human can hold**.
No interactive editing happens inside the registry's critical section, so nobody can wedge
it; a non-blocking registry lock would instead mean the TUI fails to start because an
agent happened to be registering itself at that instant — worse than the lost update it
was meant to prevent.

There is no `--wait` flag and no retry loop in Phase 3: the one place waiting is correct
is unconditional, and the one place it is wrong is never offered. If agents later prove to
need waiting on a cassette, that is an additive flag, not a redesign.

Release is `Drop`, plus the kernel on process death for any reason including `SIGKILL`.
Nothing to reap — that is the whole reason the anchor's existence carries no meaning.

`Drop` must release with an explicit **`LOCK_UN`**, and must not simply close the file.
An earlier draft of this document said "the flock lives here; dropping it releases", and
that is wrong in a way that cost a phase's worth of debugging (see
`.superpowers/sdd/2026-09-17-tui-writes-the-store/flake-fix-report.md`). `flock` attaches
to the open file **description**, not to the descriptor. Any `fork()` that happens while
the lock is held hands the child a second descriptor onto the same description, and
`flock(2)` releases the lock only "by an explicit `LOCK_UN` operation on any of these
duplicate file descriptors, or when all such file descriptors have been closed". Our
descriptors are `O_CLOEXEC`, so the child's copy does go — but not until it reaches
`execve`. Between our `close()` and that `exec`, the lock outlives the guard that owned
it, and the next acquisition of the same anchor is refused by a lock nobody holds.

`LOCK_UN` acts on the description itself, so it releases every descriptor sharing it —
the not-yet-`exec`'d child's included — which makes release synchronous with dropping the
guard, as every caller already assumes. Close-on-exit remains the backstop underneath it:
that is what makes death by `SIGKILL` safe and keeps "no reaper" true.

`Drop` releases the flock **before** it clears the in-process `HELD` registry. Clearing
first would leave a window in which `Store::holds` answers "no" about a lock we are still
holding, and a caller acting on that answer would be told `Busy` by us.

### Anchor lifecycle

The parent spec says anchors are "never renamed or deleted" but does not say who creates
them. Both paths create:

- `add_cassette` creates `.locks/<id>` alongside the cassette file.
- `lock` creates it on demand when missing.

Belt and braces, because a cassette written by hand — or one predating Phase 3 — must
still be lockable. The apparent race is benign: `File::create` from two processes on the
same path yields the same inode, since no `rename` is involved. The cost is a stray empty
file if someone locks a typo'd id, which is consistent with anchors never being deleted
anyway.

### Error type

```rust
pub enum LockError {
    /// Another live writer holds it. Carries their attribution for the message.
    Busy(Attribution),
    Io(io::Error),
}
```

`Attribution` is the `writer=<id> name=<name> pid=<pid> since=<ts>` line the parent spec
defines, written into the anchor *after* acquiring — an ordering the lock itself
serializes. A blocked writer reads it to render:

```
cassette: 'refactor notes' is open by joseph (since 2026-09-14T14:02:11Z) — try again later
```

The full RFC3339 stamp, not a bare `14:02`, is deliberate: a lock can have been held since
yesterday, and a bare time of day is ambiguous across days when that happens — exactly the
case a user most needs to understand.

Display-only. Stale bytes after a crash are harmless, because lockedness is kernel state
and never the file's contents.

## Changes to the Phase 2 API

| Item | Change |
|---|---|
| `Store::write_cassette` | Removed as a public method; becomes `LockGuard::write`. |
| `Store::add_cassette` | Creates the anchor, acquires, writes, releases, internally. A freshly minted ULID cannot be contended, so this cannot fail on `Busy`; routing it through the same path means there is exactly one way a cassette file is ever written. |
| `Store::scan_session` | **Unchanged.** Reads stay lock-free — the dividend of locking the sidecar rather than the `.md`, and what makes invariant 1's "re-read from disk" cheap. |

Invariant 3 (flush on blur) belongs to Phase 5, not here.

## Two gaps the parent spec does not cover

Both were found while designing this phase. Both are fixed in Phase 3.

### Gap 1 — the writer registry has an unguarded read-modify-write

`ensure_writer` reads the whole registry, inserts, and writes the whole registry back.
Two writers registering at the same moment both read a registry lacking the other, both
insert their own, and the second write clobbers the first. A writer id vanishes silently,
and every cassette attributed to it points at nothing.

This is not hypothetical for the intended workflow: the parent spec auto-registers the
human from `$USER` on first run, and agents register themselves. A first launch with an
agent already running is exactly the collision.

**Fix:** a `.locks/writers` anchor at the store root, acquired for the duration of the
read-modify-write inside `ensure_writer`. Same guard type, different anchor. The registry
is a single shared file rather than a per-cassette one, so its anchor lives at the root
rather than under a session.

**This lock blocks**, unlike every cassette lock — see "Never block on a lock a human can
hold" above. A caller that cannot register a writer has no fallback, and the colliding
case is first launch, where the TUI auto-registers from `$USER` while an agent registers
itself. Failing there would mean the TUI refuses to start over a few microseconds of
contention. Blocking is safe because no interactive editing happens inside the critical
section, so nobody can wedge it.

This **adds a directory to the storage layout** the parent spec does not have — that spec
places `.locks/` only inside a session directory. The root gains one:

```text
~/.local/share/cassette/
  writers.toml
  active
  .locks/
    writers                    # empty; flock anchor for the registry
  sessions/
    <session ulid>/
      …
```

Created `0700` by `ensure_private_dir`, like everything else under the root. Phase 4 will
likely want a sibling anchor for the `active` pointer if set-active ever grows a
read-modify-write; today it is a single atomic write and needs none.

### Gap 2 — multi-lock acquisition has no ordering rule

The parent spec says a priority renumber takes "just those locks" — plural. Two writers
renumbering overlapping runs in different orders would deadlock; because acquisition is
non-blocking they instead livelock, both receiving `Busy`, both retrying, both failing.

**Fix:** locks are always acquired in **ascending id order**, enforced inside
`lock_many` so a caller cannot get it wrong. A total order makes deadlock impossible at
the cost of one sort.

**This acquisition order has nothing to do with queue order.** They are two unrelated
uses of the same field, and conflating them would be a serious error:

- **Acquisition order** is a liveness device, internal and never displayed. Any total
  order would do; the id is simply the one field guaranteed present, unique and stable.
- **Queue order** is `priority` first, closed last, id only as a tiebreak — and the
  parent spec is emphatic that the id "is never load-bearing for ordering and must not be
  relied on to resolve creation order correctly 100% of the time."

Name the constant for acquisition, never "sort order", and keep this distinction in the
doc comments.

Renumbering itself is Phase 4. The rule belongs here, with the primitive it constrains.

## `cassette queue write`

One Phase 4 command is pulled forward, because it is the minimum surface that lets one
process hold a lock while another tries to take it.

```
cassette queue write <ID> [--session <ID>]     # body on stdin
```

It acquires the lock, **then** reads stdin, writes, and releases. That ordering is the
point: a child spawned with an open stdin pipe is deterministically holding the lock, with
no sleeps and no polling, and closing the pipe releases it. The correct design and the
testable design agree.

Defaults to the active session; `--session` overrides. Exit 3 on contention, with the
holder's attribution on stderr.

`--json` stays in Phase 4. A single command does not justify the contract, and inventing
half of it now risks a shape the rest of the queue commands will not fit.

## Exit codes

Phase 3 uses three of the six the parent spec defines.

| Code | Phase 3 meaning |
|---|---|
| 0 | wrote and released |
| 1 | I/O failure — the store could not be read or written |
| 2 | usage error; unknown session or cassette id |
| 3 | busy — another writer holds the lock; attribution on stderr |
| 4, 5, 6 | reserved, unused until Phase 4 |

`LockError::Busy` maps to 3; `LockError::Io` to 1 with the underlying error. Code 1 is
not in the parent spec's table, which starts at 2 — it is the conventional catch-all and
is added here rather than overloading 2, which means "fix your invocation" and would send
an agent down the wrong path on what is actually a disk problem.

## Testing

Deterministic, never sleep-based. Flaky lock tests get deleted within a month, which is
the same as having none.

| Test | Mechanism |
|---|---|
| Contention is detected | The test process holds the lock, then spawns the real binary (`env!("CARGO_BIN_EXE_cassette")`) and asserts exit 3. No race: the lock is held before the child starts. |
| Contention from the other direction | Spawn `queue write` with an open stdin pipe; it holds the lock. The parent asserts `Busy`, then closes the pipe and asserts it can acquire. |
| Crash releases the lock | Spawn a holder, `SIGKILL` it, assert the next acquisition succeeds immediately. This is the property the no-reaper decision rests on, and it should fail loudly if a future refactor swaps in a lockfile-existence scheme. |
| Two threads genuinely contend | In-process. Meaningful because `flock` is per open file description; under `fcntl` locks these would silently share the lock and pass. Small evidence the primitive is right. |
| The registry survives concurrent registration | Two processes `ensure_writer` different names at once; both survive. Guards Gap 1. |
| The registry lock waits rather than failing | Hold `.locks/writers` in the test process, spawn a registration, assert it has not completed; release, assert it then completes with exit 0 — never exit 3. Pins "blocking, not `Busy`" for the one lock that must not fail on contention. |
| Acquisition order prevents livelock | Two `lock_many` calls over overlapping id sets, requested in opposite orders; one succeeds and one reports `Busy` rather than both failing. Guards Gap 2. |

`flock` itself is not tested — that is the kernel's job.

### Scope decisions

- **Two writers, not N.** One human and one agent is the real workload and is enough to
  prove the protocol. N-way contention buys nothing here.
- **Concurrency is exercised through the CLI, not the TUI.** Spawning subcommands against
  a shared store is cheap and deterministic. The TUI still participates in the protocol at
  runtime — it holds its focused cassette's lock and flushes on blur — but driving a pty
  to prove that is expensive and covers the same code paths the CLI tests already reach.
- **Windows is out of CI for now.** `fs4` remains the cross-platform choice and nothing
  here is Unix-only by design, but Windows correctness is verified by hand on a real
  machine rather than gated in CI. Revisit before any Windows release.

## Dependencies

`fs4` — cross-platform file locking, pure Rust. No C toolchain, so the crates.io / AUR /
Homebrew distribution story is unaffected.

## Deferred

- **The sticky lock (`locked_by`).** Phase 3 ships the implicit flock only. `locked_by`
  stays a dormant frontmatter field and exit 4 stays unused. The sticky lock is a
  human-facing workflow — claim and release — rather than a concurrency primitive, so it
  lands in Phase 4 with the CLI that drives it.
- **`--json`.** Phase 4, with the rest of the queue surface.
- **Waiting on a lock.** No `--wait`, no backoff. Additive later if agents prove to need
  it.
