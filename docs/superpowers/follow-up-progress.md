# Follow-up specs — overnight progress

Written unattended on branch `follow-up-specs`, 2026-09-24, one item per loop iteration.
Nothing here is built; every spec lists its judgement calls under **Decisions** for review.
`docs/follow-up.md` entries stay until the work is actually built.

## Items

| # | Item | Status | Spec | Plan |
|---|------|--------|------|------|
| 1 | `queue topic` command | done | `specs/2026-09-24-queue-topic-design.md` | `plans/2026-09-24-queue-topic.md` |
| 2 | `$USER` bootstrap registers a human-kind writer | done | `specs/2026-09-24-implicit-writer-authority-design.md` | `plans/2026-09-24-implicit-writer-authority.md` |
| 3 | Carried triage cleanups (bundle) | done | `specs/2026-09-24-carried-triage-design.md` | `plans/2026-09-24-carried-triage.md` |
| 4 | Reader mode — focus without holding the lock | done | `specs/2026-09-24-reader-mode-design.md` | `plans/2026-09-24-reader-mode.md` |
| 5 | Man page, completions, release packaging | done | `specs/2026-09-24-man-and-completions-design.md` | `plans/2026-09-24-man-and-completions.md` |
| 6 | Document export mode 2 (agent-written document) | done | `specs/2026-09-24-session-writeup-design.md` | `plans/2026-09-24-session-writeup.md` |
| 7 | Queue ordering policy (feed vs topic order) | todo | | |
| — | Tune the agent's cassette length | skipped | needs usage data, not design | |

## Open questions for the user

- **(1) Should `queue topic` bump `last_writer`?** Spec says no — a retitle is housekeeping,
  not a turn, so `waiting_on` shouldn't flip. `close` does bump it. Easy to reverse.
- **(1) Sticky lock blocks an agent from titling a human's locked cassette,** even an untitled
  one. Conservative by choice; say if you'd rather titles be exempt.
- **(2) The fix is wider than the entry said.** An agent inherits the human's *already
  registered* `$USER`, so changing the bootstrap kind wouldn't have prevented Stage 1. The spec
  instead keeps `$USER` for attribution but gives it **agent authority** on the CLI. The
  stricter option is to refuse mutating queue commands with no explicit writer at all, which
  also fixes attribution but breaks `$USER`-only use. Which one do you want?
- **(2) `$CASSETTE_WRITER` stays explicit (human authority)**, so it must not be exported
  globally in a shell an agent uses. Documented, not enforced. OK?
- **(3) Found a real bug behind the weak test assert:** live sync keys on mtime alone, so a
  second write inside one timestamp tick is never merged. The triage spec fixes it with an
  (mtime, len, inode) stamp. This is more than the follow-up asked for; say if you want it split out.
- **(3) `StoredCassette.path` is deleted** rather than kept as a "public store field". Nothing
  reads it and the crate has no lib target. Your earlier note called this a design call.
- **(4) The big one: toggle or lock-on-intent?** The spec recommends lock-on-intent as the end
  state, built as a toggle (`r`) first. Phase 2 adds the idle trigger behind
  `release_idle_secs`, **off by default** until you've lived with it. Intent means entering
  insert mode, not the first keystroke, and idle release only happens from normal mode.
- **(4) Side effect:** cursor motion now works on Busy and Closed cassettes too (a new
  `view_focused`). Motions used to be silently dropped there.
- **(4) Key choice `r`** (vim's replace-char isn't implemented here). Fallbacks: `R` or `Ctrl+R`.
- **(4) Plan note:** a few Task 2 and 3 tests are specified as named assertions next to existing
  fixtures rather than as full code, because those fixtures are long. Worth a look before executing.
- **(5) The follow-up's premise was stale.** The clap derive tree in `cli.rs` has no `crate::`
  paths. They're all in the lowered `Args`/`QueueCmd` types and `From` impls, so option 2 is cheap.
  The spec still picks **runtime generation** (`cassette completions <shell>`, `cassette man`),
  because `cargo install` users never receive `build.rs` output. The cost is two runtime deps;
  the plan measures the size added to the binary.
- **(5) `docs/distribution.md` doesn't exist**, although CLAUDE.md cites it. The plan creates it
  and moves the README's release steps there.
- **(5) Human step:** only a real GitHub release exercises the workflow change.
- **(6) Settled as a skill (`cassette-writeup`) that writes a file outside the store**, not a
  cassette. It proposes the path and an outline in chat and waits for your yes before writing.
  Disagreement is kept under `## Open questions`, unattributed. Is the outline-approval step too
  much ceremony for you?
