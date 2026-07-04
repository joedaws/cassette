# Plan: implement distribution strategy using cargo (chainlink #6)

**Prerequisite:** #5's design doc (`docs/distribution.md`) exists and names the crate.
**Deliverable:** `Cargo.toml` ready to publish — `cargo publish --dry-run` passes.
The actual publish and CI wiring belong to #12; AUR to #48. This issue is the
cargo-side groundwork.
**Agent-executable:** fully.

## Context

- `Cargo.toml` today contains only `[package] name/version/edition` and dependencies.
- License file is BSD-3-Clause (`LICENSE` at repo root). Repo:
  `https://github.com/joedaws/cassette`.
- If #5 decided the crate name is `cassette-tui` (name collision on crates.io),
  the binary must stay `cassette`.

## Steps

1. Add publish metadata to `[package]` in `Cargo.toml`:
   ```toml
   description = "A TUI freewriting app: write on tape-like cassettes with timers, word goals, and themes"
   license = "BSD-3-Clause"
   repository = "https://github.com/joedaws/cassette"
   readme = "README.md"
   keywords = ["tui", "writing", "freewriting", "journal", "ratatui"]   # max 5
   categories = ["command-line-utilities", "text-editors"]
   ```
2. If the crate name changes per #5 (e.g. `cassette-tui`): set `name = "cassette-tui"`
   and add
   ```toml
   [[bin]]
   name = "cassette"
   path = "src/main.rs"
   ```
   Then grep the repo for `cargo install --path .` docs — README install text stays
   valid, but update any `cargo install cassette` mention to the real crate name.
3. Add an `exclude` (or `include`) list so the package stays lean — exclude
   `.chainlink/`, `.github/`, `docs/plans/`, any GIF/tape assets from #49.
4. Verify: `cargo package --list` (inspect contents), `cargo publish --dry-run`
   (must succeed; it compiles the packaged crate). `cargo build` and `cargo test`
   still pass.
5. Close #6 with a comment noting the dry-run result; #12 does the real publish.

## Acceptance criteria

- `cargo publish --dry-run` succeeds locally.
- `cargo package --list` shows no junk (no `.chainlink/`, no plans, no workflows).
- Installed binary name is `cassette` regardless of crate name.
