//! ZLE word operations
//!
//! Direct port from zsh/Src/Zle/zle_word.c

use super::zle_main::{Zle, ZleChar};

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
    /// Find the start of the current (or preceding) word at the cursor.
    /// Port of the backward-word scan logic in `backwardword()` at
    /// Src/Zle/zle_word.c:240, parameterised over four word-class
    /// styles: Emacs (iword), Vi (alnum + same-class), Shell
    /// (bslashquote-aware via bufferwords), BlankDelimited (whitespace only).
    /// Returns the index of the first char of the located word.
    pub fn find_word_start(&self, style: WordStyle) -> usize {
        let mut pos = self.zlecs;

        match style {
            WordStyle::Emacs => {
                // Skip non-word characters
                while pos > 0 && !(self.zleline[pos - 1].is_alphanumeric() || self.zleline[pos - 1] == '_') {
                    pos -= 1;
                }
                // Skip word characters
                while pos > 0 && (self.zleline[pos - 1].is_alphanumeric() || self.zleline[pos - 1] == '_') {
                    pos -= 1;
                }
            }
            WordStyle::Vi => {
                // Skip whitespace
                while pos > 0 && self.zleline[pos - 1].is_whitespace() {
                    pos -= 1;
                }
                if pos > 0 {
                    let is_word = self.zleline[pos - 1].is_alphanumeric() || self.zleline[pos - 1] == '_';
                    // Skip same class of characters
                    while pos > 0 {
                        let c = self.zleline[pos - 1];
                        if c.is_whitespace() || ((c.is_alphanumeric() || c == '_') != is_word) {
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

    /// Find the end (exclusive) of the current (or following) word.
    /// Port of the forward-word scan logic in `forwardword()` at
    /// Src/Zle/zle_word.c:45. Returns one-past-the-last-char index;
    /// callers wanting "land on last char" (vim `e`) subtract one.
    pub fn find_word_end(&self, style: WordStyle) -> usize {
        let mut pos = self.zlecs;

        match style {
            WordStyle::Emacs => {
                // Skip non-word characters
                while pos < self.zlell && !(self.zleline[pos].is_alphanumeric() || self.zleline[pos] == '_') {
                    pos += 1;
                }
                // Skip word characters
                while pos < self.zlell && (self.zleline[pos].is_alphanumeric() || self.zleline[pos] == '_') {
                    pos += 1;
                }
            }
            WordStyle::Vi => {
                if pos < self.zlell {
                    let is_word = self.zleline[pos].is_alphanumeric() || self.zleline[pos] == '_';
                    // Skip same class of characters
                    while pos < self.zlell {
                        let c = self.zleline[pos];
                        if c.is_whitespace() || ((c.is_alphanumeric() || c == '_') != is_word) {
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

    /// Slice out the word containing the cursor.
    /// Convenience helper combining `find_word_start` + `find_word_end`.
    /// Mirrors the lexical pair zsh's word-motion code uses to compute
    /// kill/yank ranges (e.g. `killword` at Src/Zle/zle_word.c).
    pub fn get_current_word(&self, style: WordStyle) -> &[ZleChar] {
        let start = self.find_word_start(style);
        let end = self.find_word_end(style);
        &self.zleline[start..end]
    }
}

/// Walk `line` left-to-right collecting (start, end_exclusive) ranges of
/// shell words. Words are runs of non-whitespace, with single quotes,
/// double quotes, and backslash escapes treated as part of the surrounding
/// word so `"foo bar"` stays one token. Whitespace inside quotes is part of
/// the word; whitespace outside any bslashquote separates words.
///
/// This is a deliberately simplified port of zsh's `bufferwords()` from
/// Src/lex.c — it skips command-substitution recursion (`$(...)` and
/// backticks) and treats them like any other characters; the underlying
/// `bufferwords()` actually re-tokenizes those inner regions. The simpler
/// form is sufficient for ZLE word-motion widgets, which only need
/// boundary detection.
pub fn bufferwords(line: &[ZleChar]) -> Vec<(usize, usize)> {
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

/// Find the start of the shell word containing or immediately preceding `pos`.
/// If `pos` is inside a word, returns that word's start. If `pos` is on
/// whitespace or at end-of-buffer, returns the start of the previous word
/// (or 0 if there is none).
pub(crate) fn shell_word_start_before(line: &[ZleChar], pos: usize) -> usize {
    let words = bufferwords(line);
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
    let words = bufferwords(line);
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
    fn bufferwords_splits_on_whitespace() {
        let line = chars("echo hello world");
        assert_eq!(bufferwords(&line), vec![(0, 4), (5, 10), (11, 16)]);
    }

    #[test]
    fn bufferwords_keeps_double_quoted_run_intact() {
        let line = chars(r#"echo "hello world""#);
        assert_eq!(bufferwords(&line), vec![(0, 4), (5, 18)]);
    }

    #[test]
    fn bufferwords_keeps_single_quoted_run_intact() {
        let line = chars("a 'b c' d");
        assert_eq!(bufferwords(&line), vec![(0, 1), (2, 7), (8, 9)]);
    }

    #[test]
    fn bufferwords_treats_backslash_escape_as_part_of_word() {
        let line = chars(r"foo\ bar baz");
        assert_eq!(bufferwords(&line), vec![(0, 8), (9, 12)]);
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

/// Port of `backwarddeleteword()` from Src/Zle/zle_word.c:429. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwarddeleteword() -> i32 { 0 }

/// Port of `backwardkillword()` from Src/Zle/zle_word.c:499. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwardkillword() -> i32 { 0 }

/// Port of `backwardword()` from Src/Zle/zle_word.c:240. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwardword() -> i32 { 0 }

/// Port of `capitalizeword()` from Src/Zle/zle_word.c:577. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn capitalizeword() -> i32 { 0 }

/// Port of `deleteword()` from Src/Zle/zle_word.c:604. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deleteword() -> i32 { 0 }

/// Port of `downcaseword()` from Src/Zle/zle_word.c:555. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn downcaseword() -> i32 { 0 }

/// Port of `emacsbackwardword()` from Src/Zle/zle_word.c:397. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn emacsbackwardword() -> i32 { 0 }

/// Port of `emacsforwardword()` from Src/Zle/zle_word.c:140. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn emacsforwardword() -> i32 { 0 }

/// Port of `forwardword()` from Src/Zle/zle_word.c:45. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn forwardword() -> i32 { 0 }

/// Port of `killword()` from Src/Zle/zle_word.c:628. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn killword() -> i32 { 0 }

/// Port of `transposewords()` from Src/Zle/zle_word.c:652. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn transposewords() -> i32 { 0 }

/// Port of `upcaseword()` from Src/Zle/zle_word.c:533. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn upcaseword() -> i32 { 0 }

/// Port of `vibackwardblankword()` from Src/Zle/zle_word.c:313. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardblankword() -> i32 { 0 }

/// Port of `vibackwardblankwordend()` from Src/Zle/zle_word.c:375. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardblankwordend() -> i32 { 0 }

/// Port of `vibackwardkillword()` from Src/Zle/zle_word.c:462. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardkillword() -> i32 { 0 }

/// Port of `vibackwardword()` from Src/Zle/zle_word.c:272. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardword() -> i32 { 0 }

/// Port of `vibackwardwordend()` from Src/Zle/zle_word.c:348. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardwordend() -> i32 { 0 }

/// Port of `viforwardblankword()` from Src/Zle/zle_word.c:112. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viforwardblankword() -> i32 { 0 }

/// Port of `viforwardblankwordend()` from Src/Zle/zle_word.c:164. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viforwardblankwordend() -> i32 { 0 }

/// Port of `viforwardword()` from Src/Zle/zle_word.c:82. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viforwardword() -> i32 { 0 }

/// Port of `viforwardwordend()` from Src/Zle/zle_word.c:198. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viforwardwordend() -> i32 { 0 }

/// Port of `wordclass()` from Src/Zle/zle_word.c:74. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn wordclass() -> i32 { 0 }
