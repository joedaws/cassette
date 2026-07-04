# Plan: crates.io API token → GitHub repo secret (chainlink #11)

**Deliverable:** repo secret `CARGO_REGISTRY_TOKEN` exists on
`github.com/joedaws/cassette`, scoped to publishing.
**Agent-executable:** partially — token creation is a browser step only the user can
do; the agent verifies and can set the secret if handed the token.

## Steps

1. **Human step (Joseph):** log in to https://crates.io (GitHub OAuth) →
   Account Settings → API Tokens → New Token.
   - Name: `cassette-ci-publish`
   - Scopes: `publish-new` **and** `publish-update` (new is needed for the very first
     publish; update for subsequent releases).
   - Optionally restrict to the crate name once it exists.
   Copy the token — crates.io shows it once.
2. Add it as a repo Actions secret. Either the user pastes it to the agent and the
   agent runs:
   ```bash
   gh secret set CARGO_REGISTRY_TOKEN --repo joedaws/cassette
   ```
   (reads the value from stdin — do **not** put the token in the command line or in
   any file), or the user does it in the browser: repo Settings → Secrets and
   variables → Actions → New repository secret.
3. **Agent verification:**
   ```bash
   gh secret list --repo joedaws/cassette
   ```
   must show `CARGO_REGISTRY_TOKEN`.
4. Comment on #11 with the verification output (name + updated date only, never the
   value) and close it. #12 unblocks.

## Acceptance criteria

- `gh secret list` shows `CARGO_REGISTRY_TOKEN`.
- Token has publish-new + publish-update scopes (user-confirmed).
