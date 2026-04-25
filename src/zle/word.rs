//! ZLE word operations
//!
//! Direct port from zsh/Src/Zle/zle_word.c

use super::main::{Zle, ZleChar};

/// Word style for movement
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStyle {
    /// Emacs-style words (alphanumeric + underscore)
    Emacs,
    /// Vi-style words (separated by whitespace and punctuation)
    Vi,
    /// Shell words (quoted strings, etc.)
    Shell,
    /// Whitespace-separated "WORDS"
    BlankDelimited,
}

impl Zle {
    /// Find the start of the current/previous word
    pub fn find_word_start(&self, style: WordStyle) -> usize {
        let mut pos = self.zlecs;

        match style {
            WordStyle::Emacs => {
                // Skip non-word characters
                while pos > 0 && !is_emacs_word_char(self.zleline[pos - 1]) {
                    pos -= 1;
                }
                // Skip word characters
                while pos > 0 && is_emacs_word_char(self.zleline[pos - 1]) {
                    pos -= 1;
                }
            }
            WordStyle::Vi => {
                // Skip whitespace
                while pos > 0 && self.zleline[pos - 1].is_whitespace() {
                    pos -= 1;
                }
                if pos > 0 {
                    let is_word = is_vi_word_char(self.zleline[pos - 1]);
                    // Skip same class of characters
                    while pos > 0 {
                        let c = self.zleline[pos - 1];
                        if c.is_whitespace() || (is_vi_word_char(c) != is_word) {
                            break;
                        }
                        pos -= 1;
                    }
                }
            }
            WordStyle::Shell => {
                // TODO: implement shell word boundaries
                while pos > 0 && !self.zleline[pos - 1].is_whitespace() {
                    pos -= 1;
                }
            }
            WordStyle::BlankDelimited => {
                // Skip whitespace
                while pos > 0 && self.zleline[pos - 1].is_whitespace() {
                    pos -= 1;
                }
                // Skip non-whitespace
                while pos > 0 && !self.zleline[pos - 1].is_whitespace() {
                    pos -= 1;
                }
            }
        }

        pos
    }

    /// Find the end of the current/next word
    pub fn find_word_end(&self, style: WordStyle) -> usize {
        let mut pos = self.zlecs;

        match style {
            WordStyle::Emacs => {
                // Skip non-word characters
                while pos < self.zlell && !is_emacs_word_char(self.zleline[pos]) {
                    pos += 1;
                }
                // Skip word characters
                while pos < self.zlell && is_emacs_word_char(self.zleline[pos]) {
                    pos += 1;
                }
            }
            WordStyle::Vi => {
                if pos < self.zlell {
                    let is_word = is_vi_word_char(self.zleline[pos]);
                    // Skip same class of characters
                    while pos < self.zlell {
                        let c = self.zleline[pos];
                        if c.is_whitespace() || (is_vi_word_char(c) != is_word) {
                            break;
                        }
                        pos += 1;
                    }
                    // Skip whitespace
                    while pos < self.zlell && self.zleline[pos].is_whitespace() {
                        pos += 1;
                    }
                }
            }
            WordStyle::Shell => {
                // TODO: implement shell word boundaries
                while pos < self.zlell && !self.zleline[pos].is_whitespace() {
                    pos += 1;
                }
                while pos < self.zlell && self.zleline[pos].is_whitespace() {
                    pos += 1;
                }
            }
            WordStyle::BlankDelimited => {
                // Skip non-whitespace
                while pos < self.zlell && !self.zleline[pos].is_whitespace() {
                    pos += 1;
                }
                // Skip whitespace
                while pos < self.zlell && self.zleline[pos].is_whitespace() {
                    pos += 1;
                }
            }
        }

        pos
    }

    /// Get the current word
    pub fn get_current_word(&self, style: WordStyle) -> &[ZleChar] {
        let start = self.find_word_start(style);
        let end = self.find_word_end(style);
        &self.zleline[start..end]
    }
}

/// Check if character is an emacs word character
fn is_emacs_word_char(c: ZleChar) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Check if character is a vi word character (alphanumeric)
fn is_vi_word_char(c: ZleChar) -> bool {
    c.is_alphanumeric() || c == '_'
}
