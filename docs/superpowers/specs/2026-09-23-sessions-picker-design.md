# Phase 5d — The sessions picker

## Purpose

Sessions are named by ULID. `cassette find` prints them and `cassette resume <id>` opens one,
which closes the loop — but it closes it through a 26-character identifier the human has to
copy by hand. The parent spec's own reasoning: notes had human-typed names, sessions do not,
so *selection has to be a picker*.

`cassette sessions` opens an interactive list of recent sessions; Enter opens the highlighted
one in the TUI.

## Why this is its own phase

Split out of 5c on 2026-09-23. 5c was four changes to the existing stack render; this is a new
full-screen UI with its own event loop, which is the same reason Phases 4 and 5 were split.

## What already exists, so this phase does not rebuild it

`find::scan_store` already produces, per session, exactly what a picker row needs: the session
id, its date from `session.toml`, words summed the way `Cassette::word_count` sums them, topics
in queue order, and a preview drawn from the highest-priority cassette — plus the `unreadable`
count. `build_haystack` already assembles the text `find`'s query matches against.

**This phase shares that code rather than copying it.** A second scanner would be a second
answer to "what sessions exist", and the two would drift.

Three changes fall out of sharing it, two of which are items already on the carried triage list
— this is the phase whose work touches them, so it is where they get fixed:

1. **`NoteEntry.path` is renamed to `id`.** It has held a session id since 5a; the name is a
   leftover from the flat-note era and actively misleads.
2. **`NoteEntry` gains `alias: Option<String>`.** `find` already *matches* on alias through
   `build_haystack` but never prints it, so a row can match a query for a word the user cannot
   see. The picker needs it displayed anyway ("alias shown where set, id otherwise"), and
   `find` shows it too once it is there.
3. `scan_store` and `build_haystack` become `pub(crate)`.

## Correction already applied to the parent spec

The parent spec said Enter "opens the session in the TUI **and marks it active**". Phase 4b
removed the active-session pointer and made `--session <id>` required on every queue command,
so there is no active session to mark. Corrected in place on 2026-09-23; the picker opens the
session and nothing more.

## Structure

The project's boundary is: pure state with no I/O and no ratatui, rendering in `ui.rs`, I/O and
the event loop in `main.rs`. The picker follows it exactly rather than inventing a second shape:

- **`src/picker.rs`** — `Picker`, a pure state machine over a `Vec<NoteEntry>`: cursor
  movement, the recent/all toggle, filter text and the filtered view, and what Enter selects.
  No `Store`, no ratatui, no `std::fs`. Testable without a terminal, the way `App` is.
- **`src/ui.rs`** — `render_picker`, taking `&Picker` and the active `Theme`. Colours come from
  the theme like everything else here; nothing is hardcoded.
- **`src/main.rs`** — `run_picker`: terminal setup/teardown, the crossterm event loop, and the
  handoff. It reuses the existing panic-hook-and-restore path, because a picker that panics
  must not leave the terminal in raw mode either.

`Picker` holds `entries` (everything scanned) and derives the visible rows, rather than keeping
a second filtered `Vec` in sync — one source of truth, the lesson 5b and 5c both paid for.

## Behaviour

**Rows.** Newest first. Each shows the date, the alias where set and the id otherwise, the word
count, and the topics or preview — the same material `find` prints, since they answer the same
question.

**Opening.** `Enter` selects the highlighted session. The picker **returns its id**; `main.rs`
then runs the ordinary TUI on it. The picker does not launch the TUI itself: returning a choice
keeps it a state machine that tests can drive to completion without a terminal.

**Keys.**

| Key | Does |
|---|---|
| `j` / `↓`, `k` / `↑` | move the highlight |
| `Enter` | open the highlighted session |
| `a` | toggle between the 15 most recent and all sessions |
| `/` | start filtering; typing narrows the list live |
| `Esc` | leave filter mode, keeping the filter; from the list, quit |
| `q` | quit without opening |
| `Ctrl+C` | quit without opening |

`/` filter mode is modal the way the TUI's topic prompt is: while it is open the list keys are
text, so `q` types a `q` rather than quitting. This is the established pattern in this codebase
(`Mode::Topic` owns the keyboard), and the alternative — filter keys leaking into navigation —
is exactly the bug that pattern exists to prevent.

The 15-session default reuses `session::DEFAULT_LIST_LIMIT` rather than declaring a second
constant, so `sessions` and `session list` cannot disagree about what "recent" means.

**Filtering** matches the same haystack `find` matches: id, alias, topics, and every cassette's
body, case-insensitively. A filter that matches nothing shows an empty list and says so; it is
not an error.

**An empty store** shows "no sessions yet" and takes `q`. It is not an error and not a panic.

**Damaged sessions.** `scan_store` already returns an `unreadable` count; the picker shows it
as a footer line, the way `find` does. A session whose `session.toml` will not parse is skipped
by `Store::list_sessions` and so never reaches the picker — unchanged from today.

## Testing

- **Unit (`picker.rs`)** — cursor movement clamps at both ends and does not wrap (a list is not
  a carousel; wrapping past the end of 200 sessions is disorienting); the `a` toggle changes
  the visible count and keeps the highlighted session highlighted **by id**, not by index, the
  way `App::sort_queue` does; filter text narrows the list; a filter matching nothing leaves
  the cursor valid; `Enter` on an empty list selects nothing rather than panicking.
- **Rendering (`ui.rs`)** — a `TestBackend` pass asserting a row's alias and date are drawn and
  the highlight is on the expected row. 5c shipped two display bugs that a green suite could
  not see; the fixture must make rows *distinguishable on screen* or the assertion proves
  nothing.
- **CLI (`tests/cli.rs`)** — `cassette sessions --help` parses, and the subcommand is rejected
  with exit 2 when given an unexpected argument.
- **pty** — launch on a store with several sessions, move, filter, toggle, and open one;
  confirm the chosen session's cassettes are what the TUI then shows. Under an isolated
  `CASSETTE_DATA_DIR`, which is not optional.

## Out of scope

- Deleting or renaming sessions from the picker. It selects; `session alias` renames.
- Previewing a session's full text in a side pane.
- Making `find` interactive. `find` stays plain stdout and pipe-friendly — the 2026-07-13
  design rejected a browser screen for it, and that reasoning still holds; what changed for
  `sessions` is that there is no name to type.
- Phase 6's removals.

## Open items carried in

Recorded; none blocking. Two of the four are fixed by this phase (the `path` rename and the
unprinted alias, above). Remaining:

- `assert!(after >= before)` in `tests/cli.rs` passes when the mtime did not move.
- After a side-B merge, `cursor_for_a` is hardcoded to 0 while the mirror case keeps side B's
  stored cursor.
- `queue::write::write_permitted` keeps an inline id→name lookup beside the shared
  `store::writers::display_name`.
