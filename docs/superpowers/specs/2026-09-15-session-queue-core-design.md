# Phase 4b — Session and queue core

**Parent spec:** `2026-09-13-session-store-design.md`. That document is the binding
authority; this one covers only what Phase 4b decides, and corrects the parent where
4a's outcome made it stale (see **Corrections to the parent spec**).

## Purpose

Phase 4b puts the scriptable surface on top of the store Phases 1–4a built: three
`session` commands and seven `queue` commands. The store is finished — `create_session`,
`scan_session`, `cassette_path`, `lock`, `lock_many`, `queue_order` and the `priority`
placement functions all exist, are tested, and need no new primitives. 4b is a CLI layer
and should read like one.

## Decisions

Four decisions were taken at design time. Each chose the more explicit option; they are
recorded with their costs so a later phase can revisit them knowingly rather than
rediscover them.

### 1. There is no active session

The `active` pointer is **removed**, not merely bypassed. `--session <id>` is required on
every `queue` command.

A single `active` file at the store root is shared mutable state between a human and any
running agent: `session use B` in a shell would silently retarget an agent mid-run through
session A. Rather than defend the pointer with an environment variable or a per-writer
pointer, 4b deletes the concept.

This is also the decision that most simplifies the implementation. The hand-written
resolution in `queue.rs` — three branches, two error strings, an `io::Result` fallback —
collapses to a struct field clap guarantees. A missing session becomes clap's own exit 2,
so `QueueError::Usage` sheds reachable states at exactly the moment seven new commands
would otherwise have multiplied them.

**Cost:** there is no shorthand for "the session I am working in". After
`cassette session new` the operator copies the id and carries it; `session list` is how a
forgotten id is recovered. If this chafes, the cheapest fix is an environment variable
read at one place, not the pointer's return.

### 2. Sessions are named by id only

`--session` and `session alias` take a ULID. An alias is a **display label** shown in
`session list` — it never resolves. Id prefixes are not accepted either.

**Cost:** `cassette today`, which the parent spec defines as "a session whose alias is
today's date", is not reachable from the CLI by that alias. It remains a TUI entry point,
which is where Phase 6 repoints it; no 4b command needs it.

### 3. `queue next` probes and skips busy cassettes

A one-shot CLI process cannot hold a lock on its caller's behalf — `flock` is released
when the process exits — so `next` can only *report* an id. The agent then calls
`queue write <id>`, which acquires the lock for the duration of its own run.

`next` walks candidates in `queue_order` and `try_lock`s each, returning the first that
acquires and releasing immediately. Two writers working at once get different answers
instead of colliding on the same id.

The TOCTOU window between the probe and the subsequent `queue write` is **accepted, not
closed**: another writer may take the cassette in between. `queue write` already returns
exit 3 with the holder's attribution, so the loser retries. Closing the window properly
would require the sticky lock, which is 4c.

### 4. "Everything is busy" is exit 3, not exit 5

`queue next` distinguishes two empty-handed outcomes, preserving the parent spec's exit
table meaning — 3 says wait, 5 says enqueue:

| Condition | Exit | What the caller should do |
|---|---|---|
| Open cassettes exist, every one is locked | 3 | retry shortly |
| No open cassettes at all | 5 | idle, or `queue new` |

## Command surface

`--session <id>` is required on every `queue` command. `--writer <name>` keeps its 4a
meaning: a registered writer, else exit 2.

```
cassette session new [--alias <name>]        # prints the id
cassette session list [--all]
cassette session alias <id> <alias>

cassette queue list --session <id> [--status open|closed|all] [--since <ts>]
cassette queue next --session <id>
cassette queue new --session <id> <topic> [--first|--last|--priority N]
cassette queue show --session <id> <cassette-id>
cassette queue close --session <id> <cassette-id> [-m "close-out sentence"]
cassette queue reopen --session <id> <cassette-id>
cassette queue move --session <id> <cassette-id> --before <id> | --after <id>
cassette queue write --session <id> <cassette-id> [--side a|b] [--append|--replace]
```

`session use` is **not** implemented — decision 1 removed its only purpose.

## Module layout

`src/queue.rs` becomes `src/queue/`, split on the lock boundary:

| Module | Contents | Locks |
|---|---|---|
| `queue/mod.rs` | `QueueError`, shared ordering and session helpers | — |
| `queue/view.rs` | `list`, `next`, `show` | `next` probes only |
| `queue/edit.rs` | `new`, `close`, `reopen`, `move` | yes |
| `queue/write.rs` | the existing `write`, moved unchanged | yes |

The split is the file layout expressing invariant 1 (`LockGuard::write` is the only code
that writes a cassette). A reviewer confirms "nothing in `view.rs` writes" by reading one
module instead of grepping seven commands. `next`'s probe is the one deliberate exception
and is confined to a single documented function.

`src/session.rs` is new and follows `writer.rs`: pure `render_*` functions over data, plus
thin I/O entry points. `src/cli.rs` remains the only place the command line is read.

## Priorities are positive

`queue new --priority N` is the first path by which a user-supplied priority reaches the
store. `priority::first` treats non-positive results as "no room left" (`below > 0`,
`halved > 0`), so a stored priority of `0` or below would be a value the placement
functions cannot reason about.

4b rejects `N <= 0` at the CLI boundary with exit 2. This is the remaining half of the
parent spec's overflow item: 4a changed the signatures to return `Option`, and 4b closes
the input side.

## The open cap

A session may hold at most **36 open cassettes**, overridable by a new `max_open` config
key. Enforced at the two commands that can raise the count — `queue new` and
`queue reopen` — and violated with exit 6.

The constant is defined **in `src/store/`**, not imported from `app.rs`. `app.rs`'s
`MAX_CASSETTES` is a TUI display concern that happens to share the value; binding the CLI
cap to it would assert an invariant the code does not have.

## Permission boundary on close

From the parent spec: a busy cassette cannot be closed by anyone until its holder
releases (exit 3), and an `agent` refuses to close a cassette whose `locked_by` is set
(exit 4) while a `human` may.

`locked_by` already exists in `CassetteMeta`, so 4b implements this rule in full. Nothing
in 4b *sets* the field — `queue lock`/`unlock` are 4c — so the exit-4 arm is correct but
unreachable until 4c lands. It is implemented now rather than deferred so that 4c adds
commands to an enforced rule rather than a rule and its enforcement at once.

This moves exit code 4 from 4c into 4b. The parent spec's phase list is corrected
accordingly.

## Exit codes

| Code | Meaning | Raised by |
|---|---|---|
| 0 | success | — |
| 1 | I/O failure | any |
| 2 | usage: unknown session, unknown cassette, unregistered writer, missing `--session`, non-positive `--priority` | any |
| 3 | busy — another writer holds the lock, or every open cassette is busy | `next`, `close`, `reopen`, `move`, `write` |
| 4 | sticky-locked — an agent may not close it | `close` |
| 5 | nothing available — no open cassettes | `next` |
| 6 | queue full — the open cap is reached | `new`, `reopen` |

## Unreadable cassettes are counted, not hidden

`Store::scan_session` silently skips any cassette it cannot read or whose frontmatter it
cannot parse. Left alone, a corrupted cassette would vanish from `queue list` — the
operator's only view of the store — and `queue next` would never hand it out. Invisible
work, reported as an empty queue.

`queue list` counts what `scan_session` skipped and prints `N unreadable` to stderr. The
full treatment — rendering a damaged cassette as an error row — stays in Phase 5. This is
only a refusal to misreport the store's contents.

## Gaps carried into 4b

Recorded during 4a and resolved here, because 4b is what makes each load-bearing:

- **`WriterError` splits per call-site capability.** Today every caller matches all four
  variants and `main.rs` carries an arm documented as unreachable. Seven new commands
  would copy that pattern before anyone revisited it. Split so each call site names only
  the failures it can actually produce.
- **`Store::write_writers`** is `pub`, unlocked, unvalidated, and uncalled. If no 4b
  command needs it, delete it; the store's writer mutations go through `ensure_writer`,
  which holds the registry lock.
- **`CASSETTE_WRITER`** is implemented beside `--writer`, with `--writer` winning.
  Decision 1 declined the equivalent for sessions, and the asymmetry is intentional:
  writer identity belongs to an agent process for its lifetime, while a session id is a
  per-invocation argument.

## Removals

Deleted, with their tests:

- the `active` file and `session::active_path`
- `session::read_active`, `session::write_active`
- `Store::active_session`, `Store::set_active_session`

Deleting rather than leaving them unused is deliberate: `Store::write_writers` earned its
place on the gap list by being `pub` and uncalled, and keeping a second such API would
repeat that.

## Testing

Unit tests cover the pure functions — `render_*` output, ordering, cap arithmetic,
priority placement at the boundaries — in the module under test, as elsewhere in this
crate.

CLI tests drive the real binary against a temporary store and assert on exit codes and
stderr, extending `tests/cli.rs`.

Cross-process tests extend `tests/lock.rs` and keep its established discipline: **no
sleeps**. Synchronization is a child's stdin pipe, and a child's acquisition is proven by
output it writes after acquiring — never by the parent's write to its pipe, which proves
only that the bytes were queued. Two cases matter, matching the two-writer concurrency
target already agreed:

1. `queue next` skips a cassette a live holder is in, and returns the next one instead.
2. `queue close` on a cassette held by another writer exits 3.

## Out of scope

- `queue lock` / `unlock`, the `--json` contract and its derived `waiting_on` / `busy` /
  `words` fields — **4c**.
- `cassette sessions`, the interactive picker — **Phase 5**, with the TUI.
- `Store::holds` and lock reentrancy — **Phase 5**; see below.
- Rendering damaged cassettes as error rows — **Phase 5**.

## Corrections to the parent spec

1. **`Store::holds` does not land in 4a, and 4b does not need it.** The parent spec says
   "`queue move` is the first real consumer of `lock_many`, which is why `Store::holds`
   lands in 4a." `Store::holds` answers "do I already hold this lock?", which only arises
   for a process that holds locks across operations — the TUI, holding its focused
   cassette. A one-shot `queue move` starts holding nothing, so `lock_many` alone is
   sufficient. `Store::holds` belongs to Phase 5 and 4a was right not to ship it.
2. **Exit code 4 moves from 4c to 4b**, with the close permission rule it serves.
3. **The active session is removed from the design**, and with it `session use` from the
   4b command list.
