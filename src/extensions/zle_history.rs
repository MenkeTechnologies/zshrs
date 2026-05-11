//! Per-session history state container used by ZLE widgets.
//!
//! **Rust-only extension. NOT a port of any zsh C struct.**
//!
//! zsh's C source spreads ZLE history state across file-scope
//! globals: `hist_ring` (Src/hist.c:103) for the entries themselves,
//! `histline` for the cursor index, `searchstr` for the last search
//! pattern, `havesusend`/`have_edits`/etc. for transient flags.
//! zshrs's port collects the per-session subset that the ZLE widgets
//! drive into one container so a `&mut Zle` can hold it directly.
//!
//! The eventual unification: drop `History.entries` entirely and have
//! every widget read from `crate::ported::hist::hist_ring`. The
//! cursor / saved_line / search state would remain as file-scope
//! statics matching zsh's globals. Until that lands, this Vec is a
//! dual-state convenience that the test harness can populate without
//! threading hist.rs init through every test.

use crate::ported::zle::zle_main::ZleString;

/// Single history entry — the Rust-only subset the ZLE widgets need
/// (line text, event number, optional time). Maps loosely to fields
/// from `struct histent` (Src/zsh.h:2234): `node.nam` ↔ `line`,
/// `histnum` ↔ `num`, `stim` ↔ `time`.
#[derive(Debug, Clone)]
pub struct HistEntry {
    /// The command line.
    pub line: String,
    /// Event number (1-based; mirrors `histent.histnum`).
    pub num: i64,
    /// Insertion time (Unix epoch seconds; mirrors `histent.stim`).
    pub time: Option<i64>,
}

/// Per-session ZLE history state — entries + cursor + search state.
/// **NOT** a port of any zsh C struct (zsh uses file-scope globals).
#[derive(Debug, Default)]
pub struct History {
    /// History entries (newest last).
    pub entries: Vec<HistEntry>,
    /// Current position in history (mirrors `histline`).
    pub cursor: usize,
    /// Maximum history size (mirrors `histsiz`).
    pub max_size: usize,
    /// Saved line when navigating history (mirrors the C `zle_text`
    /// shadow on `Histent`).
    pub saved_line: Option<ZleString>,
    /// Saved cursor position pre-navigation.
    pub saved_cs: usize,
    /// Previous search string. Mirrors `searchstr`
    /// (Src/Zle/zle_hist.c:44).
    pub search_pattern: String,
    /// Last search direction (true = backward).
    pub search_backward: bool,
    /// Originals of edited entries: when `remember_edits` mutates
    /// `entries[i].line`, the pre-edit text lands here at index `i`.
    /// `forget_edits` restores them. Mirrors the C `Histent->zle_text`
    /// shadow string + the global `have_edits` flag in
    /// Src/Zle/zle_hist.c.
    pub originals: Vec<Option<String>>,
    /// True if any entry has a recorded original — mirrors
    /// `have_edits` in Src/Zle/zle_hist.c:76.
    pub have_edits: bool,
    /// History skip-flags state. Bit-equivalent of zsh's
    /// `hist_skip_flags` in Src/Zle/zle_hist.c:794: `HIST_FOREIGN` (1)
    /// hides entries from other sessions when set; `setlocalhistory`
    /// toggles this.
    pub hist_skip_flags: u32,
}

impl History {
    /// Construct an empty history with a max-entry cap. Mirrors the
    /// role of `inithist()` from Src/hist.c:1717 (which sizes the
    /// global `hist_ring` at `$HISTSIZE`).
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

    /// Append a new entry. Mirrors `addhistnode()` from Src/hist.c
    /// (the inner add path invoked by `addhistline`/`hend`). Skips
    /// empty input and consecutive-duplicate lines (same as zsh's
    /// HIST_IGNORE_DUPS default). Trims from the front when over
    /// `max_size`.
    pub fn add(&mut self, line: String) {
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

        while self.entries.len() > self.max_size {
            self.entries.remove(0);
        }

        self.cursor = self.entries.len();
    }

    /// Look up the entry at a specific 0-based index. Mirrors
    /// `quietgethist()` from Src/Zle/zle_hist.c:1712 (event-number
    /// fetch); our entries Vec is 0-indexed so callers convert
    /// num→index themselves.
    pub fn get(&self, index: usize) -> Option<&HistEntry> {
        self.entries.get(index)
    }

    /// Step the cursor one position older. Equivalent to the
    /// cursor-decrement path of `zle_goto_hist()` at
    /// Src/Zle/zle_hist.c:805 with n=-1.
    pub fn up(&mut self) -> Option<&HistEntry> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Step the cursor one position newer. Mirrors
    /// `zle_goto_hist()` at Src/Zle/zle_hist.c:805 with n=+1.
    pub fn down(&mut self) -> Option<&HistEntry> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Search history for the most recent entry containing `pattern`.
    /// Substring-match equivalent of `historysearchbackward()` from
    /// Src/Zle/zle_hist.c:484 (without the glob/HIST_PATTERN branch).
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

    /// Search history forward for the next entry containing
    /// `pattern`. Mirror of `search_backward` against
    /// `historysearchforward()` at Src/Zle/zle_hist.c:541.
    pub fn search_forward(&mut self, pattern: &str) -> Option<&HistEntry> {
        for i in (self.cursor + 1)..self.entries.len() {
            if self.entries[i].line.contains(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }
        None
    }

    /// Reset the cursor to the live-buffer sentinel position and
    /// drop any saved pre-navigation line. Mirrors the
    /// `histline = curhist; saved_line = NULL` reset path invoked by
    /// `endofhistory()` (Src/Zle/zle_hist.c:478) and after
    /// accept-line.
    pub fn reset(&mut self) {
        self.cursor = self.entries.len();
        self.saved_line = None;
    }
}
