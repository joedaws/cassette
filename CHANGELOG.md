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
- Update the graphics and animation. The progress bar appears to do nothing and is not helpful. It could just be that I don't understand it and that is the issue. (#17)
- Add B side to each cassette accessible by shift + enter or return. This flips the current cassette to a new buffer that you can use as a scratch pad to get some words out before flipping back to side A to continue the main thought (#14)
- The cassette in focus should not have a custom background color, instead it should just use the same background as the shell or terminal from which it was spawned. The out of focus cassettes can keep their background to visually separate themselves from the other ones. (#16)
- add minimization of cassette not in focus to last line. Still show the line number (#15)
- add line numbering for each casset and also a similar to vim info line with number of characters (#13)
- Update the datastructure for each cassettes to be better for multiline. Right now sometimes the new line can start with a space, this looks awkward but I think there are classic solutions to this from word editors, even though this is a freewriting app and not a word editor. (#18)
- Currently you can only create the same number of cassettes as will fit in the current size of the terminal. Let the user create as many cassettes as they like and implement somekind of scorlling view to accommodate new cassettes that require more space. (#8)
- When users submit a file name when invoking cassette, store the output of the session in that file as markdown. Store some basic metadata like time of writing and the parameters of the sessoin in frontmatter of the markdown (#7)
- update the .github workflows from stack and haskell working to new rust and cargo (#9)
- update .gitignore so that it is appropriate for a rust project (#10)
- Change the implementation so that we can configure N lines being shown. The initial behavior is that the cursor stays directly in the middle, but we should transition to something that is more like traditional editors, and show N lines (thinking about targeting 5 to 7 lines). The text show start to fade at the top. (#3)
- Rewrite the applicatoin in Rust using the ratatui library (#2)
- create new cargo project and initialize it with ratatui as dependency. Choose a structure of project that cleanly separates application state code, UI code, and main.rs (#4)
- Rename the project as cassette and make updates to the code base to replace tape with cassette for variables and types. (#1)

- Basic interface with Brick of the Cassette.
