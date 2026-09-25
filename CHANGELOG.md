# Changelog for `cassette`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## Unreleased

The session-store redesign: `cassette` moves from one markdown file per
session to a session directory of per-cassette files, so a human and one or
more agents can write different cassettes of the same session concurrently.

### Added
- **A shared session store** under `$CASSETTE_DATA_DIR` (or the XDG data
  directory). Each session is a directory of per-cassette markdown files with
  frontmatter, plus `flock`-based lock anchors. Sessions are named by ULID;
  an alias is a display label that never resolves in place of an id.
- **`cassette queue`** — `list`, `show`, `next`, `new`, `write`, `close`,
  `reopen`, `move`, `lock`, `unlock`: the scriptable surface an agent drives.
  `--json` on `list`/`next`/`show` emits full objects, and any failing
  command emits an `{"error", "code"}` envelope.
- **`cassette session`** (`new`, `list`, `alias`) and **`cassette writer`**
  (`register`, `list`, `whoami`). A writer's `kind` — human or agent — is a
  permission boundary declared once at registration.
- **`cassette sessions`** — an interactive picker over recent sessions, since
  ULIDs leave no name to type. `j/k`, `/` to filter, `a` for all, Enter opens.
- **`cassette completions <SHELL>` and `cassette man [--out-dir DIR]`** — shell
  completions and man pages generated from the live CLI definition, so they cannot
  drift. The release tarball now ships them under `completions/` and `man/man1/`;
  see `docs/distribution.md`.
- **`cassette queue topic <ID> --session <ID> "<TOPIC>"`** — set, change or clear a
  cassette's topic from the CLI.
- **`cassette export <id> [--out PATH]`** — a session rendered to one flat
  markdown file. Closed and unreadable cassettes are included and marked.
- **Live multi-writer TUI.** The editor holds the lock only for the focused
  cassette, notices what other writers do once a second, shows a cassette
  another writer holds as read-only naming the holder, and becomes editable
  again by itself when they release it.
- **Queue-shaped display.** Cassettes appear in queue order; closed ones fold
  into a `▸ N closed` row toggled with `z`; damaged files get a visible row
  instead of vanishing.
- A warning when the store sits under a syncing folder (Dropbox, iCloud,
  OneDrive, Google Drive). Locks are local kernel state and do not sync, so
  two machines editing one session get no mutual exclusion.

### Changed
- **Ids carry their kind.** Sessions are `ses_…`, cassettes `cas_…`, writers
  `wri_…`. A bare or wrong-kind id is rejected with a message naming both kinds
  (`` `cas_…` is a cassette id; --session takes a session id (ses_…) ``), and a
  cassette whose frontmatter id disagrees with its file name shows as damaged.
  **Breaking:** stores created before this change are not read until migrated
  by hand (README, "Upgrading from bare ULIDs"), and `--json` `id` fields carry
  the prefix.
- **Where your writing lives.** `stats`, `today` and `resume` all read
  the session store. Notes written before this change are untouched on disk
  but are no longer counted by `stats`.
- The data directory is created `0700`; an existing one looser than that has
  its group and other bits cleared.

### Fixed
- **A writer taken from `$USER` no longer carries human authority.** An agent runs in
  the human's shell and inherits `$USER`, so an agent that forgot `--writer` acted as the
  human: sticky locks did not bind it. Such a writer is still attributed by name but is
  refused on sticky writes/closes and on `queue lock`/`unlock`, with a message naming
  `--writer`. Humans pass `--writer <name>` (or set `$CASSETTE_WRITER`) for those.
- **Live sync no longer misses a write that lands inside one mtime tick** — changes are
  detected by mtime, length and inode.

### Removed
- The flat-note format and its machinery: the append-and-re-sum path and its
  `## Session N — HH:MM` headings, the `draft: true` marker and crash-recovery
  prompt, and the `_1.md` conflict-rename dance (ULIDs do not collide).
- The `notes_dir` config key. Unrecognised keys are ignored, so an old config
  still loads — you can delete the line or leave it.
- **`cassette find`.** `cassette sessions` covers it: its `/` filter matches
  the same alias, topics and content, and Enter opens the session. For a
  plain-text list of ids, use `cassette session list`. `cassette find` now exits
  2 as an unknown command.

## 0.10.0 - 2026-09-13

### Added
- `cassette find [TEXT…]`: list recent notes from the notes dir, newest
  first — date, word count, draft marker, topics, and a first-line
  preview, capped at 10 with an "… N more" hint. Any words after `find`
  become one case-insensitive filter over name, topic, and content;
  the listing points at `resume` to pick a note back up. Rows show each
  note's full path (`~`-abbreviated), ready to open in an editor.

### Changed
- The CLI now parses with clap v4 (derive API) instead of a hand-rolled
  parser, restructured into idiomatic subcommands with no top-level
  positional argument. **Breaking:**
  - `cassette mynote` → `cassette new mynote` — a note name is now the
    `new` subcommand's argument, removing the ambiguity between a bare
    positional and a subcommand.
  - `cassette +themes` → `cassette themes` — the `+` sigil only existed
    to dodge a collision with the positional note name, which is gone.
  - `cassette --resume [FILE]` → `cassette resume [FILE]` — an
    optional-value flag is exactly the kind of ambiguity clap can't
    resolve on its own; `resume` is now a subcommand like the others.
  - `cassette stats --version` (and the same after any other subcommand)
    now exits 2 instead of printing the version — `--version` is
    top-level only, matching clap's convention (e.g. `git status
    --version`).

### Fixed
- Resuming a note no longer restamps its frontmatter `date:` with the new
  session's start time, so `stats` streaks and daily totals survive a
  resume on a later day.
- `cassette <name>` of an existing note now resumes it — previously it
  silently conflict-renamed to `<name>_1.md` and opened an empty session.
- A resumed session's stats are session-scoped: the word-goal reel, the
  goal celebration, the info-line `N / goal` counter, and the end-of-run
  recap (`N new words … (M total)`) count only words written this
  sitting; the file's frontmatter keeps the full total.

## 0.9.0 - 2026-07-03

### Added
- Daily practice: `cassette today` opens (or creates) a note named after
  the date in the notes dir; a second session the same day appends as a
  `## Session N — HH:MM` section and re-sums the frontmatter word count,
  instead of the `_1.md` conflict rename (which explicit names keep).
  The filename format is configurable with the `daily_format` config key
  (chrono syntax, default `%Y-%m-%d`). (#42)
- `cassette stats`: current daily streak (tolerant of an unwritten today),
  notes and words this week and this month, and totals — read straight
  from the frontmatter of the notes dir. No new state anywhere. (#43)
- Resume: `cassette --resume [FILE]` parses a saved note back into
  cassettes (topics, sides, cursor at the end) and keeps writing to the
  same file; with no argument it resumes the most recently modified note.
  Autosaves now mark the note `draft: true` until the session finishes
  cleanly, so after a crash the next launch offers the draft for resume
  ([y/N] prompt; declining clears the marker). (#44)
- Signal safety: SIGTERM and SIGHUP save the session and restore the
  terminal before exiting; Ctrl+Z (and SIGTSTP) flushes the note, hands
  the terminal back to the shell, and resumes cleanly with a redraw after
  `fg`. (#45)
- Bracketed paste: a paste lands as one edit — one undo snapshot for the
  whole chunk, `\r\n`/`\r` normalized to `\n` — instead of a stream of
  keystrokes. Pasting into the topic prompt joins lines. (#46)

### Changed
- The config file lives at `$XDG_CONFIG_HOME/cassette/config.toml`
  (falling back to `~/.config/cassette/config.toml`) on every platform;
  on macOS it previously resolved to `~/Library/Application Support`. (#47)
- A config.toml that exists but fails to parse now exits with the TOML
  error (file, line, column) instead of silently running with defaults. (#36)

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
