# Plan: publish to crates.io + CI publish step (chainlink #12)

**Prerequisites:** #6 done (`cargo publish --dry-run` passes), #11 done
(`CARGO_REGISTRY_TOKEN` secret exists).
**Deliverable:** the crate is live on crates.io and `release.yml` auto-publishes on
future GitHub releases.
**Agent-executable:** mostly — the first local `cargo publish` needs a token the user
holds (`cargo login` is a human step, or run with `CARGO_REGISTRY_TOKEN` in env).

## Steps

1. Preflight: `cargo publish --dry-run` on a clean `main` at the release version.
   Confirm `Cargo.toml` version is the one to claim (bump to `1.0.0` only if this is
   the actual 1.0 release train — otherwise publish the current version to claim the
   name early, which is the intent of the issue).
2. **First publish (claims the name):** user runs `cargo login` (token from #11 or a
   personal one), then agent or user runs `cargo publish`. Verify the crate page
   renders: description, license badge, README.
3. Add a publish job to `.github/workflows/release.yml` (same trigger,
   `release: types: [published]`), **separate job** from the tarball build so a
   packaging failure doesn't block binaries and vice versa:
   ```yaml
   publish-crate:
     name: Publish to crates.io
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
       - name: Install Rust toolchain
         uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8 # stable
       - name: Publish
         run: cargo publish --token ${{ secrets.CARGO_REGISTRY_TOKEN }}
   ```
   Pin action SHAs in the same style as the existing jobs. No `contents: write`
   permission needed for this job.
4. Guard against re-runs: `cargo publish` fails if the version already exists.
   Acceptable for now (re-running the workflow after a successful publish just fails
   this job); optionally add `continue-on-error: false` comment noting this. Do not
   add `--allow-dirty`.
5. Update README install section: add `cargo install <crate-name>` above the
   build-from-source instructions.
6. Verification: `cargo install <crate-name>` in a temp CARGO_HOME actually installs
   and `cassette -V` prints the version. Comment results on #12, close it. #48
   (AUR) unblocks.

## Acceptance criteria

- Crate visible on crates.io; `cargo install` works end-to-end.
- `release.yml` has the publish job wired to `CARGO_REGISTRY_TOKEN`.
- README documents the cargo install path.
