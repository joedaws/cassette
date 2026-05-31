# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
stack build          # compile
stack test           # run all tests (hspec + hlint)
stack test tape:tape-test   # hspec tests only
stack test tape:lint        # hlint only
stack run -- -t 10 -w 500  # run with 10-min timer and 500-word goal
stack install        # install `tape` binary to PATH
```

## Architecture

`tape` is a Haskell TUI freewriting app built on [Brick](https://github.com/jtdaugherty/brick). The user writes on one or more "tapes" displayed as cassette-style widgets.

### Core data model

**`src/Tape.hs`** — `Tape` is a cursor-zipper: text is split into `leftText` (before cursor) and `rightText` (after cursor). Operations like `insert`, `backspace`, `forward`, and `rewind` manipulate this split. `printTape` renders a fixed-width window centered on the cursor.

**`src/Event.hs`** — `St` is the Brick app state (all fields are `microlens-th` lenses). Pure state transformers (`addTapeToSt`, `focusNextSt`, etc.) are separated from the monadic `appEvent` handler so they can be tested directly. Key bindings are defined as constants at the top of this module.

**`src/Deck.hs`** — Layout constants: `tapeRows` (rows per tape widget) and `tapeWidth` (text region width derived from terminal width). `calcMaxTapes` in `Event.hs` uses these to cap tape count to available vertical space.

**`src/Render.hs`** — Character-level rendering pipeline. `Effect = RenderCtx -> Attr -> Attr` is a composable function applied per character. `edgeFadeEffect` implements the focused/unfocused color gradient.

**`app/Main.hs`** — Brick app wiring: `drawUI`, `appEvent`, attr map, CLI arg parsing (`-t`, `-w`). The countdown timer runs on a background thread that pushes `Tick` events over a `BChan`. On quit, tape contents are printed to stdout.

**`src/Document.hs`** — Stub type, currently unused.

### Conventions

- Lenses are generated with `makeLenses ''TypeName` (Template Haskell); use `microlens` operators (`^.`, `.~`, `%~`, `use`, `assign`).
- Pure state logic lives in `Event.hs` as plain functions on `St`; the monadic `appEvent` calls `T.modify` on them. Keep this separation when adding new behavior.
- GHC warnings are treated as errors (`-Wall` + compat flags in `package.yaml`); hlint runs as a separate test suite against `src/` and `app/`.

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

