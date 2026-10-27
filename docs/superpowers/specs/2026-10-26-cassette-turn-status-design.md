# Cassette turn status — design

## Why

An agent queued eight cassettes into a session. The human started working through them and
lost track of two things: which cassettes the agent had already picked up, and which of the
ones they had answered had a reply waiting. Every cassette needs to show, at a glance, whose
move it is.

## What it is

A single unicode glyph in each cassette's separator line saying which of three statuses the
cassette is in:

| Status | Meaning | Default glyph |
|---|---|---|
| `human` | ready for input from the human | `✎` (pulses) |
| `agent` | ready for processing by an agent | `⚙` |
| `busy`  | someone is writing or working on it right now | `●` |

A cassette is in exactly one status at a time.

## Goals

- The status is visible for every open cassette in the TUI, both focused and minimized.
- The human's rule holds: when they leave a cassette that was on their turn, having written in
  it, it becomes the agent's turn.
- An agent can mark a cassette "picked up" for the seconds-to-minutes it spends reading and
  thinking, and the human sees that as busy.
- It is built so that more **status sets** can be added later (for example, statuses set by
  hand from the CLI or the TUI) without reshaping this one.

## Non-goals (this round)

- User-defined statuses or status sets. The three statuses are built in; only their glyphs are
  configurable.
- Setting a status by hand.
- Protecting the human's words from an agent write that lands after the agent read the
  cassette. This hazard exists today. Claims make it less likely but do not close it. Recorded
  in `docs/follow-up.md`.
- Detecting that a *second TUI* is focused on an unfocused cassette. Probing flocks from the
  TUI would race agents' writes (see "Busy" below).

## Anti-goals

- A second stored record of a fact the store already has. "Whose turn" is already
  `last_writer`'s kind (`queue::json::waiting_on`). Storing a `turn: agent` field beside it
  would let the two drift apart, which is the two-records-of-one-fact shape CLAUDE.md warns
  about (`ReadOnly`).
- A crashed agent leaving a cassette stuck on busy.
- Eight hard-blinking glyphs. The pulse is soft.

## How the status is worked out (the `turn` set)

The turn set is **derived**, never stored. In order:

1. **busy** if either:
   - this process holds the cassette's lock (the human's focused cassette, which is what an
     agent sees as `busy: true` too), **or**
   - the cassette's lock is held by another writer and the TUI already knows it (the focused
     cassette is read-only `Busy`), **or**
   - the cassette has a live claim (below) by a writer other than the TUI's own writer.
2. **human** if both sides' text is blank (whitespace only), whoever created it (my call). An
   empty cassette is a prompt waiting for words. This covers the bare launch's cassette,
   `-T` template cassettes, `Ctrl+N`, and an agent's or human's `queue new`, all of which
   stamp the creator as `last_writer`. Without this rule, every cassette the human makes
   would show ⚙ before they type a character.
3. **agent** if `last_writer` resolves to a `human` writer.
4. **human** otherwise. That covers an agent wrote last, and also an unresolvable
   `last_writer`: when in doubt, show it as the human's move rather than hide it.

### `last_writer` means "whose turn ended last", so only turns stamp it

The derivation is only as good as `last_writer`. Today several commands that are not turns
overwrite it (verified in `src/queue/edit.rs`): `close`, `reopen`, `lock`, `unlock`,
`queue move`, and `renumber_all`, which stamps **every** cassette in the session and runs
whenever `move` runs out of priority gaps. One agent `queue move` could flip every glyph to ✎.

My call: these commands stop touching `last_writer` and keep updating `updated_at`, extending
the precedent `queue topic` already set ("a retitle is not a turn"). After this change, only
`queue write` and the TUI's flush stamp it. Creation (`queue new`, `create_cassette`) still
records the creator, and the blank-cassette rule above covers it. Consequence: `waiting_on`
in the JSON changes value for cassettes that were only moved, locked, closed or reopened. That
is a fix to its documented meaning, not a change of shape. Phase 1 updates the affected
edit.rs tests, e.g. `edit.rs:1150` asserts on `last_writer`.

Closed cassettes show no glyph.

This reproduces the human's rule without a new transition. Tabbing through a cassette without
typing writes nothing, so `last_writer` and the status are unchanged. Typing makes it dirty,
and autosave stamps the human as `last_writer`. It shows busy while focused, because it is
held, and `agent` once they leave. Seen from the TUI: ✎ → ● while you're in it → ⚙ after Tab.

The TUI needs the last writer's *kind* per cassette. Following the existing `locked_by`
precedent, `Cassette` gains a plain scalar, resolved from `writers.toml` wherever `locked_by`
is resolved today: `session_writer::refresh_from_disk`, the `sync_external_writes` path, and
`load_session_cassettes`. `cassette.rs` still imports nothing. It must survive
`merge_external`'s rebuild the way `priority`/`closed` do, and it must count as a change in its
no-op short circuit the way `locked_by` does: applied in place, without a rebuild. After this
process's own flush, the kind is the TUI writer's own kind, set in memory without re-reading.

### Built for more sets

Introduce the concept with one implementation:

- A `StatusSet` (enum or trait — builder's call) with one member, `Turn`. Each set names its
  statuses, its default glyphs, and a pure `resolve(&Cassette, &StatusContext) -> StatusId`.
  `StatusContext` carries what is not on the cassette: `{ held, now }` (own-lock/read-only
  state, and `now` for claim expiry). *As built:* it has no writer id; the TUI's own claim is
  dropped earlier, in `session_writer::annotate_from_store`, so `Cassette.claimed_until` is only
  ever someone else's.
- A later manual set will store its own frontmatter field and resolve from it. Nothing in the
  turn set should assume it is the only set. There is no config key for choosing a set yet,
  because there is only one.
- Pure. No I/O in the resolver, so it lives with `App`/`Cassette`-level code, and `ui.rs`
  only maps status → glyph + style.

## Claims: the agent's "picked up" mark

### Store

Two new optional frontmatter fields, written after `locked_by`:

```
claimed_by: wri_…        # empty when unclaimed
claimed_until: 2026-10-26T14:05:00Z   # empty when unclaimed
```

- `meta::parse_block`: `claimed_by`, when non-empty, must be a `wri_` id or the cassette is
  damaged, same as `locked_by`. An unparseable `claimed_until` reads as no claim. A typo
  must not wedge a cassette on busy.
- Files without these keys parse as unclaimed, so existing stores need no migration.
- A claim is **live** while `now < claimed_until`. Expiry is evaluated at read time; nobody
  sweeps expired claims off disk.
- Ownership: claim fields belong to the queue commands. `session_writer::flush_held` already
  preserves fields it does not own (it re-reads frontmatter under the lock and replaces only
  `topic`/`last_writer`/`updated_at`). Keep it that way and add a test that an autosave does not
  clear a claim.

### Who may claim

Claims are **agent-only**, the mirror image of `queue lock` being human-only (my call). A human
signals "I'm on this" by focusing the cassette in the TUI, which already shows busy.

- `queue claim` requires an explicit identity: `WriterSource::Flag` (`--writer` or
  `$CASSETTE_WRITER`) resolving to an **agent** writer. An `Env` source (`$USER`, which an agent
  inherits from the human's shell) is `Usage`, exit 2, with `Acting::hint`'s `--writer` advice.
  Otherwise an agent that forgot `--writer` would store the human's own id as `claimed_by`, and
  the resolver would never show that claim as busy. A human writer is also `Usage`, exit 2, like
  an agent invoking `lock`.
- Because only agents claim, the resolver's "a claim by a writer other than the TUI's own" is
  just "any live claim". Keep the comparison anyway, as a guard.

### Commands

- `cassette queue claim <ID> --session <ID> [--for <SECS>]` takes the cassette's lock briefly
  and sets `claimed_by` = acting writer and `claimed_until` = now + duration. Default duration:
  **120 s** (my call). Re-claiming your own claim extends it. Exit codes, matching existing
  conventions:
  - lock held (human focused there): **3**, `Busy`.
  - live claim held by a *different* writer: **4**, `Sticky`. A claim is a soft claim.
  - sticky `locked_by` by someone else, for an agent: **4**, same as `write_permitted`.
  - closed cassette: **2**, `Usage` (builder may revisit if `close`'s precedent differs).
  - Does not touch `last_writer` or `updated_at`. A claim is not a turn, the same reasoning
    `queue topic` uses.
- `cassette queue release <ID> --session <ID>` clears your own claim. Releasing an unclaimed
  cassette, or one whose claim has expired, is a no-op exit 0. Releasing someone else's live
  claim is **4** for an agent; a human may clear any claim (mirroring `unlock`).
- `queue write` by the claim holder clears the claim in the same locked write. A write by a
  *human* over someone's claim succeeds and leaves the claim in place (claims never block a
  human). A write by an *agent* over a different agent's live claim is refused `Sticky` (4),
  for symmetry with `next` skipping it (my call).
- `queue next` additionally skips cassettes with a live claim by another writer. Exit-code
  semantics: a queue where every open, unlocked cassette is claimed by others is **3** (`Busy`,
  wait), not 5.
- Optional convenience, builder's call: `queue next --claim` claims the cassette it returns in
  the same call, closing the gap between "next" and "claim" when two agents run. *As built:* not
  built; recorded in `docs/follow-up.md`. `next` skips every live claim, the caller's own included.

Both new commands: add to `QueueCmd`, its exhaustive `cassette_ids()`/`session()` matches,
`cli.rs`, man/completions (generated, free), and CLAUDE.md's command list.

### JSON

`CassetteView` gains `claimed_by: Option<WriterRef>` and `claimed_until: Option<String>`, both
`null` when there is no live claim (an expired claim is reported as none). Additive, so the
frozen fields don't change. `busy` keeps its meaning (flock held).

### The TUI sees claims

A claim write renames a new file into place, so `sync_external_writes`'s `ChangeStamp` pass
already notices it on the next 1 s tick. `Cassette` carries `claimed_by` (resolved name or
id — builder's call; the resolver only needs "is it someone else's") and `claimed_until`. Expiry
flips the status back to its derived turn on the tick after `claimed_until`, with no file
change needed, because the resolver takes `now`.

### Busy and flock probes

The TUI does **not** probe unfocused cassettes' flocks. `Store::is_free` acquires and releases.
Doing that every tick for every cassette would make an agent's `queue write` fail with exit 3
whenever it collided with the probe, and an agent's flock is only held for milliseconds
anyway. Claims are what make agent work visible for seconds.

## Display

- The glyph goes in `separator_pieces` **after the side tag and before the topic label**, on both
  the focused separator and minimized ones, as its own piece, e.g. `╡ SIDE A ╞ ✎ ╡ topic ╞`.
  Exact spacing is the builder's call. Keep `separator_text` (test helper) in step, as it already is
  by construction.
- Colors flow through `Theme` (never hardcode in `ui.rs`): add per-status colors to `Theme`/
  `ThemeSpec` with defaults in `builtins()` (builder's call on exact colors). Busy should read as
  "hands off", agent as calm, human as the call to action.
- **Pulse:** the `human` glyph alternates between its bright style and a dim one, driven off
  the existing 1 s tick in `main.rs` (an `App` frame counter, as the reel animation does). It
  pulses on every cassette in the `human` status, focused or not. It is not hidden on the off
  beat, so the column never jumps. *As built:* the pulse alternates bold/dim; the agent and busy
  glyphs render bold and never pulse. A 1 s period is fine. The loop does not speed up for this.
- Glyph width: `separator_pieces`' fill math must use `char_width`, not `chars().count()`, since
  a user's configured glyph may be two cells wide.

## Configuration

```toml
[status.turn]            # the set's name; the table is keyed by set from day one
human = "✎"
agent = "⚙"
busy  = "●"
pulse = true             # false turns the human glyph's pulse off
```

- All keys optional. Unset ones fall back to the built-in defaults. An empty string is a
  config error (exit 2, like a bad TOML file), since a status with no glyph is invisible.
- Unknown status names in the table are a config error. Silently ignoring `humna = "x"`
  would leave the user wondering why nothing changed. This does **not** contradict the
  no-`deny_unknown_fields` rule for top-level keys: that rule protects old configs from removed
  keys, and these are new keys validated by hand. Builder: make sure the top-level `Config`
  still ignores unknown keys (the existing test pins it).

## Agent skill

Update `.claude/skills/cassette-session/SKILL.md`:

- Claim before thinking: `queue claim <id> --session <ID> --writer bot` (`next --claim` was not built),
  then read, think, `queue write`, which releases it. If you decide not to answer, `queue release`.
- Pick a `--for` longer than your think time. Expiry is a crash safety net, not a timer to beat.
- `claimed_by` in the JSON: never work on a cassette another writer has claimed.
- What the human sees (the three glyphs), so the agent can talk about them.

## Docs

- CLAUDE.md: the new commands, the `StatusSet` concept in `app.rs`/`ui.rs`'s descriptions, the
  claim fields in `meta.rs`'s ownership note, the `[status.*]` config.
- README: a short "Cassette status" section with the glyph table.
- `docs/follow-up.md`: manual status sets (CLI + TUI), the stale-read overwrite hazard,
  second-TUI busy detection.

## Acceptance

1. A session with an agent-written cassette, a human-written cassette, an empty agent-created
   cassette and an empty `Ctrl+N` cassette shows ✎, ⚙, ✎, ✎ respectively in the TUI. Closed
   cassettes show nothing. An agent `queue move` (including one that triggers a renumber),
   `close`/`reopen`, and a human `lock`/`unlock` leave every glyph unchanged.
1a. `queue claim` without an explicit agent `--writer` exits 2.
2. Tabbing into a ✎ cassette shows ●. Tabbing out without typing restores ✎. Typing then
   tabbing out shows ⚙.
3. `queue claim` on an unfocused cassette turns its glyph to ● within ~1 s. `queue write` by the
   same agent turns it ✎ and the claim is gone from the frontmatter. A claim left to expire turns
   it back to its derived status within ~1 s of `claimed_until`, with no file change.
4. `queue claim` on the human's focused cassette exits 3. On another agent's live claim it exits 4.
   `queue next` skips claimed cassettes and returns 3 when only claimed ones remain.
5. A TUI autosave on a claimed cassette preserves the claim (test).
6. The ✎ glyph pulses. `pulse = false` stops it. Configured glyphs replace the defaults,
   including a two-cell glyph without breaking separator fill.
7. Existing stores (no claim keys) open unchanged. A non-`wri_` `claimed_by` marks the cassette damaged.
8. `cargo test`, `cargo clippy -- -D warnings` green. `/verify` captures screens for 1–3 and 6.

## Left to the builder

Exact separator spacing and per-status colors; enum vs trait for `StatusSet`; whether
`Cassette` carries a resolved claim name or the raw id; `next --claim`; the closed-cassette claim
exit code if a precedent disagrees. Phase it producer-before-consumer (see CLAUDE.md's planning
notes): a plausible split is (1) store fields + claim/release/next/write + JSON, CLI-testable on
its own; (2) TUI status resolution + display + pulse + config; (3) skill and docs.
