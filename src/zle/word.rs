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
                // Walk the buffer left-to-right as a coarse shell lexer to
                // collect (word_start, word_end_exclusive) pairs that respect
                // single quotes, double quotes, and backslash escapes — then
                // jump backwards to the start of the word containing `pos`,
                // or to the previous word if `pos` is on whitespace.
                // Matches zsh's `bufferwords()` quoting semantics in
                // Src/lex.c at a high level (no command-substitution recursion).
                pos = shell_word_start_before(&self.zleline[..self.zlell], pos);
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
                pos = shell_word_end_after(&self.zleline[..self.zlell], pos);
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

/// Walk `line` left-to-right collecting (start, end_exclusive) ranges of
/// shell words. Words are runs of non-whitespace, with single quotes,
/// double quotes, and backslash escapes treated as part of the surrounding
/// word so `"foo bar"` stays one token. Whitespace inside quotes is part of
/// the word; whitespace outside any quote separates words.
///
/// This is a deliberately simplified port of zsh's `bufferwords()` from
/// Src/lex.c — it skips command-substitution recursion (`$(...)` and
/// backticks) and treats them like any other characters; the underlying
/// `bufferwords()` actually re-tokenizes those inner regions. The simpler
/// form is sufficient for ZLE word-motion widgets, which only need
/// boundary detection.
fn shell_words(line: &[ZleChar]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = line.len();
    while i < n {
        // Skip leading whitespace.
        while i < n && line[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        let mut in_single = false;
        let mut in_double = false;
        while i < n {
            let c = line[i];
            if in_single {
                if c == '\'' {
                    in_single = false;
                }
                i += 1;
                continue;
            }
            if in_double {
                if c == '\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if c == '"' {
                    in_double = false;
                }
                i += 1;
                continue;
            }
            if c == '\\' && i + 1 < n {
                // Outside quotes, backslash escapes one char (incl. whitespace).
                i += 2;
                continue;
            }
            if c == '\'' {
                in_single = true;
                i += 1;
                continue;
            }
            if c == '"' {
                in_double = true;
                i += 1;
                continue;
            }
            if c.is_whitespace() {
                break;
            }
            i += 1;
        }
        out.push((start, i));
    }
    out
}

/// Expose the shell-word splitter for callers that need the full word
/// list (used by copy-prev-shell-word). Mirrors zsh's `bufferwords()` at
/// Src/lex.c — coarse port that respects single/double quotes + backslash
/// escapes; see `shell_words` for the detail.
pub fn shell_words_for_test(line: &[ZleChar]) -> Vec<(usize, usize)> {
    shell_words(line)
}

/// Find the start of the shell word containing or immediately preceding `pos`.
/// If `pos` is inside a word, returns that word's start. If `pos` is on
/// whitespace or at end-of-buffer, returns the start of the previous word
/// (or 0 if there is none).
pub(crate) fn shell_word_start_before(line: &[ZleChar], pos: usize) -> usize {
    let words = shell_words(line);
    // Search for the word containing pos.
    for (s, e) in words.iter().rev() {
        if *s <= pos && pos <= *e {
            // If we're sitting at the very start of a word, jump to the
            // previous word — matches the "go-back-one-word" semantics that
            // backward-word users expect when called from the first column
            // of a token.
            if pos == *s {
                continue;
            }
            return *s;
        }
        if *e < pos {
            return *s;
        }
    }
    0
}

/// Find the end (exclusive) of the shell word at or after `pos`.
/// If `pos` is inside a word, returns that word's end. If `pos` is on
/// whitespace, returns the end of the next word (or `line.len()` if none).
pub(crate) fn shell_word_end_after(line: &[ZleChar], pos: usize) -> usize {
    let words = shell_words(line);
    for (s, e) in words {
        if pos >= s && pos < e {
            return e;
        }
        if pos < s {
            return e;
        }
    }
    line.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn shell_words_splits_on_whitespace() {
        let line = chars("echo hello world");
        assert_eq!(shell_words(&line), vec![(0, 4), (5, 10), (11, 16)]);
    }

    #[test]
    fn shell_words_keeps_double_quoted_run_intact() {
        let line = chars(r#"echo "hello world""#);
        assert_eq!(shell_words(&line), vec![(0, 4), (5, 18)]);
    }

    #[test]
    fn shell_words_keeps_single_quoted_run_intact() {
        let line = chars("a 'b c' d");
        assert_eq!(shell_words(&line), vec![(0, 1), (2, 7), (8, 9)]);
    }

    #[test]
    fn shell_words_treats_backslash_escape_as_part_of_word() {
        let line = chars(r"foo\ bar baz");
        assert_eq!(shell_words(&line), vec![(0, 8), (9, 12)]);
    }

    #[test]
    fn shell_word_end_after_advances_into_next_word() {
        let line = chars("aa bb cc");
        // pos 2 is the space between "aa" and "bb" — end_after lands at 5.
        assert_eq!(shell_word_end_after(&line, 2), 5);
        // pos 0 is inside "aa" — end_after lands at 2.
        assert_eq!(shell_word_end_after(&line, 0), 2);
    }

    #[test]
    fn shell_word_start_before_returns_word_start() {
        let line = chars("aa bb cc");
        // pos 4 is inside "bb" — start_before is 3.
        assert_eq!(shell_word_start_before(&line, 4), 3);
        // pos 3 is at the start of "bb" — start_before goes back to "aa" → 0.
        assert_eq!(shell_word_start_before(&line, 3), 0);
        // pos 5 is end of "bb" — start_before is 3.
        assert_eq!(shell_word_start_before(&line, 5), 3);
    }
}
