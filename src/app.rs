use std::time::SystemTime;

use crate::cassette::{Cassette, Side};

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
    /// Words already in the note when a `resume` loaded it; the session
    /// recap reports only what was added on top.
    pub baseline_words: usize,
    /// Seconds since the last keypress; drives the idle nudge.
    pub idle_secs: u32,
    /// The store session this app's cassettes belong to (`store::ids` ULID).
    /// Plain data here — resolving or creating the session against a `Store`
    /// is `main.rs`'s job, not `App`'s. Empty only in `-o` mode, which
    /// persists nothing.
    pub session: String,
    /// The focused cassette's lock could not be taken — another writer holds
    /// it. The text is shown but not editable: accepting keystrokes with no
    /// guard to write them through would lose them at the next flush, and
    /// refusing to start would let an agent lock a human out of their own
    /// session. `modify_focused` is the gate; `main.rs` sets the flag when
    /// `SessionWriter::acquire` fails.
    pub read_only: bool,
    /// While `read_only` is set, the name of the writer holding the lock —
    /// from the lock anchor's own attribution, the same source `queue
    /// write`'s exit-3 message reads (`LockError::Busy`'s `holder`). `None`
    /// when read-only for a reason with no name to show (no writer, or a
    /// holder whose anchor could not be read). `main.rs` sets this alongside
    /// `read_only`, on every acquire attempt — keypress-driven and, from this
    /// task, tick-driven too. Distinct from a cassette's own `locked_by`: a
    /// busy holder is transient and frees itself; a sticky lock is durable
    /// and needs a human to clear it (`queue unlock`) — the two must not
    /// render the same way.
    pub busy_holder: Option<String>,
    /// One-shot request for a terminal bell, consumed by `main.rs`.
    pub bell: bool,
    /// One-shot request to suspend the process (Ctrl+Z), consumed by `main.rs`.
    pub suspend: bool,
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
        session: String,
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
            baseline_words: 0,
            idle_secs: 0,
            session,
            read_only: false,
            busy_holder: None,
            bell: false,
            suspend: false,
            status_ticks: None,
            goal_announced: false,
        }
    }

    /// Show `msg` on the status line for `STATUS_FLASH_SECS`, with a bell.
    ///
    /// `pub(crate)` for `main.rs`'s `try_acquire`, which reports a lock
    /// failure that is *not* ordinary contention this way: those have no
    /// banner of their own, and being transient news is exactly what keeps
    /// them out of the standing-condition role `status_msg` must not take on.
    pub(crate) fn flash(&mut self, msg: String) {
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
            let words = self.session_word_count();
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

    /// Replace the session's cassettes with ones loaded from a saved note
    /// (`resume`). Focus lands on the last cassette, whose cursor is
    /// already at the end of side A — ready to keep writing.
    pub fn load_cassettes(&mut self, cassettes: Vec<Cassette>) {
        if cassettes.is_empty() {
            return;
        }
        self.cassettes = cassettes;
        self.sort_queue();
        // Keep every closed cassette and the best `MAX_CASSETTES` open ones.
        // A blind `truncate` here dropped OPEN cassettes off a session with
        // enough history, because queue order puts the closed ones in the
        // same list — the cap is on the working set, not on what is kept.
        let mut open_kept = 0usize;
        self.cassettes.retain(|c| {
            if c.closed {
                return true;
            }
            open_kept += 1;
            open_kept <= MAX_CASSETTES
        });
        self.baseline_words = self.cassettes.iter().map(|c| c.word_count()).sum();
        // Land on the last OPEN cassette: resuming focused on a closed one
        // would open read-only with no way to type.
        self.focus_idx = self
            .open_count()
            .saturating_sub(1)
            .min(self.cassettes.len().saturating_sub(1));
        self.ensure_focus_visible();
    }

    /// Merge one cassette read from the store into the list: update it in
    /// place if this process already holds a copy (by store id, not by
    /// index — an earlier insertion may have shifted it), or insert `incoming`
    /// as a newcomer at `insert_at`.
    ///
    /// `insert_at` is the caller's job, not this method's: priority lives in
    /// `store::meta::CassetteMeta`, which `Cassette` — a pure data type with
    /// no store knowledge — does not carry and which this task does not add
    /// to it. `main.rs`, which already reads the store's priority order to
    /// decide what to merge in the first place, is where that ordering
    /// knowledge already lives; passing the position here keeps it there
    /// instead of duplicating it onto `Cassette`.
    ///
    /// The cursor rule (the point of this method): if the existing
    /// cassette's cursor sat at the end of its active side, the merged
    /// cursor follows the incoming text's new end — watching an agent write
    /// follows the newest words. Otherwise the cursor stays at the same
    /// character offset, clamped to the incoming text — a reader who
    /// scrolled up to re-read a paragraph is not yanked to the bottom.
    /// `ui.rs` derives `scroll_top` from the cursor's row on every render, so
    /// placing the cursor here is the whole of it; there is no separate
    /// scroll offset to update.
    ///
    /// Two invariants beyond the cursor rule:
    /// - **Focus identity.** Whichever cassette was focused before the merge
    ///   is still focused after, even though an insertion ahead of it shifts
    ///   every later index. Resolved by id, the same way `SessionWriter`
    ///   tracks the held lock across insertions.
    /// - **Never merge over unsaved edits.** `main.rs` must never call this
    ///   for a cassette this process has dirtied — asserted here rather than
    ///   trusted, the same way `SessionWriter::refresh_from_disk` guards the
    ///   identical case with a `debug_assert!` plus a defensive early return.
    /// - **A no-op when nothing changed.** Same short circuit as
    ///   `refresh_from_disk`: if `incoming`'s sides and topic already match
    ///   the existing cassette, return without rebuilding it. Without this,
    ///   this process's own flush of a cassette it just released focus on —
    ///   which moves that file's mtime with no external writer involved —
    ///   would look like a first-sight change to `main.rs`'s sync step and
    ///   silently discard the cassette's undo stack and reset its cursor on
    ///   every tab-away, even though disk and memory already agreed. `incoming`'s
    ///   `locked_by` is still applied when it's the only thing that moved
    ///   (a `queue lock`/`unlock` with no text change) — it's metadata, not
    ///   prose, and updating it in place costs the cursor/undo state nothing.
    ///
    /// Called from `main.rs`'s live-sync step (Task 4), on the existing
    /// one-second tick, for any cassette whose lock this process does not
    /// hold and whose file mtime has moved since it was last seen.
    pub fn merge_external(&mut self, id: &str, incoming: Cassette, insert_at: usize) {
        let focused_id = self.cassettes.get(self.focus_idx).map(|c| c.id.clone());

        if let Some(idx) = self.cassettes.iter().position(|c| c.id == id) {
            let existing = &self.cassettes[idx];
            debug_assert!(
                !existing.dirty,
                "a cassette this process holds unsaved edits in cannot have been \
                 changed by another writer; main.rs must never offer one here"
            );
            if existing.dirty {
                return;
            }

            // Mirrors `SessionWriter::refresh_from_disk`'s identical short
            // circuit: when nothing actually differs, skip the rebuild
            // entirely rather than resetting the cursor and discarding the
            // undo stack for content that never changed. This is not just
            // an optimization — it is what makes this process's own flush
            // of a cassette it just released focus on (which moves that
            // file's mtime with no external writer involved) a no-op here,
            // the same way `refresh_from_disk` already makes an unchanged
            // re-acquire a no-op on the read side.
            if incoming.side_a_text().trim() == existing.side_a_text().trim()
                && incoming.side_b_text().trim() == existing.side_b_text().trim()
                && incoming.topic == existing.topic
            {
                if self.cassettes[idx].locked_by != incoming.locked_by {
                    self.cassettes[idx].locked_by = incoming.locked_by;
                }
                return;
            }

            let existing_cursor = existing.cursor_pos();
            let was_at_end = existing_cursor == existing.char_count();
            let existing_on_side_b = existing.side == Side::B;

            let side_a_len = incoming.side_a_text().chars().count();
            let side_b_len = incoming.side_b_text().chars().count();
            // The active-side cursor is placed via `from_sides_with_cursor`
            // for side A directly; side B (the less common case: the reader
            // was on the scratch side when the update landed) is placed by
            // flipping and clamping afterward, since that constructor only
            // ever seeds side A.
            let cursor_for_a = if existing_on_side_b {
                0
            } else if was_at_end {
                side_a_len
            } else {
                existing_cursor
            };

            let mut merged = Cassette::from_sides_with_cursor(
                incoming.side_a_text(),
                incoming.side_b_text(),
                incoming.topic.clone(),
                cursor_for_a,
            );
            if existing_on_side_b {
                merged.flip();
                let target = if was_at_end {
                    side_b_len
                } else {
                    existing_cursor
                };
                merged.set_cursor(target);
            }
            merged.id = id.to_string();
            merged.locked_by = incoming.locked_by;
            self.cassettes[idx] = merged;
        } else {
            let mut merged = incoming;
            merged.id = id.to_string();
            let at = insert_at.min(self.cassettes.len());
            self.cassettes.insert(at, merged);
        }

        if let Some(fid) = focused_id {
            if let Some(new_idx) = self.cassettes.iter().position(|c| c.id == fid) {
                self.focus_idx = new_idx;
            }
        }
        self.ensure_focus_visible();
    }

    /// Cassettes in queue order: open before closed, then priority, then id.
    /// Mirrors `store::priority::queue_order`, which stays the authority on
    /// what queue order means — `App` is pure and cannot import it, so the
    /// two are kept in step by tests that pin the same three keys.
    ///
    /// Focus is preserved by **identity**, not position: sorting moves
    /// cassettes under `focus_idx`. Doing that here rather than in each
    /// caller is the discipline `merge_external` already follows, and the
    /// reason 5b bound the held lock guard to an id instead of an index.
    pub fn sort_queue(&mut self) {
        let focused_id = self.cassettes.get(self.focus_idx).map(|c| c.id.clone());
        self.cassettes.sort_by(|a, b| {
            // The id tie-break sorts an UNMINTED cassette (empty id) last
            // rather than first. Plain string order would put "" ahead of
            // every real ULID, so a Ctrl+N cassette would jump the queue for
            // as long as it took `create_cassette` to mint its id. An
            // unminted cassette is by definition the newest thing here.
            let key = |c: &Cassette| (c.closed, c.priority, c.id.is_empty(), c.id.clone());
            key(a).cmp(&key(b))
        });
        if let Some(id) = focused_id {
            if let Some(i) = self.cassettes.iter().position(|c| c.id == id) {
                self.focus_idx = i;
            }
        }
        self.ensure_focus_visible();
    }

    /// How many cassettes are open.
    ///
    /// Counted with a filter rather than a `take_while` over the sorted
    /// prefix: the prefix property holds only while the list is sorted, and
    /// a count that silently returns 0 because something pushed before
    /// re-sorting is a trap. Callers that need the prefix — `stack_len`, and
    /// the scroll window through it — get it from `sort_queue` maintaining
    /// the order, not from this method assuming it.
    pub fn open_count(&self) -> usize {
        self.cassettes.iter().filter(|c| !c.closed).count()
    }

    pub fn add_cassette(&mut self) {
        // The cap is on the working set, not the retained history: closed
        // cassettes fold away and are never written in, so they must not be
        // what stops a human starting a new one.
        if self.open_count() >= MAX_CASSETTES {
            self.status_msg = Some(format!("Cassette limit reached ({}).", MAX_CASSETTES));
            return;
        }
        self.cassettes.push(Cassette::new());
        // Re-sort so the new cassette sits in the open section rather than
        // after the closed ones, keeping the open-set-is-a-prefix invariant
        // the fold and the scroll window both rely on. An unminted cassette
        // carries `i64::MAX`, so it lands last among the open ones — which
        // is where the human expects a brand new cassette to be.
        self.sort_queue();
        self.focus_idx = self.open_count().saturating_sub(1);
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

    /// Apply `f` to the focused cassette and mark it dirty — unless this
    /// process does not hold its lock, in which case the edit is dropped on
    /// the floor. A keystroke that never reaches a cassette is visible to
    /// the writer; one that reaches it and is then discarded at the next
    /// flush is not.
    pub fn modify_focused<F: FnOnce(&mut Cassette)>(&mut self, f: F) {
        if self.read_only {
            return;
        }
        if let Some(c) = self.cassettes.get_mut(self.focus_idx) {
            f(c);
            c.dirty = true;
        }
    }

    /// Indices of cassettes with unsaved edits, in list order. An autosave
    /// writes only these — not every cassette in the session.
    pub fn dirty_indices(&self) -> Vec<usize> {
        self.cassettes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.dirty)
            .map(|(i, _)| i)
            .collect()
    }

    /// Mark cassette `idx` as saved. Out-of-range indices are a no-op.
    pub fn clear_dirty(&mut self, idx: usize) {
        if let Some(c) = self.cassettes.get_mut(idx) {
            c.dirty = false;
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

    /// Words written this sitting: the total minus what a `resume` loaded.
    /// Live stats (goal, reel, info line) run on this; the saved file and its
    /// frontmatter keep the full total.
    pub fn session_word_count(&self) -> usize {
        self.total_word_count().saturating_sub(self.baseline_words)
    }

    /// How much tape has wound onto the take-up reel, 0.0..=1.0: progress
    /// toward the word goal, or elapsed time when only a timer is set.
    /// `None` when neither is set — the session has no bar to show.
    pub fn tape_ratio(&self) -> Option<f64> {
        if let Some(goal) = self.word_goal {
            let goal = goal.max(1);
            return Some((self.session_word_count() as f64 / goal as f64).min(1.0));
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
        let total_wc = self.session_word_count();
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
        let mut app = App::new(
            None,
            None,
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
    fn session_stats_ignore_resumed_words() {
        let mut app = App::new(
            None,
            Some(10),
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
        app.load_cassettes(vec![Cassette::from_sides(
            "twelve resumed words already on the tape from a previous session sit here".into(),
            String::new(),
            None,
        )]);
        assert_eq!(app.tape_ratio(), Some(0.0), "old words wind no tape");
        assert!(
            app.format_stats().contains("0 / 10"),
            "stats start at zero: {}",
            app.format_stats()
        );
        app.check_goal();
        assert!(
            app.status_msg.is_none(),
            "resumed words never fire the goal celebration"
        );
        app.modify_focused(|c| {
            for ch in " one two three four five six seven eight nine ten".chars() {
                c.insert(ch);
            }
        });
        assert_eq!(app.tape_ratio(), Some(1.0), "ten new words meet the goal");
        app.check_goal();
        assert!(
            app.status_msg.is_some(),
            "goal fires on this session's words"
        );
    }

    #[test]
    fn tape_ratio_tracks_words_against_goal() {
        let mut app = App::new(
            None,
            Some(10),
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            None,
            Some(2),
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            Some(4),
            None,
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
        assert_eq!(app.tape_ratio(), Some(0.0));
        app.tick_timer();
        app.tick_timer();
        assert_eq!(app.tape_ratio(), Some(0.5), "half the session elapsed");
    }

    #[test]
    fn tape_ratio_absent_without_goal_or_timer() {
        let app = App::new(
            None,
            None,
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
        assert_eq!(app.tape_ratio(), None, "no goal, no timer: no bar");
    }

    #[test]
    fn timer_expiry_flashes_status_and_bell_once() {
        let mut app = App::new(
            Some(2),
            None,
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            None,
            Some(3),
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            Some(1),
            None,
            Some(VISIBLE_LINES),
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        assert!(!app.cassettes[0].dirty);
        app.modify_focused(|c| c.insert('a'));
        assert!(app.cassettes[0].dirty);
    }

    #[test]
    fn editing_marks_only_the_focused_cassette_dirty() {
        // The whole point of per-cassette dirty: an autosave must write the one
        // cassette that changed, not rewrite every cassette in the session.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.add_cassette();
        assert_eq!(app.dirty_indices(), Vec::<usize>::new(), "clean to start");

        app.focus_idx = 1;
        app.modify_focused(|c| c.insert('x'));
        assert_eq!(app.dirty_indices(), vec![1], "only the focused one");

        app.clear_dirty(1);
        assert_eq!(app.dirty_indices(), Vec::<usize>::new(), "cleared");
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
        let mut app = App::new(
            Some(60),
            None,
            None,
            "01JTESTSESSN00000000000000".to_string(),
        );
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
        let mut app = App::new(
            Some(1),
            None,
            None,
            "01JTESTSESSN00000000000000".to_string(),
        );
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

    /// Today's `truncate(MAX_CASSETTES)` runs over a list queue order has
    /// already put closed cassettes inside, so a resumed session with enough
    /// history drops OPEN cassettes off the end. The cap is on working set.
    #[test]
    fn loading_keeps_every_open_cassette_when_closed_ones_fill_the_list() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        let mut loaded = Vec::new();
        for i in 0..MAX_CASSETTES {
            let mut c = Cassette::new();
            c.id = format!("open-{i:02}");
            c.priority = i as i64 * 10;
            loaded.push(c);
        }
        for i in 0..5 {
            let mut c = Cassette::new();
            c.id = format!("closed-{i}");
            c.closed = true;
            c.priority = i as i64;
            loaded.push(c);
        }

        app.load_cassettes(loaded);

        assert_eq!(
            app.open_count(),
            MAX_CASSETTES,
            "no open cassette is dropped"
        );
        assert!(
            app.cassettes.iter().any(|c| c.id == "open-35"),
            "including the last one"
        );
    }

    /// `Ctrl+N` is capped on the working set too, so closed history never
    /// blocks a new cassette.
    #[test]
    fn add_cassette_caps_on_open_not_total() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        for i in 0..5 {
            let mut c = Cassette::new();
            c.id = format!("closed-{i}");
            c.closed = true;
            app.cassettes.push(c);
        }
        app.sort_queue();
        for _ in 0..MAX_CASSETTES {
            app.add_cassette();
        }
        assert_eq!(app.open_count(), MAX_CASSETTES);
        assert_eq!(
            app.cassettes.len(),
            MAX_CASSETTES + 5,
            "closed ones are retained"
        );
    }

    /// Queue order is the store's, not insertion order: open before closed,
    /// then priority, then id as the tie-break. `store::priority::queue_order`
    /// is the authority on this; `sort_queue` must not disagree with it.
    #[test]
    fn sort_queue_orders_open_before_closed_then_priority_then_id() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        for (id, priority, closed) in [
            ("z", 20, false),
            ("b", 10, true),
            ("a", 20, false),
            ("m", 10, false),
        ] {
            let mut c = Cassette::new();
            c.id = id.to_string();
            c.priority = priority;
            c.closed = closed;
            app.cassettes.push(c);
        }

        app.sort_queue();

        assert_eq!(
            app.cassettes
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m", "a", "z", "b"],
            "open by priority then id, closed last"
        );
        assert_eq!(app.open_count(), 3);
    }

    /// An unminted cassette carries an empty id, which plain string order
    /// would sort ahead of every real ULID — so at equal priority a brand
    /// new cassette would jump the queue until its id was minted.
    #[test]
    fn an_unminted_cassette_sorts_after_minted_ones_at_the_same_priority() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        let mut minted = Cassette::new();
        minted.id = "01JREAL0000000000000000000".to_string();
        app.cassettes.push(minted);
        app.cassettes.push(Cassette::new()); // same i64::MAX priority, empty id

        app.sort_queue();

        assert_eq!(app.cassettes[0].id, "01JREAL0000000000000000000");
        assert!(
            app.cassettes[1].id.is_empty(),
            "the unminted one stays last"
        );
    }

    /// Sorting moves cassettes under `focus_idx`, which is an index. Focus is
    /// identity, not position — the same rule `merge_external` already follows
    /// and the reason 5b bound the lock guard to an id.
    #[test]
    fn sort_queue_keeps_focus_on_the_same_cassette() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        for (id, priority) in [("z", 30), ("a", 10)] {
            let mut c = Cassette::new();
            c.id = id.to_string();
            c.priority = priority;
            app.cassettes.push(c);
        }
        app.focus_idx = 0; // "z"

        app.sort_queue();

        assert_eq!(
            app.cassettes[app.focus_idx].id, "z",
            "focus follows the cassette"
        );
        assert_eq!(app.focus_idx, 1, "which is now at the tail");
    }

    /// A `Ctrl+N` cassette has no store priority yet. It must sort to the tail
    /// rather than to the head, so a new cassette appears where the human
    /// expects it until `create_cassette` mints the real value.
    #[test]
    fn an_unminted_cassette_sorts_to_the_tail() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes.clear();
        let mut stored = Cassette::new();
        stored.id = "a".to_string();
        stored.priority = 10;
        app.cassettes.push(stored);
        app.cassettes.push(Cassette::new()); // unminted: empty id, i64::MAX

        app.sort_queue();

        assert_eq!(app.cassettes[0].id, "a");
        assert!(app.cassettes[1].id.is_empty(), "the new one stays last");
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

    #[test]
    fn merge_external_is_a_no_op_when_content_and_topic_already_match() {
        // The regression this pins: `main.rs`'s sync step moves a
        // cassette's own flush-induced mtime the instant focus releases it,
        // which can look like a first-sight change even though disk and
        // memory already agree. Without this short circuit, merging
        // identical content would rebuild the cassette via
        // `from_sides_with_cursor` anyway, discarding its undo stack and
        // resetting its cursor for no reason.
        //
        // `Cassette`'s undo stack has no public accessor, so this observes
        // survival behaviorally: an `undo()` after the merge must still
        // revert the insert, which is only possible if the merge left the
        // original cassette object — undo stack included — untouched. The
        // cursor position is asserted directly.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| {
            c.snapshot(); // what entering insert mode does
            c.insert_str("hello world");
            c.move_word_back();
        });
        app.clear_dirty(0);
        let cursor_before = app.cassettes[0].cursor_pos();

        // Exactly what's already in memory — e.g. this process's own flush
        // of the cassette it just released focus on, not an external
        // writer's edit.
        let incoming = Cassette::from_sides("hello world".to_string(), String::new(), None);
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(
            app.cassettes[0].cursor_pos(),
            cursor_before,
            "unchanged content must not move the cursor"
        );
        app.modify_focused(|c| c.undo());
        assert_eq!(
            app.cassettes[0].text(),
            "",
            "and the undo stack must survive a merge of identical content"
        );
    }

    #[test]
    fn merge_external_carries_locked_by_through_a_full_rebuild() {
        // `merged` in the rebuild branch is a fresh `Cassette` built by
        // `from_sides_with_cursor`, which knows nothing about
        // `incoming.locked_by` unless the branch copies it across — a sticky
        // lock must survive the same merge that follows an agent's words.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();

        let mut incoming = Cassette::from_sides("hello world".to_string(), String::new(), None);
        incoming.locked_by = Some("joseph".to_string());
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(app.cassettes[0].locked_by.as_deref(), Some("joseph"));
    }

    #[test]
    fn merge_external_updates_locked_by_even_when_the_no_op_short_circuit_fires() {
        // A sticky lock is metadata, not prose: `queue lock`/`unlock` can
        // change it with the cassette's own text and topic untouched, and the
        // no-op short circuit (which exists to protect the cursor/undo stack
        // from a self-inflicted mtime move) must not swallow that change
        // along with the rebuild the text genuinely didn't need.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| {
            c.snapshot();
            c.insert_str("hello world");
            c.move_word_back();
        });
        app.clear_dirty(0);
        let cursor_before = app.cassettes[0].cursor_pos();

        let mut incoming = Cassette::from_sides("hello world".to_string(), String::new(), None);
        incoming.locked_by = Some("joseph".to_string());
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(
            app.cassettes[0].locked_by.as_deref(),
            Some("joseph"),
            "the sticky lock must still land"
        );
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            cursor_before,
            "text/topic were unchanged, so the cursor must not move"
        );
        app.modify_focused(|c| c.undo());
        assert_eq!(
            app.cassettes[0].text(),
            "",
            "and the undo stack must survive, exactly as the plain no-op case does"
        );
    }

    #[test]
    fn merging_follows_the_new_text_when_the_cursor_was_at_the_end() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| c.insert_str("first"));
        app.clear_dirty(0);
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            5,
            "cursor at the end to start"
        );

        let incoming = Cassette::from_sides("first and more".to_string(), String::new(), None);
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(app.cassettes[0].side_a_text(), "first and more");
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            14,
            "the cursor was at the end, so it follows the new end"
        );
    }

    #[test]
    fn merging_leaves_a_scrolled_back_cursor_where_it_was() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| c.insert_str("first"));
        app.modify_focused(|c| c.move_text_start());
        app.clear_dirty(0);
        assert_eq!(app.cassettes[0].cursor_pos(), 0);

        let incoming = Cassette::from_sides("first and more".to_string(), String::new(), None);
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(
            app.cassettes[0].cursor_pos(),
            0,
            "a reader who scrolled up must not be yanked to the bottom"
        );
    }

    #[test]
    fn merge_external_updates_by_id_not_by_index() {
        // The updated cassette isn't at index 0: merging must find it by id,
        // never assume the caller already knows its position.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "zzz00000000000000000000000".to_string();
        app.add_cassette();
        app.cassettes[1].id = "aaa00000000000000000000000".to_string();
        app.clear_dirty(1);

        let incoming = Cassette::from_sides("updated".to_string(), String::new(), None);
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(
            app.cassettes.len(),
            2,
            "no insertion — this id already existed"
        );
        assert_eq!(app.cassettes[1].side_a_text(), "updated");
    }

    #[test]
    fn an_unknown_cassette_is_inserted_without_moving_focus() {
        // An agent's `queue new` arrives. The human is typing in what is
        // currently index 0; after the insert they must still be typing in it.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "bbb00000000000000000000000".to_string();
        app.focus_idx = 0;
        let focused_id = app.cassettes[0].id.clone();

        let mut newcomer = Cassette::new();
        newcomer.id = "aaa00000000000000000000000".to_string();
        // Priority puts the newcomer ahead of the focused cassette: index 0
        // shifts to index 1, so this also exercises invariant 1.
        app.merge_external("aaa00000000000000000000000", newcomer, 0);

        assert_eq!(app.cassettes.len(), 2);
        assert_eq!(
            app.cassettes[app.focus_idx].id, focused_id,
            "focus must still name the cassette the user was typing in"
        );
    }

    #[test]
    fn merge_external_inserts_an_unknown_cassette_at_its_priority_position() {
        // Priority order, not append: a cassette between two existing ones
        // must land between them, not at the tail.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "first0000000000000000000000".to_string();
        app.add_cassette();
        app.cassettes[1].id = "third0000000000000000000000".to_string();

        let mut newcomer = Cassette::new();
        newcomer.id = "second000000000000000000000".to_string();
        app.merge_external("second000000000000000000000", newcomer, 1);

        assert_eq!(app.cassettes.len(), 3);
        assert_eq!(app.cassettes[0].id, "first0000000000000000000000");
        assert_eq!(
            app.cassettes[1].id, "second000000000000000000000",
            "lands at its priority slot, not appended after the third cassette"
        );
        assert_eq!(app.cassettes[2].id, "third0000000000000000000000");
    }

    #[test]
    fn merging_follows_the_new_text_on_side_b_when_the_cursor_was_at_the_end() {
        // The less common branch: the reader was on the scratch side (B) when
        // the update landed. Traced correct by hand in Task 3's review but
        // never pinned by a test until now.
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.cassettes[0].id = "aaa00000000000000000000000".to_string();
        app.modify_focused(|c| {
            c.flip();
            c.insert_str("scratch");
        });
        app.clear_dirty(0);
        assert_eq!(app.cassettes[0].side, Side::B, "reader is on side B");
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            7,
            "cursor at the end of side B to start"
        );

        let incoming = Cassette::from_sides(
            "side a text".to_string(),
            "scratch and more".to_string(),
            None,
        );
        app.merge_external("aaa00000000000000000000000", incoming, 0);

        assert_eq!(
            app.cassettes[0].side,
            Side::B,
            "the merge must not silently switch the reader back to side A"
        );
        assert_eq!(app.cassettes[0].side_a_text(), "side a text");
        assert_eq!(app.cassettes[0].side_b_text(), "scratch and more");
        assert_eq!(
            app.cassettes[0].cursor_pos(),
            16,
            "the cursor was at the end of side B, so it follows side B's new end"
        );
    }

    #[test]
    fn read_only_ignores_text_keys_but_allows_leaving() {
        let mut app = App::new(None, None, None, "01JTESTSESSN00000000000000".to_string());
        app.read_only = true;
        let before = app.cassettes[app.focus_idx].side_a_text();

        app.modify_focused(|c| c.insert('x'));
        assert_eq!(
            app.cassettes[app.focus_idx].side_a_text(),
            before,
            "a keystroke must never reach a cassette this process does not hold"
        );
        assert!(
            !app.cassettes[app.focus_idx].dirty,
            "and it must not be marked dirty"
        );
    }
}
