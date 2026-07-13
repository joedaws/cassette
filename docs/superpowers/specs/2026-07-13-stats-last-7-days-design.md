# `cassette stats` — last-7-days hit/miss row

**Date:** 2026-07-13
**Status:** approved

## Purpose

A broken streak currently erases any sense of a good week: `streak: 0 days` is all
you see even if you wrote 6 of the last 7 days. Add a `last 7` row to
`cassette stats` showing per-day hits and misses over the trailing week, so a
mostly-good week still reads as one.

## Behavior

The row sits between `streak:` and `this week:` and spans two lines:

```
streak:      2 days
last 7:      F S S M T W T
             ● ○ ● ● ● ○ ·   4/6
this week:   3 notes · 900 words
```

- **Window:** the 7 calendar days ending today, oldest → newest (left → right).
- **Line 1:** single-letter weekday initials (chrono weekday, `M T W T F S S`
  style), one per day, aligned above the markers.
- **Line 2:** `●` for a day with at least one note, `○` for a miss, `·` for
  today when no note exists yet (pending, not a miss), then a hit count.
- **Count denominator:** excludes a pending today — `4/6` before today's
  session, `5/7` once today has a note. This matches the streak's forgiving
  treatment of an unwritten today.
- Plain Unicode, no color; output stays pipe-friendly.
- The empty-notes-dir message is unchanged (no `last 7` row without notes).
- The marker line carries the same 13-column indent as the existing labels so
  markers sit under their initials.

## Implementation shape

`src/stats.rs` only. A new pure helper (e.g.
`last_seven(dates: &HashSet<NaiveDate>, today: NaiveDate) -> String`) builds
the two-line block from the same date set `streak` uses; `render` splices it
into the output. Hits/misses depend only on day presence, so the date set —
already built in `render` — is the right input (not `metas` word counts).

## Testing

Unit tests in `stats.rs` alongside the existing ones:

- mixed hit/miss week renders the right markers in the right order;
- pending today: `·` marker and reduced denominator (`n/6`);
- today written: `●` marker and `/7` denominator;
- weekday initials correct and aligned for a known date.
