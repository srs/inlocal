//! A minimal text input for the TUI: a string and a cursor.
//!
//! The cursor is a char index (not a byte index), so editing works with
//! æ, ø, å and other multi-byte characters.

#[derive(Default, Clone)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Text before and after the cursor, for drawing it.
    pub fn split_at_cursor(&self) -> (&str, &str) {
        self.text.split_at(self.byte_index())
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        self.cursor -= 1;
        let at = self.byte_index();
        self.text.remove(at);
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index();
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self) -> usize {
        self.text
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_multibyte_text() {
        let mut input = TextInput::new("på");

        input.left();
        input.insert('æ');
        assert_eq!(input.text(), "pæå");

        input.backspace();
        input.backspace();
        assert_eq!(input.text(), "å");

        input.end();
        input.insert('!');
        assert_eq!(input.split_at_cursor(), ("å!", ""));
    }
}
