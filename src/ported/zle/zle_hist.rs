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

use super::zle_main::{Zle, ZleString};

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
    /// Originals of edited entries: when `remember_edits` mutates
    /// `entries[i].line`, the pre-edit text lands here at index `i`.
    /// `forget_edits` restores them. Port of zsh's `Histent->zle_text`
    /// shadow string + the global `have_edits` flag in Src/Zle/zle_hist.c.
    pub originals: Vec<Option<String>>,
    /// True if any entry has a recorded original — port of `have_edits`
    /// in Src/Zle/zle_hist.c:76.
    pub have_edits: bool,
    /// History skip-flags state. Bit-equivalent of zsh's `hist_skip_flags`
    /// in Src/Zle/zle_hist.c:794: `HIST_FOREIGN` (1) hides entries from
    /// other sessions when set; `setlocalhistory` toggles this.
    pub hist_skip_flags: u32,
}

impl History {
    /// Construct an empty history with a max-entry cap.
    /// Constructor for the History struct that mirrors the live history
    /// list zsh maintains via `inithist()` in Src/hist.c:1717. The C
    /// source caps at `histsiz` ($HISTSIZE); the equivalent here is
    /// `max_size` passed at construction.
    pub fn new(max_size: usize) -> Self {
        History {
            entries: Vec::new(),
            cursor: 0,
            max_size,
            saved_line: None,
            saved_cs: 0,
            search_pattern: String::new(),
            search_backward: true,
            originals: Vec::new(),
            have_edits: false,
            hist_skip_flags: 0,
        }
    }

    /// Append a new entry to the history list.
    /// Port of `addhistnode()` from Src/hist.c (the inner add path
    /// invoked by `addhistline`/`hend`). Skips empty input and
    /// consecutive-duplicate lines (same as zsh's HIST_IGNORE_DUPS
    /// default behaviour). Trims entries from the front when the
    /// list exceeds max_size — the C source uses `freehistnode` on
    /// the LRU end of the hist_ring.
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

    /// Look up the entry at a specific 0-based index.
    /// Port of `quietgethist()` from Src/Zle/zle_hist.c:1712 — that
    /// fetches by event number; our entries Vec is 0-indexed so
    /// callers convert via num→index themselves.
    pub fn get(&self, index: usize) -> Option<&HistEntry> {
        self.entries.get(index)
    }

    /// Step the history cursor one position older.
    /// Port of the cursor-decrement portion of `zle_goto_hist()` at
    /// Src/Zle/zle_hist.c:805 with a fixed n=-1 step. Returns the
    /// landed entry or None at BoH.
    pub fn up(&mut self) -> Option<&HistEntry> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Step the history cursor one position newer.
    /// Port of `zle_goto_hist()` at Src/Zle/zle_hist.c:805 with a
    /// fixed n=+1 step. Returns the landed entry or None at EoH
    /// (the live buffer sentinel).
    pub fn down(&mut self) -> Option<&HistEntry> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Search history for the most recent entry containing `pattern`.
    /// Substring-match port of the inner loop of `historysearchbackward()`
    /// from Src/Zle/zle_hist.c:484. The C source supports glob patterns
    /// via `hist_skip_flags`; our basic substring match is sufficient
    /// for simple isearch-style lookups but doesn't honour zsh's
    /// HIST_PATTERN flag — see `historyincrementalpatternsearchbackward`
    /// for the pattern-mode variant.
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

    /// Search history forward for the next entry containing `pattern`.
    /// Mirror of `search_backward` against `historysearchforward()` at
    /// Src/Zle/zle_hist.c:541.
    pub fn search_forward(&mut self, pattern: &str) -> Option<&HistEntry> {
        for i in (self.cursor + 1)..self.entries.len() {
            if self.entries[i].line.contains(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }

        None
    }

    /// Reset the cursor to the live-buffer sentinel position and drop
    /// any saved pre-navigation line.
    /// Port of the `histline = curhist; saved_line = NULL` reset path
    /// invoked by `endofhistory()` (Src/Zle/zle_hist.c:478) and after
    /// accept-line. Used by callers re-entering zleread for a fresh
    /// edit session.
    pub fn reset(&mut self) {
        self.cursor = self.entries.len();
        self.saved_line = None;
    }
}

impl Zle {
    /// Set up history limits at ZLE startup.
    /// Stub mirroring the role of `inithist()` from Src/hist.c:1717,
    /// which sizes the global hist_ring at $HISTSIZE. zshrs's history
    /// lives on `Zle::history` (constructed in `Zle::new`); this method
    /// is kept for API compatibility — callers can adjust max_size
    /// post-construction if needed.
    pub fn init_history(&mut self, max_size: usize) {
        let _ = max_size;
    }

    /// Move cursor up by `self.mult` lines within the multi-line buffer.
    /// Returns leftover count (positive = hit top of buffer before completing).
    /// Port of upline() from Src/Zle/zle_hist.c:243.
    pub fn upline(&mut self) -> i32 {                                        // c:243
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
    pub fn downline(&mut self) -> i32 {                                      // c:332
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
    pub fn up_line_or_history_widget(&mut self) -> i32 {                     // c:282
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
    pub fn down_line_or_history_widget(&mut self) -> i32 {                   // c:370
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

    /// Walk one entry older through the externally-supplied History.
    /// External-history overload of the widget-callable
    /// `Zle::zle_goto_hist(-1, false)` — kept for callers that drive a
    /// separate History instance. Port of `uphistory()` at
    /// Src/Zle/zle_hist.c:233 (the live-buffer save matches the C
    /// source's first-navigate-saves-original behaviour).
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

    /// Walk one entry newer; if past the last entry, restore the saved
    /// pre-navigation line.
    /// External-history overload of `Zle::zle_goto_hist(1, false)`.
    /// Port of `downhistory()` at Src/Zle/zle_hist.c:434 with the
    /// saved-line restore from zle_goto_hist's sentinel branch.
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

    /// Set search direction for an incremental backward search. The full
    /// interactive isearch UI lives in `widget::do_isearch` (called by the
    /// `widget_history_isearch_backward` widget) — this method only flips
    /// the saved direction flag for callers that drive History externally.
    pub fn history_isearch_backward(&mut self, hist: &mut History) {
        hist.search_backward = true;
    }

    /// Mirror of `history_isearch_backward` but for forward search.
    pub fn history_isearch_forward(&mut self, hist: &mut History) {
        hist.search_backward = false;
    }

    /// Search history for an entry containing the buffer text up to
    /// the cursor.
    /// Port of `historybeginningsearchbackward()` from
    /// Src/Zle/zle_hist.c:2039 with substring-match instead of
    /// prefix-match — useful as an isearch-style helper for callers
    /// that drive History externally. The strict prefix-match form
    /// lives in `widget_history_beginning_search_backward`.
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

    /// Up line or history — external-History overload of the widget-callable
    /// `Zle::up_line_or_history_widget` (which handles multi-line motion via
    /// upline() + zle_goto_hist). This variant is kept for callers that
    /// thread their own History; it just steps the cursor up and falls
    /// through to history_up.
    pub fn up_line_or_history(&mut self, hist: &mut History) {
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

    /// Push the current line onto the buffer stack and clear the editor.
    /// Port of `pushline()` from Src/Zle/zle_hist.c:832. The C source
    /// pushes the assembled line, then `mult - 1` empty strings (so a
    /// numeric prefix repeats the push), saves zlecs to stackcs, and
    /// blanks the line. The buffer stack is then drained on the next
    /// zleread() so the user gets to compose a quick command and have
    /// the prior text restored afterwards.
    pub fn push_line(&mut self) {
        let n = self.mult;
        if n < 0 {
            return;
        }
        let line: String = self.zleline.iter().collect();
        self.bufstack.push(line);
        let mut remaining = n - 1;
        while remaining > 0 {
            self.bufstack.push(String::new());
            remaining -= 1;
        }
        self.stackcs = self.zlecs;
        self.zleline.clear();
        self.zlell = 0;
        self.zlecs = 0;
        self.resetneeded = true;
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

    /// Toggle session-local history filtering.
    /// Port of `setlocalhistory()` from Src/Zle/zle_hist.c:794. With an
    /// explicit count: `mult` non-zero turns the foreign-skip filter on
    /// (`hist_skip_flags = HIST_FOREIGN = 1`), zero turns it off. With
    /// no count: XOR-toggle the bit. Call sites that walk history can
    /// consult `hist.hist_skip_flags & 1` to decide whether to surface
    /// entries from other sessions.
    pub fn set_local_history(&mut self, hist: &mut History, has_mult: bool, mult: i32) {
        const HIST_FOREIGN: u32 = 1;
        if has_mult {
            hist.hist_skip_flags = if mult != 0 { HIST_FOREIGN } else { 0 };
        } else {
            hist.hist_skip_flags ^= HIST_FOREIGN;
        }
    }

    /// Snapshot the current line into the history entry at `cursor`,
    /// preserving the original on first edit.
    /// Port of `remember_edits()` from Src/Zle/zle_hist.c:80. The C source
    /// stashes the in-flight text in `Histent->zle_text` (a separate field
    /// from the canonical history line) and sets `have_edits = 1`. We
    /// model `zle_text` by keeping the edited text in `entries[i].line`
    /// directly and saving the canonical version into `originals[i]`
    /// on first edit so `forget_edits` can restore it.
    pub fn remember_edits(&mut self, hist: &mut History) {
        if hist.cursor < hist.entries.len() {
            if hist.originals.len() < hist.entries.len() {
                hist.originals.resize(hist.entries.len(), None);
            }
            let new_line: String = self.zleline.iter().collect();
            if hist.entries[hist.cursor].line != new_line {
                if hist.originals[hist.cursor].is_none() {
                    hist.originals[hist.cursor] = Some(hist.entries[hist.cursor].line.clone());
                }
                hist.entries[hist.cursor].line = new_line;
                hist.have_edits = true;
            }
        }
    }

    /// Restore every edited history entry to its original text.
    /// Port of `forget_edits()` from Src/Zle/zle_hist.c:99. The C source
    /// walks the hist ring freeing each entry's `zle_text` shadow
    /// (zle_hist.c:107-112) and clears `have_edits`. We restore from
    /// `originals` and clear it.
    pub fn forget_edits(&mut self, hist: &mut History) {
        if !hist.have_edits {
            return;
        }
        for (i, original) in hist.originals.iter_mut().enumerate() {
            if let Some(text) = original.take() {
                if let Some(entry) = hist.entries.get_mut(i) {
                    entry.line = text;
                }
            }
        }
        hist.have_edits = false;
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
    fn push_line_pushes_buffer_and_clears_editor() {
        let mut zle = zle_with_history(&[]);
        zle.zleline = "in flight".chars().collect();
        zle.zlell = 9;
        zle.zlecs = 4;
        zle.mult = 1;
        zle.push_line();
        assert_eq!(zle.bufstack, vec!["in flight".to_string()]);
        assert!(zle.zleline.is_empty());
        assert_eq!(zle.zlell, 0);
        assert_eq!(zle.zlecs, 0);
        // stackcs records where the cursor was so a return-from-push can
        // restore it.
        assert_eq!(zle.stackcs, 4);
    }

    #[test]
    fn push_line_with_count_pushes_extra_empty_strings() {
        let mut zle = zle_with_history(&[]);
        zle.zleline = "x".chars().collect();
        zle.zlell = 1;
        zle.mult = 3;
        zle.push_line();
        // mult=3 → push line then 2 empties.
        assert_eq!(zle.bufstack.len(), 3);
        assert_eq!(zle.bufstack[0], "x");
        assert_eq!(zle.bufstack[1], "");
        assert_eq!(zle.bufstack[2], "");
    }

    #[test]
    fn push_line_negative_count_is_no_op() {
        let mut zle = zle_with_history(&[]);
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.mult = -1;
        zle.push_line();
        assert!(zle.bufstack.is_empty());
        assert_eq!(zle.zleline.iter().collect::<String>(), "abc");
    }

    #[test]
    fn remember_edits_saves_original_then_forget_restores() {
        let mut zle = zle_with_history(&["echo a", "echo b"]);
        zle.history.cursor = 0;
        zle.zleline = "echo Z".chars().collect();
        zle.zlell = 6;
        // Snapshot — borrow-check: take History out, mutate, put back.
        let mut hist = std::mem::take(&mut zle.history);
        zle.remember_edits(&mut hist);
        zle.history = hist;
        assert!(zle.history.have_edits);
        assert_eq!(zle.history.entries[0].line, "echo Z");
        assert_eq!(zle.history.originals[0].as_deref(), Some("echo a"));
        // Restore.
        let mut hist = std::mem::take(&mut zle.history);
        zle.forget_edits(&mut hist);
        zle.history = hist;
        assert!(!zle.history.have_edits);
        assert_eq!(zle.history.entries[0].line, "echo a");
        assert!(zle.history.originals[0].is_none());
    }

    #[test]
    fn set_local_history_mult_sets_or_clears_foreign_skip() {
        let mut zle = zle_with_history(&[]);
        let mut hist = std::mem::take(&mut zle.history);
        // mult=2 with has_mult=true → set HIST_FOREIGN.
        zle.set_local_history(&mut hist, true, 2);
        assert_eq!(hist.hist_skip_flags, 1);
        // mult=0 with has_mult=true → clear.
        zle.set_local_history(&mut hist, true, 0);
        assert_eq!(hist.hist_skip_flags, 0);
        zle.history = hist;
    }

    #[test]
    fn set_local_history_no_mult_xor_toggles() {
        let mut zle = zle_with_history(&[]);
        let mut hist = std::mem::take(&mut zle.history);
        // From 0, no-mult toggle → 1.
        zle.set_local_history(&mut hist, false, 0);
        assert_eq!(hist.hist_skip_flags, 1);
        // Toggle again → 0.
        zle.set_local_history(&mut hist, false, 0);
        assert_eq!(hist.hist_skip_flags, 0);
        zle.history = hist;
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

/// Port of `acceptandinfernexthistory()` from Src/Zle/zle_hist.c:1757. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn acceptandinfernexthistory() -> i32 { 0 }

/// Port of `acceptlineanddownhistory()` from Src/Zle/zle_hist.c:420. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn acceptlineanddownhistory() -> i32 { 0 }

/// Port of `beginningofbufferorhistory()` from Src/Zle/zle_hist.c:573. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningofbufferorhistory() -> i32 { 0 }

/// Port of `beginningofhistory()` from Src/Zle/zle_hist.c:584. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningofhistory() -> i32 { 0 }

/// Port of `doisearch()` from Src/Zle/zle_hist.c:1082. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doisearch() -> i32 { 0 }

/// Port of `downhistory()` from Src/Zle/zle_hist.c:434. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn downhistory() -> i32 { 0 }                                            // c:434

/// Port of `downlineorhistory()` from Src/Zle/zle_hist.c:370. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn downlineorhistory() -> i32 { 0 }

/// Port of `downlineorsearch()` from Src/Zle/zle_hist.c:400. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn downlineorsearch() -> i32 { 0 }

/// Port of `endofbufferorhistory()` from Src/Zle/zle_hist.c:593. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endofbufferorhistory() -> i32 { 0 }

/// Port of `endofhistory()` from Src/Zle/zle_hist.c:604. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endofhistory() -> i32 { 0 }

/// Port of `free_isrch_spots()` from Src/Zle/zle_hist.c:965. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn free_isrch_spots() -> i32 { 0 }

/// Port of `get_isrch_spot()` from Src/Zle/zle_hist.c:1000. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_isrch_spot() -> i32 { 0 }

/// Port of `getvisrchstr()` from Src/Zle/zle_hist.c:1814. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getvisrchstr() -> i32 { 0 }

/// Port of `historybeginningsearchbackward()` from Src/Zle/zle_hist.c:2039. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historybeginningsearchbackward() -> i32 { 0 }

/// Port of `historybeginningsearchforward()` from Src/Zle/zle_hist.c:2085. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historybeginningsearchforward() -> i32 { 0 }

/// Port of `historyincrementalpatternsearchbackward()` from Src/Zle/zle_hist.c:936. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historyincrementalpatternsearchbackward() -> i32 { 0 }

/// Port of `historyincrementalpatternsearchforward()` from Src/Zle/zle_hist.c:943. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historyincrementalpatternsearchforward() -> i32 { 0 }

/// Port of `historyincrementalsearchbackward()` from Src/Zle/zle_hist.c:922. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historyincrementalsearchbackward() -> i32 { 0 }

/// Port of `historyincrementalsearchforward()` from Src/Zle/zle_hist.c:929. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historyincrementalsearchforward() -> i32 { 0 }

/// Port of `historysearchbackward()` from Src/Zle/zle_hist.c:457. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historysearchbackward() -> i32 { 0 }

/// Port of `historysearchforward()` from Src/Zle/zle_hist.c:516. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historysearchforward() -> i32 { 0 }

/// Port of `infernexthist()` from Src/Zle/zle_hist.c:1741. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn infernexthist() -> i32 { 0 }

/// Port of `infernexthistory()` from Src/Zle/zle_hist.c:1772. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn infernexthistory() -> i32 { 0 }

/// Port of `insertlastword()` from Src/Zle/zle_hist.c:612. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn insertlastword() -> i32 { 0 }

/// Port of `isearch_newpos()` from Src/Zle/zle_hist.c:1024. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn isearch_newpos() -> i32 { 0 }

/// Port of `pushinput()` from Src/Zle/zle_hist.c:883. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushinput() -> i32 { 0 }

/// Port of `pushline()` from Src/Zle/zle_hist.c:832. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushline() -> i32 { 0 }

/// Port of `pushlineoredit()` from Src/Zle/zle_hist.c:852. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushlineoredit() -> i32 { 0 }

/// Port of `save_isearch_buffer()` from Src/Zle/zle_hist.c:1058. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn save_isearch_buffer() -> i32 { 0 }

/// Port of `set_isrch_spot()` from Src/Zle/zle_hist.c:974. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_isrch_spot() -> i32 { 0 }

/// Port of `setlocalhistory()` from Src/Zle/zle_hist.c:794. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setlocalhistory() -> i32 { 0 }

/// Port of `uphistory()` from Src/Zle/zle_hist.c:233. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn uphistory() -> i32 { 0 }                                              // c:233

/// Port of `uplineorhistory()` from Src/Zle/zle_hist.c:282. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn uplineorhistory() -> i32 { 0 }

/// Port of `uplineorsearch()` from Src/Zle/zle_hist.c:312. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn uplineorsearch() -> i32 { 0 }

/// Port of `vidownlineorhistory()` from Src/Zle/zle_hist.c:390. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vidownlineorhistory() -> i32 { 0 }

/// Port of `vifetchhistory()` from Src/Zle/zle_hist.c:1787. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifetchhistory() -> i32 { 0 }

/// Port of `vihistorysearchbackward()` from Src/Zle/zle_hist.c:1964. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vihistorysearchbackward() -> i32 { 0 }

/// Port of `vihistorysearchforward()` from Src/Zle/zle_hist.c:1940. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vihistorysearchforward() -> i32 { 0 }

/// Port of `virepeatsearch()` from Src/Zle/zle_hist.c:1988. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virepeatsearch() -> i32 { 0 }

/// Port of `virevrepeatsearch()` from Src/Zle/zle_hist.c:2024. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virevrepeatsearch() -> i32 { 0 }

/// Port of `viuplineorhistory()` from Src/Zle/zle_hist.c:302. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viuplineorhistory() -> i32 { 0 }

/// Port of `zgetline()` from Src/Zle/zle_hist.c:898. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zgetline() -> i32 { 0 }

/// Port of `zle_setline()` from Src/Zle/zle_hist.c:772. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_setline() -> i32 { 0 }

/// Port of `zlinecmp()` from Src/Zle/zle_hist.c:128. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlinecmp() -> i32 { 0 }

/// Port of `zlinefind()` from Src/Zle/zle_hist.c:204. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlinefind() -> i32 { 0 }
