//! ZLE movement operations
//!
//! Direct port from zsh/Src/Zle/zle_move.c

use super::main::Zle;

impl Zle {
    /// Move cursor to start of current physical line
    pub fn move_to_bol(&mut self) {
        while self.zlecs > 0 && self.zleline[self.zlecs - 1] != '\n' {
            self.zlecs -= 1;
        }
    }

    /// Move cursor to end of current physical line
    pub fn move_to_eol(&mut self) {
        while self.zlecs < self.zlell && self.zleline[self.zlecs] != '\n' {
            self.zlecs += 1;
        }
    }

    /// Move cursor up one line
    pub fn move_up(&mut self) -> bool {
        let col = self.current_column();

        // Find start of current line
        let mut line_start = self.zlecs;
        while line_start > 0 && self.zleline[line_start - 1] != '\n' {
            line_start -= 1;
        }

        if line_start == 0 {
            return false; // Already on first line
        }

        // Move to end of previous line
        self.zlecs = line_start - 1;

        // Find start of previous line
        let mut prev_start = self.zlecs;
        while prev_start > 0 && self.zleline[prev_start - 1] != '\n' {
            prev_start -= 1;
        }

        // Move to same column or end of line
        self.zlecs = prev_start + col.min(self.zlecs - prev_start);

        true
    }

    /// Move cursor down one line
    pub fn move_down(&mut self) -> bool {
        let col = self.current_column();

        // Find end of current line
        let mut line_end = self.zlecs;
        while line_end < self.zlell && self.zleline[line_end] != '\n' {
            line_end += 1;
        }

        if line_end >= self.zlell {
            return false; // Already on last line
        }

        // Move to start of next line
        self.zlecs = line_end + 1;

        // Find end of next line
        let mut next_end = self.zlecs;
        while next_end < self.zlell && self.zleline[next_end] != '\n' {
            next_end += 1;
        }

        // Move to same column or end of line
        self.zlecs = (self.zlecs + col).min(next_end);

        true
    }

    /// Get current column (0-indexed)
    pub fn current_column(&self) -> usize {
        let mut col = 0;
        let mut i = self.zlecs;
        while i > 0 && self.zleline[i - 1] != '\n' {
            i -= 1;
            col += 1;
        }
        col
    }

    /// Get current line number (0-indexed)
    pub fn current_line(&self) -> usize {
        self.zleline[..self.zlecs]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
    }

    /// Count total lines
    pub fn count_lines(&self) -> usize {
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }
}
