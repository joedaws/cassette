# Changelog for `cassette`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to the
[Haskell Package Versioning Policy](https://pvp.haskell.org/).

## 0.7.0 - 2026-05-31

### Added
- Sessions are now saved to a markdown file by default on quit. Files are written to
  `~/.local/share/cassette/notes/` with an auto-generated timestamp filename
  (e.g. `2026-05-31T10-30-00.md`).
- Pass a name as a positional argument to choose the output filename:
  `cassette myjournal` saves to `myjournal.md` in the notes directory.
- Pass an absolute or relative path (anything containing `/`) to write to an arbitrary
  location: `cassette /tmp/draft.md`.
- Add `-o` flag to print output to stdout instead of writing a file (restores the
  previous default behaviour).
- Config file at `~/.config/cassette/config.toml` with optional `notes_dir` key to
  override the default notes directory.
- If the resolved output file already exists, the session is saved with an incremented
  suffix (`_1`, `_2`, …) and a warning is printed.
- Frontmatter in every saved file records `date`, `word_count`, `cassettes`, and
  optionally `timer` and `word_goal` when those flags were passed.

## 0.1.0.0 - 2025-11-13

### Changed
- When users submit a file name when invoking cassette, store the output of the session in that file as markdown. Store some basic metadata like time of writing and the parameters of the sessoin in frontmatter of the markdown (#7)
- update the .github workflows from stack and haskell working to new rust and cargo (#9)
- update .gitignore so that it is appropriate for a rust project (#10)
- Change the implementation so that we can configure N lines being shown. The initial behavior is that the cursor stays directly in the middle, but we should transition to something that is more like traditional editors, and show N lines (thinking about targeting 5 to 7 lines). The text show start to fade at the top. (#3)
- Rewrite the applicatoin in Rust using the ratatui library (#2)
- create new cargo project and initialize it with ratatui as dependency. Choose a structure of project that cleanly separates application state code, UI code, and main.rs (#4)
- Rename the project as cassette and make updates to the code base to replace tape with cassette for variables and types. (#1)

- Basic interface with Brick of the Cassette.
