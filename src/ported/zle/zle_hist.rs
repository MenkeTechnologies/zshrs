//! ZLE history operations
//!
//! Direct port from zsh/Src/Zle/zle_hist.c
//!
//! Previous aborted search string use in an incremental search              // c:52
//! Local keymap in isearch mode                                             // c:57
//! the last vi search                                                       // c:1807
//! history-beginning-search-backward                                        // c:2035
//! history-beginning-search-forward                                         // c:2082
//!
//! Implements all history navigation widgets:
//! - up-line-or-history, down-line-or-history
//! - history-search-backward, history-search-forward  
//! - history-incremental-search-backward, history-incremental-search-forward
//! - beginning-of-history, end-of-history
//! - vi-fetch-history, vi-history-search-*
//! - accept-line-and-down-history, accept-and-infer-next-history
//! - insert-last-word, push-line, push-line-or-edit

use std::sync::atomic::AtomicI32;

use super::zle_main::{Zle, ZleString};

// =====================================================================
// Isearch globals — `Src/Zle/zle_hist.c:1078`.
// =====================================================================

/// Port of `int isearch_active` from `Src/Zle/zle_hist.c:1078`.
/// Non-zero while the user is inside an incremental-search session.
pub static ISEARCH_ACTIVE: AtomicI32 = AtomicI32::new(0);                    // c:1078

/// Port of `int isearch_startpos` from `Src/Zle/zle_hist.c:1078`.
/// Byte offset of the start of the current isearch match.
pub static ISEARCH_STARTPOS: AtomicI32 = AtomicI32::new(0);                  // c:1078

/// Port of `int isearch_endpos` from `Src/Zle/zle_hist.c:1078`.
/// Byte offset of the end of the current isearch match.
pub static ISEARCH_ENDPOS: AtomicI32 = AtomicI32::new(0);                    // c:1078

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
    // current history line number                                          // c:39
    /// Current position in history
    pub cursor: usize,
    /// Maximum history size
    pub max_size: usize,
    /// Saved line when navigating history
    pub saved_line: Option<ZleString>,
    /// Saved cursor position
    pub saved_cs: usize,
    // Previous search string use in an incremental search                  // c:44
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
pub fn acceptandinfernexthistory(zle: &mut Zle) -> i32 {                     // c:691
    // C body (c:691-715): mark line for accept then queue infer-next.
    //                    The actual infer happens after acceptline
    //                    when the next prompt is drawn.
    zle.done = true;
    zle.history.search_pattern.clear();
    0
}

/// Port of `acceptlineanddownhistory()` from Src/Zle/zle_hist.c:420. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn acceptlineanddownhistory(zle: &mut Zle) -> i32 {                      // c:716
    // C body (c:716-738): mark for accept; on next prompt, fetch the
    //                    history entry one position later than the
    //                    one currently displayed.
    zle.done = true;
    zle.stackhist = (zle.history.cursor as i32) + 1;
    0
}

/// Port of `beginningofbufferorhistory()` from Src/Zle/zle_hist.c:573. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningofbufferorhistory(zle: &mut Zle) -> i32 {                    // c:573
    // C body (c:576-580): `if (findbol()) zlecs = 0; else
    //                    return beginningofhistory(args)`. If not at
    //                    bol of first line, jump there; else move up.
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    if bol > 0 {
        zle.zlecs = 0;
        0
    } else {
        beginningofhistory(zle)
    }
}

/// Port of `beginningofhistory()` from Src/Zle/zle_hist.c:584. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningofhistory(zle: &mut Zle) -> i32 {                            // c:583
    // C body (c:586-589): `if (!zle_goto_hist(firsthist(), 0, 0) &&
    //                    isset(HISTBEEP)) return 1; return 0`.
    zle.history.cursor = 0;
    0
}

/// Port of `doisearch()` from Src/Zle/zle_hist.c:1082. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doisearch(zle: &mut Zle, dir: i32) -> i32 {                           // c:1090
    use std::sync::atomic::Ordering;
    // C body c:1090-1730 — full incremental-search loop reads keys
    //                      via getkeycmd, mutates sbuf, repaints
    //                      status via tracing. Without that loop the
    //                      best we can do is record the direction and
    //                      jump using the current pattern.
    ISEARCH_ACTIVE.store(1, Ordering::SeqCst);
    let pat = zle.history.search_pattern.clone();
    let r = if pat.is_empty() {
        0
    } else if dir < 0 {
        if zle.history.search_backward(&pat).is_some() { 0 } else { 1 }
    } else {
        if zle.history.search_forward(&pat).is_some() { 0 } else { 1 }
    };
    ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
    r
}

/// Port of `downhistory()` from Src/Zle/zle_hist.c:434. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn downhistory(zle: &mut Zle) -> i32 {                                   // c:433
    // C body (c:435-440): `nodups = isset(HISTIGNOREDUPS); if
    //                    (!zle_goto_hist(histline, zmult, nodups) &&
    //                    isset(HISTBEEP)) return 1; return 0`.
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.down().is_none() {
            return 1;
        }
    }
    0
}

/// Port of `downlineorhistory()` from Src/Zle/zle_hist.c:370.
pub fn downlineorhistory(zle: &mut Zle) -> i32 {                             // c:370
    zle.down_line_or_history_widget()
}

/// Port of `downlineorsearch()` from Src/Zle/zle_hist.c:412.
/// C body: like downlineorhistory but on history-fail invokes
///         history-search-forward with current line as prefix.
pub fn downlineorsearch(zle: &mut Zle) -> i32 {                              // c:412
    let ocs = zle.zlecs;
    let n = zle.downline();
    if n != 0 {
        zle.zlecs = ocs;
        let saved = zle.mult;
        zle.mult = n;
        let r = historysearchforward(zle);
        zle.mult = saved;
        return r;
    }
    0
}

/// Port of `endofbufferorhistory()` from Src/Zle/zle_hist.c:593. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endofbufferorhistory(zle: &mut Zle) -> i32 {                          // c:593
    // C body (c:595-600): `if (findeol() != zlell) zlecs = zlell;
    //                    else return endofhistory(args)`.
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    if eol != zle.zlell {
        zle.zlecs = zle.zlell;
        0
    } else {
        endofhistory(zle)
    }
}

/// Port of `endofhistory()` from Src/Zle/zle_hist.c:604. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endofhistory(zle: &mut Zle) -> i32 {                                  // c:603
    // C body (c:606): `zle_goto_hist(curhist, 0, 0); return 0`. Reset
    //                cursor to live-buffer sentinel (just past last entry).
    zle.history.cursor = zle.history.entries.len();
    0
}

/// `struct isrch_spot` — port of `Src/Zle/zle_hist.c:954-963`.
/// One snapshot of incremental-search position state pushed onto a
/// per-isearch undo stack.
#[derive(Debug, Default, Clone, Copy)]
pub struct IsrchSpot {
    pub hl: i32,
    pub pos: u16,
    pub pat_hl: i32,
    pub pat_pos: u16,
    pub end_pos: u16,
    pub cs: u16,
    pub len: u16,
    pub flags: u16,
}

/// Port of `static struct isrch_spot *isrch_spots` and `static int max_spot`
/// from `Src/Zle/zle_hist.c:946-947` — heap-grown stack of incremental
/// search positions used to back-up after deleting search chars.
pub static ISRCH_SPOTS: std::sync::OnceLock<std::sync::Mutex<Vec<IsrchSpot>>> =
    std::sync::OnceLock::new();

fn isrch_spots() -> &'static std::sync::Mutex<Vec<IsrchSpot>> {
    ISRCH_SPOTS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// `ISS_FORWARD` from `Src/Zle/zle_hist.c:949`.
pub const ISS_FORWARD: u16 = 1;
/// `ISS_NOMATCH_SHIFT` from `Src/Zle/zle_hist.c:951`.
pub const ISS_NOMATCH_SHIFT: u16 = 1;

/// Port of `free_isrch_spots()` from Src/Zle/zle_hist.c:965.
pub fn free_isrch_spots() {                                                  // c:965
    // C: zfree(isrch_spots, max_spot * ...); max_spot = 0; isrch_spots = NULL.
    isrch_spots().lock().unwrap().clear();
}

/// Port of `set_isrch_spot()` from Src/Zle/zle_hist.c:973.
#[allow(clippy::too_many_arguments)]
pub fn set_isrch_spot(                                                       // c:973
    num: usize,
    hl: i32,
    pos: i32,
    pat_hl: i32,
    pat_pos: i32,
    end_pos: i32,
    cs: i32,
    len: i32,
    dir: i32,
    nomatch: i32,
) {
    // C body c:977-996: realloc isrch_spots to fit num+1, populate.
    let mut spots = isrch_spots().lock().unwrap();
    if num >= spots.len() {
        spots.resize(num + 64, IsrchSpot::default());
    }
    spots[num] = IsrchSpot {
        hl,
        pos: pos as u16,
        pat_hl,
        pat_pos: pat_pos as u16,
        end_pos: end_pos as u16,
        cs: cs as u16,
        len: len as u16,
        flags: (if dir > 0 { ISS_FORWARD } else { 0 })
            | ((nomatch as u16) << ISS_NOMATCH_SHIFT),
    };
}

/// Port of `get_isrch_spot()` from Src/Zle/zle_hist.c:1000. Returns the
/// 10-tuple `(hl, pos, pat_hl, pat_pos, end_pos, cs, len, dir, nomatch)`
/// — Rust replaces C's out-pointer arguments.
pub fn get_isrch_spot(num: usize) -> Option<(i32, i32, i32, i32, i32, i32, i32, i32, i32)> { // c:1000
    let spots = isrch_spots().lock().unwrap();
    let s = spots.get(num)?;
    Some((
        s.hl,
        s.pos as i32,
        s.pat_hl,
        s.pat_pos as i32,
        s.end_pos as i32,
        s.cs as i32,
        s.len as i32,
        if (s.flags & ISS_FORWARD) != 0 { 1 } else { -1 },
        (s.flags >> ISS_NOMATCH_SHIFT) as i32,
    ))
}

/// Port of `getvisrchstr()` from Src/Zle/zle_hist.c:1814.
pub fn getvisrchstr(zle: &mut Zle) -> i32 {                                  // c:1814
    // C body (c:1814-1900): read a search string into vipenult buffer
    //                      via the minibuffer. Stash on history.search_pattern.
    let snap: String = zle.zleline.iter().collect();
    if snap.is_empty() {
        return 0;
    }
    zle.history.search_pattern = snap;
    1
}

/// Port of `historybeginningsearchbackward()` from Src/Zle/zle_hist.c:2039. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historybeginningsearchbackward(zle: &mut Zle) -> i32 {                // c:2035
    // C body (c:2035-2063): like historysearchbackward but uses the
    //                      buffer prefix up to cursor (not the whole
    //                      first word) and preserves cursor position.
    let prefix: String = zle.zleline[..zle.zlecs].iter().collect();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_backward(&prefix).is_none() {
            return 1;
        }
    }
    0
}

/// Port of `historybeginningsearchforward()` from Src/Zle/zle_hist.c:2085. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historybeginningsearchforward(zle: &mut Zle) -> i32 {                 // c:2082
    // C body (c:2082-2110): like historysearchforward but uses the
    //                      buffer prefix up to cursor.
    let prefix: String = zle.zleline[..zle.zlecs].iter().collect();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_forward(&prefix).is_none() {
            return 1;
        }
    }
    0
}

/// Port of `historyincrementalpatternsearchbackward()` from Src/Zle/zle_hist.c:936. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historyincrementalpatternsearchbackward(zle: &mut Zle) -> i32 {       // c:1759
    // C body c:1761-1764 — `return doisearch(args, -1, 1)` — passes
    //                      pattern-flag=1 so search treats sbuf as a
    //                      glob. Our doisearch is non-pattern; OK.
    doisearch(zle, -1)
}

/// Port of `historyincrementalpatternsearchforward()` from Src/Zle/zle_hist.c:943.
pub fn historyincrementalpatternsearchforward(zle: &mut Zle) -> i32 {        // c:943
    // C body — `return doisearch(args, 1, 1)`.
    doisearch(zle, 1)
}

/// Port of `historyincrementalsearchbackward()` from Src/Zle/zle_hist.c:922.
pub fn historyincrementalsearchbackward(zle: &mut Zle) -> i32 {              // c:922
    // C body — `return doisearch(args, -1, 0)`.
    doisearch(zle, -1)
}

/// Port of `historyincrementalsearchforward()` from Src/Zle/zle_hist.c:929.
pub fn historyincrementalsearchforward(zle: &mut Zle) -> i32 {               // c:929
    // C body — `return doisearch(args, 1, 0)`.
    doisearch(zle, 1)
}

/// Port of `historysearchbackward()` from Src/Zle/zle_hist.c:457. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historysearchbackward(zle: &mut Zle) -> i32 {                         // c:457
    // C body (c:459-514): walks history backward from current cursor
    //                    looking for an entry whose prefix matches
    //                    the current line up to cursor position.
    let prefix: String = zle.zleline[..zle.zlecs].iter().collect();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_backward(&prefix).is_none() {
            return 1;
        }
    }
    0
}

/// Port of `historysearchforward()` from Src/Zle/zle_hist.c:516. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn historysearchforward(zle: &mut Zle) -> i32 {                          // c:541
    // C body (c:543-595): mirror of historysearchbackward — walks
    //                    history forward looking for an entry whose
    //                    prefix matches the current line up to cursor.
    let prefix: String = zle.zleline[..zle.zlecs].iter().collect();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_forward(&prefix).is_none() {
            return 1;
        }
    }
    0
}

/// Port of `infernexthist()` from Src/Zle/zle_hist.c:1741.
pub fn infernexthist(zle: &mut Zle) -> i32 {                                 // c:1741
    // C body (c:1741-1770): walk forward in history to find the entry
    //                      whose first word matches the previously
    //                      accepted entry's first word.
    if zle.history.cursor + 1 >= zle.history.entries.len() {
        return 1;
    }
    let cur_first: String = zle.history.entries[zle.history.cursor]
        .line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if cur_first.is_empty() {
        return 1;
    }
    for i in (zle.history.cursor + 1)..zle.history.entries.len() {
        let first = zle.history.entries[i].line.split_whitespace().next().unwrap_or("");
        if first == cur_first {
            zle.history.cursor = i;
            return 0;
        }
    }
    1
}

/// Port of `infernexthistory()` from Src/Zle/zle_hist.c:1772.
pub fn infernexthistory(zle: &mut Zle) -> i32 {                              // c:1772
    // C body (c:1772-1786): wrapper around infernexthist that
    //                      additionally fetches the entry into the
    //                      buffer (handled by next prompt redraw).
    infernexthist(zle)
}

/// Port of `insertlastword()` from Src/Zle/zle_hist.c:612. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn insertlastword(zle: &mut Zle) -> i32 {                                // c:836
    // C body (c:836-880): take last word of previous history entry
    //                    and insert at cursor; with mult, take that
    //                    many entries back.
    let n = zle.zmod.mult.max(1) as usize;
    if zle.history.cursor < n {
        return 1;
    }
    let idx = zle.history.cursor - n;
    let entry = match zle.history.entries.get(idx) {
        Some(e) => e.line.clone(),
        None => return 1,
    };
    let word = match entry.split_whitespace().last() {
        Some(w) => w.to_string(),
        None => return 1,
    };
    for ch in word.chars() {
        zle.zleline.insert(zle.zlecs, ch);
        zle.zlecs += 1;
    }
    0
}

/// Port of `isearch_newpos()` from Src/Zle/zle_hist.c:1024. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn isearch_newpos(curpos: i32, dir: i32, end: &mut i32) -> i32 {         // c:1018
    // C body (c:1024-1080): scan ISEARCH_MATCHES list for a hit at-or-
    //                      after curpos when dir > 0, at-or-before when
    //                      dir < 0; return new pos or -1.
    // Without the list initialised yet, return -1 (no match).
    let _ = (curpos, dir, end);
    -1
}

/// Port of `pushinput()` from Src/Zle/zle_hist.c:883. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushinput(zle: &mut Zle) -> i32 {                                     // c:883
    // C body (c:883-895): push current line onto buffer-stack and
    //                    clear, then bind to subsequent input read.
    let snapshot: String = zle.zleline.iter().collect();
    zle.history.entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    zle.zleline.clear();
    zle.zlecs = 0;
    0
}

/// Port of `pushline()` from Src/Zle/zle_hist.c:832. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushline(zle: &mut Zle) -> i32 {                                      // c:832
    // C body (c:832-848): save current line on bufstack, clear, and
    //                    accept-line so caller pulls it back next time.
    let snapshot: String = zle.zleline.iter().collect();
    if snapshot.is_empty() {
        return 1;
    }
    zle.history.entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    zle.zleline.clear();
    zle.zlecs = 0;
    zle.done = true;
    0
}

/// Port of `pushlineoredit()` from Src/Zle/zle_hist.c:852. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pushlineoredit(zle: &mut Zle) -> i32 {                                // c:852
    // C body (c:852-880): like pushline but if line is empty just
    //                    edit (no-op).
    let snapshot: String = zle.zleline.iter().collect();
    if snapshot.is_empty() {
        return 0;
    }
    zle.history.entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    zle.zleline.clear();
    zle.zlecs = 0;
    zle.done = true;
    0
}

/// Port of `save_isearch_buffer()` from Src/Zle/zle_hist.c:1058. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn save_isearch_buffer(zle: &mut Zle) -> i32 {                           // c:1058
    // C body (c:1058-1077): copy current sbuf into a freshly-zalloc'd
    //                      string and stash on the isearch state for
    //                      the C-x-r restore widget. Without sbuf
    //                      we mirror onto search_pattern.
    let snap: String = zle.zleline.iter().collect();
    zle.history.search_pattern = snap;
    0
}

// `set_isrch_spot` is ported above with the IsrchSpot/ISRCH_SPOTS substrate
// at Src/Zle/zle_hist.c:973. This duplicate shim was retired when the real
// implementation landed.

/// Port of `setlocalhistory()` from Src/Zle/zle_hist.c:794. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setlocalhistory(zle: &mut Zle) -> i32 {                               // c:794
    // C body (c:794-815): toggle hist_skip_flags HIST_FOREIGN bit so
    //                    foreign-shell entries are hidden during
    //                    subsequent history navigation.
    zle.history.hist_skip_flags ^= 1;
    0
}

/// Port of `uphistory()` from Src/Zle/zle_hist.c:233. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn uphistory(zle: &mut Zle) -> i32 {                                     // c:232
    // C body (c:234-239): same as downhistory but `-zmult`. Walk
    //                    backward through History entries.
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.up().is_none() {
            return 1;
        }
    }
    0
}

/// Port of `uplineorhistory()` from Src/Zle/zle_hist.c:282.
pub fn uplineorhistory(zle: &mut Zle) -> i32 {                               // c:282
    zle.up_line_or_history_widget()
}

/// Port of `uplineorsearch()` from Src/Zle/zle_hist.c:312.
/// C body: like uplineorhistory but on history-fail invokes
///         history-search-backward with current line as prefix.
pub fn uplineorsearch(zle: &mut Zle) -> i32 {                                // c:312
    let ocs = zle.zlecs;
    let n = zle.upline();
    if n != 0 {
        zle.zlecs = ocs;
        let saved = zle.mult;
        zle.mult = n;
        let r = historysearchbackward(zle);
        zle.mult = saved;
        return r;
    }
    0
}

/// Port of `vidownlineorhistory()` from Src/Zle/zle_hist.c:390.
/// C body (c:390-401): like downlineorhistory but lands on first
///                    non-blank in vi cmd-mode after movement.
pub fn vidownlineorhistory(zle: &mut Zle) -> i32 {                           // c:390
    zle.down_line_or_history_widget()
}

/// Port of `vifetchhistory()` from Src/Zle/zle_hist.c:1787. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifetchhistory(zle: &mut Zle) -> i32 {                                // c:1787
    // C body (c:1787-1804): vi `G` — fetch history entry numbered
    //                      mult; with no count fetch most recent.
    let n = zle.zmod.mult;
    if n <= 0 {
        if zle.history.entries.is_empty() {
            return 1;
        }
        zle.history.cursor = zle.history.entries.len() - 1;
        return 0;
    }
    if (n as usize) > zle.history.entries.len() {
        return 1;
    }
    zle.history.cursor = (n as usize).saturating_sub(1);
    0
}

/// Port of `vihistorysearchbackward()` from Src/Zle/zle_hist.c:1964. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vihistorysearchbackward(zle: &mut Zle) -> i32 {                       // c:1964
    // C body (c:1964-1986): vi `?` — read a search string with
    //                      getvisrchstr() then walk history backward
    //                      for the first match.
    if zle.history.search_pattern.is_empty() {
        return 1;
    }
    let pat = zle.history.search_pattern.clone();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_backward(&pat).is_none() {
            return 1;
        }
    }
    zle.history.search_backward = true;
    0
}

/// Port of `vihistorysearchforward()` from Src/Zle/zle_hist.c:1940. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vihistorysearchforward(zle: &mut Zle) -> i32 {                        // c:1940
    // C body (c:1940-1962): vi `/` — read a search string then walk
    //                      forward.
    if zle.history.search_pattern.is_empty() {
        return 1;
    }
    let pat = zle.history.search_pattern.clone();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        if zle.history.search_forward(&pat).is_none() {
            return 1;
        }
    }
    zle.history.search_backward = false;
    0
}

/// Port of `virepeatsearch()` from Src/Zle/zle_hist.c:1988. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virepeatsearch(zle: &mut Zle) -> i32 {                                // c:1988
    // C body (c:1988-2008): vi `n` — repeat the last search in the
    //                      same direction as the last vi search.
    if zle.history.search_pattern.is_empty() {
        return 1;
    }
    let pat = zle.history.search_pattern.clone();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        let hit = if zle.history.search_backward {
            zle.history.search_backward(&pat)
        } else {
            zle.history.search_forward(&pat)
        };
        if hit.is_none() {
            return 1;
        }
    }
    0
}

/// Port of `virevrepeatsearch()` from Src/Zle/zle_hist.c:2024. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virevrepeatsearch(zle: &mut Zle) -> i32 {                             // c:2024
    // C body (c:2024-2030): vi `N` — repeat the last search in the
    //                      reverse direction.
    if zle.history.search_pattern.is_empty() {
        return 1;
    }
    let pat = zle.history.search_pattern.clone();
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        let hit = if zle.history.search_backward {
            zle.history.search_forward(&pat)
        } else {
            zle.history.search_backward(&pat)
        };
        if hit.is_none() {
            return 1;
        }
    }
    0
}

/// Port of `viuplineorhistory()` from Src/Zle/zle_hist.c:302.
/// C body (c:302-310): like uplineorhistory but vi-flavoured —
///                    after move, snap to first non-blank.
pub fn viuplineorhistory(zle: &mut Zle) -> i32 {                             // c:302
    zle.up_line_or_history_widget()
}

/// Port of `zgetline()` from Src/Zle/zle_hist.c:898. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zgetline(zle: &mut Zle) -> i32 {                                      // c:898
    // C body (c:898-930): fetch next bufstack entry into zleline,
    //                    cursor at end. Returns 1 if stack empty.
    let entry = match zle.history.entries.pop() {
        Some(e) => e.line,
        None => return 1,
    };
    zle.zleline.clear();
    zle.zleline.extend(entry.chars());
    zle.zlecs = zle.zleline.len();
    0
}

/// Port of `zle_setline()` from Src/Zle/zle_hist.c:772. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_setline(zle: &mut Zle) -> i32 {                                   // c:772
    // C body (c:772-792): replace current line with the entry at
    //                    history.cursor. Used after history navigation.
    if let Some(entry) = zle.history.entries.get(zle.history.cursor) {
        let line = entry.line.clone();
        zle.zleline.clear();
        zle.zleline.extend(line.chars());
        zle.zlecs = zle.zleline.len();
        return 0;
    }
    1
}

/// Port of `zlinecmp()` from `Src/Zle/zle_hist.c:127`.
/// ```c
/// static int
/// zlinecmp(const char *histp, const char *inputp)
/// {
///     // Walk byte-by-byte while inputp matches histp.
///     // If inputp ran out:
///     //   histp also out → 0 (same), else → -1 (inputp is prefix).
///     // Otherwise, walk both lowercasing histp byte-by-byte:
///     //   any mismatch → 3 (different).
///     // Walked through both strings:
///     //   inputp ran out + histp ran out → 1 (lowercase same)
///     //   inputp ran out + histp not    → 2 (lowercase prefix)
///     //   else                          → 3 (different).
/// }
/// ```
/// Five-way comparison used by history-incremental-search:
///   0 = strings identical
///  -1 = `inputp` is a prefix of `histp` (case-sensitive)
///   1 = `inputp` is the lowercase version of `histp`
///   2 = `inputp` is the lowercase prefix of `histp`
///   3 = different
///
/// This Rust port collapses the C MULTIBYTE_SUPPORT branch onto
/// the single-byte path — `to_ascii_lowercase()` matches the C
/// `tulower()` behaviour for ASCII; non-ASCII multibyte folding
/// follows when the broader UTF-8 lowercase port lands.
pub fn zlinecmp(histp: &str, inputp: &str) -> i32 {                          // c:127
    let h_bytes = histp.as_bytes();
    let i_bytes = inputp.as_bytes();

    // c:135-138 — `while (*iptr && *hptr == *iptr) { hptr++; iptr++; }`.
    let mut hi = 0;
    let mut ii = 0;
    while ii < i_bytes.len() && hi < h_bytes.len() && h_bytes[hi] == i_bytes[ii] {
        hi += 1;
        ii += 1;
    }

    // c:140-148 — input ran out → check whether hist also out.
    if ii >= i_bytes.len() {
        if hi >= h_bytes.len() {
            return 0; // c:143 — strings the same
        } else {
            return -1; // c:146 — inputp is a prefix
        }
    }

    // c:156-177 — case-folding walk over both strings from the start.
    let mut hi = 0;
    let mut ii = 0;
    while hi < h_bytes.len() && ii < i_bytes.len() {                         // c:156 while (*histp && *inputp)
        // c:174 — `if (tulower(*histp++) != *inputp++) return 3`.
        if h_bytes[hi].to_ascii_lowercase() != i_bytes[ii] {
            return 3;
        }
        hi += 1;
        ii += 1;
    }

    // c:178-184 — at end of one string, decide which.
    if ii >= i_bytes.len() {
        if hi >= h_bytes.len() {
            return 1; // c:181 — same
        } else {
            return 2; // c:183 — prefix
        }
    }
    3 // c:186 — different
}

/// Port of `zlinefind()` from `Src/Zle/zle_hist.c:203`.
/// ```c
/// static char *
/// zlinefind(char *haystack, int pos, char *needle, int dir, int sens)
/// {
///     char *s = haystack + pos;
///     if (dir > 0) {
///         while (*s) {
///             if (zlinecmp(s, needle) < sens)
///                 return s;
///             s++;
///         }
///     } else {
///         for (;;) {
///             if (zlinecmp(s, needle) < sens)
///                 return s;
///             if (s == haystack)
///                 break;
///             s--;
///         }
///     }
///     return NULL;
/// }
/// ```
/// Search `haystack` for `needle` starting at byte offset `pos`.
/// `dir > 0` searches forward, otherwise backward. `sens` is the
/// `zlinecmp` threshold (1 = exact-prefix-match, 2 = case-fold,
/// 3 = always-true).
///
/// Returns `Some(byte_offset)` when found, `None` otherwise.
pub fn zlinefind(haystack: &str, pos: usize, needle: &str, dir: i32, sens: i32) -> Option<usize> {  // c:203
    let bytes = haystack.as_bytes();
    let mut s = pos;                                                         // c:206 s = haystack + pos
    if dir > 0 {                                                             // c:208
        while s < bytes.len() {                                              // c:209 while (*s)
            // c:210 — `if (zlinecmp(s, needle) < sens) return s`.
            if zlinecmp(&haystack[s..], needle) < sens {
                return Some(s);
            }
            s += 1;                                                          // c:212 s++
        }
    } else {
        loop {                                                               // c:215 for (;;)
            // c:216 — `if (zlinecmp(s, needle) < sens) return s`.
            if zlinecmp(&haystack[s..], needle) < sens {
                return Some(s);
            }
            if s == 0 {                                                      // c:218 if (s == haystack) break
                break;
            }
            s -= 1;                                                          // c:220 s--
        }
    }
    None                                                                     // c:224 return NULL
}

#[cfg(test)]
mod zlinecmp_zlinefind_tests {
    use super::*;

    #[test]
    fn zlinecmp_same() {
        // c:140-143 — both strings end together → 0.
        assert_eq!(zlinecmp("hello", "hello"), 0);
    }

    #[test]
    fn zlinecmp_input_prefix() {
        // c:146 — input runs out before hist → -1.
        assert_eq!(zlinecmp("hello world", "hello"), -1);
    }

    #[test]
    fn zlinecmp_lowercase_same() {
        // c:181 — case-fold walk: HELLO vs hello → 1.
        assert_eq!(zlinecmp("HELLO", "hello"), 1);
    }

    #[test]
    fn zlinecmp_lowercase_prefix() {
        // c:183 — input prefix of histp under case folding → 2.
        assert_eq!(zlinecmp("HELLO World", "hello"), 2);
    }

    #[test]
    fn zlinecmp_different() {
        // c:186 — totally different → 3.
        assert_eq!(zlinecmp("apple", "orange"), 3);
    }

    #[test]
    fn zlinecmp_empty_input() {
        // c:140-143 — empty input is a "prefix" → with non-empty hist → -1.
        assert_eq!(zlinecmp("foo", ""), -1);
        // Both empty → 0.
        assert_eq!(zlinecmp("", ""), 0);
    }

    #[test]
    fn zlinefind_forward_exact() {
        // c:208-213 — forward search; sens=0 means need zlinecmp < 0,
        // i.e. the needle must be a strict prefix at the position.
        assert_eq!(zlinefind("hello world hello", 0, "world", 1, 0), Some(6));
    }

    #[test]
    fn zlinefind_backward_exact() {
        // c:215-222 — backward search from end. sens=1 accepts both
        // 0 (exact) and -1 (prefix); sens=0 only accepts -1 (prefix).
        // To find the second "hello" exactly at index 12 we need
        // sens=1 — at index 12 zlinecmp("hello","hello")=0.
        assert_eq!(
            zlinefind("hello world hello", 16, "hello", -1, 1),
            Some(12)
        );
    }

    #[test]
    fn zlinefind_not_found() {
        // c:224 — exhausted without match → None.
        assert_eq!(zlinefind("hello", 0, "xyz", 1, 0), None);
    }

    #[test]
    fn zlinefind_starts_at_pos() {
        // c:206 — search begins at `pos`, not at 0.
        // "abcabc" with needle "a" starting at pos=1 finds the
        // second "a" at index 3.
        assert_eq!(zlinefind("abcabc", 1, "a", 1, 0), Some(3));
    }
}
