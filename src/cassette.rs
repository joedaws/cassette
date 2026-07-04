/// Which side of the cassette is currently active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    A,
    B,
}

/// Cursor-zipper text buffer: left holds text before the cursor, right holds text after.
/// Text may contain '\n'; display wrapping is computed with `wrap_spans`.
///
/// Each cassette has two sides (A and B, like a tape). The zipper always holds the
/// active side; `flip` swaps it with the stored back side, so each side keeps its
/// own cursor position across flips.
#[derive(Clone, Debug, Default)]
pub struct Cassette {
    pub left: String,
    pub right: String,
    back_left: String,
    back_right: String,
    pub side: Side,
    /// Optional label for what this cassette is about; shared by both sides.
    pub topic: Option<String>,
    /// Undo snapshots of the active side, oldest first.
    undo: Vec<(String, String)>,
    /// Undo snapshots of the side currently flipped away.
    back_undo: Vec<(String, String)>,
}

/// Maximum undo snapshots kept per cassette side.
const UNDO_DEPTH: usize = 100;

/// Terminal cells occupied by `c` (0 for combining marks, 2 for CJK/emoji).
pub fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

fn cells(chars: &[char]) -> usize {
    chars.iter().map(|&c| char_width(c)).sum()
}

/// Display rows for `chars` word-wrapped at `width` columns, breaking on '\n'.
/// Returns `(start, end)` char-index spans; a terminating '\n' is not part of its span.
///
/// Columns are terminal cells, not chars: CJK and emoji count as 2, combining
/// marks as 0 (`char_width`). Wrapping happens at word boundaries: when a word
/// overflows the row it moves whole to the next row, and the space before it
/// stays (trailing) on the previous row, so wrapped rows never start with a
/// space. Words longer than `width` are hard-broken. Runs of trailing spaces
/// hang past the right edge rather than wrap (renderers clip them), like
/// classic word processors.
pub fn wrap_spans(chars: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut spans = Vec::new();
    let mut start = 0;
    let mut col = 0;
    // Wrap candidate: index just after the last space of the current row.
    let mut last_break: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            spans.push((start, i));
            start = i + 1;
            col = 0;
            last_break = None;
            continue;
        }
        col += char_width(c);
        if col > width && c != ' ' {
            match last_break {
                Some(b) if b > start => {
                    spans.push((start, b));
                    start = b;
                }
                _ => {
                    // No space on this row: hard-break the long word.
                    spans.push((start, i));
                    start = i;
                }
            }
            col = cells(&chars[start..=i]);
            last_break = None;
        }
        if c == ' ' {
            last_break = Some(i + 1);
        }
    }
    spans.push((start, chars.len()));
    spans
}

/// Row and column of char position `pos` within `spans`.
/// On a soft-wrap boundary the position belongs to the start of the following row.
pub fn pos_to_row_col(spans: &[(usize, usize)], pos: usize) -> (usize, usize) {
    for (i, &(s, e)) in spans.iter().enumerate() {
        if pos < e {
            return (i, pos - s);
        }
        if pos == e {
            match spans.get(i + 1) {
                Some(&(ns, _)) if ns == e => continue,
                _ => return (i, pos - s),
            }
        }
    }
    let (s, e) = *spans.last().expect("wrap_spans always yields one span");
    (spans.len() - 1, e - s)
}

fn row_col_to_pos(spans: &[(usize, usize)], row: usize, col: usize) -> usize {
    let (s, e) = spans[row.min(spans.len() - 1)];
    s + col.min(e - s)
}

/// Bounds of the logical line (between '\n's) containing char position `pos`.
/// `start` is the position after the previous '\n' (or 0); `end` is the position
/// of the next '\n' (or the text length).
fn line_bounds(chars: &[char], pos: usize) -> (usize, usize) {
    let start = chars[..pos]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |i| i + 1);
    let end = chars[pos..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |i| pos + i);
    (start, end)
}

impl Cassette {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild a cassette from saved text (resume): side A active with the
    /// cursor at its end, side B stored, no undo history.
    pub fn from_sides(side_a: String, side_b: String, topic: Option<String>) -> Self {
        Self {
            left: side_a,
            back_left: side_b,
            topic,
            ..Self::default()
        }
    }

    pub fn text(&self) -> String {
        format!("{}{}", self.left, self.right)
    }

    /// Number of characters before the cursor.
    pub fn cursor_pos(&self) -> usize {
        self.left.chars().count()
    }

    pub fn char_count(&self) -> usize {
        self.left.chars().count() + self.right.chars().count()
    }

    /// Flip to the other side, preserving each side's cursor position.
    pub fn flip(&mut self) {
        std::mem::swap(&mut self.left, &mut self.back_left);
        std::mem::swap(&mut self.right, &mut self.back_right);
        std::mem::swap(&mut self.undo, &mut self.back_undo);
        self.side = match self.side {
            Side::A => Side::B,
            Side::B => Side::A,
        };
    }

    /// Record the active side's state so `undo` can restore it.
    /// Call before a destructive edit or when entering insert mode.
    pub fn snapshot(&mut self) {
        if self.undo.len() >= UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push((self.left.clone(), self.right.clone()));
    }

    /// Restore the most recent snapshot of the active side (vim `u`).
    /// Text and cursor come back together; a no-op with no history.
    pub fn undo(&mut self) {
        if let Some((left, right)) = self.undo.pop() {
            self.left = left;
            self.right = right;
        }
    }

    /// Text of the side currently flipped away.
    pub fn back_text(&self) -> String {
        format!("{}{}", self.back_left, self.back_right)
    }

    pub fn side_a_text(&self) -> String {
        match self.side {
            Side::A => self.text(),
            Side::B => self.back_text(),
        }
    }

    pub fn side_b_text(&self) -> String {
        match self.side {
            Side::A => self.back_text(),
            Side::B => self.text(),
        }
    }

    /// 1-based (logical line, column) of the cursor.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.left.chars().filter(|&c| c == '\n').count() + 1;
        let col = self.left.chars().rev().take_while(|&c| c != '\n').count() + 1;
        (line, col)
    }

    fn chars_and_pos(&self) -> (Vec<char>, usize) {
        let pos = self.left.chars().count();
        let chars = self.left.chars().chain(self.right.chars()).collect();
        (chars, pos)
    }

    /// Move the cursor to char position `pos` (clamped to the text length).
    pub fn set_cursor(&mut self, pos: usize) {
        let (chars, _) = self.chars_and_pos();
        let pos = pos.min(chars.len());
        self.left = chars[..pos].iter().collect();
        self.right = chars[pos..].iter().collect();
    }

    /// Words across both sides of the cassette.
    pub fn word_count(&self) -> usize {
        self.text().split_whitespace().count() + self.back_text().split_whitespace().count()
    }

    pub fn insert(&mut self, c: char) {
        self.left.push(c);
    }

    /// Insert a whole string at the cursor (bracketed paste): one edit,
    /// so callers can wrap it in a single undo snapshot.
    pub fn insert_str(&mut self, s: &str) {
        self.left.push_str(s);
    }

    pub fn backspace(&mut self) {
        self.left.pop();
    }

    pub fn delete(&mut self) {
        let mut chars = self.right.chars();
        if chars.next().is_some() {
            self.right = chars.collect();
        }
    }

    pub fn move_left(&mut self) {
        if let Some(c) = self.left.pop() {
            let mut new_right = String::with_capacity(c.len_utf8() + self.right.len());
            new_right.push(c);
            new_right.push_str(&self.right);
            self.right = new_right;
        }
    }

    pub fn move_right(&mut self) {
        let mut chars = self.right.chars();
        if let Some(c) = chars.next() {
            self.left.push(c);
            self.right = chars.collect();
        }
    }

    /// Move the cursor one display row up, keeping the column when possible.
    pub fn move_up(&mut self, width: usize) {
        let (chars, pos) = self.chars_and_pos();
        let spans = wrap_spans(&chars, width);
        let (row, col) = pos_to_row_col(&spans, pos);
        if row > 0 {
            self.set_cursor(row_col_to_pos(&spans, row - 1, col));
        }
    }

    /// Move the cursor one display row down, keeping the column when possible.
    pub fn move_down(&mut self, width: usize) {
        let (chars, pos) = self.chars_and_pos();
        let spans = wrap_spans(&chars, width);
        let (row, col) = pos_to_row_col(&spans, pos);
        if row + 1 < spans.len() {
            self.set_cursor(row_col_to_pos(&spans, row + 1, col));
        }
    }

    /// Move to the start of the current display row (vim `0`).
    pub fn move_row_start(&mut self, width: usize) {
        let (chars, pos) = self.chars_and_pos();
        let spans = wrap_spans(&chars, width);
        let (row, _) = pos_to_row_col(&spans, pos);
        self.set_cursor(spans[row].0);
    }

    /// Move to the end of the current display row (vim `$`). On a soft-wrapped
    /// row the cursor lands before the last char so it stays on the same row.
    pub fn move_row_end(&mut self, width: usize) {
        let (chars, pos) = self.chars_and_pos();
        let spans = wrap_spans(&chars, width);
        let (row, _) = pos_to_row_col(&spans, pos);
        let (s, e) = spans[row];
        let soft = spans.get(row + 1).is_some_and(|&(ns, _)| ns == e);
        let target = if soft { e.saturating_sub(1).max(s) } else { e };
        self.set_cursor(target);
    }

    /// Move to the start of the next word (vim `w`).
    pub fn move_word_forward(&mut self) {
        let (chars, mut p) = self.chars_and_pos();
        while p < chars.len() && !chars[p].is_whitespace() {
            p += 1;
        }
        while p < chars.len() && chars[p].is_whitespace() {
            p += 1;
        }
        self.set_cursor(p);
    }

    /// Move to the start of the previous word (vim `b`).
    pub fn move_word_back(&mut self) {
        let (chars, mut p) = self.chars_and_pos();
        while p > 0 && chars[p - 1].is_whitespace() {
            p -= 1;
        }
        while p > 0 && !chars[p - 1].is_whitespace() {
            p -= 1;
        }
        self.set_cursor(p);
    }

    /// Move to the very start of the text (vim `gg`).
    pub fn move_text_start(&mut self) {
        self.set_cursor(0);
    }

    /// Move to the very end of the text (vim `G`).
    pub fn move_text_end(&mut self) {
        self.left.push_str(&self.right);
        self.right.clear();
    }

    /// Delete the logical line under the cursor including its newline (vim `dd`).
    pub fn delete_line(&mut self) {
        let (chars, pos) = self.chars_and_pos();
        let (start, end) = line_bounds(&chars, pos);
        // Take the trailing '\n'; the last line has none, so take its leading one instead.
        let (del_start, del_end) = if end < chars.len() {
            (start, end + 1)
        } else {
            (start.saturating_sub(1), end)
        };
        self.left = chars[..del_start].iter().collect();
        self.right = chars[del_end..].iter().collect();
    }

    /// Open a new logical line below the current one and move onto it (vim `o`).
    pub fn open_below(&mut self) {
        let (chars, pos) = self.chars_and_pos();
        let (_, end) = line_bounds(&chars, pos);
        self.set_cursor(end);
        self.insert('\n');
    }

    /// Delete the word before the cursor (readline Ctrl+W).
    /// Stops at a line boundary; at the start of a line it removes the '\n',
    /// joining onto the previous line.
    pub fn delete_word_back(&mut self) {
        let before = self.left.len();
        while self
            .left
            .chars()
            .last()
            .is_some_and(|c| c.is_whitespace() && c != '\n')
        {
            self.left.pop();
        }
        while self.left.chars().last().is_some_and(|c| !c.is_whitespace()) {
            self.left.pop();
        }
        if self.left.len() == before && self.left.ends_with('\n') {
            self.left.pop();
        }
    }

    /// Delete from the cursor back to the start of the logical line (readline Ctrl+U).
    pub fn delete_to_line_start(&mut self) {
        while self.left.chars().last().is_some_and(|c| c != '\n') {
            self.left.pop();
        }
    }

    /// Open a new logical line above the current one and move onto it (vim `O`).
    pub fn open_above(&mut self) {
        let (chars, pos) = self.chars_and_pos();
        let (start, _) = line_bounds(&chars, pos);
        self.set_cursor(start);
        self.insert('\n');
        self.move_left();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cassette_with(text: &str, cursor: usize) -> Cassette {
        let mut c = Cassette::new();
        for ch in text.chars() {
            c.insert(ch);
        }
        c.set_cursor(cursor);
        c
    }

    #[test]
    fn insert_and_text() {
        let mut c = Cassette::new();
        c.insert('h');
        c.insert('i');
        assert_eq!(c.text(), "hi");
        assert_eq!(c.cursor_pos(), 2);
    }

    #[test]
    fn backspace_removes_left() {
        let mut c = Cassette::new();
        c.insert('a');
        c.insert('b');
        c.backspace();
        assert_eq!(c.text(), "a");
    }

    #[test]
    fn delete_removes_right() {
        let mut c = Cassette::new();
        c.insert('a');
        c.insert('b');
        c.move_left();
        c.delete();
        assert_eq!(c.text(), "a");
    }

    #[test]
    fn move_left_right() {
        let mut c = Cassette::new();
        c.insert('a');
        c.insert('b');
        c.move_left();
        assert_eq!(c.cursor_pos(), 1);
        c.move_right();
        assert_eq!(c.cursor_pos(), 2);
    }

    #[test]
    fn word_count() {
        let mut c = Cassette::new();
        for ch in "hello world foo".chars() {
            c.insert(ch);
        }
        assert_eq!(c.word_count(), 3);
    }

    #[test]
    fn wrap_spans_breaks_on_newline_and_width() {
        let chars: Vec<char> = "abcd\nefghij".chars().collect();
        let spans = wrap_spans(&chars, 4);
        assert_eq!(spans, vec![(0, 4), (5, 9), (9, 11)]);
    }

    #[test]
    fn wrap_spans_empty_text() {
        assert_eq!(wrap_spans(&[], 10), vec![(0, 0)]);
    }

    #[test]
    fn wrap_breaks_at_word_boundary() {
        // "hello " stays on row 0 (trailing space), "world" moves whole to row 1.
        let chars: Vec<char> = "hello world".chars().collect();
        let spans = wrap_spans(&chars, 8);
        assert_eq!(spans, vec![(0, 6), (6, 11)]);
        assert_ne!(chars[6], ' ', "wrapped row must not start with a space");
    }

    #[test]
    fn wrap_hard_breaks_long_words() {
        let chars: Vec<char> = "abcdefghij".chars().collect();
        assert_eq!(wrap_spans(&chars, 4), vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn wrap_lets_trailing_spaces_hang() {
        // Spaces past the edge hang on the row instead of wrapping.
        let chars: Vec<char> = "ab   ".chars().collect();
        assert_eq!(wrap_spans(&chars, 3), vec![(0, 5)]);
    }

    #[test]
    fn wrap_consumes_space_run_at_break() {
        let chars: Vec<char> = "ab  cd".chars().collect();
        let spans = wrap_spans(&chars, 4);
        assert_eq!(spans, vec![(0, 4), (4, 6)]);
        assert_eq!(chars[4], 'c', "next row starts at the word");
    }

    #[test]
    fn pos_maps_to_row_after_soft_wrap() {
        let chars: Vec<char> = "abcdef".chars().collect();
        let spans = wrap_spans(&chars, 3); // [(0,3), (3,6)]
        assert_eq!(pos_to_row_col(&spans, 3), (1, 0));
        assert_eq!(pos_to_row_col(&spans, 2), (0, 2));
        assert_eq!(pos_to_row_col(&spans, 6), (1, 3));
    }

    #[test]
    fn flip_switches_sides_and_keeps_cursors() {
        let mut c = cassette_with("main thought", 4);
        assert_eq!(c.side, Side::A);
        c.flip();
        assert_eq!(c.side, Side::B);
        assert_eq!(c.text(), "");
        for ch in "scratch".chars() {
            c.insert(ch);
        }
        c.set_cursor(2);
        c.flip();
        assert_eq!(c.side, Side::A);
        assert_eq!(c.text(), "main thought");
        assert_eq!(c.cursor_pos(), 4, "side A cursor survives the round trip");
        c.flip();
        assert_eq!(c.text(), "scratch");
        assert_eq!(c.cursor_pos(), 2, "side B keeps its own cursor");
    }

    #[test]
    fn side_texts_are_stable_regardless_of_active_side() {
        let mut c = cassette_with("side a words", 0);
        c.flip();
        for ch in "side b words".chars() {
            c.insert(ch);
        }
        assert_eq!(c.side_a_text(), "side a words");
        assert_eq!(c.side_b_text(), "side b words");
        c.flip();
        assert_eq!(c.side_a_text(), "side a words");
        assert_eq!(c.side_b_text(), "side b words");
    }

    #[test]
    fn word_count_covers_both_sides() {
        let mut c = cassette_with("one two three", 0);
        c.flip();
        for ch in "four five".chars() {
            c.insert(ch);
        }
        assert_eq!(c.word_count(), 5);
    }

    #[test]
    fn cursor_line_col_and_counts() {
        let c = cassette_with("hello\nworld", 8); // on "world", after "wo"
        assert_eq!(c.cursor_line_col(), (2, 3));
        assert_eq!(c.char_count(), 11);
    }

    #[test]
    fn pos_stays_on_line_before_newline() {
        let chars: Vec<char> = "ab\ncd".chars().collect();
        let spans = wrap_spans(&chars, 10); // [(0,2), (3,5)]
        assert_eq!(pos_to_row_col(&spans, 2), (0, 2));
        assert_eq!(pos_to_row_col(&spans, 3), (1, 0));
    }

    #[test]
    fn move_up_down_across_logical_lines() {
        let mut c = cassette_with("hello\nworld", 8); // on "world", col 2
        c.move_up(20);
        assert_eq!(c.cursor_pos(), 2); // "hello", col 2
        c.move_down(20);
        assert_eq!(c.cursor_pos(), 8);
    }

    #[test]
    fn move_up_clamps_column_to_shorter_line() {
        let mut c = cassette_with("hi\nlonger line", 10); // col 7 on line 2
        c.move_up(20);
        assert_eq!(c.cursor_pos(), 2); // end of "hi"
    }

    #[test]
    fn move_down_across_soft_wrap() {
        let mut c = cassette_with("abcdefgh", 1); // width 4 → rows "abcd", "efgh"
        c.move_down(4);
        assert_eq!(c.cursor_pos(), 5);
        c.move_up(4);
        assert_eq!(c.cursor_pos(), 1);
    }

    #[test]
    fn move_up_on_first_row_is_noop() {
        let mut c = cassette_with("abc", 1);
        c.move_up(20);
        assert_eq!(c.cursor_pos(), 1);
    }

    #[test]
    fn row_start_and_end() {
        let mut c = cassette_with("hello\nworld", 8);
        c.move_row_start(20);
        assert_eq!(c.cursor_pos(), 6);
        c.move_row_end(20);
        assert_eq!(c.cursor_pos(), 11);
    }

    #[test]
    fn row_end_on_soft_wrapped_row_stays_on_row() {
        let mut c = cassette_with("abcdefgh", 1); // width 4, row 0 = "abcd"
        c.move_row_end(4);
        assert_eq!(c.cursor_pos(), 3); // before 'd', still row 0
    }

    #[test]
    fn word_motions() {
        let mut c = cassette_with("hello world foo", 0);
        c.move_word_forward();
        assert_eq!(c.cursor_pos(), 6);
        c.move_word_forward();
        assert_eq!(c.cursor_pos(), 12);
        c.move_word_back();
        assert_eq!(c.cursor_pos(), 6);
        c.move_word_back();
        assert_eq!(c.cursor_pos(), 0);
    }

    #[test]
    fn word_forward_crosses_newline() {
        let mut c = cassette_with("hello\nworld", 0);
        c.move_word_forward();
        assert_eq!(c.cursor_pos(), 6);
    }

    #[test]
    fn text_start_and_end() {
        let mut c = cassette_with("hello\nworld", 4);
        c.move_text_end();
        assert_eq!(c.cursor_pos(), 11);
        c.move_text_start();
        assert_eq!(c.cursor_pos(), 0);
    }

    #[test]
    fn delete_line_middle() {
        let mut c = cassette_with("one\ntwo\nthree", 5); // on "two"
        c.delete_line();
        assert_eq!(c.text(), "one\nthree");
        assert_eq!(c.cursor_pos(), 4); // start of "three"
    }

    #[test]
    fn delete_line_last_takes_leading_newline() {
        let mut c = cassette_with("one\ntwo", 5);
        c.delete_line();
        assert_eq!(c.text(), "one");
    }

    #[test]
    fn delete_line_only_line() {
        let mut c = cassette_with("only", 2);
        c.delete_line();
        assert_eq!(c.text(), "");
        assert_eq!(c.cursor_pos(), 0);
    }

    #[test]
    fn wrap_counts_wide_chars_as_two_cells() {
        // Four CJK chars are 8 cells: at width 4 they wrap two per row.
        let chars: Vec<char> = "日本語字".chars().collect();
        assert_eq!(wrap_spans(&chars, 4), vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn wrap_word_boundary_uses_cell_width() {
        // "日本 " is 5 cells; "kana" would end at cell 9 > 8, so it wraps whole.
        let chars: Vec<char> = "日本 kana".chars().collect();
        assert_eq!(wrap_spans(&chars, 8), vec![(0, 3), (3, 7)]);
    }

    #[test]
    fn wrap_ignores_zero_width_chars() {
        // Combining acute accents take no cells: "e\u{301}" repeated stays one row.
        let chars: Vec<char> = "e\u{301}e\u{301}e\u{301}".chars().collect();
        assert_eq!(wrap_spans(&chars, 3), vec![(0, 6)]);
    }

    #[test]
    fn undo_restores_text_and_cursor() {
        let mut c = cassette_with("one\ntwo\nthree", 5); // on "two"
        c.snapshot();
        c.delete_line();
        assert_eq!(c.text(), "one\nthree");
        c.undo();
        assert_eq!(c.text(), "one\ntwo\nthree");
        assert_eq!(c.cursor_pos(), 5, "cursor comes back with the text");
        c.undo(); // empty history: no-op
        assert_eq!(c.text(), "one\ntwo\nthree");
    }

    #[test]
    fn undo_history_is_per_side() {
        let mut c = cassette_with("side a", 6);
        c.snapshot();
        c.insert('!');
        c.flip();
        c.undo(); // side B has no history: no-op
        assert_eq!(c.text(), "");
        c.flip();
        c.undo();
        assert_eq!(c.text(), "side a", "side A history survives the flip");
    }

    #[test]
    fn delete_word_back_takes_word_and_trailing_space() {
        let mut c = cassette_with("hello world ", 12);
        c.delete_word_back();
        assert_eq!(c.text(), "hello ");
        c.delete_word_back();
        assert_eq!(c.text(), "");
    }

    #[test]
    fn delete_word_back_stops_at_line_start_then_joins() {
        let mut c = cassette_with("one\ntwo", 7);
        c.delete_word_back();
        assert_eq!(c.text(), "one\n", "deletes the word, not past the newline");
        c.delete_word_back();
        assert_eq!(c.text(), "one", "at line start it removes the newline");
    }

    #[test]
    fn delete_to_line_start_spares_previous_lines() {
        let mut c = cassette_with("one\ntwo three", 13);
        c.delete_to_line_start();
        assert_eq!(c.text(), "one\n");
        c.delete_to_line_start(); // already at line start: no-op
        assert_eq!(c.text(), "one\n");
    }

    #[test]
    fn open_below_and_above() {
        let mut c = cassette_with("one\ntwo", 1); // on "one"
        c.open_below();
        assert_eq!(c.text(), "one\n\ntwo");
        assert_eq!(c.cursor_pos(), 4); // on the new empty line

        let mut c = cassette_with("one\ntwo", 5); // on "two"
        c.open_above();
        assert_eq!(c.text(), "one\n\ntwo");
        assert_eq!(c.cursor_pos(), 4);
    }
}
