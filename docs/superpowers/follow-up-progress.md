# Follow-up specs — overnight progress

Written unattended on branch `follow-up-specs`, 2026-09-24, one item per loop iteration.
Nothing here is built; every spec lists its judgement calls under **Decisions** for review.
`docs/follow-up.md` entries stay until the work is actually built.

## Items

| # | Item | Status | Spec | Plan |
|---|------|--------|------|------|
| 1 | `queue topic` command | done | `specs/2026-09-24-queue-topic-design.md` | `plans/2026-09-24-queue-topic.md` |
| 2 | `$USER` bootstrap registers a human-kind writer | done | `specs/2026-09-24-implicit-writer-authority-design.md` | `plans/2026-09-24-implicit-writer-authority.md` |
| 3 | Carried triage cleanups (bundle) | todo | | |
| 4 | Reader mode — focus without holding the lock | todo | | |
| 5 | Man page, completions, release packaging | todo | | |
| 6 | Document export mode 2 (agent-written document) | todo | | |
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
