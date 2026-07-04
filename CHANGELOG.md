# Changelog for `cassette`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## 0.8.0 - 2026-07-04

### Added
- Vim-style modal editing: insert and normal modes with `h j k l`, `w b`,
  `0 $`, `gg G`, `x`, `dd`, `i a I A o O`, and undo (`u`) with per-side
  history — entering insert mode snapshots, so one `u` takes back the whole
  typed burst. Insert mode gains the readline shortcuts `Ctrl+W` (delete
  word) and `Ctrl+U` (delete to line start). (#22, #27, and friends)
- Two-sided cassettes: `Ctrl+B` (or `Shift+Enter` on kitty-protocol
  terminals) flips to side B, a scratch pad with its own cursor and undo
  stack. Both sides are labeled on screen (`╡ SIDE A ╞` / `╡ SIDE B ╞`,
  gutter accents, info line) and saved under `## Side A` / `## Side B`
  headings. (#14, #33)
- Per-cassette topics: set from the `t` / `Ctrl+T` status-line prompt
  (returns to the mode it was opened from), or seed a session with one
  labeled cassette per topic via `-T <name>` and a `[templates]` entry in
  the config. Topics appear in the separator and in the markdown headings.
  (#29, #38)
- Themes: six built-ins (`default`, `dracula`, `gruvbox`, `nord`,
  `solarized-dark`, `solarized-light`), user-defined `[themes.<name>]`
  config tables that extend or override built-ins field-by-field, a
  `theme` config key, a `--theme` CLI override, and a ghostty-style
  `cassette +themes` listing with truecolor swatches. The help line is
  theme-aware: bold key combos, dimmer descriptions. (#35, #37)
- Record mode (`-R` / `--record`, strictly opt-in): the tape only rolls
  forward — typing and Enter work; deletions, cursor movement, and normal
  mode are disabled. Flipping, topics, and cassette switching still work.
  (#39)
- Idle nudge: after 10 quiet seconds in a timed or record session the info
  line shows "tape's still rolling — keep writing"; no bell, cleared by the
  next keypress. Untimed sessions are never nudged. (#40)
- End-of-session summary printed to the terminal: words, duration, pace
  (sessions ≥ 30s), and a per-cassette breakdown. (#41)
- As many cassettes as you like (up to 36) with whole-cassette scrolling
  and "N more ↑/↓" overflow hints; unfocused cassettes minimize to their
  last line. (#8, #15)
- Line-number gutter and a vim-style info line (mode, ln/col, chars,
  cassette n/m, side). (#13)
- Configurable rows per cassette: `-l` flag or `visible_lines` config key
  (2–40). (#3)
- `-h`/`--help` and `-V`/`--version`; invalid flags and values exit 2 with
  an error instead of being silently ignored. (#25)
- Crash safety: dirty sessions autosave every 30 seconds, and the terminal
  is restored and the session saved even on error or panic. Empty sessions
  write no file and clean up their autosaved draft. (#19, #21, #26)
- Timer expiry and word-goal celebrations: transient status flash and a
  terminal bell; reaching the goal locks the stats green. (#23, #24)

### Changed
- Typewriter scrolling: the viewport keeps the cursor row centered and rows
  fade with distance from the cursor, not the widget center. (#30)
- The focused cassette renders on the terminal's default background; the
  reel/info/help footer is pinned to the bottom of the window; progress
  bars render only when a timer or word goal is set, and the reel spinners
  are gone. (#16, #31, #32)
- Side A now carries the loud accent (yellow by default) and side B the
  calm dark gray — swapped from their introduction. (#34)
- Flip side moved from `Ctrl+F` to `Ctrl+B`. (#33)
- Word wrapping counts terminal cells: CJK and emoji are two columns,
  combining marks zero, and wrapped rows never start with a space. (#18, #20)

### Fixed
- The normal-mode block cursor no longer hides a character or shifts the
  text by a cell. (#28)

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

### Added
- Basic interface with Brick of the Cassette.
