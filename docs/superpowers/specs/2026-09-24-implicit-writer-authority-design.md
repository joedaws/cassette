# Implicit writers carry no human authority

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s carried-triage entry
> "`$USER` bootstrap registers a *human*-kind writer". Judgement calls are under **Decisions**.

## The problem, restated precisely

Every queue command picks its writer the same way (`main.rs` `resolve_writer_name`):
`--writer` → `$CASSETTE_WRITER` → `$USER`. The first two are `WriterSource::Flag` and must name
a registered writer; `$USER` is `WriterSource::Env` and is bootstrapped as **human** on first
sight (`store::writers::resolve`).

The follow-up entry frames this as "the bootstrap registers the wrong kind". The trial shows the
problem is wider. An agent runs in the human's shell and inherits the human's `$USER`, which is
almost always *already registered* — by the human's own first TUI launch. So an agent that
forgets `--writer` does not bootstrap anything: it resolves to the human's existing identity and
acts with every privilege the human has. Changing the bootstrap kind would not have prevented the
Stage 1 false bug report at all. (The skill's "one trap" paragraph documents the symptom; it is
guidance, and guidance is what failed.)

No environment variable can tell the agent and the human apart — the agent inherits all of
them, `$CASSETTE_WRITER` included. The only thing that differs between the two invocations is
whether the caller **said who it is on the command line**.

## The rule

**Identity may be implicit; authority may not.**

- A writer resolved from `$USER` (`WriterSource::Env`) is still *attributed* to that name —
  `last_writer`, the lock anchor, bootstrap-on-first-run all stay as they are.
- But for every **permission** decision it is treated as `Kind::Agent`, the unprivileged kind,
  regardless of the kind registered under that name.
- Human authority requires the name to have been *given*: `--writer <name>` or
  `$CASSETTE_WRITER`, resolving to a writer registered as `human`.

What "authority" covers today — the four places a `Kind` is checked:

| Check | Where | Implicit writer now gets |
|---|---|---|
| write over a sticky `locked_by` | `queue::write::write_permitted` | `Sticky`, exit 4 |
| close over a sticky `locked_by` | `queue::edit::close_permitted` | `Sticky`, exit 4 |
| `queue lock` / `queue unlock` | `queue::edit::{lock,unlock}` | `Usage`, exit 2 |
| retitle over a sticky lock (if `queue topic` lands first) | `queue::edit::retopic` via `close_permitted` | `Sticky`, exit 4 |

Everything else — `write`/`close`/`reopen`/`move`/`new` on an unclaimed cassette — works exactly
as before. An implicit writer loses only what an agent never had.

**The TUI is unaffected.** `resolve_tui_writer` resolves identity for attribution and discards
the kind; it makes no permission decision. A person at the keyboard of a full-screen TUI is the
one case where `$USER` genuinely identifies who is acting.

## Shape

One resolver replaces the eight copies of the `match source { Env => resolve_writer, Flag =>
require_writer }` block (seven in `queue/{write,edit}.rs`, one in `main.rs`):

```rust
// src/queue/mod.rs
pub struct Acting {
    /// Writer id — attribution. Always the registered identity.
    pub id: String,
    /// Kind for permission checks. `Agent` whenever `source` is `Env`.
    pub authority: Kind,
    pub source: WriterSource,
}

pub fn resolve_acting(store: &Store, name: &str, source: WriterSource) -> Result<Acting, QueueError>
```

Folding the eight copies into one is the point, not tidying: with eight, the next command added
copies the block and silently skips the downgrade. `main.rs`'s `resolve_tui_writer` keeps
calling the store directly — it wants the registered identity and no authority, and routing it
through `resolve_acting` would hand the TUI a kind it must then remember to ignore.

### Error messages

A refusal caused by the downgrade must say so, or the human who hits it at their own shell will
be as confused as the agent was. When `source == Env` and the registered kind is human, the
`Sticky`/`Usage` message gains a suffix:

> `… — '$USER' alone does not carry human authority; pass --writer jozzef to act as yourself`

`writer whoami` shows it too, since that is where the skill tells an agent to look:

```
jozzef  human  01K…   (implicit via $USER: acts as agent — pass --writer for human authority)
```

## Decisions

1. **Downgrade authority, don't change the bootstrap kind.** Registering a first-seen `$USER`
   as agent would fail closed for the wrong reason — it would mint the *human's* identity as an
   agent on a fresh store whenever the agent happened to run first, and the TUI would then write
   under an agent id forever. And it would not fix the observed case (see "The problem").
2. **Downgrade, don't refuse.** The stricter alternative — every mutating queue command requires
   an explicit writer — also closes the hole but breaks every existing `$USER`-only script and the
   ordinary case of a human typing `queue write` at their shell. The downgrade only bites when a
   sticky lock or a human-only command is actually involved. *Open question for the user.*
3. **`$CASSETTE_WRITER` stays `Flag`.** It is inherited by agents just like `$USER`, so a human who
   exports it globally re-opens the hole for themselves. Treating it as implicit too would leave
   only `--writer`, which is honest but makes the documented env override pointless. Keep it
   explicit and **document** that it must not be exported globally in a shell an agent uses. The
   skill already tells agents to pass `--writer bot`, which outranks it.
4. **Attribution unchanged.** An agent that forgets `--writer` still writes under the human's id,
   so `waiting_on` will misroute that one turn. Fixing attribution needs the explicit-writer
   refusal of Decision 2; the downgrade fixes the part that produced a false bug report and
   silently bypassed a claim.
5. **Unregistered `$USER` still bootstraps as human.** Unchanged — with the downgrade, that kind
   confers nothing on the CLI until someone names it explicitly.

## Out of scope

- Any agent-detection heuristic (`$CLAUDECODE` etc.). Rejected: a heuristic that fails open is
  the problem being fixed.
- Changing `--json` wire shapes. No new fields; error envelopes carry the new message text only.

## Acceptance criteria

- With `$USER=joseph` registered human and a cassette sticky-locked by another writer:
  `queue write` with no `--writer` exits 4; with `--writer joseph` exits 0.
- `queue lock` with no `--writer` exits 2 and the message names `--writer`; with `--writer joseph`
  exits 0.
- `queue write` on an unclaimed cassette with no `--writer` still exits 0 and attributes the write
  to `joseph`.
- `writer whoami` with no `--writer` prints the implicit-authority note; with `--writer` it does not.
- `grep -n "WriterSource::Env =>" src/queue` finds only `resolve_acting`.
- The TUI still attributes to `$USER` and still opens on a sticky-locked cassette as before.
- SKILL.md's "one trap" paragraph is rewritten as a failsafe description; CLAUDE.md's
  `WriterSource` sentence names the downgrade; the follow-up entry is removed when built.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` green.
