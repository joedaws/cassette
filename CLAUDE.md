# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                    # compile
cargo test                     # run unit tests
cargo run -- -t 10 -w 500      # run with 10-min timer and 500-word goal
cargo install --path .         # install `cassette` binary to PATH
```

## Architecture

`cassette` is a Rust TUI freewriting app built on [ratatui](https://ratatui.rs) with crossterm as the backend. The user writes on one or more "cassettes", each displayed as a colored multi-line widget with a top-fade effect.

### Source layout

**`src/cassette.rs`** — `Cassette` is a cursor-zipper: text is split into `left` (before cursor) and `right` (after cursor). Operations `insert`, `backspace`, `delete`, `move_left`, and `move_right` manipulate this split. Pure data model with no rendering logic.

**`src/app.rs`** — `App` holds all application state: the list of cassettes, focus index, terminal dimensions, timer, word goal, and reel animation frame. State transitions (`add_cassette`, `focus_next`, `tick_timer`, etc.) are plain `&mut self` methods with no I/O. `modify_focused` accepts a closure so callers in `main.rs` can apply any `Cassette` operation.

**`src/ui.rs`** — All ratatui rendering. `render` builds a vertical layout with one chunk per cassette plus the stats bar, status line, and help row. Each cassette shows `VISIBLE_LINES` (6) rows of character-wrapped text; the viewport scrolls to keep the cursor at the bottom row and fades older lines toward the cassette background color.

**`src/main.rs`** — Terminal setup/teardown (raw mode, alternate screen), the crossterm event loop, key dispatch via `handle_key`, and CLI arg parsing (`-t`, `-w`). Prints cassette text to stdout on quit.

### Conventions

- `App` and `Cassette` are pure: no I/O, no ratatui types. Keep rendering in `ui.rs` and I/O in `main.rs`.
- The cursor is stored implicitly as the split between `left` and `right`; `cursor_pos()` is `left.chars().count()`.
- Character wrapping uses `cassette_width()` (terminal width − 2); the cursor marker `│` occupies one display cell in the wrap calculation.
- `VISIBLE_LINES` in `app.rs` controls how many text rows each cassette widget shows.

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

