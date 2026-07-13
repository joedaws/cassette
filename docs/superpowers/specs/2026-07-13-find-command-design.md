# `cassette find` — browse recent notes

**Date:** 2026-07-13
**Status:** approved (autonomous session for issue #55 — flag concerns on the issue)

## Purpose

The notes dir is the database, but nothing lists it: to continue an old session you
have to remember its name or take whatever `--resume` (newest by mtime) gives you.
`cassette find` prints the recent notes — newest first, with enough context (date,
size, topics, a first-line preview) to recognize the one you want — and an optional
text filter to narrow the list. Opening one stays `cassette --resume <name>`.

## Approaches considered

1. **Plain stdout listing (chosen)** — a read-only action like `stats`: scan the
   notes dir, print, exit. Pipe-friendly, no terminal takeover, composes with
   `--resume`.
2. Interactive numbered picker on stdin that launches the TUI on selection —
   couples browsing to resuming, breaks piping, and duplicates resume wiring.
3. A ratatui browser screen — far more code than the problem warrants.

## Behavior

```
$ cassette find
2026-07-13 09:12    412 words  2026-07-13.md — gratitude, priorities
    woke up thinking about the demo and how much of it is still unbuilt…
2026-07-11 22:40    188 words  late-night.md (draft)
    can't sleep again so here we are
… 27 more — 'cassette find <text>' narrows the list

resume one: cassette --resume <name>
```

- **`cassette find [TEXT…]`** — bare action word like `today`/`stats`; any
  remaining positional args join (space-separated) into one TEXT filter, applied
  as a case-insensitive substring match over the note's file name
  and full content (topics live in `# Cassette N — topic` headings, so they match).
- **One entry per `.md` note** in the notes dir (non-recursive, like the writer and
  `stats`). Sort key is the frontmatter `date:` (with or without a time part);
  notes without one still list, falling back to file mtime — the browser shows
  everything the writer could have left there.
- **Entry line:** `YYYY-MM-DD HH:MM`, right-aligned word count (`word_count:`
  frontmatter, 0 when absent), file name, ` (draft)` when the autosave marker is
  set (`output::is_draft`), ` — topic, topic` when any cassette has a topic.
- **Preview line:** first body line that is not blank and not a heading, indented
  4 spaces, truncated to 72 chars with a trailing `…`. Omitted when the note has
  no such line.
- **Cap:** 10 newest matches; more get the single `… N more` hint line.
- Plain text, no color; footer hint points at `--resume`. Exit 0 always.
- **Empty states:** empty/missing notes dir prints the `stats` message
  (`no notes yet — the first session starts the count`); a filter with no matches
  prints `no notes match '<text>'`.

## Implementation shape

New `src/find.rs`, same pattern as `stats.rs` — pure functions over note content,
one thin I/O scanner:

- `NoteEntry { name, date: NaiveDateTime, words, topics, preview, draft }`
- `parse_entry(name, content, mtime_fallback) -> NoteEntry` — frontmatter fields
  via the same line-prefix style as `stats::parse_note_meta`, topics from
  `# Cassette … — ` headings, preview from the body.
- `scan_notes_dir(dir) -> Vec<NoteEntry>` — reads files, supplies mtime fallback.
- `render(entries, query) -> String` — filter, sort newest-first, cap, format.

`main.rs` gains the `find` action in `parse_args` (optional TEXT positional) and an
early-return branch next to `stats`. USAGE gains one line.

## Testing

Unit tests in `find.rs` alongside the module, over the pure functions:

- `parse_entry` reads date/words/draft/topics/preview; date-only and missing
  frontmatter (mtime fallback) both work; headings never become previews.
- `render` sorts newest first, filters case-insensitively (name, topic, and body
  hits), caps at 10 with the `… N more` line, and prints both empty-state messages.
- Long preview truncates with `…`; a note with no body text gets no preview line.
