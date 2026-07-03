use std::time::SystemTime;

use crate::cassette::Cassette;

/// Default number of text lines shown per cassette (excluding the separator row).
pub const VISIBLE_LINES: usize = 5;

/// Bounds for the configurable per-cassette line count (`-l` / config `visible_lines`).
pub const MIN_VISIBLE_LINES: usize = 2;
pub const MAX_VISIBLE_LINES: usize = 40;

/// Fixed row overhead below all cassettes: bottom separator + reel stats + status + help.
const UI_OVERHEAD: u16 = 4;

/// Hard cap on the number of cassettes.
pub const MAX_CASSETTES: usize = 36;

/// Columns of the line-number gutter inside each cassette (3 digits + 1 space).
pub const GUTTER_WIDTH: usize = 4;

/// Rows of a minimized (unfocused) cassette: separator + its last text line.
pub const MINIMIZED_ROWS: u16 = 2;

/// Reel "tape length" in words when no word goal is set.
pub const DEFAULT_SPOOL_WORDS: usize = 500;

/// Vim-style editing mode for the focused cassette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
}

pub struct App {
    pub cassettes: Vec<Cassette>,
    pub focus_idx: usize,
    /// Index of the first cassette shown on screen (whole-cassette scrolling).
    pub cassette_scroll: usize,
    pub term_width: u16,
    pub term_height: u16,
    pub status_msg: Option<String>,
    pub timer_secs: Option<u32>,
    pub timer_original_secs: Option<u32>,
    pub reel_rotation: usize,
    pub word_goal: Option<usize>,
    pub should_quit: bool,
    pub visible_lines: usize,
    pub started_at: SystemTime,
    pub mode: Mode,
    /// First key of a pending two-key normal-mode sequence (`dd`, `gg`).
    pub pending: Option<char>,
}

impl App {
    pub fn new(
        timer_secs: Option<u32>,
        word_goal: Option<usize>,
        visible_lines: Option<usize>,
    ) -> Self {
        Self {
            cassettes: vec![Cassette::new()],
            focus_idx: 0,
            cassette_scroll: 0,
            term_width: 80,
            term_height: 24,
            status_msg: None,
            timer_secs,
            timer_original_secs: timer_secs,
            reel_rotation: 0,
            word_goal,
            should_quit: false,
            visible_lines: visible_lines
                .unwrap_or(VISIBLE_LINES)
                .clamp(MIN_VISIBLE_LINES, MAX_VISIBLE_LINES),
            started_at: SystemTime::now(),
            mode: Mode::Insert,
            pending: None,
        }
    }

    /// Width of the text region inside a cassette
    /// (terminal width minus side padding and the line-number gutter).
    pub fn cassette_width(&self) -> usize {
        (self.term_width as usize)
            .saturating_sub(2 + GUTTER_WIDTH)
            .max(20)
    }

    /// Total rows consumed by the focused cassette widget: separator + visible_lines of text.
    pub fn rows_per_cassette(&self) -> u16 {
        self.visible_lines as u16 + 1
    }

    /// How many cassettes fit on screen at once: the focused one full-height,
    /// the rest minimized to `MINIMIZED_ROWS` each.
    pub fn visible_cassette_count(&self) -> usize {
        let available = self.term_height.saturating_sub(UI_OVERHEAD);
        let focused = self.rows_per_cassette();
        if available <= focused {
            return 1;
        }
        1 + ((available - focused) / MINIMIZED_ROWS) as usize
    }

    /// Keep `focus_idx` inside the visible window, clamped to the cassette list.
    fn ensure_focus_visible(&mut self) {
        let visible = self.visible_cassette_count();
        let max_scroll = self.cassettes.len().saturating_sub(visible);
        self.cassette_scroll = self.cassette_scroll.min(max_scroll);
        if self.focus_idx < self.cassette_scroll {
            self.cassette_scroll = self.focus_idx;
        } else if self.focus_idx >= self.cassette_scroll + visible {
            self.cassette_scroll = self.focus_idx + 1 - visible;
        }
    }

    pub fn add_cassette(&mut self) {
        if self.cassettes.len() >= MAX_CASSETTES {
            self.status_msg = Some(format!("Cassette limit reached ({}).", MAX_CASSETTES));
            return;
        }
        self.cassettes.push(Cassette::new());
        self.focus_idx = self.cassettes.len() - 1;
        self.status_msg = None;
        self.ensure_focus_visible();
    }

    pub fn focus_next(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + 1) % n;
        self.status_msg = None;
        self.ensure_focus_visible();
    }

    pub fn focus_prev(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + n - 1) % n;
        self.status_msg = None;
        self.ensure_focus_visible();
    }

    pub fn modify_focused<F: FnOnce(&mut Cassette)>(&mut self, f: F) {
        if let Some(c) = self.cassettes.get_mut(self.focus_idx) {
            f(c);
        }
    }

    pub fn tick_timer(&mut self) {
        if let Some(n) = self.timer_secs {
            if n > 0 {
                self.timer_secs = Some(n - 1);
            }
        }
    }

    pub fn advance_reel(&mut self) {
        self.reel_rotation = (self.reel_rotation + 1) % 4;
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.term_width = width;
        self.term_height = height;
        self.ensure_focus_visible();
    }

    pub fn total_word_count(&self) -> usize {
        self.cassettes.iter().map(|c| c.word_count()).sum()
    }

    /// How much tape has wound onto the take-up reel: total words written over
    /// the word goal, or over a default spool of `DEFAULT_SPOOL_WORDS` when no
    /// goal is set. 0.0..=1.0.
    pub fn tape_ratio(&self) -> f64 {
        let spool = self.word_goal.unwrap_or(DEFAULT_SPOOL_WORDS).max(1);
        (self.total_word_count() as f64 / spool as f64).min(1.0)
    }

    /// Number of cassettes hidden above and below the visible window.
    pub fn hidden_cassettes(&self) -> (usize, usize) {
        let visible = self.visible_cassette_count();
        let below = self
            .cassettes
            .len()
            .saturating_sub(self.cassette_scroll + visible);
        (self.cassette_scroll, below)
    }

    pub fn format_stats(&self) -> String {
        let total_wc = self.total_word_count();
        match (self.timer_secs, self.word_goal) {
            (None, None) => "◆".into(),
            _ => {
                let timer_part = self
                    .timer_secs
                    .map(|n| format!("{:02}:{:02}", n / 60, n % 60))
                    .unwrap_or_default();
                let goal_part = self
                    .word_goal
                    .map(|g| format!("{} / {}", total_wc, g))
                    .unwrap_or_default();
                let sep = if self.timer_secs.is_some() && self.word_goal.is_some() {
                    "  ·  "
                } else {
                    ""
                };
                format!("{}{}{}", timer_part, sep, goal_part)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 14-row terminal, 5-line focused cassette: 14 - 4 overhead = 10 rows;
    /// focused takes 6, leaving 4 for two minimized cassettes → 3 visible.
    fn test_app() -> App {
        let mut app = App::new(None, None, Some(VISIBLE_LINES));
        app.resize(80, 14);
        app
    }

    #[test]
    fn add_cassette_beyond_screen_scrolls() {
        let mut app = test_app();
        assert_eq!(app.visible_cassette_count(), 3);
        for _ in 0..5 {
            app.add_cassette();
        }
        assert_eq!(app.cassettes.len(), 6);
        assert_eq!(app.focus_idx, 5);
        // Focus is on the last cassette; window shows the last 3.
        assert_eq!(app.cassette_scroll, 3);
        assert_eq!(app.hidden_cassettes(), (3, 0));
    }

    #[test]
    fn focus_wraps_and_scrolls_back_to_top() {
        let mut app = test_app();
        for _ in 0..5 {
            app.add_cassette();
        }
        app.focus_next(); // wraps 5 -> 0
        assert_eq!(app.focus_idx, 0);
        assert_eq!(app.cassette_scroll, 0);
        app.focus_prev(); // wraps 0 -> 5
        assert_eq!(app.focus_idx, 5);
        assert_eq!(app.cassette_scroll, 3);
    }

    #[test]
    fn focus_prev_scrolls_up_one_at_a_time() {
        let mut app = test_app();
        for _ in 0..5 {
            app.add_cassette();
        }
        // focus 5, scroll 3; stepping back to 2 pulls the window up.
        app.focus_prev();
        app.focus_prev();
        app.focus_prev();
        assert_eq!(app.focus_idx, 2);
        assert_eq!(app.cassette_scroll, 2);
    }

    #[test]
    fn tape_ratio_tracks_words_against_goal() {
        let mut app = App::new(None, Some(10), Some(VISIBLE_LINES));
        assert_eq!(app.tape_ratio(), 0.0);
        app.modify_focused(|c| {
            for ch in "one two three four five".chars() {
                c.insert(ch);
            }
        });
        assert_eq!(app.tape_ratio(), 0.5);
    }

    #[test]
    fn tape_ratio_clamps_and_defaults_to_spool() {
        let mut app = App::new(None, Some(2), Some(VISIBLE_LINES));
        app.modify_focused(|c| {
            for ch in "a b c d".chars() {
                c.insert(ch);
            }
        });
        assert_eq!(app.tape_ratio(), 1.0, "past the goal the reel stays full");

        let app = App::new(None, None, Some(VISIBLE_LINES));
        assert_eq!(app.word_goal, None);
        assert_eq!(app.tape_ratio(), 0.0, "no goal: measured against default spool");
    }

    #[test]
    fn add_cassette_stops_at_max() {
        let mut app = test_app();
        for _ in 0..MAX_CASSETTES + 5 {
            app.add_cassette();
        }
        assert_eq!(app.cassettes.len(), MAX_CASSETTES);
        assert_eq!(app.focus_idx, MAX_CASSETTES - 1);
        assert!(app.status_msg.as_deref().unwrap_or("").contains("limit"));
    }

    #[test]
    fn resize_clamps_scroll_and_keeps_focus_visible() {
        let mut app = test_app();
        for _ in 0..5 {
            app.add_cassette();
        }
        // Taller terminal fits all 6: scroll must clamp back to 0.
        app.resize(80, 60);
        assert_eq!(app.cassette_scroll, 0);
        // Tiny terminal shows 1: focused (last) cassette must stay visible.
        app.resize(80, 10);
        assert_eq!(app.visible_cassette_count(), 1);
        assert_eq!(app.cassette_scroll, 5);
    }
}
