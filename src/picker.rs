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
    /// Index into `visible()`, not into `entries`.
    pub cursor: usize,
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
    pub fn new(entries: Vec<NoteEntry>) -> Self {
        Self {
            entries,
            cursor: 0,
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
    }

    /// How many sessions exist in total, for the "showing N of M" footer.
    pub fn total(&self) -> usize {
        self.entries.len()
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
        let mut p = Picker::new(vec![entry("a", None, "x"), entry("b", None, "y")]);
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
        let mut p = Picker::new(entries);
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
        let mut p = Picker::new(vec![
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
        let mut p = Picker::new(vec![
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
        let mut p = Picker::new(vec![entry("a", Some("morning"), "x")]);
        p.start_filter();
        p.push_filter('m');
        p.end_filter();
        assert!(!p.filtering);
        assert_eq!(p.query, "m", "the narrowing survives closing the prompt");
    }

    /// An empty store is a normal state, not an error and not a panic.
    #[test]
    fn an_empty_picker_selects_nothing() {
        let p = Picker::new(Vec::new());
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
        let mut p = Picker::new(entries);

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
