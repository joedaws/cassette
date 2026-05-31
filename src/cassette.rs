/// Cursor-zipper text buffer: left holds text before the cursor, right holds text after.
#[derive(Clone, Debug, Default)]
pub struct Cassette {
    pub left: String,
    pub right: String,
}

impl Cassette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        format!("{}{}", self.left, self.right)
    }

    /// Number of characters before the cursor.
    pub fn cursor_pos(&self) -> usize {
        self.left.chars().count()
    }

    pub fn word_count(&self) -> usize {
        let t = self.text();
        if t.trim().is_empty() {
            0
        } else {
            t.split_whitespace().count()
        }
    }

    pub fn insert(&mut self, c: char) {
        self.left.push(c);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
