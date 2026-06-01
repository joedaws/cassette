use std::time::SystemTime;

use crate::cassette::Cassette;

/// Number of text lines shown per cassette (excluding the separator row).
pub const VISIBLE_LINES: usize = 5;

/// Fixed row overhead below all cassettes: bottom separator + reel stats + status + help.
const UI_OVERHEAD: u16 = 4;

pub struct App {
    pub cassettes: Vec<Cassette>,
    pub focus_idx: usize,
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
}

impl App {
    pub fn new(timer_secs: Option<u32>, word_goal: Option<usize>) -> Self {
        Self {
            cassettes: vec![Cassette::new()],
            focus_idx: 0,
            term_width: 80,
            term_height: 24,
            status_msg: None,
            timer_secs,
            timer_original_secs: timer_secs,
            reel_rotation: 0,
            word_goal,
            should_quit: false,
            visible_lines: VISIBLE_LINES,
            started_at: SystemTime::now(),
        }
    }

    /// Width of the text region inside a cassette (terminal width minus side padding).
    pub fn cassette_width(&self) -> usize {
        (self.term_width as usize).saturating_sub(2).max(20)
    }

    /// Total rows consumed by one cassette widget: separator + visible_lines of text.
    pub fn rows_per_cassette(&self) -> u16 {
        self.visible_lines as u16 + 1
    }

    pub fn max_cassettes(&self) -> usize {
        let available = self.term_height.saturating_sub(UI_OVERHEAD);
        (available / self.rows_per_cassette()).max(1) as usize
    }

    pub fn add_cassette(&mut self) {
        if self.cassettes.len() >= self.max_cassettes() {
            self.status_msg = Some("No more vertical space for additional cassettes.".into());
            return;
        }
        self.cassettes.push(Cassette::new());
        self.focus_idx = self.cassettes.len() - 1;
        self.status_msg = None;
    }

    pub fn focus_next(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + 1) % n;
        self.status_msg = None;
    }

    pub fn focus_prev(&mut self) {
        let n = self.cassettes.len().max(1);
        self.focus_idx = (self.focus_idx + n - 1) % n;
        self.status_msg = None;
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
    }

    pub fn total_word_count(&self) -> usize {
        self.cassettes.iter().map(|c| c.word_count()).sum()
    }

    /// Cursor position as a ratio 0.0..=1.0 for the focused cassette.
    pub fn focused_cursor_ratio(&self) -> f64 {
        let c = &self.cassettes[self.focus_idx];
        let left = c.cursor_pos();
        let total = c.text().chars().count().max(1);
        left as f64 / total as f64
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
