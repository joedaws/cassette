# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                    # compile
cargo test                     # run unit tests
cargo run -- -t 10 -w 500      # run with 10-min timer and 500-word goal
cargo run -- -l 8              # run with 8 visible text rows per cassette
cargo install --path .         # install `cassette` binary to PATH
```

## Architecture

`cassette` is a Rust TUI freewriting app built on [ratatui](https://ratatui.rs) with crossterm as the backend. The user writes on one or more "cassettes", each displayed as a colored multi-line widget with a top-fade effect.

### Source layout

**`src/cassette.rs`** — `Cassette` is a cursor-zipper: text is split into `left` (before cursor) and `right` (after cursor); text may contain `\n`. Each cassette has two sides like a tape: the zipper always holds the active side, and `flip()` swaps it with the stored back buffer (`Side::A`/`Side::B`), so each side keeps its own cursor. Side B is a scratch pad; output writes it under a `## Side B` heading when non-empty, and `word_count` covers both sides. Basic ops (`insert`, `backspace`, `delete`, `move_left/right`) plus vim motions (`move_up/down`, `move_row_start/end`, `move_word_forward/back`, `move_text_start/end`, `delete_line`, `open_below/above`). Width-aware motions take the wrap width as a parameter. `wrap_spans`/`pos_to_row_col` compute display rows (character wrap at width, hard break on `\n`) and are shared with `ui.rs`. Pure data model with no rendering logic.

**`src/app.rs`** — `App` holds all application state: the list of cassettes, focus index, terminal dimensions, timer, word goal, reel animation frame, editing `Mode` (`Insert`/`Normal`), and `pending` prefix key for two-key sequences (`dd`, `gg`). State transitions are plain `&mut self` methods with no I/O. `modify_focused` accepts a closure so callers in `main.rs` can apply any `Cassette` operation.

**`src/ui.rs`** — All ratatui rendering. `render` builds a vertical layout with one chunk per *visible* cassette plus the stats bar, a vim-style info line (mode, ln/col, char count, cassette i/n — overridden by `status_msg`), and help row. Cassettes can exceed the screen (up to `MAX_CASSETTES`, 36): `App.cassette_scroll` is the first visible cassette, `ensure_focus_visible` keeps the focused one in the window, and separator rows show "N more ↑/↓" hints for cassettes scrolled out of view. The focused cassette renders full-height on the terminal's default background with a line-number gutter (`GUTTER_WIDTH` cols, `~` on rows past the end); its viewport scrolls typewriter-style (cursor row centered, clamped at the ends) and rows fade toward both viewport edges via `DIM`/`DarkGray` steps. Unfocused cassettes are minimized to their last line (colored background, line number shown). The cursor renders as a bar `│` in insert mode and a solid block in normal mode (both `REVERSED`).

**`src/main.rs`** — Terminal setup/teardown (raw mode, alternate screen), the crossterm event loop, modal key dispatch (`handle_key` → `handle_insert_key`/`handle_normal_key`), and CLI arg parsing (`-t`, `-w`, `-l`, `-o`). Writes markdown on quit (or stdout with `-o`).

### Key bindings

- Both modes: Tab/Shift+Tab switch cassettes, Ctrl+N new cassette, Ctrl+F (or Shift+Enter on kitty-protocol terminals) flips the cassette to its other side, Ctrl+C quit. Side B cues: yellow `╡ SIDE B ╞` woven into the separator, yellow line-number gutter, `· side B` in the info line, `╡ B ╞` tag on minimized cassettes.
- Insert: type to write, Enter for newline, Esc → normal mode.
- Normal (mini vim): `h j k l`, `w b`, `0 $`, `gg G`, `x`, `dd`, `i a I A o O`, `q` quits.

### Conventions

- `App` and `Cassette` are pure: no I/O, no ratatui types. Keep rendering in `ui.rs` and I/O in `main.rs`.
- The cursor is stored implicitly as the split between `left` and `right`; `cursor_pos()` is `left.chars().count()`.
- Wrapping is word-boundary (`wrap_spans`): overflowing words move whole to the next row, the space stays trailing on the previous row (wrapped rows never start with a space), words longer than the width hard-break, and trailing space runs hang past the edge. Width comes from `cassette_width()` (terminal width − 2 − `GUTTER_WIDTH`); the cursor marker occupies one display cell in the wrap calculation. `j`/`k`/`0`/`$` operate on display rows; `dd`/`o`/`O` operate on logical (`\n`-delimited) lines.
- Rows per cassette default to `VISIBLE_LINES` (5) and are configurable via the `-l` CLI flag or `visible_lines` in `~/.config/cassette/config.toml`, clamped to `MIN_VISIBLE_LINES..=MAX_VISIBLE_LINES`.

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

## Best Practices

- Start sessions when beginning work
- Use `ready` to find unblocked issues
- Use subissues for tasks >500 lines
- End with handoff notes before context compresses

---

*Language rules, security requirements, and testing guidelines are in `.chainlink/rules/` and auto-injected based on detected project languages.*

