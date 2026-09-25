use std::collections::HashSet;

use chrono::{Datelike, NaiveDate};

use crate::queue::json::split_sides;
use crate::store::{Store, StoredCassette};

/// What `cassette stats` needs from one session: its day and word count. One
/// session is one entry, not one per cassette.
pub struct NoteMeta {
    pub date: NaiveDate,
    pub words: usize,
}

/// A session's total words, summed across its cassettes and counted the same
/// way `Cassette::word_count` does (`split_whitespace` over both sides) —
/// the TUI and the reporting commands must not disagree about how long a
/// session is.
fn session_word_count(cassettes: &[StoredCassette]) -> usize {
    cassettes
        .iter()
        .map(|c| {
            let (side_a, side_b) = split_sides(&c.body);
            side_a.split_whitespace().count() + side_b.split_whitespace().count()
        })
        .sum()
}

/// One `NoteMeta` per session in the store — its day from `session.toml`'s
/// `created` (RFC3339 UTC, read back in local time so a streak lines up with
/// the user's calendar day) and its words summed across its cassettes — plus
/// the total count of cassette files across all sessions that could not be
/// read or parsed (`SessionScan::unreadable`, see `Store::scan_session`).
///
/// That count must survive to `render`: a damaged cassette silently drops
/// its words from every bucket it would have counted toward, and a streak or
/// weekly total that is quietly too low is worse than one that visibly says
/// so — the same reasoning `queue list`'s `N unreadable` line already acts
/// on (`queue::view::render_list`).
///
/// The legacy notes dir is deliberately not consulted — see the design's
/// decision 7. A session whose `created` timestamp fails to parse is
/// skipped, the same treatment `Store::list_sessions` gives a `session.toml`
/// that fails to parse at all.
pub fn scan_store(store: &Store) -> (Vec<NoteMeta>, usize) {
    let Ok(sessions) = store.list_sessions() else {
        return (Vec::new(), 0);
    };
    let mut metas = Vec::new();
    let mut unreadable = 0usize;
    for (id, meta) in sessions {
        let Some(date) = chrono::DateTime::parse_from_rfc3339(&meta.created)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
        else {
            continue;
        };
        let Ok(scan) = store.scan_session(&id) else {
            continue;
        };
        unreadable += scan.unreadable();
        metas.push(NoteMeta {
            date,
            words: session_word_count(&scan.cassettes),
        });
    }
    (metas, unreadable)
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

/// The plain-text `cassette stats` screen. `unreadable` — the count of
/// cassette files `scan_store` could not read or parse — is appended as a
/// trailing `N unreadable` line when nonzero, the same presentation
/// `queue::view::render_list` uses: never invent a new one for the same
/// failure mode.
pub fn render(metas: &[NoteMeta], today: NaiveDate, unreadable: usize) -> String {
    if metas.is_empty() {
        return "no notes yet — the first session starts the count".into();
    }
    let dates: HashSet<NaiveDate> = metas.iter().map(|m| m.date).collect();
    let days = streak(&dates, today);
    let day_plural = if days == 1 { "" } else { "s" };
    let week_start = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    let first = metas.iter().map(|m| m.date).min().expect("non-empty");

    let mut out = format!(
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
    );
    if unreadable > 0 {
        out.push_str(&format!("\n{unreadable} unreadable"));
    }
    out
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
    fn a_session_becomes_one_stats_entry_summing_its_cassettes() {
        use crate::store::meta::{CassetteMeta, Status};

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        for (i, body) in ["## Side A\n\none two\n", "## Side A\n\nthree\n"]
            .iter()
            .enumerate()
        {
            let m = CassetteMeta {
                id: crate::store::ids::new(crate::store::ids::IdKind::Cassette),
                topic: Some(format!("topic {i}")),
                priority: (i as i64 + 1) * 10,
                status: Status::Open,
                locked_by: None,
                created_by: crate::store::ids::TEST_WRITER.to_string(),
                last_writer: crate::store::ids::TEST_WRITER.to_string(),
                updated_at: crate::store::meta::now_utc(),
            };
            store.add_cassette(&sid, &m, body).expect("add");
        }

        let (metas, unreadable) = scan_store(&store);
        assert_eq!(
            metas.len(),
            1,
            "one session is one entry, not one per cassette"
        );
        assert_eq!(
            metas[0].words, 3,
            "words sum across the session's cassettes"
        );
        assert_eq!(unreadable, 0, "no damaged cassettes in this fixture");
    }

    #[test]
    fn scan_store_surfaces_unreadable_cassettes_rather_than_dropping_them() {
        use crate::store::meta::{CassetteMeta, Status};

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let sid = store
            .create_session(&crate::store::session::SessionMeta {
                alias: None,
                created: crate::store::meta::now_utc(),
                timer_secs: None,
                word_goal: None,
            })
            .expect("create session");
        let m = CassetteMeta {
            id: crate::store::ids::new(crate::store::ids::IdKind::Cassette),
            topic: Some("gratitude".to_string()),
            priority: 10,
            status: Status::Open,
            locked_by: None,
            created_by: crate::store::ids::TEST_WRITER.to_string(),
            last_writer: crate::store::ids::TEST_WRITER.to_string(),
            updated_at: crate::store::meta::now_utc(),
        };
        store
            .add_cassette(&sid, &m, "## Side A\n\none two three\n")
            .expect("add");
        // A cassette file with no parseable frontmatter: `Store::scan_session`
        // counts it as unreadable rather than erroring the whole scan out.
        std::fs::write(
            store.cassettes_dir(&sid).join("damaged.md"),
            "not a cassette file\n",
        )
        .expect("write damaged file");

        let (metas, unreadable) = scan_store(&store);
        assert_eq!(metas.len(), 1);
        assert_eq!(
            metas[0].words, 3,
            "the readable cassette's words still count"
        );
        assert_eq!(
            unreadable, 1,
            "the damaged cassette must be counted, not silently dropped"
        );

        let out = render(&metas, metas[0].date, unreadable);
        assert!(
            out.contains("1 unreadable"),
            "the reader must see the count: {out}"
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
        let out = render(&metas, d("2026-07-03"), 0);
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
        let dates: HashSet<NaiveDate> = ["2026-06-28", "2026-06-30", "2026-07-02", "2026-07-03"]
            .map(d)
            .into();
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
        let out = render(&metas, d("2026-07-03"), 0);
        assert!(
            out.contains("last 7:      S S M T W T F\n             ○ ○ ○ ○ ○ ● ●   2/7"),
            "{out}"
        );
    }

    #[test]
    fn render_empty_dir_message() {
        assert!(render(&[], d("2026-07-03"), 0).contains("no notes yet"));
    }
}
