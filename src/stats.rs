use std::collections::HashSet;
use std::path::Path;

use chrono::{Datelike, NaiveDate};

/// What `cassette stats` needs from one saved note: its day and word count,
/// straight from the YAML frontmatter — the notes dir is the database.
pub struct NoteMeta {
    pub date: NaiveDate,
    pub words: usize,
}

/// Parse `date:` and `word_count:` out of a note's frontmatter.
/// Notes without a parseable frontmatter date are not stats material.
pub fn parse_note_meta(content: &str) -> Option<NoteMeta> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut date = None;
    let mut words = 0;
    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(v) = line.strip_prefix("date:") {
            date = NaiveDate::parse_from_str(v.trim().get(..10)?, "%Y-%m-%d").ok();
        } else if let Some(v) = line.strip_prefix("word_count:") {
            words = v.trim().parse().unwrap_or(0);
        }
    }
    Some(NoteMeta { date: date?, words })
}

/// Read every `.md` note in the notes dir (non-recursive, like the writer).
pub fn scan_notes_dir(dir: &Path) -> Vec<NoteMeta> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|c| parse_note_meta(&c))
        .collect()
}

/// Consecutive days with at least one note, counting back from today —
/// or from yesterday, so the streak isn't broken before today's session.
fn streak(dates: &HashSet<NaiveDate>, today: NaiveDate) -> u32 {
    let mut day = today;
    if !dates.contains(&day) {
        day = day.pred_opt().expect("date within calendar range");
    }
    let mut n = 0;
    while dates.contains(&day) {
        n += 1;
        day = day.pred_opt().expect("date within calendar range");
    }
    n
}

/// The two-line `last 7:` block: weekday initials over hit/miss markers for
/// the 7 calendar days ending today, oldest first, plus a hit count.
/// An unwritten today is pending (`·`), not a miss, and leaves the denominator.
fn last_seven(dates: &HashSet<NaiveDate>, today: NaiveDate) -> String {
    let (mut initials, mut marks) = (Vec::new(), Vec::new());
    let (mut hits, mut denom) = (0, 0);
    for back in (0..7).rev() {
        let day = today - chrono::Days::new(back);
        initials.push(day.weekday().to_string()[..1].to_owned());
        let written = dates.contains(&day);
        if day == today && !written {
            marks.push("·");
            continue;
        }
        marks.push(if written { "●" } else { "○" });
        denom += 1;
        hits += usize::from(written);
    }
    format!(
        "last 7:      {}\n             {}   {hits}/{denom}",
        initials.join(" "),
        marks.join(" ")
    )
}

fn notes_and_words<'a>(metas: impl Iterator<Item = &'a NoteMeta>) -> String {
    let (mut n, mut words) = (0usize, 0usize);
    for m in metas {
        n += 1;
        words += m.words;
    }
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} note{plural} · {words} words")
}

/// The plain-text `cassette stats` screen.
pub fn render(metas: &[NoteMeta], today: NaiveDate) -> String {
    if metas.is_empty() {
        return "no notes yet — the first session starts the count".into();
    }
    let dates: HashSet<NaiveDate> = metas.iter().map(|m| m.date).collect();
    let days = streak(&dates, today);
    let day_plural = if days == 1 { "" } else { "s" };
    let week_start = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    let first = metas.iter().map(|m| m.date).min().expect("non-empty");

    format!(
        "streak:      {days} day{day_plural}\n\
         {}\n\
         this week:   {}\n\
         this month:  {}\n\
         total:       {} · since {first}",
        last_seven(&dates, today),
        notes_and_words(
            metas
                .iter()
                .filter(|m| m.date >= week_start && m.date <= today)
        ),
        notes_and_words(
            metas
                .iter()
                .filter(|m| m.date.year() == today.year() && m.date.month() == today.month())
        ),
        notes_and_words(metas.iter()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn meta(date: &str, words: usize) -> NoteMeta {
        NoteMeta {
            date: d(date),
            words,
        }
    }

    #[test]
    fn parse_note_meta_reads_frontmatter() {
        let m = parse_note_meta(
            "---\ndate: 2026-07-04T09:30:00\nword_count: 250\ncassettes: 2\n---\nbody\n",
        )
        .unwrap();
        assert_eq!(m.date, d("2026-07-04"));
        assert_eq!(m.words, 250);
        assert!(parse_note_meta("no frontmatter here").is_none());
        assert!(
            parse_note_meta("---\nword_count: 9\n---\n").is_none(),
            "a note without a date can't join the timeline"
        );
    }

    #[test]
    fn streak_counts_back_and_tolerates_missing_today() {
        let dates: HashSet<NaiveDate> = ["2026-07-01", "2026-07-02", "2026-07-03"].map(d).into();
        assert_eq!(streak(&dates, d("2026-07-03")), 3);
        assert_eq!(streak(&dates, d("2026-07-04")), 3, "today not written yet");
        assert_eq!(
            streak(&dates, d("2026-07-05")),
            0,
            "a full missed day breaks it"
        );
    }

    #[test]
    fn render_buckets_week_month_and_total() {
        // 2026-07-03 is a Friday; the week starts Monday 2026-06-29.
        let metas = [
            meta("2026-06-10", 100), // June: month excludes, total includes
            meta("2026-06-28", 50),  // Sunday before the week starts
            meta("2026-06-30", 200), // in week, out of month
            meta("2026-07-02", 300),
            meta("2026-07-03", 400),
        ];
        let out = render(&metas, d("2026-07-03"));
        assert!(out.contains("streak:      2 days"), "{out}");
        assert!(out.contains("this week:   3 notes · 900 words"), "{out}");
        assert!(out.contains("this month:  2 notes · 700 words"), "{out}");
        assert!(
            out.contains("total:       5 notes · 1050 words · since 2026-06-10"),
            "{out}"
        );
    }

    #[test]
    fn last_seven_marks_hits_and_misses() {
        // Today 2026-07-03 is a Friday and has a note: window Sat Jun 27 → Fri Jul 3.
        let dates: HashSet<NaiveDate> =
            ["2026-06-28", "2026-06-30", "2026-07-02", "2026-07-03"].map(d).into();
        assert_eq!(
            last_seven(&dates, d("2026-07-03")),
            "last 7:      S S M T W T F\n             ○ ● ○ ● ○ ● ●   4/7"
        );
    }

    #[test]
    fn last_seven_pending_today_is_not_a_miss() {
        let dates: HashSet<NaiveDate> = ["2026-06-28", "2026-06-30", "2026-07-02"].map(d).into();
        assert_eq!(
            last_seven(&dates, d("2026-07-03")),
            "last 7:      S S M T W T F\n             ○ ● ○ ● ○ ● ·   3/6",
            "unwritten today shows · and leaves the denominator"
        );
    }

    #[test]
    fn render_includes_last_seven_row() {
        let metas = [meta("2026-07-02", 300), meta("2026-07-03", 400)];
        let out = render(&metas, d("2026-07-03"));
        assert!(
            out.contains("last 7:      S S M T W T F\n             ○ ○ ○ ○ ○ ● ●   2/7"),
            "{out}"
        );
    }

    #[test]
    fn render_empty_dir_message() {
        assert!(render(&[], d("2026-07-03")).contains("no notes yet"));
    }
}
