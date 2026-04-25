//! ZLE text objects
//!
//! Direct port from zsh/Src/Zle/zle_thingy.c text object support
//!
//! Text objects for vi mode operations (e.g., "iw" for inner word, "a)" for a-parenthesis)

use super::main::Zle;

/// Text object type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectType {
    /// Inner (inside delimiters)
    Inner,
    /// A (including delimiters)
    A,
}

/// Text object kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectKind {
    Word,
    BigWord,
    Sentence,
    Paragraph,
    Parenthesis,
    Bracket,
    Brace,
    Angle,
    SingleQuote,
    DoubleQuote,
    BackQuote,
}

/// A text object selection (start and end positions)
#[derive(Debug, Clone, Copy)]
pub struct TextObject {
    pub start: usize,
    pub end: usize,
}

impl Zle {
    /// Select a text object
    pub fn select_text_object(
        &self,
        obj_type: TextObjectType,
        kind: TextObjectKind,
    ) -> Option<TextObject> {
        match kind {
            TextObjectKind::Word => self.select_word_object(obj_type, false),
            TextObjectKind::BigWord => self.select_word_object(obj_type, true),
            TextObjectKind::Sentence => self.select_sentence_object(obj_type),
            TextObjectKind::Paragraph => self.select_paragraph_object(obj_type),
            TextObjectKind::Parenthesis => self.select_pair_object(obj_type, '(', ')'),
            TextObjectKind::Bracket => self.select_pair_object(obj_type, '[', ']'),
            TextObjectKind::Brace => self.select_pair_object(obj_type, '{', '}'),
            TextObjectKind::Angle => self.select_pair_object(obj_type, '<', '>'),
            TextObjectKind::SingleQuote => self.select_quote_object(obj_type, '\''),
            TextObjectKind::DoubleQuote => self.select_quote_object(obj_type, '"'),
            TextObjectKind::BackQuote => self.select_quote_object(obj_type, '`'),
        }
    }

    fn select_word_object(&self, obj_type: TextObjectType, big_word: bool) -> Option<TextObject> {
        if self.zlell == 0 {
            return None;
        }

        let is_word_char = if big_word {
            |c: char| !c.is_whitespace()
        } else {
            |c: char| c.is_alphanumeric() || c == '_'
        };

        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Determine if we're on a word or whitespace
        let on_word = if self.zlecs < self.zlell {
            is_word_char(self.zleline[self.zlecs])
        } else {
            false
        };

        if on_word {
            // Find word boundaries
            while start > 0 && is_word_char(self.zleline[start - 1]) {
                start -= 1;
            }
            while end < self.zlell && is_word_char(self.zleline[end]) {
                end += 1;
            }

            // For "a word", include trailing whitespace
            if obj_type == TextObjectType::A {
                while end < self.zlell && self.zleline[end].is_whitespace() {
                    end += 1;
                }
            }
        } else {
            // On whitespace - select whitespace
            while start > 0 && self.zleline[start - 1].is_whitespace() {
                start -= 1;
            }
            while end < self.zlell && self.zleline[end].is_whitespace() {
                end += 1;
            }

            // For "a whitespace", include adjacent word
            if obj_type == TextObjectType::A && end < self.zlell {
                while end < self.zlell && is_word_char(self.zleline[end]) {
                    end += 1;
                }
            }
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_sentence_object(&self, obj_type: TextObjectType) -> Option<TextObject> {
        // Simplified sentence detection
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find sentence start (after previous . ! ?)
        while start > 0 {
            let c = self.zleline[start - 1];
            if c == '.' || c == '!' || c == '?' {
                break;
            }
            start -= 1;
        }

        // Skip whitespace at start (for inner)
        if obj_type == TextObjectType::Inner {
            while start < self.zlell && self.zleline[start].is_whitespace() {
                start += 1;
            }
        }

        // Find sentence end
        while end < self.zlell {
            let c = self.zleline[end];
            end += 1;
            if c == '.' || c == '!' || c == '?' {
                break;
            }
        }

        // Include trailing whitespace for "a sentence"
        if obj_type == TextObjectType::A {
            while end < self.zlell && self.zleline[end].is_whitespace() {
                end += 1;
            }
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_paragraph_object(&self, obj_type: TextObjectType) -> Option<TextObject> {
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find paragraph start (blank line)
        while start > 0 {
            if start >= 2 && self.zleline[start - 1] == '\n' && self.zleline[start - 2] == '\n' {
                break;
            }
            start -= 1;
        }

        // Find paragraph end
        while end < self.zlell {
            if end + 1 < self.zlell && self.zleline[end] == '\n' && self.zleline[end + 1] == '\n' {
                if obj_type == TextObjectType::A {
                    end += 2;
                }
                break;
            }
            end += 1;
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_pair_object(
        &self,
        obj_type: TextObjectType,
        open: char,
        close: char,
    ) -> Option<TextObject> {
        let mut depth = 0;
        let mut start = None;
        let mut end = None;

        // Find opening bracket
        for i in (0..=self.zlecs).rev() {
            let c = self.zleline[i];
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    start = Some(i);
                    break;
                }
                depth -= 1;
            }
        }

        // Find closing bracket
        depth = 0;
        for i in self.zlecs..self.zlell {
            let c = self.zleline[i];
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
                depth -= 1;
            }
        }

        match (start, end) {
            (Some(s), Some(e)) => {
                if obj_type == TextObjectType::Inner {
                    Some(TextObject {
                        start: s + 1,
                        end: e - 1,
                    })
                } else {
                    Some(TextObject { start: s, end: e })
                }
            }
            _ => None,
        }
    }

    fn select_quote_object(&self, obj_type: TextObjectType, quote: char) -> Option<TextObject> {
        let mut start = None;
        let mut end = None;

        // Find opening quote (searching backward)
        for i in (0..=self.zlecs).rev() {
            if self.zleline[i] == quote {
                start = Some(i);
                break;
            }
        }

        // Find closing quote (searching forward)
        if let Some(s) = start {
            for i in (s + 1)..self.zlell {
                if self.zleline[i] == quote {
                    end = Some(i + 1);
                    break;
                }
            }
        }

        match (start, end) {
            (Some(s), Some(e)) => {
                if obj_type == TextObjectType::Inner {
                    Some(TextObject {
                        start: s + 1,
                        end: e - 1,
                    })
                } else {
                    Some(TextObject { start: s, end: e })
                }
            }
            _ => None,
        }
    }
}
