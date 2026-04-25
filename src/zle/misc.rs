//! ZLE miscellaneous operations
//!
//! Direct port from zsh/Src/Zle/zle_misc.c
//!
//! Implements misc editing widgets:
//! - self-insert, self-insert-unmeta
//! - accept-line, accept-and-hold
//! - quoted-insert, bracketed-paste
//! - delete-char, backward-delete-char
//! - kill-line, backward-kill-line, kill-buffer, kill-whole-line
//! - copy-region-as-kill, kill-region
//! - yank, yank-pop
//! - transpose-chars, quote-line, quote-region
//! - what-cursor-position, universal-argument, digit-argument
//! - undefined-key, send-break
//! - vi-put-after, vi-put-before, overwrite-mode

use super::main::Zle;

/// Clipboard/paste buffer for yank operations
#[derive(Debug, Default)]
pub struct PasteBuffer {
    pub content: Vec<char>,
}

impl Zle {
    /// Self insert - insert the typed character
    /// Port of selfinsert() from zle_misc.c
    pub fn self_insert(&mut self, c: char) {
        self.zleline.insert(self.zlecs, c);
        self.zlecs += 1;
        self.zlell += 1;
        self.resetneeded = true;
    }

    /// Self insert unmeta - insert character with meta bit stripped
    /// Port of selfinsertunmeta() from zle_misc.c
    pub fn self_insert_unmeta(&mut self, c: char) {
        let unmetaed = if (c as u32) >= 0x80 && (c as u32) < 0x100 {
            char::from_u32((c as u32) & 0x7f).unwrap_or(c)
        } else {
            c
        };
        self.self_insert(unmetaed);
    }

    /// Accept line - return the current line for execution
    /// Port of acceptline() from zle_misc.c
    pub fn accept_line(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Accept and hold - accept line but keep it in the buffer
    /// Port of acceptandhold() from zle_misc.c
    pub fn accept_and_hold(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Quoted insert - insert next char literally
    /// Port of quotedinsert() from zle_misc.c
    pub fn quoted_insert(&mut self, c: char) {
        self.zleline.insert(self.zlecs, c);
        self.zlecs += 1;
        self.zlell += 1;
        self.resetneeded = true;
    }

    /// Bracketed paste - handle paste mode
    /// Port of bracketedpaste() from zle_misc.c
    pub fn bracketed_paste(&mut self, text: &str) {
        for c in text.chars() {
            if c != '\x1b' {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
                self.zlell += 1;
            }
        }
        self.resetneeded = true;
    }

    /// Delete char under cursor
    /// Port of deletechar() from zle_misc.c
    pub fn delete_char(&mut self) {
        if self.zlecs < self.zlell {
            self.zleline.remove(self.zlecs);
            self.zlell -= 1;
            self.resetneeded = true;
        }
    }

    /// Delete char before cursor
    /// Port of backwarddeletechar() from zle_misc.c
    pub fn backward_delete_char(&mut self) {
        if self.zlecs > 0 {
            self.zlecs -= 1;
            self.zleline.remove(self.zlecs);
            self.zlell -= 1;
            self.resetneeded = true;
        }
    }

    /// Kill from cursor to end of line
    /// Port of killline() from zle_misc.c
    pub fn kill_line(&mut self) {
        if self.zlecs < self.zlell {
            let text: Vec<char> = self.zleline.drain(self.zlecs..).collect();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
            self.zlell = self.zlecs;
            self.resetneeded = true;
        }
    }

    /// Kill from beginning of line to cursor
    /// Port of backwardkillline() from zle_misc.c
    pub fn backward_kill_line(&mut self) {
        if self.zlecs > 0 {
            let text: Vec<char> = self.zleline.drain(..self.zlecs).collect();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
            self.zlell -= self.zlecs;
            self.zlecs = 0;
            self.resetneeded = true;
        }
    }

    /// Kill entire buffer
    /// Port of killbuffer() from zle_misc.c
    pub fn kill_buffer(&mut self) {
        if !self.zleline.is_empty() {
            let text: Vec<char> = self.zleline.drain(..).collect();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
            self.zlell = 0;
            self.zlecs = 0;
            self.mark = 0;
            self.resetneeded = true;
        }
    }

    /// Kill whole line (including newlines in multi-line mode)
    /// Port of killwholeline() from zle_misc.c
    pub fn kill_whole_line(&mut self) {
        self.kill_buffer();
    }

    /// Exchange point and mark
    pub fn exchange_point_and_mark(&mut self) {
        std::mem::swap(&mut self.zlecs, &mut self.mark);
        self.resetneeded = true;
    }

    /// Set mark at current position
    pub fn set_mark_here(&mut self) {
        self.mark = self.zlecs;
    }

    /// Copy region as kill
    /// Port of copyregionaskill() from zle_misc.c
    pub fn copy_region_as_kill(&mut self) {
        let (start, end) = if self.zlecs < self.mark {
            (self.zlecs, self.mark)
        } else {
            (self.mark, self.zlecs)
        };

        let text: Vec<char> = self.zleline[start..end].to_vec();
        self.killring.push_front(text);
        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }
    }

    /// Kill region (between point and mark)
    /// Port of killregion() from zle_misc.c
    pub fn kill_region(&mut self) {
        let (start, end) = if self.zlecs < self.mark {
            (self.zlecs, self.mark)
        } else {
            (self.mark, self.zlecs)
        };

        let text: Vec<char> = self.zleline.drain(start..end).collect();
        self.killring.push_front(text);
        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }

        self.zlell -= end - start;
        self.zlecs = start;
        self.mark = start;
        self.resetneeded = true;
    }

    /// Yank - insert from kill ring
    /// Port of yank() from zle_misc.c
    pub fn yank(&mut self) {
        if let Some(text) = self.killring.front() {
            self.mark = self.zlecs;
            for &c in text {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
            }
            self.zlell = self.zleline.len();
            self.yanklast = true;
            self.resetneeded = true;
        }
    }

    /// Yank pop - cycle through kill ring
    /// Port of yankpop() from zle_misc.c
    pub fn yank_pop(&mut self) {
        if !self.yanklast || self.killring.is_empty() {
            return;
        }

        // Remove previously yanked text
        let prev_len = self.killring.front().map(|v| v.len()).unwrap_or(0);
        let start = self.mark;
        for _ in 0..prev_len {
            if start < self.zleline.len() {
                self.zleline.remove(start);
            }
        }
        self.zlecs = start;
        self.zlell = self.zleline.len();

        // Rotate kill ring
        if let Some(front) = self.killring.pop_front() {
            self.killring.push_back(front);
        }

        // Insert new text
        if let Some(text) = self.killring.front() {
            for &c in text {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
            }
            self.zlell = self.zleline.len();
        }

        self.resetneeded = true;
    }

    /// Transpose chars
    /// Port of transposechars() from zle_misc.c
    pub fn transpose_chars(&mut self) {
        if self.zlecs == 0 || self.zlell < 2 {
            return;
        }

        let pos = if self.zlecs == self.zlell {
            self.zlecs - 1
        } else {
            self.zlecs
        };

        if pos > 0 {
            self.zleline.swap(pos - 1, pos);
            self.zlecs = pos + 1;
            self.resetneeded = true;
        }
    }

    /// Capitalize word
    pub fn capitalize_word(&mut self) {
        while self.zlecs < self.zlell && !self.zleline[self.zlecs].is_alphanumeric() {
            self.zlecs += 1;
        }

        if self.zlecs < self.zlell && self.zleline[self.zlecs].is_alphabetic() {
            self.zleline[self.zlecs] = self.zleline[self.zlecs]
                .to_uppercase()
                .next()
                .unwrap_or(self.zleline[self.zlecs]);
            self.zlecs += 1;
        }

        while self.zlecs < self.zlell && self.zleline[self.zlecs].is_alphanumeric() {
            self.zleline[self.zlecs] = self.zleline[self.zlecs]
                .to_lowercase()
                .next()
                .unwrap_or(self.zleline[self.zlecs]);
            self.zlecs += 1;
        }

        self.resetneeded = true;
    }

    /// Downcase word
    pub fn downcase_word(&mut self) {
        while self.zlecs < self.zlell && !self.zleline[self.zlecs].is_alphanumeric() {
            self.zlecs += 1;
        }

        while self.zlecs < self.zlell && self.zleline[self.zlecs].is_alphanumeric() {
            self.zleline[self.zlecs] = self.zleline[self.zlecs]
                .to_lowercase()
                .next()
                .unwrap_or(self.zleline[self.zlecs]);
            self.zlecs += 1;
        }

        self.resetneeded = true;
    }

    /// Upcase word
    pub fn upcase_word(&mut self) {
        while self.zlecs < self.zlell && !self.zleline[self.zlecs].is_alphanumeric() {
            self.zlecs += 1;
        }

        while self.zlecs < self.zlell && self.zleline[self.zlecs].is_alphanumeric() {
            self.zleline[self.zlecs] = self.zleline[self.zlecs]
                .to_uppercase()
                .next()
                .unwrap_or(self.zleline[self.zlecs]);
            self.zlecs += 1;
        }

        self.resetneeded = true;
    }

    /// Transpose words
    /// Port of transpose words logic
    pub fn transpose_words(&mut self) {
        if self.zlell < 3 {
            return;
        }

        // Find boundaries of two words
        let mut end2 = self.zlecs;
        while end2 < self.zlell && self.zleline[end2].is_alphanumeric() {
            end2 += 1;
        }
        while end2 < self.zlell && !self.zleline[end2].is_alphanumeric() {
            end2 += 1;
        }
        while end2 < self.zlell && self.zleline[end2].is_alphanumeric() {
            end2 += 1;
        }

        let mut start2 = end2;
        while start2 > 0 && self.zleline[start2 - 1].is_alphanumeric() {
            start2 -= 1;
        }

        let mut end1 = start2;
        while end1 > 0 && !self.zleline[end1 - 1].is_alphanumeric() {
            end1 -= 1;
        }

        let mut start1 = end1;
        while start1 > 0 && self.zleline[start1 - 1].is_alphanumeric() {
            start1 -= 1;
        }

        if start1 < end1 && start2 < end2 {
            let word1: Vec<char> = self.zleline[start1..end1].to_vec();
            let word2: Vec<char> = self.zleline[start2..end2].to_vec();

            // Replace word2 first (higher index)
            self.zleline.drain(start2..end2);
            for (i, c) in word1.iter().enumerate() {
                self.zleline.insert(start2 + i, *c);
            }

            // Replace word1
            let new_end1 = end1 - (end2 - start2) + word1.len();
            let _new_start1 = start1;
            self.zleline.drain(start1..end1);
            for (i, c) in word2.iter().enumerate() {
                self.zleline.insert(start1 + i, *c);
            }

            self.zlell = self.zleline.len();
            self.zlecs = new_end1;
            self.resetneeded = true;
        }
    }

    /// Quote line
    /// Port of quoteline() from zle_misc.c
    pub fn quote_line(&mut self) {
        self.zleline.insert(0, '\'');
        self.zlell += 1;
        self.zlecs += 1;
        self.zleline.push('\'');
        self.zlell += 1;
        self.resetneeded = true;
    }

    /// Quote region
    /// Port of quoteregion() from zle_misc.c
    pub fn quote_region(&mut self) {
        let (start, end) = if self.zlecs < self.mark {
            (self.zlecs, self.mark)
        } else {
            (self.mark, self.zlecs)
        };

        self.zleline.insert(end, '\'');
        self.zleline.insert(start, '\'');
        self.zlell += 2;
        self.zlecs = end + 2;
        self.mark = start;
        self.resetneeded = true;
    }

    /// What cursor position - display cursor info
    /// Port of whatcursorposition() from zle_misc.c
    pub fn what_cursor_position(&self) -> String {
        if self.zlecs >= self.zlell {
            return format!("point={} of {} (EOL)", self.zlecs, self.zlell);
        }

        let c = self.zleline[self.zlecs];
        let code = c as u32;
        format!(
            "Char: {} (0{:o}, {:?}, 0x{:x})  point {} of {} ({}%)",
            c,
            code,
            code,
            code,
            self.zlecs,
            self.zlell,
            if self.zlell == 0 {
                0
            } else {
                self.zlecs * 100 / self.zlell
            }
        )
    }

    /// Universal argument - multiply next command
    /// Port of universalargument() from zle_misc.c
    pub fn universal_argument(&mut self) {
        self.mult = self.mult.saturating_mul(4);
    }

    /// Digit argument - accumulate numeric argument
    /// Port of digitargument() from zle_misc.c
    pub fn digit_argument(&mut self, digit: u8) {
        if self.mult == 1 && !self.neg_arg {
            self.mult = 0;
        }
        self.mult = self.mult.saturating_mul(10).saturating_add(digit as i32);
    }

    /// Negative argument
    /// Port of negargument() from zle_misc.c
    pub fn neg_argument(&mut self) {
        self.neg_arg = !self.neg_arg;
    }

    /// Undefined key - beep
    /// Port of undefinedkey() from zle_misc.c
    pub fn undefined_key(&self) {
        print!("\x07"); // Bell
    }

    /// Send break - abort current operation
    /// Port of sendbreak() from zle_misc.c
    pub fn send_break(&mut self) {
        self.zleline.clear();
        self.zlell = 0;
        self.zlecs = 0;
        self.mark = 0;
        self.resetneeded = true;
    }

    /// Vi put after cursor
    /// Port of viputafter() from zle_misc.c
    pub fn vi_put_after(&mut self) {
        if self.zlecs < self.zlell {
            self.zlecs += 1;
        }
        self.yank();
        if self.zlecs > 0 {
            self.zlecs -= 1;
        }
    }

    /// Vi put before cursor
    /// Port of viputbefore() from zle_misc.c
    pub fn vi_put_before(&mut self) {
        self.yank();
    }

    /// Overwrite mode toggle
    /// Port of overwritemode() from zle_misc.c
    pub fn overwrite_mode(&mut self) {
        self.insmode = !self.insmode;
    }

    /// Copy previous word
    /// Port of copyprevword() from zle_misc.c
    pub fn copy_prev_word(&mut self) {
        if self.zlecs == 0 {
            return;
        }

        // Find start of previous word
        let mut end = self.zlecs;
        while end > 0 && self.zleline[end - 1].is_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && !self.zleline[start - 1].is_whitespace() {
            start -= 1;
        }

        if start < end {
            let word: Vec<char> = self.zleline[start..end].to_vec();
            for c in word {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
            }
            self.zlell = self.zleline.len();
            self.resetneeded = true;
        }
    }

    /// Copy previous shell word (respects quoting)
    /// Port of copyprevshellword() from zle_misc.c
    pub fn copy_prev_shell_word(&mut self) {
        // Simplified - doesn't handle full shell quoting
        self.copy_prev_word();
    }

    /// Pound insert - comment toggle for vi mode
    /// Port of poundinsert() from zle_misc.c
    pub fn pound_insert(&mut self) {
        if !self.zleline.is_empty() && self.zleline[0] == '#' {
            self.zleline.remove(0);
            self.zlell -= 1;
            if self.zlecs > 0 {
                self.zlecs -= 1;
            }
        } else {
            self.zleline.insert(0, '#');
            self.zlell += 1;
            self.zlecs += 1;
        }
        self.resetneeded = true;
    }
}
