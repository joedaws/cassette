# `queue topic` — set a cassette's topic from the CLI

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s "`queue topic`" entry.
> Every judgement call is listed under **Decisions** so it can be overturned in review.

## Purpose

A topic can be set at creation (`queue new "<topic>"`) or from the TUI (`Ctrl+T`/`t`), and
nowhere else. An agent therefore cannot title a cassette it did not create, and cannot retitle
one whose subject has drifted — which, under the distillation model the Stage 2 trial arrived
at, is normal: a cassette is rewritten into what it is now about, and its title should follow.
In the trial this left two human-created cassettes `(untitled)` in every listing permanently.

## Shape

```
cassette queue topic <ID> --session <ID> "<TOPIC>"
```

- `<TOPIC>` is a required positional. It is trimmed; an empty or all-whitespace value
  **clears** the topic (`topic: None`) — the TUI's blank-input-clears rule, verbatim
  (`main.rs` `handle_topic_key`: `trim()`, then `(!trimmed.is_empty()).then_some(..)`).
- A topic containing `\n` is a usage error (exit 2), mirroring `close -m`'s
  `close_message_line`. The TUI prompt joins pasted lines with spaces, but the CLI rejects
  rather than silently rewriting what an agent passed — `meta::one_line` would otherwise
  flatten it on write and the agent would never learn its title was altered.
- Exit codes follow the queue contract unchanged: 0 ok, 2 usage (bad id, newline, unknown
  `--writer`), 3 busy (someone holds the cassette's `flock`), 4 sticky (agent over another
  writer's `queue lock`), 1 I/O.
- No output on success, like `close`/`reopen`. `--json` changes only the error shape, as it
  does for them.

## Behaviour

`queue::edit::retopic(store, session, id, topic, who_name, source)`, shaped exactly like
`close`:

1. Validate the topic (newline check, trim-to-`Option`) — no I/O.
2. Resolve the writer, keeping its `Kind` (`Env` → `resolve_writer`, `Flag` → `require_writer`).
3. `store.lock(session, id, &who)` → `lock_error_to_queue_error` (Busy 3 / unknown id 2 / Io 1).
4. `guard.read()`, then the sticky check: an **agent** facing a set `locked_by` gets `Sticky`
   (exit 4). This is `close_permitted`'s rule, reused — see Decisions.
5. Set `meta.topic` and `meta.updated_at`; **leave `last_writer` alone**; write the unchanged
   body through the guard.
6. If the new topic equals the stored one, return `Ok(())` without writing (idempotent no-op,
   the `lock`/`unlock` precedent) — so a repeated retitle does not bump `updated_at` and
   wake every TUI's sync for nothing.

Closed cassettes may be retitled. A closed cassette is still listed (`--status all`), shown by
`export`, and searched by `find`; a wrong title on one is as misleading as on an open one.

The file name is **not** renamed. `ids::file_name`'s slug is frozen at creation and
`meta.topic` is already documented as the source of truth for the display name
(`store/meta.rs`), and `LockGuard` resolves its path once at acquisition — a rename under a
held lock would strand the anchor.

## Precedence with the TUI

The follow-up entry asks how this and the TUI's `flush_held` (which writes `topic` from memory)
agree. They already do, because both go through the cassette's lock and the TUI re-reads on
every acquire:

- **TUI holds the cassette** (it is focused): `queue topic` gets `Busy`, exit 3 — the same as
  `queue write`/`close`. The agent retries later. No lost update.
- **TUI does not hold it** (unfocused, or in another session): `queue topic` writes.
  `sync_external_writes` notices the mtime change on the next tick and `App::merge_external`
  adopts the new topic (topic is already part of its change test). If the human then focuses
  it, `SessionWriter::refresh_from_disk` re-reads under the lock before any flush, so the
  in-memory topic is current before `flush_held` can write it back.

Nothing new is needed on the TUI side. The plan adds a test that pins the unfocused case
end-to-end (write via the CLI, sync, flush after refocus → the CLI's topic survives), since
that is the path a regression would break silently.

## Decisions

1. **Command name `topic`, not `retitle`/`rename`.** Matches the field name, the TUI's
   wording ("topic prompt"), and the follow-up entry's proposed shape.
2. **Positional topic, not `--topic`.** `queue new` takes the topic positionally; the two
   should read alike. The function is named `retopic` only because `topic` reads as a noun in
   Rust call sites (`edit::topic(..)`).
3. **Sticky rule = `close_permitted`'s** (agent blocked by *any* `locked_by`, human never
   blocked), not a new rule. A sticky claim says "this cassette is mine"; its title is part
   of it. Consequence: an agent cannot title even an untitled human cassette the human has
   `queue lock`ed. That is the conservative direction.
4. **`last_writer` is not changed.** `last_writer` is the "whose turn ended last" routing hint
   `waiting_on` derives from. Retitling is housekeeping, not a turn — if an agent retitles the
   human's cassette, the session should still say it is waiting on the agent (or whoever it
   was waiting on). `close` does bump `last_writer`, but closing ends the cassette's
   conversation; retitling does not. *Flagged as an open question.*
5. **`updated_at` is bumped.** It is "when did this file last change", which `--since`
   filters on, and a retitle is a change.
6. **Reject newlines instead of flattening.** See Shape.
7. **No length cap.** `queue new` has none, the TUI has none, and `ids::slug` already caps
   what reaches the file name. Adding one only here would make the CLI stricter than creation.

## Out of scope

- Renaming the file on retitle (see Behaviour).
- A `--topic` on `queue write` to retitle-and-rewrite in one lock — tempting under the
  distillation model, but it is two concerns in one flag; revisit if agents turn out to do
  the pair every time.

## Acceptance criteria

- `queue topic <id> --session <sid> "new"` exits 0; `queue show` then prints `new`; the file
  name is unchanged; the body bytes are unchanged; `last_writer` is unchanged.
- `queue topic <id> --session <sid> "   "` clears the topic; `queue list` shows `(untitled)`
  (or the list's empty-topic rendering).
- A topic containing `\n` exits 2 and writes nothing.
- Against a cassette whose lock another process holds: exit 3.
- Agent against a sticky-locked cassette: exit 4; human against the same: exit 0.
- Same topic twice: second call exits 0 and leaves `updated_at` untouched.
- An unfocused cassette retitled from the CLI keeps its new topic after the TUI syncs,
  focuses it, and flushes a keystroke.
- CLAUDE.md's command list and the `queue/{mod,view,write,edit}.rs` paragraph mention
  `queue topic`; `.claude/skills/cassette-session/SKILL.md` teaches it; the follow-up entry is
  removed from `docs/follow-up.md` in the implementing change.
- `cargo test` and `cargo clippy --all-targets -- -D warnings` green.
