<!-- Project-Specific Rules -->
<!-- Add rules specific to your project here. Examples: -->
<!-- - Don't modify the /v1/ API endpoints without approval -->
<!-- - Always update CHANGELOG.md when adding features -->
<!-- - Database migrations must be backward-compatible -->

## Implementation plans

- Issues may reference a pre-written plan at `docs/plans/issue-NN-<slug>.md` (see the
  issue's comments). Read and follow the plan before implementing; verify its claims
  against current source.
- Plans flag human-only steps (logins, tokens, external pushes) — do the
  agent-executable parts and report what remains.
- Check the plan's acceptance criteria before closing the issue; if the plan is stale,
  update the plan file and note it in an issue comment.
- When closing the issue: promote durable decisions from the plan into real docs, put
  deviations/outcomes in the closing comment, then delete the plan file in the same
  change (git history is the archive).
- Full guidance: "Implementation plans" section in CLAUDE.md.
