use crate::find::NoteEntry;
use crate::session::DEFAULT_LIST_LIMIT;

/// The `cassette sessions` list: which rows are showing, which one is
/// highlighted, and what a filter has narrowed it to.
///
/// Pure, the way `App` is — no `Store`, no `std::fs`, no ratatui. Selection
/// is a decision, not an effect: `selected()` returns the entry and
/// `main.rs` decides what to do with it, so the whole screen's behaviour is
/// testable without a terminal.
///
/// The visible rows are **derived** on each call rather than kept in a
/// second `Vec` alongside `entries`. Two collections would be two records of
/// one fact, which is the shape Phases 5b and 5c each shipped a bug from.
pub(crate) struct Picker {
    entries: Vec<NoteEntry>,
    /// Cassette files that could not be read, shown as a footer the way
    /// `find` does. An `eprintln!` here landed on the normal screen an
    /// instant before the alternate screen hid it, which is no report at all.
    pub unreadable: usize,
    /// Session directories `Store::list_sessions` skipped because their
    /// names are not `ses_` ids, shown in the same footer.
    pub skipped: usize,
    /// Index into `visible()`, not into `entries`.
    pub cursor: usize,
    /// First visible row drawn, so a cursor past the bottom of the screen
    /// scrolls the list instead of walking off it.
    pub scroll: usize,
    /// Show every session rather than the `DEFAULT_LIST_LIMIT` most recent.
    pub show_all: bool,
    /// The live filter text. Empty means no filter, whether or not the
    /// prompt is open.
    pub query: String,
    /// Whether the `/` prompt owns the keyboard. Modal on purpose: while it
    /// is open a `q` is a character, not a quit — the same rule `Mode::Topic`
    /// follows in the main TUI, and for the same reason.
    pub filtering: bool,
}

impl Picker {
    pub fn new(entries: Vec<NoteEntry>, unreadable: usize) -> Self {
        Self {
            entries,
            unreadable,
            skipped: 0,
            cursor: 0,
            scroll: 0,
            show_all: false,
            query: String::new(),
            filtering: false,
        }
    }

    /// The rows on screen: filtered first, then capped unless `show_all`.
    ///
    /// Filtering before capping is what makes the cap mean "the 15 most
    /// recent **matches**". Capping first would hide a match that exists,
    /// which is worse than showing fewer rows.
    pub fn visible(&self) -> Vec<&NoteEntry> {
        let q = self.query.trim().to_lowercase();
        let matched = self
            .entries
            .iter()
            .filter(|e| q.is_empty() || e.matches(&q));
        if self.show_all {
            matched.collect()
        } else {
            matched.take(DEFAULT_LIST_LIMIT).collect()
        }
    }

    /// The highlighted entry, or `None` when nothing is showing — an empty
    /// store and a filter that matches nothing are ordinary states here, not
    /// errors, so callers get an `Option` rather than a panic.
    pub fn selected(&self) -> Option<&NoteEntry> {
        self.visible().get(self.cursor).copied()
    }

    pub fn move_down(&mut self) {
        let n = self.visible().len();
        if n > 0 && self.cursor + 1 < n {
            self.cursor += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Toggle between the recent window and every session.
    ///
    /// The row count changes under the cursor, so the highlight is restored
    /// **by id**: the human is looking at a session, not at a position. Same
    /// rule `App::sort_queue` follows when the stack reorders beneath focus.
    ///
    /// Today this is forward-insurance rather than an observable guarantee:
    /// the recent view is a strict prefix of the all view, so an entry's
    /// index is identical in both and pure clamping would agree. It stops
    /// being equivalent the moment `visible()` ever sorts or groups, which
    /// is exactly when the bug would be hardest to see.
    pub fn toggle_all(&mut self) {
        let was = self.selected().map(|e| e.id.clone());
        self.show_all = !self.show_all;
        if let Some(id) = was {
            if let Some(i) = self.visible().iter().position(|e| e.id == id) {
                self.cursor = i;
                return;
            }
        }
        self.clamp_cursor();
    }

    pub fn start_filter(&mut self) {
        self.filtering = true;
    }

    /// Close the prompt, keeping whatever it narrowed to. Clearing the
    /// filter on Esc would throw away work the human just did; `q` from the
    /// list is how they leave entirely.
    pub fn end_filter(&mut self) {
        self.filtering = false;
    }

    pub fn push_filter(&mut self, c: char) {
        self.query.push(c);
        self.clamp_cursor();
    }

    pub fn pop_filter(&mut self) {
        self.query.pop();
        self.clamp_cursor();
    }

    /// Keep `cursor` inside the visible rows after anything that can shorten
    /// them. Left unclamped, `selected()` silently returns `None` on a list
    /// that plainly has rows on it.
    fn clamp_cursor(&mut self) {
        let n = self.visible().len();
        if n == 0 {
            self.cursor = 0;
        } else if self.cursor >= n {
            self.cursor = n - 1;
        }
        // The window has to follow: a scroll left past the end of a
        // narrowed list would draw blank rows below the last match.
        if self.scroll > self.cursor {
            self.scroll = self.cursor;
        }
    }

    /// A picker with no damaged files, for tests that are not about them.
    #[cfg(test)]
    pub fn new_for_test(entries: Vec<NoteEntry>) -> Self {
        Self::new(entries, 0)
    }

    /// How many sessions exist in total, for the "showing N of M" footer.
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// How many session rows fit in a terminal `height` rows tall.
    ///
    /// One place, used by both `run_picker` (to clamp the scroll before
    /// drawing) and `render_picker` (to slice). Two answers here would put
    /// the cursor outside the rows actually drawn, which is the whole bug
    /// this exists to prevent. At least one row always, so a very short
    /// terminal still shows the highlighted session rather than nothing.
    pub fn rows_capacity(height: u16) -> usize {
        // title, blank, blank-before-footer, filter/standing line, help.
        const CHROME_ROWS: u16 = 5;
        height.saturating_sub(CHROME_ROWS).max(1) as usize
    }

    /// Keep the cursor inside the `capacity` rows that will actually be
    /// drawn, scrolling the window rather than letting it walk off screen.
    ///
    /// Without this the list looks frozen while the cursor keeps advancing
    /// into undrawn rows, and Enter opens a session the human never saw —
    /// which at a session a day is the steady state within a month.
    pub fn ensure_cursor_visible(&mut self, capacity: usize) {
        let n = self.visible().len();
        let max_scroll = n.saturating_sub(capacity);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + capacity {
            self.scroll = self.cursor + 1 - capacity;
        }
    }

    /// Rows above and below the window, for the `↑ N more` / `↓ N more`
    /// hints the cassette stack already uses for the same situation.
    pub fn hidden_rows(&self, capacity: usize) -> (usize, usize) {
        let n = self.visible().len();
        let below = n.saturating_sub(self.scroll + capacity);
        (self.scroll, below)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn entry(id: &str, alias: Option<&str>, topic: &str) -> NoteEntry {
        NoteEntry::for_test(
            id,
            alias,
            NaiveDate::from_ymd_opt(2026, 9, 23)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap(),
            10,
            &[topic],
        )
    }

    /// A list is not a carousel: wrapping past the end of 200 sessions is
    /// disorienting, so movement clamps at both ends.
    #[test]
    fn movement_clamps_rather_than_wrapping() {
        let mut p = Picker::new_for_test(vec![entry("a", None, "x"), entry("b", None, "y")]);
        p.move_up();
        assert_eq!(p.cursor, 0, "already at the top");
        p.move_down();
        p.move_down();
        p.move_down();
        assert_eq!(p.cursor, 1, "stops at the last row");
    }

    /// The `a` toggle changes how many rows show; the session the human was
    /// looking at must still be highlighted, by id rather than by position.
    #[test]
    fn toggling_all_keeps_the_same_session_highlighted() {
        let entries: Vec<NoteEntry> = (0..20)
            .map(|i| entry(&format!("id{i:02}"), None, "t"))
            .collect();
        let mut p = Picker::new_for_test(entries);
        assert_eq!(p.visible().len(), DEFAULT_LIST_LIMIT, "recent by default");
        p.move_down();
        p.move_down();
        let before = p.selected().expect("a row").id.clone();

        p.toggle_all();

        assert_eq!(p.visible().len(), 20, "all of them now");
        assert_eq!(p.selected().expect("a row").id, before, "same session");
    }

    /// Filtering narrows live, and a filter matching nothing must leave the
    /// cursor valid rather than pointing past the end.
    #[test]
    fn filtering_narrows_and_leaves_the_cursor_valid() {
        let mut p = Picker::new_for_test(vec![
            entry("a", Some("morning"), "gratitude"),
            entry("b", Some("evening"), "review"),
        ]);
        p.start_filter();
        for c in "morn".chars() {
            p.push_filter(c);
        }
        assert_eq!(p.visible().len(), 1);
        assert_eq!(
            p.selected().expect("a row").alias.as_deref(),
            Some("morning")
        );

        for c in "zzz".chars() {
            p.push_filter(c);
        }
        assert!(p.visible().is_empty(), "nothing matches");
        assert!(
            p.selected().is_none(),
            "and nothing is selected, rather than panicking"
        );
    }

    /// Backspacing out of a dead filter must bring the rows back and leave
    /// the cursor usable — the recovery path from the case above.
    #[test]
    fn popping_a_filter_restores_the_rows() {
        let mut p = Picker::new_for_test(vec![
            entry("a", Some("morning"), "gratitude"),
            entry("b", Some("evening"), "review"),
        ]);
        p.start_filter();
        for c in "zzz".chars() {
            p.push_filter(c);
        }
        assert!(p.selected().is_none());

        for _ in 0..3 {
            p.pop_filter();
        }

        assert_eq!(p.visible().len(), 2);
        assert!(p.selected().is_some(), "the cursor is usable again");
    }

    /// Esc closes the prompt but keeps what it narrowed to: clearing the
    /// filter would throw away work the human just did.
    #[test]
    fn ending_the_filter_keeps_the_query() {
        let mut p = Picker::new_for_test(vec![entry("a", Some("morning"), "x")]);
        p.start_filter();
        p.push_filter('m');
        p.end_filter();
        assert!(!p.filtering);
        assert_eq!(p.query, "m", "the narrowing survives closing the prompt");
    }

    /// The discriminating case for `clamp_cursor`: a cursor PAST the end of
    /// a narrowed list. The empty-list case cannot catch it — `selected()`
    /// is `None` either way — so without this the clamp could be deleted
    /// with the suite still green.
    #[test]
    fn narrowing_under_a_moved_cursor_keeps_a_row_selected() {
        let mut p = Picker::new_for_test(vec![
            entry("a", Some("morning"), "gratitude"),
            entry("b", Some("evening"), "review"),
        ]);
        p.move_down();
        assert_eq!(p.cursor, 1);

        p.start_filter();
        for c in "morn".chars() {
            p.push_filter(c);
        }

        assert_eq!(p.visible().len(), 1, "one row left");
        assert!(
            p.selected().is_some(),
            "a list with a row on it must have that row selected — unclamped, \
             the cursor still points at index 1 and Enter does nothing"
        );
        assert_eq!(
            p.selected().expect("a row").alias.as_deref(),
            Some("morning")
        );
    }

    /// The cursor must never point at a row the draw will omit: the list
    /// would look frozen while the highlight walked off the bottom, and
    /// Enter would open a session the human never saw.
    #[test]
    fn the_window_follows_the_cursor_past_the_bottom() {
        let entries: Vec<NoteEntry> = (0..40)
            .map(|i| entry(&format!("id{i:02}"), None, "t"))
            .collect();
        let mut p = Picker::new_for_test(entries);
        p.toggle_all();
        let capacity = 10;

        for _ in 0..25 {
            p.move_down();
            p.ensure_cursor_visible(capacity);
        }

        assert_eq!(p.cursor, 25);
        assert!(
            p.cursor >= p.scroll && p.cursor < p.scroll + capacity,
            "cursor {} outside the drawn window {}..{}",
            p.cursor,
            p.scroll,
            p.scroll + capacity
        );
        let (above, below) = p.hidden_rows(capacity);
        assert!(above > 0 && below > 0, "hints both ways: {above} / {below}");
    }

    /// Scrolling back up brings the window with it.
    #[test]
    fn the_window_follows_the_cursor_back_up() {
        let entries: Vec<NoteEntry> = (0..40)
            .map(|i| entry(&format!("id{i:02}"), None, "t"))
            .collect();
        let mut p = Picker::new_for_test(entries);
        p.toggle_all();
        for _ in 0..30 {
            p.move_down();
            p.ensure_cursor_visible(8);
        }
        assert!(p.scroll > 0);

        for _ in 0..30 {
            p.move_up();
            p.ensure_cursor_visible(8);
        }

        assert_eq!(p.cursor, 0);
        assert_eq!(p.scroll, 0, "the window came back with it");
    }

    /// A very short terminal still draws the highlighted row rather than
    /// nothing at all.
    #[test]
    fn capacity_never_falls_to_zero() {
        assert_eq!(Picker::rows_capacity(0), 1);
        assert_eq!(Picker::rows_capacity(5), 1);
        assert!(Picker::rows_capacity(24) > 10);
    }

    /// An empty store is a normal state, not an error and not a panic.
    #[test]
    fn an_empty_picker_selects_nothing() {
        let p = Picker::new_for_test(Vec::new());
        assert!(p.visible().is_empty());
        assert!(p.selected().is_none());
        assert_eq!(p.total(), 0);
    }

    /// The cap means "the 15 most recent MATCHES", so filtering happens
    /// before it. Capping first would hide a match that exists.
    #[test]
    fn the_filter_reaches_past_the_recent_window() {
        let mut entries: Vec<NoteEntry> = (0..20)
            .map(|i| entry(&format!("id{i:02}"), None, "common"))
            .collect();
        entries.push(entry("needle", Some("needle"), "rare"));
        let mut p = Picker::new_for_test(entries);

        p.start_filter();
        for c in "rare".chars() {
            p.push_filter(c);
        }

        assert_eq!(
            p.visible().len(),
            1,
            "a match past the 15th row is still found"
        );
        assert_eq!(p.selected().expect("a row").id, "needle");
    }
}
