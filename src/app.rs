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

/// Vim-style editing mode for the focused cassette. `Topic` captures a topic
/// label for the focused cassette on the status line instead of editing text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
    Topic,
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
    pub word_goal: Option<usize>,
    pub should_quit: bool,
    pub visible_lines: usize,
    pub started_at: SystemTime,
    pub mode: Mode,
    /// First key of a pending two-key normal-mode sequence (`dd`, `gg`).
    pub pending: Option<char>,
    /// Topic text being typed while in `Mode::Topic`.
    pub topic_input: String,
    /// Mode to return to when the topic prompt closes (it can be opened
    /// from insert mode via Ctrl+T or normal mode via `t`).
    pub topic_return: Mode,
    /// Record mode (`--record`): the tape only rolls forward — no deletions,
    /// no normal mode. Opt-in; plain sessions keep full editing.
    pub record: bool,
    /// Seconds since the last keypress; drives the idle nudge.
    pub idle_secs: u32,
    /// Set on any cassette mutation; cleared by the autosaver in `main.rs`.
    pub dirty: bool,
    /// One-shot request for a terminal bell, consumed by `main.rs`.
    pub bell: bool,
    /// Remaining seconds before a transient `status_msg` clears itself.
    status_ticks: Option<u32>,
    /// The word-goal celebration fires once per session.
    goal_announced: bool,
}

/// How long transient status messages (timer up, goal reached) stay visible.
const STATUS_FLASH_SECS: u32 = 8;

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
            word_goal,
            should_quit: false,
            visible_lines: visible_lines
                .unwrap_or(VISIBLE_LINES)
                .clamp(MIN_VISIBLE_LINES, MAX_VISIBLE_LINES),
            started_at: SystemTime::now(),
            mode: Mode::Insert,
            pending: None,
            topic_input: String::new(),
            topic_return: Mode::Normal,
            record: false,
            idle_secs: 0,
            dirty: false,
            bell: false,
            status_ticks: None,
            goal_announced: false,
        }
    }

    /// Show `msg` on the status line for `STATUS_FLASH_SECS`, with a bell.
    fn flash(&mut self, msg: String) {
        self.status_msg = Some(msg);
        self.status_ticks = Some(STATUS_FLASH_SECS);
        self.bell = true;
    }

    fn clear_status(&mut self) {
        self.status_msg = None;
        self.status_ticks = None;
    }

    /// True when nothing has been written on any side of any cassette.
    pub fn is_empty(&self) -> bool {
        self.cassettes
            .iter()
            .all(|c| c.side_a_text().trim().is_empty() && c.side_b_text().trim().is_empty())
    }

    /// Announce the word goal the first time total words reach it.
    pub fn check_goal(&mut self) {
        if self.goal_announced {
            return;
        }
        if let Some(goal) = self.word_goal {
            let words = self.total_word_count();
            if words >= goal {
                self.goal_announced = true;
                self.flash(format!("goal reached — {} words. keep rolling!", words));
            }
        }
    }

    /// Seconds of idleness before the "keep the tape rolling" nudge shows.
    pub const IDLE_NUDGE_SECS: u32 = 10;

    /// One second of no keypresses has passed.
    pub fn tick_idle(&mut self) {
        self.idle_secs = self.idle_secs.saturating_add(1);
    }

    /// Gentle nudge when a timed or record session has gone quiet: shown in
    /// the info line, no bell, cleared by the next keypress. Plain untimed
    /// sessions are never nudged — wandering off is allowed there.
    pub fn idle_nudge(&self) -> bool {
        (self.timer_secs.is_some() || self.record) && self.idle_secs >= Self::IDLE_NUDGE_SECS
    }

    /// Count down a transient status message; clears it when time is up.
    pub fn tick_status(&mut self) {
        if let Some(t) = self.status_ticks {
            if t <= 1 {
                self.clear_status();
            } else {
                self.status_ticks = Some(t - 1);
            }
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

    /// Seed the session from a topic template: one cassette per topic, in
    /// order, capped at `MAX_CASSETTES`. The first topic lands on the initial
    /// cassette; focus stays on it.
    pub fn apply_topics(&mut self, topics: &[String]) {
        for (i, topic) in topics.iter().take(MAX_CASSETTES).enumerate() {
            if i >= self.cassettes.len() {
                self.cassettes.push(Cassette::new());
            }
            self.cassettes[i].topic = Some(topic.clone());
        }
        self.focus_idx = 0;
        self.ensure_focus_visible();
    }

    pub fn add_cassette(&mut self) {
        if self.cassettes.len() >= MAX_CASSETTES {
            self.status_msg = Some(format!("Cassette limit reached ({}).", MAX_CASSETTES));
            return;
        }
        self.cassettes.push(Cassette::new());
        self.focus_idx = self.cassettes.len() - 1;
        self.dirty = true;
        self.clear_status();
        self.ensure_focus_visible();
    }

    pub fn focus_next(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + 1) % n;
        self.clear_status();
        self.ensure_focus_visible();
    }

    pub fn focus_prev(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + n - 1) % n;
        self.clear_status();
        self.ensure_focus_visible();
    }

    pub fn modify_focused<F: FnOnce(&mut Cassette)>(&mut self, f: F) {
        if let Some(c) = self.cassettes.get_mut(self.focus_idx) {
            f(c);
            self.dirty = true;
        }
    }

    pub fn tick_timer(&mut self) {
        if let Some(n) = self.timer_secs {
            if n > 0 {
                self.timer_secs = Some(n - 1);
                if n == 1 {
                    // Record mode has no normal mode, so no `q`.
                    let quit_key = if self.record { "^C" } else { "q" };
                    self.flash(format!("time — keep going, or {quit_key} to save"));
                }
            }
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.term_width = width;
        self.term_height = height;
        self.ensure_focus_visible();
    }

    pub fn total_word_count(&self) -> usize {
        self.cassettes.iter().map(|c| c.word_count()).sum()
    }

    /// How much tape has wound onto the take-up reel, 0.0..=1.0: progress
    /// toward the word goal, or elapsed time when only a timer is set.
    /// `None` when neither is set — the session has no bar to show.
    pub fn tape_ratio(&self) -> Option<f64> {
        if let Some(goal) = self.word_goal {
            let goal = goal.max(1);
            return Some((self.total_word_count() as f64 / goal as f64).min(1.0));
        }
        if let (Some(orig), Some(left)) = (self.timer_original_secs, self.timer_secs) {
            if orig > 0 {
                return Some(f64::from(orig - left) / f64::from(orig));
            }
        }
        None
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
        assert_eq!(app.tape_ratio(), Some(0.0));
        app.modify_focused(|c| {
            for ch in "one two three four five".chars() {
                c.insert(ch);
            }
        });
        assert_eq!(app.tape_ratio(), Some(0.5));
    }

    #[test]
    fn tape_ratio_clamps_at_full() {
        let mut app = App::new(None, Some(2), Some(VISIBLE_LINES));
        app.modify_focused(|c| {
            for ch in "a b c d".chars() {
                c.insert(ch);
            }
        });
        assert_eq!(
            app.tape_ratio(),
            Some(1.0),
            "past the goal the reel stays full"
        );
    }

    #[test]
    fn tape_ratio_follows_timer_when_no_goal() {
        let mut app = App::new(Some(4), None, Some(VISIBLE_LINES));
        assert_eq!(app.tape_ratio(), Some(0.0));
        app.tick_timer();
        app.tick_timer();
        assert_eq!(app.tape_ratio(), Some(0.5), "half the session elapsed");
    }

    #[test]
    fn tape_ratio_absent_without_goal_or_timer() {
        let app = App::new(None, None, Some(VISIBLE_LINES));
        assert_eq!(app.tape_ratio(), None, "no goal, no timer: no bar");
    }

    #[test]
    fn timer_expiry_flashes_status_and_bell_once() {
        let mut app = App::new(Some(2), None, Some(VISIBLE_LINES));
        app.tick_timer();
        assert!(app.status_msg.is_none());
        app.tick_timer(); // 1 -> 0
        assert!(app.status_msg.as_deref().unwrap().contains("time"));
        assert!(app.bell);
        app.bell = false;
        app.tick_timer(); // already 0: stays quiet
        assert!(!app.bell);
        assert_eq!(app.timer_secs, Some(0));
    }

    #[test]
    fn goal_reached_announces_once() {
        let mut app = App::new(None, Some(3), Some(VISIBLE_LINES));
        app.modify_focused(|c| {
            for ch in "one two three".chars() {
                c.insert(ch);
            }
        });
        app.check_goal();
        assert!(app.status_msg.as_deref().unwrap().contains("goal reached"));
        assert!(app.bell);
        app.bell = false;
        app.status_msg = None;
        app.check_goal(); // one-shot: no second announcement
        assert!(app.status_msg.is_none());
        assert!(!app.bell);
    }

    #[test]
    fn transient_status_clears_after_countdown() {
        let mut app = App::new(Some(1), None, Some(VISIBLE_LINES));
        app.tick_timer(); // fires the flash
        assert!(app.status_msg.is_some());
        for _ in 0..20 {
            app.tick_status();
        }
        assert!(app.status_msg.is_none(), "flash expires on its own");
    }

    #[test]
    fn is_empty_ignores_whitespace_and_sees_side_b() {
        let mut app = test_app();
        assert!(app.is_empty());
        app.modify_focused(|c| c.insert(' '));
        assert!(app.is_empty(), "whitespace-only still counts as empty");
        app.modify_focused(|c| {
            c.flip();
            c.insert('x');
        });
        assert!(!app.is_empty(), "side B text counts");
    }

    #[test]
    fn modify_focused_marks_dirty() {
        let mut app = test_app();
        assert!(!app.dirty);
        app.modify_focused(|c| c.insert('a'));
        assert!(app.dirty);
    }

    #[test]
    fn idle_nudge_needs_a_timed_or_record_session() {
        // Plain session: never nudged, wandering off is allowed.
        let mut app = test_app();
        for _ in 0..App::IDLE_NUDGE_SECS + 5 {
            app.tick_idle();
        }
        assert!(!app.idle_nudge());

        // Timed session: nudged after the threshold.
        let mut app = App::new(Some(60), None, None);
        for _ in 0..App::IDLE_NUDGE_SECS {
            app.tick_idle();
        }
        assert!(app.idle_nudge());

        // Record session (untimed) also qualifies.
        let mut app = test_app();
        app.record = true;
        for _ in 0..App::IDLE_NUDGE_SECS {
            app.tick_idle();
        }
        assert!(app.idle_nudge());
        app.idle_secs = 0;
        assert!(!app.idle_nudge());
    }

    #[test]
    fn timer_expiry_message_matches_record_mode() {
        let mut app = App::new(Some(1), None, None);
        app.record = true;
        app.tick_timer();
        assert!(app.status_msg.as_deref().unwrap().contains("^C to save"));
    }

    #[test]
    fn apply_topics_creates_labeled_cassettes() {
        let mut app = test_app();
        let topics: Vec<String> = ["one", "two", "three"].map(String::from).into();
        app.apply_topics(&topics);
        assert_eq!(app.cassettes.len(), 3);
        assert_eq!(app.cassettes[0].topic.as_deref(), Some("one"));
        assert_eq!(app.cassettes[2].topic.as_deref(), Some("three"));
        assert_eq!(app.focus_idx, 0, "session starts on the first topic");
    }

    #[test]
    fn apply_topics_caps_at_max_cassettes() {
        let mut app = test_app();
        let topics: Vec<String> = (0..MAX_CASSETTES + 5).map(|i| format!("t{i}")).collect();
        app.apply_topics(&topics);
        assert_eq!(app.cassettes.len(), MAX_CASSETTES);
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
