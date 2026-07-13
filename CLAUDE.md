# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                    # compile
cargo test                     # run unit tests
cargo run -- -t 10 -w 500      # run with 10-min timer and 500-word goal
cargo run -- -l 8              # run with 8 visible text rows per cassette
cargo run -- -T morning        # one cassette per topic from the 'morning' config template
cargo run -- --theme gruvbox   # themed session; `cassette +themes` lists themes
cargo run -- today             # daily note: one file per day, sessions append
cargo run -- stats             # streak + weekly/monthly totals from frontmatter
cargo run -- find [text]       # list recent notes newest-first, optionally filtered
cargo run -- --resume          # load the newest note back into the TUI
cargo install --path .         # install `cassette` binary to PATH
```

## Architecture

`cassette` is a Rust TUI freewriting app built on [ratatui](https://ratatui.rs) with crossterm as the backend. The user writes on one or more "cassettes", each displayed as a colored multi-line widget with a top-fade effect.

### Source layout

**`src/cassette.rs`** — `Cassette` is a cursor-zipper: text is split into `left` (before cursor) and `right` (after cursor); text may contain `\n`. Each cassette has two sides like a tape: the zipper always holds the active side, and `flip()` swaps it with the stored back buffer (`Side::A`/`Side::B`), so each side keeps its own cursor — and its own undo stack (`snapshot`/`undo`, capped at `UNDO_DEPTH`). Side B is a scratch pad; output writes side A under a `## Side A` heading always and side B under `## Side B` when non-empty, and `word_count` covers both sides. Each cassette has an optional `topic` label (shared by both sides) that appears in the separator and in the markdown heading (`# Cassette 1 — topic`). Basic ops (`insert`, `insert_str` — bracketed paste as one edit, `backspace`, `delete`, `move_left/right`, `delete_word_back`, `delete_to_line_start`) plus vim motions (`move_up/down`, `move_row_start/end`, `move_word_forward/back`, `move_text_start/end`, `delete_line`, `open_below/above`). Width-aware motions take the wrap width as a parameter. `from_sides` rebuilds a cassette from saved text for `--resume` (side A active, cursor at its end, no undo history). `wrap_spans`/`pos_to_row_col` compute display rows (cell-width wrap via `char_width` — CJK/emoji count 2, combining marks 0 — hard break on `\n`) and are shared with `ui.rs`. Pure data model with no rendering logic.

**`src/app.rs`** — `App` holds all application state: the list of cassettes, focus index, terminal dimensions, timer, word goal, reel animation frame, editing `Mode` (`Insert`/`Normal`/`Topic` — `Topic` is a status-line prompt that captures `topic_input` for the focused cassette and returns to `topic_return`, the mode it was opened from), and `pending` prefix key for two-key sequences (`dd`, `gg`). `apply_topics` seeds the session from a `-T` template: one cassette per topic, capped at `MAX_CASSETTES`. `record` (the `-R` flag) marks a forward-only session; `idle_secs`/`tick_idle`/`idle_nudge` drive the "tape's still rolling" hint, shown after `IDLE_NUDGE_SECS` quiet seconds in timed or record sessions only and reset by any keypress in `handle_key`. State transitions are plain `&mut self` methods with no I/O. `modify_focused` accepts a closure so callers in `main.rs` can apply any `Cassette` operation; it also sets `dirty` for the autosaver. `load_cassettes` replaces the list with cassettes parsed from a saved note (`--resume`), focusing the last one. Timer expiry and reaching the word goal `flash()` a transient status message (self-clears after `STATUS_FLASH_SECS` via `tick_status`) and request a terminal bell through the `bell` flag, which `main.rs` consumes — as is `suspend`, the one-shot Ctrl+Z suspend request.

**`src/ui.rs`** — All ratatui rendering. `render` builds a vertical layout with one chunk per *visible* cassette plus the reel stats bar (drawn only when a word goal or timer is set: tape winds from the supply reel onto the take-up reel as `tape_ratio()` — words over the word goal, or elapsed time when only a timer is set; `tape_ratio()` is `None` and no bars render without either), a vim-style info line (mode, ln/col, char count, cassette i/n — overridden by `status_msg`), and help row; the reel/info/help footer is pinned to the bottom of the window by a `Min(0)` filler between it and the cassette stack (the closing `─` separator stays directly under the stack and carries the ↓ overflow hint). Cassettes can exceed the screen (up to `MAX_CASSETTES`, 36): `App.cassette_scroll` is the first visible cassette, `ensure_focus_visible` keeps the focused one in the window, and separator rows show "N more ↑/↓" hints for cassettes scrolled out of view. The focused cassette renders full-height on the terminal's default background with a line-number gutter (`GUTTER_WIDTH` cols, `~` on rows past the end); its viewport scrolls typewriter-style (cursor row centered, clamped at the ends) and rows fade with distance from the cursor row via `DIM`/`DarkGray` steps, so the bright band follows the cursor even when the scroll clamp leaves it off-center. Unfocused cassettes are minimized to their last line (colored background, line number shown). In insert mode the cursor is a dedicated `REVERSED` bar cell (`│`) between left and right; in normal mode it's a `REVERSED` block overlaid on the char under the cursor (a space at line ends), so columns stay true — the text may reflow by one cell on mode switch. A reached word goal styles the stats green+bold, winning over the expired-timer red.

**`src/main.rs`** — Terminal setup/teardown (raw mode, alternate screen, bracketed paste), a panic hook + `catch_unwind` so the terminal is restored and the session saved even on error/panic, unix signal handling (`signal-hook` flags polled by the event loop: SIGTERM/SIGHUP quit through the normal save path; SIGTSTP or the Ctrl+Z binding run `suspend_session` — flush the note, restore the terminal, raise SIGSTOP, re-enter raw mode + redraw on SIGCONT), the crossterm event loop (`Event::Paste` → `handle_paste`: newline-normalized, one undo snapshot, topic prompt joins lines), modal key dispatch (`handle_key` → `handle_insert_key`/`handle_normal_key`/`handle_topic_key`; topic mode owns the keyboard so Tab/flip can't move the prompt off its cassette; in record mode `handle_insert_key` accepts only chars and Enter — no deletions, movement, or Esc), CLI arg parsing (`-t`, `-w`, `-l`, `-T`, `--theme`, `-R`, `-o`, `-h`, `-V`, `--resume [file]`, plus the `today`, `stats`, `find`, and `+themes` actions; invalid values and unknown `-T` templates or `--theme` names exit 2 with an error), and `session_summary` — the words/duration/wpm recap (plus per-cassette breakdown) printed to stderr after every non-empty session. Resume resolution happens before the terminal is touched: `--resume` picks its note (explicit name or newest by mtime via `newest_note`), or — when a note in the notes dir still carries the autosave `draft: true` marker from a crashed session — a stderr `[y/N]` prompt offers it (declining strips the marker with `clear_draft_flag`); the parsed cassettes land in the app via `load_cassettes` and the `Sink` writes straight back to the same file.

**`src/stats.rs`** — the `cassette stats` action: `scan_notes_dir` reads every note's YAML frontmatter into `NoteMeta` (`date`, `words`; notes without a parseable date are skipped), and `render` prints streak (consecutive days, tolerant of an unwritten today), this-week (Monday start) and this-month notes · words, and totals. Pure functions over the frontmatter — the notes dir is the only database.

**`src/find.rs`** — the `cassette find [TEXT…]` action: `parse_entry` builds a `NoteEntry` (frontmatter date — file mtime as fallback — word count, draft marker, topics from `# Cassette … — topic` headings, first-body-line preview truncated to 72 chars) and `render` prints the newest-first listing (capped at 10 with an `… N more` hint, `--resume` footer), filtered by the query words joined into one case-insensitive substring match over name + content. Same shape as `stats.rs`: pure functions plus a thin `scan_notes_dir`.

**`src/theme.rs`** — `Theme` (resolved ratatui colors: optional focused `text`/`background` — `None` means terminal defaults — plus `unfocused_bg`/`unfocused_fg`, per-side `accent_a`/`accent_b`, and `help_key`/`help_text` for the help line, whose key combos render bold in `help_key` and descriptions in `help_text` via `ui::help_line`) and `ThemeSpec` (the config-file form: optional color strings, `"#rrggbb"` or ANSI names via `parse_color`). `builtins()` ships default, dracula, gruvbox, nord, solarized-dark/-light. `resolve(name, user_themes)` looks up user `[themes.<name>]` specs first — a spec overrides the same-named built-in field-by-field, or extends the default look for new names — then built-ins; theme selection is config `theme = "…"` overridden by `--theme`. `all()` builds the `+themes` listing.

**`src/output.rs` / `src/config.rs`** — The output file (`Sink`) is resolved at startup; dirty sessions autosave to it every `AUTOSAVE_SECS` with a `draft: true` frontmatter marker (`write_markdown(app, path, draft)`), and `finish_session` writes final markdown on quit (or stdout with `-o`) — empty sessions write nothing and clean up any autosaved draft. `cassette today` names the note after the date (`daily_format` config key, validated chrono format); when today's note already exists the `Sink` carries an `output::AppendBase` and every save rewrites base content + a `## Session N — HH:MM` section with re-summed frontmatter counts (`write_markdown_appended`), so autosaves never double-count — an empty appending session restores the base instead of deleting the file. `output::parse_markdown` is the inverse of `build_body` (used by `--resume`); `output::is_draft` checks the marker. `config::load_config` returns `Result` — a config file that exists but doesn't parse exits 2 with the TOML error — and `config::config_path` resolves `$XDG_CONFIG_HOME/cassette/config.toml` falling back to `~/.config/cassette/config.toml` on every OS (never `dirs::config_dir()`, which would be `~/Library/Application Support` on macOS).

### Key bindings

- Both modes: Tab/Shift+Tab switch cassettes, Ctrl+N new cassette, Ctrl+T topic prompt (returns to the mode it came from), Ctrl+B (or Shift+Enter on kitty-protocol terminals) flips the cassette to its other side, Ctrl+Z suspend (flushes the note first; raw mode swallows the tty's own TSTP so this is an explicit binding), Ctrl+C quit. The active side is always tagged in the separator (`╡ SIDE A ╞` in the loud accent — yellow by default — `╡ SIDE B ╞` in the calm one — dark gray; `╡ A ╞`/`╡ B ╞` on minimized cassettes) and in the info line (`· side A`/`· side B`); the line-number gutter carries the same per-side accent. A set topic follows the side tag as a bold `╡ topic ╞` label.
- Insert: type to write, Enter for newline, Ctrl+W deletes the word before the cursor, Ctrl+U deletes to line start, Esc → normal mode.
- Normal (mini vim): `h j k l`, `w b`, `0 $`, `gg G`, `x`, `dd`, `u` (undo; entering insert mode snapshots so one `u` undoes the whole typed burst), `i a I A o O`, `t` opens the topic prompt (Enter commits, blank clears, Esc cancels), `q` quits.
- Record mode (`-R`, opt-in): chars and Enter only — deletions, cursor movement, and normal mode are disabled; flip/topic/cassette switching still work; quit with Ctrl+C. Info line shows `-- RECORD --`.

### Conventions

- `App` and `Cassette` are pure: no I/O, no ratatui types. Keep rendering in `ui.rs` and I/O in `main.rs`.
- The cursor is stored implicitly as the split between `left` and `right`; `cursor_pos()` is `left.chars().count()`.
- Wrapping is word-boundary (`wrap_spans`) and counts terminal cells, not chars (`char_width`): overflowing words move whole to the next row, the space stays trailing on the previous row (wrapped rows never start with a space), words longer than the width hard-break, and trailing space runs hang past the edge. Width comes from `cassette_width()` (terminal width − 2 − `GUTTER_WIDTH`); in insert mode the cursor bar occupies one display cell in the wrap calculation. `j`/`k`/`0`/`$` operate on display rows; `dd`/`o`/`O` operate on logical (`\n`-delimited) lines.
- Rows per cassette default to `VISIBLE_LINES` (5) and are configurable via the `-l` CLI flag or `visible_lines` in `~/.config/cassette/config.toml`, clamped to `MIN_VISIBLE_LINES..=MAX_VISIBLE_LINES`.
- Topic templates live in config.toml as `[templates]` entries (`morning = ["gratitude", "priorities"]`) and are selected with `-T <name>`.
- All rendering colors flow through the active `Theme` (resolved in `main.rs`, passed to `ui::render`); never hardcode colors in `ui.rs`. Themes come from config (`theme = "name"`, `[themes.<name>]` tables) or `--theme`; with explicit text+background colors the focused fade lerps between them (`themed_fade`), otherwise it falls back to the modifier-based fade.

## Tools

Use `chainlink` cli to track tasks across AI sessions. Data in `.chainlink/issues.db`.

## Commands

```bash
# Issues
chainlink create "title" [-p high] [-d "desc"]
chainlink list [-s all|closed] [-l label] [-p priority]
chainlink show|update|close|reopen|delete <id>
chainlink subissue <parent> "title"

# Organization
chainlink comment <id> "text"
chainlink label|unlabel <id> <label>
chainlink block|unblock <id> <blocker>
chainlink blocked|ready

# Sessions
chainlink session start|end|status|work <id>
chainlink session end --notes "handoff context"
```

## Workflow

1. `session start` → see previous handoff
2. `session work <id>` → mark focus
3. Work, add comments
4. `session end --notes "..."` → save context

## Implementation plans (`docs/plans/`)

Some issues have a pre-written plan at `docs/plans/issue-NN-<slug>.md`, referenced in a
comment on the issue (`chainlink show <id>` surfaces it). When implementing an issue:

- Read the plan first and follow it — steps, file paths, and acceptance criteria were
  written against this repo. Verify claims about the code against the current source
  before acting; the code may have moved since the plan was written.
- Plans flag **human steps** (browser logins, tokens, pushes to external services).
  Don't attempt these — do the agent-executable parts, then tell the user exactly what
  remains.
- Check the plan's acceptance criteria before closing the issue, and note in the closing
  comment anything that deviated from the plan or couldn't be verified.
- If a plan turns out to be wrong or stale, update the plan file in the same change and
  say so in an issue comment — the files are the durable record, not this session.
- **Delete the plan file when closing its issue** (same change). First promote any
  durable decisions out of it into real docs (`docs/distribution.md`, README) — the plan
  is scaffolding, not documentation. Git history is the archive; `docs/plans/` should
  only ever contain live, actionable plans.

## Best Practices

- Start sessions when beginning work
- Use `ready` to find unblocked issues
- Use subissues for tasks >500 lines
- End with handoff notes before context compresses

---

*Language rules, security requirements, and testing guidelines are in `.chainlink/rules/` and auto-injected based on detected project languages.*

