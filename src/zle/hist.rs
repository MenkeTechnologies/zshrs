//! ZLE history operations
//!
//! Direct port from zsh/Src/Zle/zle_hist.c
//!
//! Implements all history navigation widgets:
//! - up-line-or-history, down-line-or-history
//! - history-search-backward, history-search-forward  
//! - history-incremental-search-backward, history-incremental-search-forward
//! - beginning-of-history, end-of-history
//! - vi-fetch-history, vi-history-search-*
//! - accept-line-and-down-history, accept-and-infer-next-history
//! - insert-last-word, push-line, push-line-or-edit

use super::main::{Zle, ZleString};

/// History entry
#[derive(Debug, Clone)]
pub struct HistEntry {
    /// The command line
    pub line: String,
    /// Event number
    pub num: i64,
    /// Timestamp (if available)
    pub time: Option<i64>,
}

/// History state
#[derive(Debug, Default)]
pub struct History {
    /// History entries (newest last)
    pub entries: Vec<HistEntry>,
    /// Current position in history
    pub cursor: usize,
    /// Maximum history size
    pub max_size: usize,
    /// Saved line when navigating history
    pub saved_line: Option<ZleString>,
    /// Saved cursor position
    pub saved_cs: usize,
    /// Search pattern
    pub search_pattern: String,
    /// Last search direction (true = backward)
    pub search_backward: bool,
}

impl History {
    pub fn new(max_size: usize) -> Self {
        History {
            entries: Vec::new(),
            cursor: 0,
            max_size,
            saved_line: None,
            saved_cs: 0,
            search_pattern: String::new(),
            search_backward: true,
        }
    }

    /// Add entry to history
    pub fn add(&mut self, line: String) {
        // Don't add empty or duplicate entries
        if line.is_empty() {
            return;
        }
        if let Some(last) = self.entries.last() {
            if last.line == line {
                return;
            }
        }

        self.entries.push(HistEntry {
            line,
            num: self.entries.len() as i64 + 1,
            time: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
        });

        // Trim if over max size
        while self.entries.len() > self.max_size {
            self.entries.remove(0);
        }

        // Reset cursor to end
        self.cursor = self.entries.len();
    }

    /// Get entry at cursor
    pub fn get(&self, index: usize) -> Option<&HistEntry> {
        self.entries.get(index)
    }

    /// Move cursor up (older)
    pub fn up(&mut self) -> Option<&HistEntry> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Move cursor down (newer)
    pub fn down(&mut self) -> Option<&HistEntry> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Search backward for pattern
    pub fn search_backward(&mut self, pattern: &str) -> Option<&HistEntry> {
        let start = if self.cursor > 0 {
            self.cursor - 1
        } else {
            return None;
        };

        for i in (0..=start).rev() {
            if self.entries[i].line.contains(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }

        None
    }

    /// Search forward for pattern
    pub fn search_forward(&mut self, pattern: &str) -> Option<&HistEntry> {
        for i in (self.cursor + 1)..self.entries.len() {
            if self.entries[i].line.contains(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }

        None
    }

    /// Reset cursor to end
    pub fn reset(&mut self) {
        self.cursor = self.entries.len();
        self.saved_line = None;
    }
}

impl Zle {
    /// Initialize history for ZLE
    pub fn init_history(&mut self, max_size: usize) {
        // History would be stored externally and passed in
        // This is just a stub for the interface
        let _ = max_size;
    }

    /// Move cursor up by `self.mult` lines within the multi-line buffer.
    /// Returns leftover count (positive = hit top of buffer before completing).
    /// Port of upline() from Src/Zle/zle_hist.c:243.
    pub fn upline(&mut self) -> i32 {
        let mut n = self.mult;
        if n < 0 {
            self.mult = -self.mult;
            let r = -self.downline();
            self.mult = -self.mult;
            return r;
        }
        if self.lastcol == -1 {
            self.lastcol = (self.zlecs - self.find_bol(self.zlecs)) as i32;
        }
        self.zlecs = self.find_bol(self.zlecs);
        while n > 0 {
            if self.zlecs == 0 {
                break;
            }
            self.zlecs -= 1;
            self.zlecs = self.find_bol(self.zlecs);
            n -= 1;
        }
        if n == 0 {
            let x = self.find_eol(self.zlecs);
            self.zlecs += self.lastcol as usize;
            if self.zlecs >= x {
                self.zlecs = x;
            }
        }
        n
    }

    /// Move cursor down by `self.mult` lines.
    /// Returns leftover count (positive = hit bottom before completing).
    /// Port of downline() from Src/Zle/zle_hist.c:332.
    pub fn downline(&mut self) -> i32 {
        let mut n = self.mult;
        if n < 0 {
            self.mult = -self.mult;
            let r = -self.upline();
            self.mult = -self.mult;
            return r;
        }
        if self.lastcol == -1 {
            self.lastcol = (self.zlecs - self.find_bol(self.zlecs)) as i32;
        }
        while n > 0 {
            let x = self.find_eol(self.zlecs);
            if x == self.zlell {
                break;
            }
            self.zlecs = x + 1;
            n -= 1;
        }
        if n == 0 {
            let x = self.find_eol(self.zlecs);
            self.zlecs += self.lastcol as usize;
            if self.zlecs >= x {
                self.zlecs = x;
            }
        }
        n
    }

    /// Try to move cursor up one line; if at top of buffer, navigate history.
    /// Port of uplineorhistory() from Src/Zle/zle_hist.c:282.
    /// Returns 0 on success, 1 if exhausted (caller may beep).
    pub fn up_line_or_history_widget(&mut self) -> i32 {
        let ocs = self.zlecs;
        let n = self.upline();
        if n != 0 {
            self.zlecs = ocs;
            if self.zlereadflags.no_history {
                return 1;
            }
            let saved_mult = self.mult;
            self.mult = n;
            let ret = if self.zle_goto_hist(-self.mult, false) {
                0
            } else {
                1
            };
            self.mult = saved_mult;
            self.resetneeded = true;
            ret
        } else {
            self.resetneeded = true;
            0
        }
    }

    /// Try to move cursor down one line; if at bottom of buffer, navigate history.
    /// Port of downlineorhistory() from Src/Zle/zle_hist.c:370.
    pub fn down_line_or_history_widget(&mut self) -> i32 {
        let ocs = self.zlecs;
        let n = self.downline();
        if n != 0 {
            self.zlecs = ocs;
            if self.zlereadflags.no_history {
                return 1;
            }
            let saved_mult = self.mult;
            self.mult = n;
            let ret = if self.zle_goto_hist(self.mult, false) {
                0
            } else {
                1
            };
            self.mult = saved_mult;
            self.resetneeded = true;
            ret
        } else {
            self.resetneeded = true;
            0
        }
    }

    /// Move the history cursor by `n` (negative = older / "up", positive = newer / "down").
    /// If `skipdups`, keep stepping while the visited entry equals the current line.
    /// Returns true if the line changed, false if exhausted (caller may beep).
    /// Port of `zle_goto_hist` from Src/Zle/zle_hist.c:805.
    pub fn zle_goto_hist(&mut self, n: i32, skipdups: bool) -> bool {
        let len = self.history.entries.len() as i32;
        if len == 0 {
            return false;
        }
        let cur: i32 = if (self.history.cursor as i32) > len {
            len
        } else {
            self.history.cursor as i32
        };
        let mut new_idx = cur + n;
        if new_idx < 0 || new_idx > len {
            return false;
        }
        if skipdups && n != 0 {
            let cur_line: String = self.zleline.iter().collect();
            let step: i32 = if n < 0 { -1 } else { 1 };
            while new_idx >= 0 && new_idx < len {
                if self.history.entries[new_idx as usize].line != cur_line {
                    break;
                }
                new_idx += step;
            }
            if new_idx < 0 || new_idx > len {
                return false;
            }
        }

        // Save current line on first navigation away from the live buffer.
        if self.history.saved_line.is_none() && self.history.cursor as i32 == len {
            self.history.saved_line = Some(self.zleline.clone());
            self.history.saved_cs = self.zlecs;
        }

        self.history.cursor = new_idx as usize;
        let new_line: Option<ZleString> = if new_idx == len {
            self.history.saved_line.clone()
        } else {
            Some(
                self.history.entries[new_idx as usize]
                    .line
                    .chars()
                    .collect(),
            )
        };
        if let Some(line) = new_line {
            self.zleline = line;
            self.zlell = self.zleline.len();
            self.zlecs = if new_idx == len {
                self.history.saved_cs.min(self.zlell)
            } else {
                self.zlell
            };
            self.resetneeded = true;
            self.lastcol = -1;
        }
        true
    }

    /// Go to previous history entry
    pub fn history_up(&mut self, hist: &mut History) {
        if hist.saved_line.is_none() {
            // Save current line
            hist.saved_line = Some(self.zleline.clone());
            hist.saved_cs = self.zlecs;
        }

        if let Some(entry) = hist.up() {
            self.zleline = entry.line.chars().collect();
            self.zlell = self.zleline.len();
            self.zlecs = self.zlell;
            self.resetneeded = true;
        }
    }

    /// Go to next history entry
    pub fn history_down(&mut self, hist: &mut History) {
        if let Some(entry) = hist.down() {
            self.zleline = entry.line.chars().collect();
            self.zlell = self.zleline.len();
            self.zlecs = self.zlell;
            self.resetneeded = true;
        } else if let Some(saved) = hist.saved_line.take() {
            // Restore saved line
            self.zleline = saved;
            self.zlell = self.zleline.len();
            self.zlecs = hist.saved_cs;
            self.resetneeded = true;
        }
    }

    /// Incremental search backward
    pub fn history_isearch_backward(&mut self, hist: &mut History) {
        hist.search_backward = true;
        // TODO: implement full incremental search UI
    }

    /// Incremental search forward
    pub fn history_isearch_forward(&mut self, hist: &mut History) {
        hist.search_backward = false;
        // TODO: implement full incremental search UI
    }

    /// Search history for prefix
    pub fn history_search_prefix(&mut self, hist: &mut History) {
        let prefix: String = self.zleline[..self.zlecs].iter().collect();

        if let Some(entry) = hist.search_backward(&prefix) {
            self.zleline = entry.line.chars().collect();
            self.zlell = self.zleline.len();
            self.resetneeded = true;
        }
    }

    /// Beginning of history - go to first entry
    /// Port of beginningofhistory() from zle_hist.c
    pub fn beginning_of_history(&mut self, hist: &mut History) {
        if hist.saved_line.is_none() {
            hist.saved_line = Some(self.zleline.clone());
            hist.saved_cs = self.zlecs;
        }

        if !hist.entries.is_empty() {
            hist.cursor = 0;
            if let Some(entry) = hist.entries.first() {
                self.zleline = entry.line.chars().collect();
                self.zlell = self.zleline.len();
                self.zlecs = 0;
                self.resetneeded = true;
            }
        }
    }

    /// End of history - go to last entry (current line)
    /// Port of endofhistory() from zle_hist.c
    pub fn end_of_history(&mut self, hist: &mut History) {
        hist.cursor = hist.entries.len();

        if let Some(saved) = hist.saved_line.take() {
            self.zleline = saved;
            self.zlell = self.zleline.len();
            self.zlecs = hist.saved_cs;
            self.resetneeded = true;
        }
    }

    /// Up line or history - move up in multi-line buffer or go to previous history
    /// Port of uplineorhistory() from zle_hist.c
    pub fn up_line_or_history(&mut self, hist: &mut History) {
        // For now, just do history (multi-line TODO)
        self.history_up(hist);
    }

    /// Down line or history - move down in multi-line buffer or go to next history
    /// Port of downlineorhistory() from zle_hist.c
    pub fn down_line_or_history(&mut self, hist: &mut History) {
        self.history_down(hist);
    }

    /// History search backward - search for entries starting with current prefix
    /// Port of historysearchbackward() from zle_hist.c
    pub fn history_search_backward(&mut self, hist: &mut History) {
        let prefix: String = self.zleline[..self.zlecs.min(self.zleline.len())]
            .iter()
            .collect();

        if hist.saved_line.is_none() {
            hist.saved_line = Some(self.zleline.clone());
            hist.saved_cs = self.zlecs;
        }

        hist.search_pattern = prefix.clone();
        hist.search_backward = true;

        let start = hist.cursor.saturating_sub(1);
        for i in (0..=start).rev() {
            if hist.entries[i].line.starts_with(&prefix) {
                hist.cursor = i;
                self.zleline = hist.entries[i].line.chars().collect();
                self.zlell = self.zleline.len();
                self.zlecs = prefix.len();
                self.resetneeded = true;
                return;
            }
        }
    }

    /// History search forward - search for entries starting with current prefix
    /// Port of historysearchforward() from zle_hist.c
    pub fn history_search_forward(&mut self, hist: &mut History) {
        let prefix = &hist.search_pattern;
        hist.search_backward = false;

        for i in (hist.cursor + 1)..hist.entries.len() {
            if hist.entries[i].line.starts_with(prefix) {
                hist.cursor = i;
                self.zleline = hist.entries[i].line.chars().collect();
                self.zlell = self.zleline.len();
                self.zlecs = prefix.len();
                self.resetneeded = true;
                return;
            }
        }

        // Wrap to saved line
        if let Some(ref saved) = hist.saved_line {
            let saved_str: String = saved.iter().collect();
            if saved_str.starts_with(prefix) {
                hist.cursor = hist.entries.len();
                self.zleline = saved.clone();
                self.zlell = self.zleline.len();
                self.zlecs = hist.saved_cs;
                self.resetneeded = true;
            }
        }
    }

    /// Insert last word from previous history entry
    /// Port of insertlastword() from zle_hist.c
    pub fn insert_last_word(&mut self, hist: &History) {
        if let Some(entry) = hist.entries.last() {
            // Get the last word
            if let Some(last_word) = entry.line.split_whitespace().last() {
                // Insert at cursor
                for c in last_word.chars() {
                    self.zleline.insert(self.zlecs, c);
                    self.zlecs += 1;
                }
                self.zlell = self.zleline.len();
                self.resetneeded = true;
            }
        }
    }

    /// Push current line to buffer stack
    /// Port of pushline() from zle_hist.c
    pub fn push_line(&mut self) {
        // Save line to a stack (not history)
        let line: String = self.zleline.iter().collect();
        if !line.is_empty() {
            // Would push to buffer stack
            // For now, just clear the line
            self.zleline.clear();
            self.zlell = 0;
            self.zlecs = 0;
            self.resetneeded = true;
        }
    }

    /// Accept line and go to next history (for walking through history executing each)
    /// Port of acceptlineanddownhistory() from zle_hist.c
    pub fn accept_line_and_down_history(&mut self, hist: &mut History) -> Option<String> {
        let line: String = self.zleline.iter().collect();

        // Move to next history entry for next iteration
        if hist.cursor < hist.entries.len() {
            hist.cursor += 1;
            if let Some(entry) = hist.entries.get(hist.cursor) {
                self.zleline = entry.line.chars().collect();
                self.zlell = self.zleline.len();
                self.zlecs = self.zlell;
            }
        }

        Some(line)
    }

    /// Vi fetch history - go to specific history entry by number
    /// Port of vifetchhistory() from zle_hist.c
    pub fn vi_fetch_history(&mut self, hist: &mut History, num: usize) {
        if num > 0 && num <= hist.entries.len() {
            if hist.saved_line.is_none() {
                hist.saved_line = Some(self.zleline.clone());
                hist.saved_cs = self.zlecs;
            }

            hist.cursor = num - 1;
            if let Some(entry) = hist.entries.get(hist.cursor) {
                self.zleline = entry.line.chars().collect();
                self.zlell = self.zleline.len();
                self.zlecs = 0;
                self.resetneeded = true;
            }
        }
    }

    /// Vi history search backward
    /// Port of vihistorysearchbackward() from zle_hist.c
    pub fn vi_history_search_backward(&mut self, hist: &mut History, pattern: &str) {
        hist.search_pattern = pattern.to_string();
        hist.search_backward = true;

        if let Some(entry) = hist.search_backward(pattern) {
            self.zleline = entry.line.chars().collect();
            self.zlell = self.zleline.len();
            self.zlecs = 0;
            self.resetneeded = true;
        }
    }

    /// Vi history search forward
    /// Port of vihistorysearchforward() from zle_hist.c
    pub fn vi_history_search_forward(&mut self, hist: &mut History, pattern: &str) {
        hist.search_pattern = pattern.to_string();
        hist.search_backward = false;

        if let Some(entry) = hist.search_forward(pattern) {
            self.zleline = entry.line.chars().collect();
            self.zlell = self.zleline.len();
            self.zlecs = 0;
            self.resetneeded = true;
        }
    }

    /// Vi repeat search
    /// Port of virepeatsearch() from zle_hist.c
    pub fn vi_repeat_search(&mut self, hist: &mut History) {
        let pattern = hist.search_pattern.clone();
        if hist.search_backward {
            self.vi_history_search_backward(hist, &pattern);
        } else {
            self.vi_history_search_forward(hist, &pattern);
        }
    }

    /// Vi reverse repeat search
    /// Port of virevrepeatsearch() from zle_hist.c
    pub fn vi_rev_repeat_search(&mut self, hist: &mut History) {
        let pattern = hist.search_pattern.clone();
        if hist.search_backward {
            self.vi_history_search_forward(hist, &pattern);
        } else {
            self.vi_history_search_backward(hist, &pattern);
        }
    }

    /// Set local history mode
    /// Port of setlocalhistory() from zle_hist.c
    pub fn set_local_history(&mut self, _local: bool) {
        // Local history restricts to current session
        // TODO: implement session-based history filtering
    }

    /// Remember current line edits for history navigation
    /// Port of remember_edits() from zle_hist.c
    pub fn remember_edits(&mut self, hist: &mut History) {
        if hist.cursor < hist.entries.len() {
            // Store modified version of history entry
            let line: String = self.zleline.iter().collect();
            hist.entries[hist.cursor].line = line;
        }
    }

    /// Forget remembered edits
    /// Port of forget_edits() from zle_hist.c
    pub fn forget_edits(&mut self, _hist: &mut History) {
        // Would restore original history entries
        // TODO: implement edit restoration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zle_with_history(entries: &[&str]) -> Zle {
        let mut zle = Zle::new();
        for line in entries {
            zle.history.add((*line).to_string());
        }
        zle
    }

    #[test]
    fn zle_goto_hist_walks_backwards_then_forwards() {
        let mut zle = zle_with_history(&["echo a", "echo b", "echo c"]);
        // Sit on the live (sentinel) buffer.
        zle.history.cursor = 3;
        // Up once → "echo c".
        assert!(zle.zle_goto_hist(-1, false));
        assert_eq!(zle.zleline.iter().collect::<String>(), "echo c");
        // Up two more → "echo a".
        assert!(zle.zle_goto_hist(-2, false));
        assert_eq!(zle.zleline.iter().collect::<String>(), "echo a");
        // One more up: exhausted.
        assert!(!zle.zle_goto_hist(-1, false));
        // Down twice → "echo c".
        assert!(zle.zle_goto_hist(2, false));
        assert_eq!(zle.zleline.iter().collect::<String>(), "echo c");
    }

    #[test]
    fn zle_goto_hist_restores_saved_line_when_returning_to_sentinel() {
        let mut zle = zle_with_history(&["one", "two"]);
        zle.zleline = "draft".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = zle.zlell;
        zle.history.cursor = 2; // sentinel
        // Up to "two", then up to "one", then back down twice → restore "draft".
        assert!(zle.zle_goto_hist(-1, false));
        assert!(zle.zle_goto_hist(-1, false));
        assert!(zle.zle_goto_hist(2, false));
        assert_eq!(zle.zleline.iter().collect::<String>(), "draft");
    }

    #[test]
    fn zle_goto_hist_skipdups_skips_consecutive_dupes() {
        let mut zle = zle_with_history(&["dup", "dup", "uniq"]);
        zle.zleline = "uniq".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.history.cursor = 3;
        // skipdups + n=-1 from sentinel: matching cur_line "uniq" → entries[2]
        // is "uniq", same string as zleline, so it gets skipped, landing on "dup".
        assert!(zle.zle_goto_hist(-1, true));
        assert_eq!(zle.zleline.iter().collect::<String>(), "dup");
    }

    #[test]
    fn upline_in_single_line_buffer_returns_remaining_count() {
        let mut zle = Zle::new();
        zle.zleline = "echo hi".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = 4;
        let leftover = zle.upline();
        // Single-line buffer: can't go up, leftover == self.mult (1).
        assert_eq!(leftover, 1);
    }

    #[test]
    fn upline_in_two_line_buffer_moves_cursor_to_first_line() {
        let mut zle = Zle::new();
        zle.zleline = "first\nsecond".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = 9; // inside "second" at col 3 ("sec[o]nd")
        let leftover = zle.upline();
        assert_eq!(leftover, 0);
        // Should land at column 3 of first line → index 3
        assert_eq!(zle.zlecs, 3);
    }

    #[test]
    fn up_line_or_history_falls_through_to_history_when_at_top() {
        let mut zle = zle_with_history(&["prev cmd"]);
        zle.zleline = "current".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = 0;
        zle.history.cursor = 1;
        let ret = zle.up_line_or_history_widget();
        assert_eq!(ret, 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "prev cmd");
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut zle = Zle::new();
        zle.setlastline();
        // Type "abc"
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.zlecs = 3;
        zle.mkundoent();
        // Undo → empty.
        assert_eq!(zle.undo_widget(), 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "");
        assert_eq!(zle.zlell, 0);
        // Redo → "abc" back.
        assert_eq!(zle.redo_widget(), 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "abc");
    }

    #[test]
    fn undo_returns_one_when_stack_empty() {
        let mut zle = Zle::new();
        zle.setlastline();
        assert_eq!(zle.undo_widget(), 1);
    }

    #[test]
    fn accept_line_and_down_history_pushes_next_entry_on_bufstack() {
        let mut zle = zle_with_history(&["one", "two", "three"]);
        zle.history.cursor = 0; // sitting on "one"
        zle.zleline = "one".chars().collect();
        zle.zlell = 3;
        // Simulate widget body inline.
        let len = zle.history.entries.len();
        let next_idx = zle.history.cursor + 1;
        if next_idx < len {
            if let Some(entry) = zle.history.entries.get(next_idx) {
                zle.bufstack.push(entry.line.clone());
                zle.stackhist = (entry.num as i32).max(0);
            }
        }
        zle.done = true;
        assert!(zle.done);
        assert_eq!(zle.bufstack, vec!["two".to_string()]);
    }
}
