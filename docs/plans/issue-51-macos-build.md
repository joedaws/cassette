# Plan: macOS build support (chainlink #51)

**Blocker status:** chainlink still lists "Blocked by: #47", but #47 closed on
2026-07-04 (config now resolves `~/.config` on every OS) — this issue is ready;
unblock it in chainlink if the tool doesn't auto-clear.
**Deliverable:** `cassette-macos-aarch64.tar.gz` in releases + verified/documented
macOS behavior.
**Agent-executable:** mostly — CI does the building; hands-on verification needs a
Mac or a user with one (flag what couldn't be verified rather than claiming it).

## Context

- `.github/workflows/release.yml` already contains the macOS matrix entry,
  commented out: `runner: macos-14, os: macos, arch: aarch64,
  artifact: cassette-macos-aarch64.tar.gz`.
- Platform-sensitive code: `signal-hook` is already `cfg(unix)` (fine on macOS);
  config path is XDG-style everywhere after #47; **notes dir** is
  `dirs::data_local_dir()/cassette/notes` (`src/config.rs:58`), which on macOS is
  `~/Library/Application Support/cassette/notes`.
- Decision the issue defers: keep `data_local_dir` on macOS or go XDG for data too.
  **Recommendation: keep `data_local_dir`, document it.** Rationale: notes are user
  data, `notes_dir` in config already lets anyone relocate them, and changing the
  default later is non-breaking for new installs but silently splits data for
  existing ones — decide now, before 1.0 ships. Record the decision in the README
  and in `docs/distribution.md`.

## Steps

1. Uncomment the macOS matrix block in `.github/workflows/release.yml`. Check the
   cache keys already vary on `matrix.os`/`matrix.arch` (they do) and that the
   "Package artifact" step is platform-neutral (bsdtar on macOS handles `tar -czf`
   fine). If #50 landed, its packaging changes must work on macOS too (`cp -r` is
   fine).
2. Cheap pre-release check: run a one-off workflow_dispatch or push a test tag on a
   branch to confirm the macOS job compiles — don't discover a compile break during
   a real release. Alternatively `cargo check --target aarch64-apple-darwin` locally
   after `rustup target add aarch64-apple-darwin` (links won't run, but catches
   cfg/code errors — note: full linking needs a Mac SDK, so treat check-passing as
   necessary, not sufficient).
3. README updates:
   - Replace "Only Linux is supported" (README ~line 57) with Linux + macOS
     (Apple Silicon), listing both tarballs.
   - Document macOS paths: config `~/.config/cassette/config.toml` (same as Linux),
     notes `~/Library/Application Support/cassette/notes` (or `notes_dir` in config).
4. Runtime verification on real macOS (**human step** if no Mac available; ask the
   user or note it on the issue as follow-up):
   - Terminal.app **and** a kitty-protocol terminal (kitty/Ghostty): Shift+Enter
     flip works on the latter, Ctrl+B everywhere; Option-key input doesn't produce
     stray chars.
   - Ctrl+Z suspend/resume (SIGTSTP path), Ctrl+C save-on-quit, `--resume`,
     `today`, `stats`.
   - Config + themes load from `~/.config/cassette/`.
5. Comment findings on #51 (what was CI-verified vs. Mac-verified vs. untested),
   close when the release artifact exists and the README is updated.

## Acceptance criteria

- Release workflow produces `cassette-macos-aarch64.tar.gz` on the next release.
- README no longer says Linux-only and documents macOS paths.
- Notes-dir decision recorded (keep `data_local_dir`, per recommendation) — in
  README and `docs/distribution.md`.
- Untested items (if any) listed explicitly in the closing comment.
