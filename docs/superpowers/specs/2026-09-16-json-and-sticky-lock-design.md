# Phase 4c — JSON and the sticky lock

**Parent spec:** `2026-09-13-session-store-design.md`, the binding authority. This document
covers what 4c decides, and records where the parent's command surface has drifted from the
implementation (see **Inherited gaps**).

**Predecessor:** `2026-09-15-session-queue-core-design.md` (4b), merged as `b1a624c`.

## Purpose

4b gave a human and an agent a shared queue. 4c makes it machine-drivable — a structured
output contract an agent can parse — and gives the human a way to say *hands off this one*
that the agent must respect.

These two halves are one phase because they are the same promise from opposite ends: the
JSON tells an agent what it may take, and the sticky lock is how a human removes something
from that answer.

## Inherited gaps

Three divergences between the parent spec and the code, all found while designing 4c. The
first two are 4c's to fix; the third is deferred with the reason recorded.

1. **`queue next` ignores `locked_by`.** The parent spec defines it as "the highest-priority
   open cassette carrying **no sticky lock** and no live flock", but the implementation
   filters only on `Status::Open` and the advisory lock. This was invisible in 4b because
   nothing could set `locked_by`; the moment `queue lock` exists, `next` starts handing out
   cassettes a human has claimed. **4c fixes this.**
2. **`queue write` ignores `locked_by` entirely.** See decision 1 — 4c makes it bind.
3. **`queue write` has no `--side`, `--append` or `--replace`.** The parent spec's command
   surface lists all three; 4a implemented replace-whole-body only, and the omission was not
   recorded. Cassette bodies are therefore flat text with no side structure. **Deferred to
   Phase 5**, where the TUI makes sides real — but written down here because it has now been
   missed twice, once in 4a and once in 4b's review.

## Decisions

### 1. The sticky lock binds on write, not only on close

An agent's `queue write` against a cassette whose `locked_by` is set fails with **exit 4**.

The parent spec's lock table promises a blocked writer "durable — escalate (exit 4)", but the
only rule it states is that an agent may not *close* a sticky-locked cassette. Enforcing on
close alone would make the lock a scheduling hint: it would hide the cassette from
`queue next`, while any agent already holding the id — from an earlier `next`, or from a
`list` — could still overwrite the text a human locked. The lock must mean what a human
reaching for it assumes it means.

The lock does **not** restrict other humans. Only `human` writers may set or clear it
(parent spec, "Capability follows `kind`"), so a human blocked by one can always
`queue unlock` and proceed. Blocking humans from each other would add multi-writer semantics
this tool does not have, and would let one terminal lock another out of the same person's
work.

Where the lock binds, in full:

| Actor and action | Result |
|---|---|
| agent `queue write` on a sticky-locked cassette | exit 4 |
| agent `queue close` on one | exit 4 (implemented in 4b, unreachable until now) |
| `queue next`, any caller | skips sticky-locked cassettes |
| human, any of the above | permitted |
| agent `queue lock` or `queue unlock` | exit 2 — see below |

An agent invoking `queue lock`/`unlock` is **exit 2**, not 4. Exit 4 means "you have
encountered a durable claim; escalate to a human". An agent calling a human-only command has
encountered no claim — it has misused the CLI, which is what exit 2 is for. Reusing 4 would
tell an agent to escalate a bug in its own invocation.

### 2. `--json` covers the read commands, and errors everywhere

- `queue list`, `queue next`, `queue show` emit the full contract object on success.
- **Any** command invoked with `--json` emits `{"error": "<message>", "code": N}` on failure,
  where `N` is the exit code the command would otherwise return.
- Mutating commands (`queue new`, `write`, `close`, `reopen`, `move`, `lock`, `unlock`,
  and the `session`/`writer` commands) emit nothing extra on success. `--json` is accepted
  and changes only their failure output.

An agent's error handling is then uniform across the whole loop, without freezing a success
shape for every mutating command before anything needs one. `queue new` already prints a bare
id, which is parseable as it stands.

### 3. `side_a` and `side_b` stay in the contract

The contract keeps both fields even though nothing writes sides yet (inherited gap 3). The
body is split on `## Side A` / `## Side B` headings — the format `output.rs` already writes —
and when neither heading is present the whole body is `side_a` and `side_b` is `""`.

Agents code against the final shape now, TUI-written cassettes parse correctly the moment
Phase 5 lands, and the contract never changes. The alternative — emitting a flat `body` today
— would force either a breaking change in Phase 5 or a permanent duplicate field.

### 4. `serde_json`, not hand-rolled serialization

Cassette bodies are arbitrary user prose: quotes, backslashes, newlines, control characters,
and any Unicode the writer typed. Hand-rolling escaping for that is a correctness bug waiting
to be found by someone's apostrophe. `serde` is already a dependency; `serde_json` joins it.

### 5. `queue lock` is idempotent for its own holder

- `queue lock` on a cassette **you** already hold: success, no write, exit 0.
- `queue lock` on a cassette **another writer** holds: exit 4.
- `queue unlock` on a cassette that is not locked: success, no write, exit 0.
- `queue unlock` on a cassette **another writer** holds: permitted — any human may clear any
  sticky lock, per the parent spec's capability rule.

Settled explicitly because 4b's final review found no-op transitions handled inconsistently
(`queue close` on an already-closed cassette appends a second close-out blockquote, while
`queue reopen` on an already-open one can exit 6). A no-op should not write.

## The JSON contract

Emitted by `queue list` (an array of cassettes), `queue next` (the one cassette), and
`queue show` (the one cassette). The envelope carries the session so an agent has its context
without a second call.

```json
{
  "session": { "id": "01K5GQ2R8V3XQZ0000000000AB", "alias": "monday" },
  "cassettes": [{
    "id": "01K5GR7T2M9WPD0000000000AB",
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
  }],
  "unreadable": 0
}
```

`alias` is `null` when unset. `topic` is `null` when unset. `unreadable` (only on
`queue list --json`'s `Listing`, not on the single-cassette `queue next --json` /
`queue show --json` payload) is the same count `render_list`'s prose `N unreadable`
line carries, from the same scan: a cassette file the store cannot parse must not
leave the machine consumer of `queue list` any less aware of store damage than the
human one, which already counts rather than hides it. `0` in the overwhelmingly
common case.

### Derived fields

None of these are stored; all are computed at read time, which is why no two of them can
disagree with each other.

| Field | Derivation | Edge case |
|---|---|---|
| `words` | `split_whitespace().count()` over `side_a` + `side_b`, matching `Cassette::word_count` | empty body → `0` |
| `busy` | `store::lock::probe` on the cassette's anchor — the **non-stamping** probe 4b added for `queue next` | absent anchor → `false` |
| `waiting_on` | the inverse of `last_writer.kind`: `"agent"` when a human wrote last | `last_writer` unresolvable → `null` |
| `sticky_lock` | `locked_by` resolved against `writers.toml` to `{name, kind}` | unset → `null`; unresolvable id → `null` |
| `created_by`, `last_writer` | writer ids resolved to `{name, kind}` | unresolvable id → `null` |

**`busy` must use `probe`, never `Store::lock`.** `lock::acquire` stamps the holder's
attribution into the anchor after acquiring, so deriving `busy` by acquiring and dropping a
guard would make a read-only command write to every cassette it reports on. 4b added `probe`
for exactly this reason; `--json` is its second caller.

An unresolvable writer id emits `null` rather than a fabricated name or an error. A writer
missing from `writers.toml` is a damaged store, and a listing is not a repair tool — the same
stance `queue list` already takes toward unparseable cassettes, which it counts rather than
hides.

### Reading N files is not a snapshot

Each cassette file is written atomically, so no body is ever torn. But a `list` of nine
cassettes is nine reads, and cassette 1 may be staler than cassette 9. Agents must not assume
the array is a point-in-time view of the session. This is inherent to plain files and is not
worth a lock to fix: holding every cassette's lock to build a listing would let one busy
cassette block all reading, which contradicts "never block on a lock a human can hold".

### Bodies are emitted in full

Cassettes are short by design, and a second round trip to fetch 400 words is not worth
saving. A polling agent re-ingests every body on each call — `--since` and `queue next` exist
to scope that, and `list` defaults to open-only so bodies stay bounded. `--status all` grows
without limit as closed cassettes accumulate; that is the caller's choice to make.

## Command surface

```
cassette queue lock <ID> --session <ID>      # human writers only
cassette queue unlock <ID> --session <ID>    # human writers only
```

`--json` is a global flag, accepted on every command, changing output only as decision 2
describes.

This is deliberately unlike the TUI-only globals listed under **Open items carried from 4b**
(`-R`, `-t`, `--theme` and the rest), which are accepted on `queue` subcommands and do
nothing at all. `--json` has a defined effect on every command it is accepted on — the error
envelope, at minimum — so being global is honest for it and misleading for them.

## Exit codes

Unchanged from 4b except that **exit 4 becomes reachable**.

| Code | Meaning | Newly raised by in 4c |
|---|---|---|
| 2 | usage | an agent invoking `queue lock`/`unlock` |
| 3 | busy — another writer holds the advisory lock | `queue lock`, `queue unlock` |
| 4 | sticky-locked | agent `queue write`; `queue lock` over another writer's claim |

## Module layout

- `src/queue/json.rs` (new) — the contract types and their `serde::Serialize` impls, plus the
  body splitter and the derived-field computation. Pure functions over data: no I/O, so the
  contract's shape is unit-testable without a store.
- `src/queue/edit.rs` — gains `lock` and `unlock` beside `close`/`reopen`, which they
  resemble: acquire, read through the guard, check permission, write one frontmatter field.
- `src/queue/view.rs` — `list`/`next`/`show` gain a rendering branch. Still no writes.
- `src/queue/write.rs` — gains the sticky-lock check.
- `src/cli.rs` — the `--json` global flag and the two new subcommands.

## Testing

- **Unit** — the contract's shape against known data (every derived field, including each
  edge case in the table above), the body splitter with and without side headings, and the
  permission matrix for lock/unlock/write/close across both writer kinds.
- **CLI** — `--json` output parsed back with `serde_json` and asserted field by field, never
  by string comparison against a formatted blob. The error envelope on a representative
  failure from each exit code.
- **Cross-process** — `busy: true` is observed while a real second process holds the lock,
  following `tests/lock.rs`'s existing discipline: no sleeps, no polling, acquisition proven
  by output the child writes after acquiring.

## Out of scope

- `--side` / `--append` / `--replace` on `queue write` — **Phase 5** (inherited gap 3).
- The TUI's sticky-lock indicators and the collapsed closed row — **Phase 5**.
- `cassette sessions`, the interactive picker — **Phase 5**.
- An agent-settable claim on a cassette — **deferred** in the parent spec, and unchanged here:
  `last_writer` plus priority ordering is still assumed sufficient for multi-agent contention.

## Open items carried from 4b

Recorded for triage, not necessarily 4c's to fix:

- An unreadable `session.toml` (a permissions error, say) reads as "no session" → exit 2,
  where the parent spec reserves exit 1 for I/O failures. **4c should settle this**, because
  `--json` gives the error a `code` field and a wrong code is now machine-visible.
- The `--session` validation gate lives in `main.rs`, so module-level entry points still
  accept arbitrary strings. Unchanged risk while the CLI is the only caller.
- TUI-only flags (`-R`, `-t`, `-w`, `-l`, `-T`, `--theme`, `-o`) are accepted and silently
  ignored on `queue`/`session`/`writer` subcommands, and advertised in their `--help`.
- `queue close` on an already-closed cassette appends a second close-out blockquote;
  `queue reopen` on an already-open one can exit 6. Decision 5 above sets the pattern for
  lock/unlock; these two remain inconsistent with it.
- `writer whoami` exits 0 on an unknown name, where every queue command exits 2.
- `queue new` accepts an empty topic.
