# Plan: design distribution strategy using cargo (chainlink #5)

**Deliverable:** a short design doc, `docs/distribution.md`, that the implementation
issues (#6, #12, #48, #50, #51) execute against. No code changes.
**Agent-executable:** fully.

## Context

- Repo: `github.com/joedaws/cassette`, Rust 2021, version 0.9.0, license BSD-3-Clause.
- `Cargo.toml` currently has **no publish metadata** (no description, license,
  repository, readme, keywords, categories).
- `.github/workflows/release.yml` builds `cassette-linux-x86_64.tar.gz` on GitHub
  release publish; a macOS matrix entry exists but is commented out.
- This issue predates #11/#12/#48/#50/#51, which are effectively the concrete pieces
  of the strategy. The design doc's job is to make the overall picture explicit and
  record the decisions those issues need.

## Steps

1. Check crate-name availability: `https://crates.io/crates/cassette`. As of writing
   this is likely **taken** (a parser-combinator crate has used the name). Record the
   finding. If taken, decide and record the fallback name — recommend `cassette-tui`
   as the *crate/package* name while the installed *binary* stays `cassette` (set
   `[[bin]] name = "cassette"` in Cargo.toml). The same name question applies to the
   AUR (#48).
2. Write `docs/distribution.md` covering, in order of priority:
   - **crates.io** (`cargo install <crate-name>`): primary channel. Metadata needed,
     publish is manual-first then automated in CI (#12).
   - **GitHub release binaries**: already working for Linux; macOS added by #51.
   - **AUR** (#48): source-build PKGBUILD from the release tag; primary dev is on Arch.
   - **Versioning/tagging**: tags are `vX.Y.Z` matching `Cargo.toml` version; a GitHub
     release publish triggers the workflow; crates.io publish keys off the same event.
   - **Artifacts per release**: tarball(s) per platform, plus man page + completions
     (#50) once they exist.
   - Explicitly out of scope for 1.0: Homebrew, Windows, distro repos beyond AUR.
3. Keep the doc under ~80 lines. Decisions, not prose.

## Acceptance criteria

- `docs/distribution.md` exists, states the crate name decision (with the
  availability check result), the channels, and the release/tag flow.
- Issue #5 closed with a comment pointing at the doc; #6 unblocks.
