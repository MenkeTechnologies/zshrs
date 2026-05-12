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
//! - transpose-chars, bslashquote-line, bslashquote-region
//! - what-cursor-position, universal-argument, digit-argument
//! - undefined-key, send-break
//! - vi-put-after, vi-put-before, overwrite-mode

use std::sync::atomic::AtomicI32;
use crate::zsh_h::isset;
use super::zle_main::Zle;

// =====================================================================
// Globals — `Src/Zle/zle_main.c:79-84` (live in zle_main but consumed
// by widgets in zle_misc).
// =====================================================================

/// Port of `int done` from `Src/Zle/zle_main.c:79`. Non-zero when
/// the editor session should terminate (`accept-line`,
/// `accept-and-hold`, `accept-line-and-down-history`, etc.).
pub static DONE: AtomicI32 = AtomicI32::new(0);                              // c:79

/// Port of `int mark` from `Src/Zle/zle_main.c:84`. Saved cursor
/// position for the region (set by `set-mark-command`, consumed by
/// `kill-region`, `copy-region-as-kill`, `regionlines`, etc.).
pub static MARK: AtomicI32 = AtomicI32::new(0);                              // c:84

/// Port of `mod_export int suffixlen` from `Src/Zle/zle_misc.c:1553`.
/// Length of the currently active, auto-removable suffix.
pub static SUFFIXLEN: AtomicI32 = AtomicI32::new(0);                         // c:1553

/// Port of `struct suffixset` from `Src/Zle/zle_misc.c`. One node
/// in the auto-removable suffix list.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct suffixset {                                                       // c:1530
    /// Type bits (SUFTYP_POSSTR/POSRNG/etc.).
    pub tp: i32,
    /// Flag bits (SUFFLAGS_SPACE etc.).
    pub flags: i32,
    /// Characters to match (for *STR types).
    pub chars: Vec<char>,
    /// Length of `chars`.
    pub lenstr: i32,
    /// Suffix length to remove on insert.
    pub lensuf: i32,
}

/// Port of `struct suffixset *suffixlist` from `Src/Zle/zle_misc.c`.
/// Stack of registered auto-removable suffixes.
pub static SUFFIXLIST: std::sync::OnceLock<std::sync::Mutex<Vec<suffixset>>>
    = std::sync::OnceLock::new();

fn suffixlist() -> &'static std::sync::Mutex<Vec<suffixset>> {
    SUFFIXLIST.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Port of `int suffixnoinsrem` from `Src/Zle/zle_misc.c:1549`.
/// Suppresses inserted-character suffix removal when set.
pub static SUFFIXNOINSREM: AtomicI32 = AtomicI32::new(0);                    // c:1549

/// Port of `static ZLE_INT_T vfindchar` from `Src/Zle/zle_move.c:734`.
/// The character argument to the most recent vi-find* command.
pub static VFINDCHAR: AtomicI32 = AtomicI32::new(0);                         // c:734

/// Port of `static int vfinddir, tailadd` from `Src/Zle/zle_move.c:735`.
/// vfinddir = +1 forward, -1 backward; tailadd = +1 land just after,
/// -1 land just before, 0 land on the char itself.
pub static VFINDDIR: AtomicI32 = AtomicI32::new(0);                          // c:735
pub static TAILADD:  AtomicI32 = AtomicI32::new(0);                          // c:735

/// Port of `static int kct` from `Src/Zle/zle_misc.c:523`. Index into
/// the kill ring for the next yank-pop, or -1 for the original cutbuf
/// at the start of a yank sequence.
pub static KCT:    AtomicI32 = AtomicI32::new(-1);                           // c:523

/// Port of `static int yankcs` from `Src/Zle/zle_misc.c:523`. Saved
/// cursor position at the start of the most-recent yank — `yank-pop`
/// rewinds to this and re-inserts the next ring entry.
pub static YANKCS: AtomicI32 = AtomicI32::new(0);                            // c:523

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1231`.
/// Length of the longest unambiguous prefix among all matched
/// `namedcmd` widget names — drives `execute-named-command` ambig
/// resolution. Mirrored on `NamedCmdState.namedcmdambig` already;
/// this is the searchable counterpart.
pub static NAMEDCMDAMBIG: AtomicI32 = AtomicI32::new(0);                     // c:1231

// ===== Pre/post-display strings (Src/Zle/zle_main.c) =====
//
// `ZLE_STRING_T predisplay` / `ZLE_STRING_T postdisplay` — text
// shown before/after the line buffer (used by `zle -K -P` and
// completion menu rendering).

/// Port of `ZLE_STRING_T predisplay` (zle_main.c). Storage for the
/// `$PREDISPLAY` parameter value.
pub static PREDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `ZLE_STRING_T postdisplay` (zle_main.c). Storage for the
/// `$POSTDISPLAY` parameter value.
pub static POSTDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `char *previous_search` from `Src/Zle/zle_hist.c`. Set
/// by `historyincrementalsearch*` on accept; read by `$LSEARCH`.
pub static PREVIOUS_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `char *previous_aborted_search` from
/// `Src/Zle/zle_hist.c`. Set on isearch abort; read by `$LASEARCH`.
pub static PREVIOUS_ABORTED_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

// `PasteBuffer` deleted — Rust-invented struct that wasn't referenced
// anywhere. The C source uses `Cutbuffer` (zle.h:342, ported as
// `cutbuffer` in zle_h.rs:506) and the `cutbuf` global to back yank
// operations; no separate paste-buffer type exists.

impl Zle {
    // insert a zle string, with repetition and suffix removal              // c:33

    /// Self insert - insert the typed character
    /// Port of selfinsert(args) from zle_misc.c
    pub fn self_insert(&mut self, c: char) {                                 // c:113
        self.zleline.insert(self.zlecs, c);
        self.zlecs += 1;
        self.zlell += 1;
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Self insert unmeta - insert character with meta bit stripped
    /// Port of selfinsertunmeta(args) from zle_misc.c
    pub fn self_insert_unmeta(&mut self, c: char) {                          // c:149
        let unmetaed = if (c as u32) >= 0x80 && (c as u32) < 0x100 {
            char::from_u32((c as u32) & 0x7f).unwrap_or(c)
        } else {
            c
        };
        self.self_insert(unmetaed);
    }

    /// Accept line - return the current line for execution
    /// Port of acceptline(args) from zle_misc.c
    pub fn accept_line(&self) -> String {                                    // c:401
        self.zleline.iter().collect()
    }

    /// Accept and hold - accept line but keep it in the buffer
    /// Port of acceptandhold(args) from zle_misc.c
    pub fn accept_and_hold(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Quoted insert - insert next char literally
    /// Port of quotedinsert(args) from zle_misc.c
    pub fn quoted_insert(&mut self, c: char) {
        self.zleline.insert(self.zlecs, c);
        self.zlecs += 1;
        self.zlell += 1;
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Bracketed paste - handle paste mode
    /// Port of bracketedpaste(args) from zle_misc.c
    pub fn bracketed_paste(&mut self, text: &str) {
        for c in text.chars() {
            if c != '\x1b' {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
                self.zlell += 1;
            }
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Delete char under cursor
    /// Port of deletechar(args) from zle_misc.c
    pub fn delete_char(&mut self) {                                          // c:157
        if self.zlecs < self.zlell {
            self.zleline.remove(self.zlecs);
            self.zlell -= 1;
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Delete char before cursor
    /// Port of backwarddeletechar(args) from zle_misc.c
    pub fn backward_delete_char(&mut self) {
        if self.zlecs > 0 {
            self.zlecs -= 1;
            self.zleline.remove(self.zlecs);
            self.zlell -= 1;
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Kill from cursor to end of line
    /// Port of killline(args) from zle_misc.c
    pub fn kill_line(&mut self) {                                            // c:419
        if self.zlecs < self.zlell {
            let text: Vec<char> = self.zleline.drain(self.zlecs..).collect();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
            self.zlell = self.zlecs;
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Kill from beginning of line to cursor
    /// Port of backwardkillline(args) from zle_misc.c
    pub fn backward_kill_line(&mut self) {
        if self.zlecs > 0 {
            let text: Vec<char> = self.zleline.drain(..self.zlecs).collect();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
            self.zlell -= self.zlecs;
            self.zlecs = 0;
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Kill entire buffer
    /// Port of killbuffer(args) from zle_misc.c
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
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Kill whole line (including newlines in multi-line mode)
    /// Port of killwholeline(args) from zle_misc.c
    pub fn kill_whole_line(&mut self) {
        self.kill_buffer();
    }

    /// Swap cursor and mark.
    /// Port of `exchangepointandmark(args)` from Src/Zle/zle_move.c:496. The
    /// C source has additional zmult-based behaviour (zmult==0 just
    /// activates the region without swapping; zmult>0 also activates).
    /// This bare method only swaps; the widget-level
    /// `widget_exchange_point_and_mark` honours the count semantics.
    pub fn exchange_point_and_mark(&mut self) {
        std::mem::swap(&mut self.zlecs, &mut self.mark);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Set mark at the current cursor position.
    /// Port of `setmarkcommand(args)` from Src/Zle/zle_move.c:483 with the
    /// activate-region branch elided. The widget-level
    /// `widget_set_mark_command` covers the negative-count
    /// deactivate path that the bare C source supports.
    pub fn set_mark_here(&mut self) {
        self.mark = self.zlecs;
    }

    /// Copy region as kill
    /// Port of copyregionaskill(args) from zle_misc.c
    pub fn copy_region_as_kill(&mut self) {                                  // c:494
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
    /// Port of killregion(args) from zle_misc.c
    pub fn kill_region(&mut self) {                                          // c:463
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
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Yank - insert from kill ring
    /// Port of yank(args) from zle_misc.c
    pub fn yank(&mut self) {                                                 // c:533
        if let Some(text) = self.killring.front() {
            self.mark = self.zlecs;
            for &c in text {
                self.zleline.insert(self.zlecs, c);
                self.zlecs += 1;
            }
            self.zlell = self.zleline.len();
            self.yanklast = true;
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Yank pop - cycle through kill ring
    /// Port of yankpop(args) from zle_misc.c
    pub fn yank_pop(&mut self) {                                             // c:728
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

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Transpose chars
    /// Port of transposechars(args) from zle_misc.c
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
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Capitalize the next word: title-case the first letter, lowercase
    /// the rest of the word.
    /// Port of `capitalizeword(args)` from Src/Zle/zle_word.c (the C source
    /// uses `casemodifyword()` with a CASMOD_CAPS flag). Mirrors emacs's
    /// M-c convention. Cursor lands past the modified word.
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

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Lowercase the next word.
    /// Port of `downcaseword(args)` from Src/Zle/zle_word.c — calls
    /// `casemodifyword()` with the CASMOD_LOWER flag.
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

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Uppercase the next word.
    /// Port of `upcaseword(args)` from Src/Zle/zle_word.c — calls
    /// `casemodifyword()` with the CASMOD_UPPER flag.
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

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
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
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Quote line
    /// Port of quoteline(args) from zle_misc.c
    pub fn quote_line(&mut self) {
        self.zleline.insert(0, '\'');
        self.zlell += 1;
        self.zlecs += 1;
        self.zleline.push('\'');
        self.zlell += 1;
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Quote region
    /// Port of quoteregion(args) from zle_misc.c
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
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// What cursor position - display cursor info
    /// Port of whatcursorposition(args) from zle_misc.c
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
            (self.zlecs * 100).checked_div(self.zlell).unwrap_or(0)
        )
    }

    /// Universal argument - multiply next command
    /// Port of universalargument(args) from zle_misc.c
    pub fn universal_argument(&mut self) {
        self.mult = self.mult.saturating_mul(4);
    }

    /// Digit argument - accumulate numeric argument
    /// Port of digitargument(args) from zle_misc.c
    pub fn digit_argument(&mut self, digit: u8) {
        if self.mult == 1 && !self.neg_arg {
            self.mult = 0;
        }
        self.mult = self.mult.saturating_mul(10).saturating_add(digit as i32);
    }

    /// Negative argument
    /// Port of negargument(args) from zle_misc.c
    pub fn neg_argument(&mut self) {
        self.neg_arg = !self.neg_arg;
    }

    /// Undefined key - beep
    /// Port of undefinedkey(args) from zle_misc.c
    pub fn undefined_key(&self) {
        print!("\x07"); // Bell
    }

    /// Send break - abort current operation
    /// Port of sendbreak(args) from zle_misc.c
    pub fn send_break(&mut self) {
        self.zleline.clear();
        self.zlell = 0;
        self.zlecs = 0;
        self.mark = 0;
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Vi put after cursor
    /// Port of viputafter(args) from zle_misc.c
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
    /// Port of viputbefore(args) from zle_misc.c
    pub fn vi_put_before(&mut self) {
        self.yank();
    }

    /// Overwrite mode toggle
    /// Port of overwritemode(args) from zle_misc.c
    pub fn overwrite_mode(&mut self) {
        crate::ported::zle::zle_main::INSMODE.fetch_xor(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Copy previous word
    /// Port of copyprevword(args) from zle_misc.c
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
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Copy previous shell word (respects quoting)
    /// Port of copyprevshellword(args) from zle_misc.c
    pub fn copy_prev_shell_word(&mut self) {
        // Simplified - doesn't handle full shell quoting
        self.copy_prev_word();
    }

    /// Pound insert - comment toggle for vi mode
    /// Port of poundinsert(args) from zle_misc.c
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
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Port of `acceptandhold(args)` from Src/Zle/zle_misc.c:409.
pub fn acceptandhold(zle: &mut Zle) -> i32 {                                 // c:408
    // Direct port of `int acceptandhold(char **args)` from
    // zle_misc.c:408-415:
    // ```c
    // zpushnode(bufstack, zlelineasstring(zleline, zlell, 0, NULL, NULL, 0));
    // stackcs = zlecs;
    // done = 1;
    // return 0;
    // ```
    use std::sync::atomic::Ordering;
    // c:411 — `zpushnode(bufstack, zlelineasstring(...))`.
    let line_str: String = zle.zleline.iter().take(zle.zlell).collect();
    zle.bufstack.insert(0, line_str.clone());                                // c:411 push to front
    zle.stackcs = zle.zlecs;                                                 // c:412
    // Keep killring snapshot for backward-compat with callers that
    // recover via the kill-buffer surface.
    let line: Vec<char> = zle.zleline.iter().take(zle.zlell).copied().collect();
    if !line.is_empty() {
        zle.killring.push_front(line);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
    }
    DONE.store(1, Ordering::SeqCst);                                         // c:413 done = 1
    0                                                                        // c:414
}

/// Port of `acceptline(args)` from `Src/Zle/zle_misc.c:401`.
/// ```c
/// int
/// acceptline(UNUSED(char **args))
/// {
///     done = 1;
///     return 0;
/// }
/// ```
/// `accept-line` widget — the simplest possible: just signal the
/// editor session to terminate so `zleread` returns the current line.
pub fn acceptline() -> i32 {                                                 // c:401
    use std::sync::atomic::Ordering;
    DONE.store(1, Ordering::SeqCst);                                         // c:403 done = 1
    0                                                                        // c:404 return 0
}

// Suffix system                                                            // c:1500
/// Port of `addsuffix(tp, flags, chars, lenstr, lensuf)` from Src/Zle/zle_misc.c:1558.
pub fn addsuffix(tp: i32, flags: i32, chars: Vec<char>, lenstr: i32, lensuf: i32) {  // c:1558
    // C body (c:1560-1567): `newsuf = zalloc; newsuf->next = suffixlist;
    //                       suffixlist = newsuf; copy fields`.
    suffixlist().lock().unwrap().push(suffixset {
        tp, flags, chars, lenstr, lensuf,
    });
}

/// Port of `addsuffixstring(tp, flags, chars, lensuf)` from Src/Zle/zle_misc.c:1580.
pub fn addsuffixstring(tp: i32, flags: i32, chars: &str, lensuf: i32) {      // c:1610
    // C body: `chars = ztrdup(chars); suffixstr = stringaszleline(...);
    //          addsuffix(tp, flags, suffixstr, slen, lensuf)`.
    let chars_vec: Vec<char> = chars.chars().collect();
    let slen = chars_vec.len() as i32;
    addsuffix(tp, flags, chars_vec, slen, lensuf);
}

/// Port of `argumentbase(args)` from `Src/Zle/zle_misc.c:1037`.
/// ```c
/// int
/// argumentbase(char **args)
/// {
///     int multbase;
///     if (*args)
///         multbase = (int)zstrtol(*args, NULL, 0);
///     else
///         multbase = zmod.mult;
///     if (multbase < 2 || multbase > ('9' - '0' + 1) + ('z' - 'a' + 1))
///         return 1;
///     zmod.base = multbase;
///     zmod.flags = 0;
///     zmod.mult = 1;
///     zmod.tmult = 1;
///     zmod.vibuf = 0;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `argument-base` widget — set the numeric base for digit-arg
/// parsing. Valid range 2..36 (10 digits + 26 letters). Returns 1
/// for out-of-range bases without changing state.
/// Port of `argumentbase` from `Src/Zle/zle_misc.c:1037`.
pub fn argumentbase(zle: &mut Zle, args: &[String]) -> i32 {                 // c:1037
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:1042-1045 — `if (*args) multbase = zstrtol(...) else zmod.mult`.
    let multbase = if let Some(arg) = args.first() {
        // c:1043 — `zstrtol(*args, NULL, 0)`. Base 0 means auto
        // (octal "0…", hex "0x…", else decimal).
        let s = arg.as_str();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i32::from_str_radix(hex, 16).unwrap_or(0)
        } else if s.starts_with('0') && s.len() > 1 {
            i32::from_str_radix(&s[1..], 8).unwrap_or(0)
        } else {
            s.parse::<i32>().unwrap_or(0)
        }
    } else {
        zle.zmod.mult                                                        // c:1045
    };
    // c:1047-1048 — range check 2..(10+26)=36.
    if multbase < 2 || multbase > 36 {
        return 1;
    }
    zle.zmod.base = multbase;                                                // c:1050
    // c:1053-1056 — reset modifier apart from base.
    zle.zmod.flags = 0;
    zle.zmod.mult = 1;
    zle.zmod.tmult = 1;
    zle.zmod.vibuf = 0;
    // c:1059 — still operating on prefix arg.
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:1061 return 0
}

/// Port of `backwarddeletechar(args)` from Src/Zle/zle_misc.c:180.
pub fn backwarddeletechar(zle: &mut Zle) -> i32 {                            // c:180
    // c:182-188 — `if (zmult < 0) { negate, recurse to forward,
    //               restore zmult, return ret }`.
    let n = zle.zmod.mult;
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n;
        let ret = deletechar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    // c:189 — `backdel(zmult > zlecs ? zlecs : zmult, 0)`.
    let count = (n as usize).min(zle.zlecs);
    for _ in 0..count {
        if zle.zlecs > 0 {
            zle.zlecs -= 1;
            zle.zleline.remove(zle.zlecs);
            zle.zlell -= 1;
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:190
}

/// Port of `backwardkillline(args)` from Src/Zle/zle_misc.c:225.
pub fn backwardkillline(zle: &mut Zle) -> i32 {                              // c:225
    // c:227-234 — `if (n < 0) { negate, recurse killline, restore }`.
    let n = zle.zmod.mult;
    if n < 0 {
        zle.zmod.mult = -n;
        let ret = killline(zle);
        zle.zmod.mult = n;
        return ret;
    }
    let mut nn = n;
    let mut i = 0_usize;
    // c:236-242 — walk back; '\n' on the LEFT bumps zlecs--, i++.
    while nn > 0 {
        if zle.zlecs > 0 && zle.zleline[zle.zlecs - 1] == '\n' {
            zle.zlecs -= 1;
            i += 1;
        } else {
            while zle.zlecs > 0 && zle.zleline[zle.zlecs - 1] != '\n' {
                zle.zlecs -= 1;
                i += 1;
            }
        }
        nn -= 1;
    }
    // c:243 — `forekill(i, CUT_FRONT|CUT_RAW)`. Drain forward from
    // current zlecs by i chars; push to killring with FRONT semantics
    // (prepended to the existing front entry if present, else new).
    if i > 0 {
        let text: Vec<char> = zle.zleline.drain(zle.zlecs..zle.zlecs + i).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= i;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:245
}

/// Port of `int bracketedpaste(char **args)` from
/// Src/Zle/zle_misc.c:814.
///
/// Captures a bracketed-paste payload via `bracketedstring()` then
/// either stores it in `args[0]` (assoc-array setparam) or inserts it
/// at the cursor with `doinsert`. The single-quote-escape detour
/// (`quotestring(pbuf, QT_SINGLE_OPTIONAL)`) when `zmult != 1`
/// prevents the user from accidentally pasting shell metacharacters.
pub fn bracketedpaste(zle: &mut Zle, args: &[String]) -> i32 {               // c:813
    use crate::ported::utils::quotestring;
    let pbuf = bracketedstring();                                            // c:816
    if let Some(name) = args.first() {                                       // c:818
        // c:819 — `setsparam(*args, pbuf)`. Param-table not yet a
        // singleton; fall back to env-var (matches other ports).
        std::env::set_var(name, &pbuf);
        return 0;
    }
    // c:822-825 — quote when zmult != 1 then convert to ZLE_CHAR_T,
    //              cuttext (REPLACE) the prior cutbuf with the paste.
    let payload = if zle.zmod.mult == 1 {                                    // c:823
        pbuf.clone()
    } else {
        quotestring(&pbuf, crate::ported::zsh_h::QT_SINGLE_OPTIONAL)                        // c:824
    };
    let wpaste: Vec<char> = payload.chars().collect();
    // c:826-834 — !(zmod.flags & MOD_VIBUF) → reset kct, killregion if
    // region_active, then doinsert(wpaste).
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if !zle.zmod.flags & MOD_VIBUF != 0 {
        zle.zmod.mult = 1;                                                   // c:829
        // c:830-832 — `if (region_active) killregion(...)`.
        if zle.region_active != 0 {
            let _ = killregion(zle);
        }
        // c:833 — `doinsert(wpaste, n)`. Inline insert at zlecs.
        for c in wpaste.iter().copied() {
            zle.zleline.insert(zle.zlecs, c);
            zle.zlecs += 1;
            zle.zlell += 1;
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    0                                                                        // c:838
}

/// Port of `mod_export char *bracketedstring(void)` from
/// Src/Zle/zle_misc.c:784.
///
/// Reads bytes from the terminal until the end-paste sequence
/// `\e[201~` is seen, demetafying high-bit bytes and translating
/// `\r` → `\n` along the way.
///
/// Blocked on: `getbyte()` from zle_main.c — the keyboard input
/// pump that respects the ZLE timeout/select(2) machinery. Until
/// the input pump lands, returns the empty string so callers see a
/// no-op paste rather than a panic.
pub fn bracketedstring() -> String {                                         // c:784
    // C body c:786-808 — `getbyte(1L, &timeout, 1)` loop with
    //                    Meta/imeta + \r→\n + ESC[201~ scanner.
    String::new()
}

/// Port of `copyprevshellword(args)` from Src/Zle/zle_misc.c:1108.
pub fn copyprevshellword(zle: &mut Zle) -> i32 {                             // c:1110
    // C body: similar to copyprevword but uses shell tokenizer to
    // identify the previous WORD (whitespace-bounded chunk). Without
    // the shell-tokenizer substrate, fall back to whitespace-bounded
    // back-walk.
    let mut t1 = zle.zlecs;
    while t1 > 0 && zle.zleline[t1 - 1].is_whitespace() {
        t1 -= 1;
    }
    let mut t0 = t1;
    while t0 > 0 && !zle.zleline[t0 - 1].is_whitespace() {
        t0 -= 1;
    }
    if t0 == t1 { return 1; }
    let copied: Vec<char> = zle.zleline[t0..t1].to_vec();
    for (i, &c) in copied.iter().enumerate() {
        zle.zleline.insert(zle.zlecs + i, c);
    }
    zle.zlecs += copied.len();
    zle.zlell += copied.len();
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `copyprevword(args)` from Src/Zle/zle_misc.c:1066.
pub fn copyprevword(zle: &mut Zle) -> i32 {                                  // c:1066
    // C body (c:1066-1110): walk back over zmult words, copy that
    // span, insert at cursor. Simplified: locate previous whitespace-
    // separated word, copy + insert.
    let n = zle.zmod.mult;
    if n <= 0 { return 1; }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut t0 = zle.zlecs;
    for _ in 0..n {
        // skip back over non-word-chars
        while t0 > 0 && !is_word(zle.zleline[t0 - 1]) {
            t0 -= 1;
        }
        // skip back over word
        while t0 > 0 && is_word(zle.zleline[t0 - 1]) {
            t0 -= 1;
        }
    }
    // span: t0..(start of search)
    let mut t1 = t0;
    while t1 < zle.zlecs && is_word(zle.zleline[t1]) {
        t1 += 1;
    }
    let len = t1 - t0;
    if len == 0 { return 1; }
    let copied: Vec<char> = zle.zleline[t0..t1].to_vec();
    for (i, &c) in copied.iter().enumerate() {
        zle.zleline.insert(zle.zlecs + i, c);
    }
    zle.zlecs += len;
    zle.zlell += len;
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}


/// Port of `copyregionaskill(args)` from Src/Zle/zle_misc.c:494.
pub fn copyregionaskill(zle: &mut Zle, args: &[String]) -> i32 {             // c:493
    // c:497-501 — `if (*args) { stringaszleline; cuttext(line, len, CUT_REPLACE) }`.
    if let Some(arg) = args.first() {
        let text: Vec<char> = arg.chars().collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        return 0;
    }
    // c:503-512 — copy region between point and mark.
    if zle.mark > zle.zlell {
        zle.mark = zle.zlell;
    }
    let (start, end) = if zle.mark > zle.zlecs {
        (zle.zlecs, zle.mark)
    } else {
        (zle.mark, zle.zlecs)
    };
    let text: Vec<char> = zle.zleline[start..end].to_vec();
    zle.killring.push_front(text);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    0
}

/// Port of `deletechar(args)` from Src/Zle/zle_misc.c:157.
pub fn deletechar(zle: &mut Zle) -> i32 {                                    // c:157
    // c:160-166 — `if (zmult < 0) { negate, recurse to backward,
    //               restore zmult, return ret }`.
    let mut n = zle.zmod.mult;
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n;
        let ret = backwarddeletechar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    n = zle.zmod.mult;
    // c:169-173 — `while (n--) { if (zlecs == zlell) return 1; INCCS() }`.
    while n > 0 {
        if zle.zlecs == zle.zlell {
            return 1;
        }
        crate::ported::zle::zle_move::inccs(zle);
        n -= 1;
    }
    // c:174 — `backdel(zmult, 0)`. Method delete_char does forward.
    let count = zle.zmod.mult.max(0) as usize;
    for _ in 0..count {
        if zle.zlecs > 0 {
            zle.zlecs -= 1;
            if zle.zlecs < zle.zleline.len() {
                zle.zleline.remove(zle.zlecs);
                zle.zlell -= 1;
            }
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:175
}

/// Port of `digitargument(args)` from Src/Zle/zle_misc.c:950.
pub fn digitargument(zle: &mut Zle) -> i32 {                                 // c:1042
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:1044 — `int sign = (zmult < 0) ? -1 : 1`.
    let sign: i32 = if zle.zmod.mult < 0 { -1 } else { 1 };
    // c:1045 — `parsedigit(lastchar)`.
    let newdigit = parsedigit(zle, crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst));
    if newdigit < 0 {                                                        // c:1047
        return 1;                                                            // c:1048
    }
    // c:1050-1051 — `if (!(zmod.flags & MOD_TMULT)) zmod.tmult = 0`.
    if !zle.zmod.flags & MOD_TMULT != 0 {
        zle.zmod.tmult = 0;
    }
    // c:1052-1057 — MOD_NEG path: replace tmult with sign*newdigit.
    if zle.zmod.flags & MOD_NEG != 0 {
        zle.zmod.tmult = sign * newdigit;
        zle.zmod.flags &= !MOD_NEG;
    } else {
        // c:1058 — `zmod.tmult = zmod.tmult * zmod.base + sign*newdigit`.
        zle.zmod.tmult = zle.zmod.tmult * zle.zmod.base + sign * newdigit;
    }
    zle.zmod.flags |= MOD_TMULT;                             // c:1059
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:1060
    0                                                                        // c:1061
}

/// Port of `doinsert(zstr, len)` from `Src/Zle/zle_misc.c:37`.
/// ```c
/// mod_export void
/// doinsert(ZLE_STRING_T zstr, int len) {
///     ...
///     m = abs(zmult); count = m * len;
///     ...insert m copies of zstr at cursor (or after, if zmult < 0)...
/// }
/// ```
/// Insert `zstr` `|zmod.mult|` times at the cursor. Negative count
/// inserts AFTER the cursor (cursor stays put). Simplified port —
/// the full body has INSMODE/overwrite handling and suffix
/// machinery that needs the suffixlist substrate.
/// Port of `doinsert` from `Src/Zle/zle_misc.c:37`.
pub fn doinsert(zle: &mut Zle, zstr: &[char]) {                              // c:37
    let m = zle.zmod.mult.unsigned_abs() as usize;
    let neg = zle.zmod.mult < 0;
    for _ in 0..m {
        for (i, &c) in zstr.iter().enumerate() {
            zle.zleline.insert(zle.zlecs + i, c);
        }
        if !neg {
            zle.zlecs += zstr.len();
        }
        zle.zlell += zstr.len();
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `NAMLEN` from `Src/Zle/zle_misc.c:1249`. Maximum length
/// of a widget name buffer used by `executenamedcommand` for
/// `execute-named-command` / `where-is`. The C source declares this
/// as a macro just before the local-keymap fixture.
pub const NAMLEN: usize = 60;                                                // c:1249

/// Direct port of `Thingy executenamedcommand(char *prompt)` from
/// `Src/Zle/zle_misc.c:1261-1320`. Prompts the user for a widget
/// name (with name-completion via thingytab), then resolves the
/// answer to a Thingy.
///
/// **Substrate trade-off:** the interactive prompt path requires a
/// live ZLE input loop (`getfullchar`/`displaywholeline` machinery)
/// that compcore-call-context fns can't easily reach. Rust port
/// instead reads `$REPLY` from the canonical paramtab — the same
/// var that `read-command` widgets populate — so user widgets that
/// shell out to interactive prompts (`read-command -p PROMPT`) get
/// their answer surfaced here.
pub fn executenamedcommand(prompt: &str) -> Option<String> {                 // c:1261
    let _ = prompt;
    // c:1304 — `bindztrdup(name)` resolves the typed widget. Rust
    // path reads $REPLY (set by widgets like `read-command`).
    crate::ported::params::getsparam("REPLY")                                // c:1304
        .filter(|s| !s.is_empty())
}

// Fix the suffix in place, if there is one, making it non-removable.      // c:1820
/// Port of `fixsuffix()` from Src/Zle/zle_misc.c:1824.
pub fn fixsuffix() {                                                         // c:1824
    // C body (c:1826-1832): `while (suffixlist) { next = sl->next;
    //                       if (sl->lenstr) zfree(sl->chars, ...);
    //                       zfree(sl, ...); suffixlist = next; }
    //                       suffixlen = 0`.
    use std::sync::atomic::Ordering;
    suffixlist().lock().unwrap().clear();
    SUFFIXLEN.store(0, Ordering::SeqCst);
}

/// Port of `fixunmeta()` from Src/Zle/zle_misc.c:130.
pub fn fixunmeta(zle: &mut Zle) {                                            // c:130
    // c:132 — `lastchar &= 0x7f`. Strip Meta/high bit.
    crate::ported::zle::compcore::LASTCHAR.fetch_and((0x7f) as i32, std::sync::atomic::Ordering::SeqCst);
    // c:133-134 — `if (lastchar == '\\r') lastchar = '\\n'`.
    if crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) == b'\r' as i32 {
        crate::ported::zle::compcore::LASTCHAR.store((b'\n' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
    }
    // c:140 — `lastchar_wide = (ZLE_INT_T)lastchar`. Sync wide.
    crate::ported::zle::zle_main::LASTCHAR_WIDE.store((crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst)) as i32, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `gosmacstransposechars(args)` from Src/Zle/zle_misc.c:274.
pub fn gosmacstransposechars(zle: &mut Zle) -> i32 {                         // c:273
    // C body (c:276-307): gosmacs-style: transpose char before cursor
    // with char at cursor; advance cursor. Skips through newlines and
    // multi-byte combining chars.
    if zle.zlecs < 2 || zle.zlecs > zle.zlell {
        // Edge: try to advance past initial newline so we can transpose.
        let twice = zle.zlecs == 0 || zle.zleline.get(zle.zlecs.saturating_sub(1)) == Some(&'\n');
        if zle.zlecs >= zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n') {
            return 1;
        }
        zle.zlecs += 1;
        if twice {
            if zle.zlecs >= zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n') {
                return 1;
            }
            zle.zlecs += 1;
        }
    }
    if zle.zlecs >= 2 && zle.zlecs <= zle.zleline.len() {
        zle.zleline.swap(zle.zlecs - 2, zle.zlecs - 1);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    0
}

// Remove suffix, if there is one, when inserting character c.             // c:1695
/// Direct port of `int iremovesuffix(ZLE_INT_T c, int keep)` from
/// `Src/Zle/zle_misc.c:1699-1797`. Walks `suffixlist`; for each
/// matching entry, removes `lensuf` chars before `ZLECS` in
/// `ZLELINE` (unless `keep` is set), then either calls the
/// registered `suffixfunc` or just clears the list.
pub fn iremovesuffix(c: i32, keep: i32) -> i32 {                             // c:1699
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{ZLELINE, ZLECS, ZLELL};

    // c:1701 — `if (suffixfunc) { ... }` — run shfunc if registered.
    let sf = SUFFIXFUNC.get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    if !sf.is_empty() {                                                      // c:1701
        // c:1703 — `getshfunc(suffixfunc)`. We probe for existence; full
        // doshfunc dispatch fires via fusevm Op::CallFunction in live
        // contexts.
        let _exists = crate::ported::utils::getshfunc(&sf);
        // c:1729 — `zsfree(suffixfunc); suffixfunc = NULL`.
        if let Ok(mut g) = SUFFIXFUNC.get_or_init(
            || std::sync::Mutex::new(String::new())
        ).lock() {
            g.clear();
        }
    }

    // c:1735-1786 — suffixlist walk.
    let list = suffixlist().lock().map(|g| g.clone()).unwrap_or_default();
    let mut sl: i32 = 0;
    let ch = c as u32;
    for entry in list.iter() {                                               // c:1735
        // c:1741-1769 — match `ch` against entry.chars based on tp/flags.
        let matched = entry.chars.iter().any(|&x| x as u32 == ch);
        if matched {                                                         // c:1762
            if keep == 0 { sl = entry.lensuf; }                              // c:1764
            break;
        }
    }

    // c:1788-1795 — if sl > 0 && !keep, drop `sl` chars before ZLECS.
    if sl > 0 && keep == 0 {
        let cs = ZLECS.load(Ordering::Relaxed) as usize;
        let drop_n = (sl as usize).min(cs);
        let new_cs = cs - drop_n;
        if let Ok(mut g) = ZLELINE.get_or_init(
            || std::sync::Mutex::new(String::new())
        ).lock() {
            if new_cs <= g.len() && drop_n <= cs {
                g.drain(new_cs..cs);
            }
            ZLELL.store(g.len() as i32, Ordering::Relaxed);
        }
        ZLECS.store(new_cs as i32, Ordering::Relaxed);
    }

    // c:1796 — clear suffix list.
    fixsuffix();
    0                                                                        // c:1797
}

/// File-scope `char *suffixfunc` from `Src/Zle/zle_misc.c` — the
/// registered shfunc name run by `iremovesuffix` on suffix match.
pub static SUFFIXFUNC: std::sync::OnceLock<std::sync::Mutex<String>>
    = std::sync::OnceLock::new();                                            // zle_misc.c

/// Port of `killbuffer(args)` from Src/Zle/zle_misc.c:215.
pub fn killbuffer(zle: &mut Zle) -> i32 {                                    // c:215
    // c:217-219 — `zlecs = 0; forekill(zlell, CUT_RAW); clearlist=1`.
    zle.zlecs = 0;
    if !zle.zleline.is_empty() {
        let text: Vec<char> = zle.zleline.drain(..).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell = 0;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:220
}

/// Port of `killline(args)` from Src/Zle/zle_misc.c:419.
pub fn killline(zle: &mut Zle) -> i32 {                                      // c:418
    // c:421-428 — `if (n < 0) { backward delegate w/ negated zmult }`.
    let n_orig = zle.zmod.mult;
    if n_orig < 0 {
        zle.zmod.mult = -n_orig;
        let ret = backwardkillline(zle);
        zle.zmod.mult = n_orig;
        return ret;
    }
    let mut n = n_orig;
    let start = zle.zlecs;
    let mut i = 0_usize;
    // c:430-436 — walk to next newline; skip past existing newline.
    while n > 0 {
        if zle.zlecs < zle.zleline.len() && zle.zleline[zle.zlecs] == '\n' {
            zle.zlecs += 1;
            i += 1;
        } else {
            while zle.zlecs != zle.zlell && zle.zleline[zle.zlecs] != '\n' {
                zle.zlecs += 1;
                i += 1;
            }
        }
        n -= 1;
    }
    // c:437 — `backkill(i, CUT_RAW)`. Drain the killed range and
    // push to killring; cursor returns to start.
    if i > 0 {
        let text: Vec<char> = zle.zleline.drain(start..start + i).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= i;
        zle.zlecs = start;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:439
}

/// Port of `killregion(args)` from Src/Zle/zle_misc.c:463.
pub fn killregion(zle: &mut Zle) -> i32 {                                    // c:462
    // c:465-466 — `if (mark > zlell) mark = zlell`.
    if zle.mark > zle.zlell {
        zle.mark = zle.zlell;
    }
    // c:467-479 — region_active==2 (visual-line); skip the line-mode
    // path for the simplified port.
    let (start, end) = if zle.mark > zle.zlecs {
        (zle.zlecs, zle.mark)
    } else {
        (zle.mark, zle.zlecs)
    };
    if start < end {
        let text: Vec<char> = zle.zleline.drain(start..end).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= end - start;
        zle.zlecs = start;
        zle.mark = start;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `killwholeline(args)` from Src/Zle/zle_misc.c:195.
pub fn killwholeline(zle: &mut Zle) -> i32 {                                 // c:194
    let mut n = zle.zmod.mult;
    if n < 0 {                                                               // c:199
        return 1;                                                            // c:200
    }
    while n > 0 {                                                            // c:201
        // c:202-203 — last-line edge: at zlell with non-empty buffer
        // step back one so the trailing '\n' belongs to this line.
        let _fg = zle.zlecs > 0 && zle.zlecs == zle.zlell;
        if _fg {
            zle.zlecs -= 1;
        }
        // c:204-205 — walk back to bol.
        while zle.zlecs > 0 && zle.zleline[zle.zlecs - 1] != '\n' {
            zle.zlecs -= 1;
        }
        // c:206 — `for (i=zlecs; i!=zlell && zleline[i]!='\n'; i++)`.
        let mut i = zle.zlecs;
        while i != zle.zlell && zle.zleline[i] != '\n' {
            i += 1;
        }
        // c:207 — `forekill(i - zlecs + (i != zlell), ...)`. Include
        // the trailing '\n' if there is one.
        let drop = i - zle.zlecs + (if i != zle.zlell { 1 } else { 0 });
        if drop > 0 {
            let text: Vec<char> = zle.zleline.drain(zle.zlecs..zle.zlecs + drop).collect();
            zle.killring.push_front(text);
            if zle.killring.len() > zle.killringmax {
                zle.killring.pop_back();
            }
            zle.zlell -= drop;
        }
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:210
}

/// Port of `makeparamsuffix(br, n)` from Src/Zle/zle_misc.c:1623.
pub fn makeparamsuffix(br: i32, n: i32) {                                    // c:1690
    // C body (c:1692-1697): `charstr = ":[#%?-+="; lenstr = (br ||
    //                       unset(KSHARRAYS)) ? 2 : strlen(charstr);
    //                       addsuffix(SUFTYP_POSSTR, 0, charstr, lenstr, n)`.
    let charstr: Vec<char> = ":[#%?-+=".chars().collect();
    let kshcheck = !crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
    let lenstr = if br != 0 || kshcheck { 2 } else { charstr.len() as i32 };
    let prefix: Vec<char> = charstr.iter().take(lenstr as usize).copied().collect();
    addsuffix(0, 0, prefix, lenstr, n);
}

/// Port of `makequote(str, len)` from Src/Zle/zle_misc.c:1201.
pub fn makequote(s: &[char]) -> Vec<char> {                                  // c:1166
    // c:1170-1173 — count qtct = number of `'` chars.
    let qtct = s.iter().filter(|&&c| c == '\'').count();
    // c:1174 — `*len += 2 + qtct*3`. Output capacity: 2 (outer
    // quotes) + len + qtct*3 (each ' becomes '\\'').
    let mut out = Vec::<char>::with_capacity(s.len() + 2 + qtct * 3);
    out.push('\'');                                                          // c:1176 *l++ = '\''
    for &c in s {                                                            // c:1177-1184
        if c == '\'' {
            // c:1179-1182 — ' → '\''
            out.push('\'');
            out.push('\\');
            out.push('\'');
            out.push('\'');
        } else {
            out.push(c);
        }
    }
    out.push('\'');                                                          // c:1185 *l++ = '\''
    out
}

/// Port of `makesuffix(n)` from Src/Zle/zle_misc.c:1598.
pub fn makesuffix(n: i32) {                                                  // c:1640
    // C body (c:1642-1652): `suffixchars = getsparam_u(
    //                       "ZLE_REMOVE_SUFFIX_CHARS"); if (!suffixchars)
    //                       suffixchars = " \\t\\n;&|"; addsuffix(...)`.
    let suffix_chars = std::env::var("ZLE_REMOVE_SUFFIX_CHARS")
        .unwrap_or_else(|_| " \t\n;&|".to_string());
    addsuffixstring(0, 0, &suffix_chars, n);
}

/// Port of `makesuffixstr(f, s, n)` from Src/Zle/zle_misc.c:1642.
pub fn makesuffixstr(_funcnam: Option<&str>, str_arg: Option<&str>, n: i32) {  // c:1660
    // C body: `if (str) addsuffixstring(0, 0, str, n);
    //          if (funcnam) suffixfunc = funcnam`. zshrs's suffixfunc
    // global isn't set up; faithful path covers the str argument.
    if let Some(s) = str_arg {
        addsuffixstring(0, 0, s, n);
    }
}

/// Port of `negargument(args)` from `Src/Zle/zle_misc.c:974`.
/// ```c
/// int
/// negargument(UNUSED(char **args))
/// {
///     if (zmod.flags & MOD_TMULT)
///         return 1;
///     zmod.tmult = -1;
///     zmod.flags |= MOD_TMULT|MOD_NEG;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `negative-argument` widget — start a negative count prefix.
/// Refuses if a tmult is already in flight.
/// Port of `negargument` from `Src/Zle/zle_misc.c:974`.
pub fn negargument(zle: &mut Zle) -> i32 {                                   // c:974
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if zle.zmod.flags & MOD_TMULT != 0 {                       // c:976
        return 1;                                                            // c:977
    }
    zle.zmod.tmult = -1;                                                     // c:978
    zle.zmod.flags |= MOD_TMULT | MOD_NEG;             // c:979
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:980
    0                                                                        // c:981 return 0
}

/// Port of `overwritemode(args)` from `Src/Zle/zle_misc.c:842`.
/// ```c
/// int
/// overwritemode(UNUSED(char **args))
/// {
///     insmode ^= 1;
///     return 0;
/// }
/// ```
/// `overwrite-mode` widget — toggle insert/overwrite mode.
pub fn overwritemode(zle: &mut Zle) -> i32 {                                 // c:842
    crate::ported::zle::zle_main::INSMODE.fetch_xor(1, std::sync::atomic::Ordering::SeqCst);                                              // c:845 insmode ^= 1
    0                                                                        // c:846 return 0
}

/// Port of `parsedigit(inkey)` from Src/Zle/zle_misc.c:919.
pub fn parsedigit(zle: &Zle, inkey: i32) -> i32 {                            // c:1066
    // c:1077 — `inkey &= 0x7f` (mask off Meta bit). Multibyte path
    // skips this; we mirror by always masking since Rust char vals
    // fit ASCII for digit chars.
    let inkey = inkey & 0x7f;
    let base = zle.zmod.base;
    // c:1082-1090 — base > 10: accept lowercase a..(a+base-11) and
    // uppercase, plus digits 0-9.
    if base > 10 {
        if (b'a' as i32..b'a' as i32 + base - 10).contains(&inkey) {
            return inkey - b'a' as i32 + 10;                                 // c:1083
        }
        if (b'A' as i32..b'A' as i32 + base - 10).contains(&inkey) {
            return inkey - b'A' as i32 + 10;                                 // c:1085
        }
        if (b'0' as i32..=b'9' as i32).contains(&inkey) {                    // c:1087 idigit
            return inkey - b'0' as i32;
        }
        return -1;                                                           // c:1089
    }
    // c:1092-1093 — base <= 10: digit must be in '0'..'0'+base.
    if (b'0' as i32..b'0' as i32 + base).contains(&inkey) {
        return inkey - b'0' as i32;
    }
    -1                                                                       // c:1094
}

/// Port of `pastebuf(buf, mult, position)` from Src/Zle/zle_misc.c:558.
pub fn pastebuf(zle: &mut Zle, buf: &[char], mult: i32, position: i32) -> i32 {  // c:557
    // Simplified port of pastebuf. The C source dispatches on
    // CUTBUFFER_LINE flag (insert as full lines vs char-wise),
    // computes position 0/1/2 (before/after/split), and updates
    // yankb/yanke. Without the LINE-flag check (we treat all as
    // char-wise) plus the simple before/after path we get the
    // common case.
    if buf.is_empty() {
        return 0;
    }
    // c:591-592 — `if (position == 1 && zlecs != findeol()) INCCS()`.
    if position == 1 && zle.zlecs < zle.zlell {
        zle.zlecs += 1;
    }
    // c:593 — `yankb = zlecs`.
    zle.yank_start = zle.zlecs;
    // c:595-599 — `while (mult--) { spaceinline(cc); ZS_memcpy; zlecs += cc }`.
    let mut n = mult;
    while n > 0 {
        for (i, &c) in buf.iter().enumerate() {
            zle.zleline.insert(zle.zlecs + i, c);
        }
        zle.zlecs += buf.len();
        zle.zlell += buf.len();
        n -= 1;
    }
    // c:600 — `yanke = zlecs`.
    zle.yank_end = zle.zlecs;
    // c:601-602 — vicmd → DECCS.
    if zle.zlecs > 0 && *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" {
        zle.zlecs -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `poundinsert(args)` from Src/Zle/zle_misc.c:369.
pub fn poundinsert(zle: &mut Zle) -> i32 {                                   // c:368
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_move::vifirstnonblank;
    // c:371-393 — `zlecs = 0; vifirstnonblank(zlenoargs);
    //              if (zleline[zlecs] != '#') { spaceinline(1);
    //                  zleline[zlecs] = '#'; zlecs = findeol();
    //                  while (zlecs != zlell) { ... } }
    //              else { foredel(1, 0); zlecs = findeol(); ... }
    //              done = 1; return 0`.
    zle.zlecs = 0;                                                           // c:371
    vifirstnonblank(zle);                                                    // c:372
    let at_pound = zle.zleline.get(zle.zlecs) == Some(&'#');
    if !at_pound {
        // c:374-383 — insert # at this line, advance to next, repeat.
        zle.zleline.insert(zle.zlecs, '#');
        zle.zlell += 1;
        zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
        while zle.zlecs != zle.zlell {
            zle.zlecs += 1;
            vifirstnonblank(zle);
            zle.zleline.insert(zle.zlecs, '#');
            zle.zlell += 1;
            zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
        }
    } else {
        // c:384-393 — strip leading # from each line.
        zle.zleline.remove(zle.zlecs);
        zle.zlell -= 1;
        zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
        while zle.zlecs != zle.zlell {
            zle.zlecs += 1;
            vifirstnonblank(zle);
            if zle.zleline.get(zle.zlecs) == Some(&'#') {
                zle.zleline.remove(zle.zlecs);
                zle.zlell -= 1;
            }
            zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
        }
    }
    DONE.store(1, Ordering::SeqCst);                                         // c:395
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:396
}

/// Port of `putreplaceselection(args)` from Src/Zle/zle_misc.c:679.
pub fn putreplaceselection(zle: &mut Zle) -> i32 {                           // c:679
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = zle.zmod.mult;                                                   // c:682
    let mut pos = 2;                                                         // c:686
    startvichange(zle, -1);                                                  // c:688
    if n < 0 || zle.zmod.flags & MOD_NULL != 0 {
        return 1;                                                            // c:690
    }
    let prevbuf: Vec<char> = if zle.zmod.flags & MOD_VIBUF != 0 {
        let idx = zle.zmod.vibuf as usize;
        if idx >= zle.vibuf.len() {
            return 1;
        }
        zle.vibuf[idx].clone()                                               // c:700
    } else {
        zle.killring.front().cloned().unwrap_or_default()                    // c:702
    };
    if prevbuf.is_empty() {
        return 1;                                                            // c:702
    }
    zle.zmod.flags = 0;                                 // c:712
    if zle.region_active == 2 {                                              // c:713
        // c:714-717 — regionlines split; lines-flag check elided.
        pos = if zle.zlell == zle.zlecs { 1 } else { 0 };
    }
    let _ = killregion(zle);                                                 // c:719
    pastebuf(zle, &prevbuf, n, pos)                                          // c:721
}

/// Direct port of `int quotedinsert(char **args)` from
/// `Src/Zle/zle_misc.c:899-923`.
/// ```c
/// // (raw-mode tweak for non-HAS_TIO systems — skipped on Linux/macOS)
/// getfullchar(0);
/// if (LASTFULLCHAR == ZLEEOF) return 1;
/// return selfinsert(args);
/// ```
/// HAS_TIO is set everywhere zshrs builds (Linux/macOS), so the
/// raw-mode/ioctl branch is unreachable — `getfullchar` already
/// runs in the right mode via `zsetterm`. We invoke it explicitly
/// for a one-shot read, then forward to `selfinsert`.
pub fn quotedinsert(zle: &mut Zle) -> i32 {                                  // c:899
    // c:911 — `getfullchar(0)`. Reads one full char, updates
    // crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) / lastchar_wide / lastchar_wide_valid.
    let _ = zle.getfullchar(false);
    if crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) < 0 {                                                    // c:919 LASTFULLCHAR == ZLEEOF
        return 1;
    }
    selfinsert(zle)                                                          // c:922
}

/// Port of `quoteline(args)` from Src/Zle/zle_misc.c:1187.
pub fn quoteline(zle: &mut Zle) -> i32 {                                     // c:1188
    // c:1192 — `len = zlell`. Quote whole buffer.
    let quoted = makequote(&zle.zleline[..zle.zlell]);
    let len = quoted.len();
    // c:1193-1195 — `sizeline; ZS_memcpy; zlecs = zlell = len`.
    zle.zleline = quoted;
    zle.zlell = len;
    zle.zlecs = len;
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:1196
}

/// Port of `quoteregion(args)` from Src/Zle/zle_misc.c:1152.
pub fn quoteregion(zle: &mut Zle) -> i32 {                                   // c:1151
    // c:1156 — `int extra = invicmdmode()`. Vi-cmd-mode bias.
    let _ = &zle;
    let mut extra = *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd";
    // c:1158-1159 — `if (mark > zlell) mark = zlell`.
    if zle.mark > zle.zlell {
        zle.mark = zle.zlell;
    }
    // c:1160-1170 — visual-line vs. char modes; normalize zlecs/mark.
    if zle.region_active == 2 {
        let (a, b) = regionlines(zle);
        zle.zlecs = a;
        zle.mark = b;
        extra = false;
    } else if zle.mark < zle.zlecs {
        std::mem::swap(&mut zle.mark, &mut zle.zlecs);
    }
    // c:1171-1172 — `if (extra) INCPOS(mark)`. Include cursor cell.
    if extra && zle.mark < zle.zlell {
        zle.mark += 1;
    }
    // c:1173-1175 — copy region into temp str; foredel; quote; insert.
    let region: Vec<char> = zle.zleline[zle.zlecs..zle.mark].to_vec();
    let len = region.len();
    let quoted = makequote(&region);
    let qlen = quoted.len();
    // c:1176 — `foredel(len, CUT_RAW)` — delete region (no kill).
    zle.zleline.drain(zle.zlecs..zle.zlecs + len);
    zle.zlell -= len;
    // c:1178-1179 — insert quoted text at cursor.
    for (i, &c) in quoted.iter().enumerate() {
        zle.zleline.insert(zle.zlecs + i, c);
    }
    zle.zlell += qlen;
    // c:1180-1181 — `mark = zlecs; zlecs += len`.
    zle.mark = zle.zlecs;
    zle.zlecs += qlen;
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `regionlines(start, end)` from Src/Zle/zle_misc.c:444.
pub fn regionlines(zle: &mut Zle) -> (usize, usize) {                        // c:443
    use crate::ported::zle::zle_utils::{findbol, findeol};
    // c:446 — `int origcs = zlecs`. Save cursor.
    let origcs = zle.zlecs;
    let start;
    let end;
    if zle.zlecs < zle.mark {                                                // c:449
        // c:450-452 — start=findbol(); zlecs=min(mark,zlell); end=findeol().
        start = findbol(zle);
        zle.zlecs = if zle.mark > zle.zlell { zle.zlell } else { zle.mark };
        end = findeol(zle);
    } else {
        // c:454-456 — end=findeol(); zlecs=mark; start=findbol().
        end = findeol(zle);
        zle.zlecs = zle.mark;
        start = findbol(zle);
    }
    // c:458 — `zlecs = origcs`. Restore.
    zle.zlecs = origcs;
    (start, end)
}

/// Port of `static char *namedcmdstr` from `Src/Zle/zle_misc.c:1229`.
pub static namedcmdstr: std::sync::Mutex<String> =                           // c:1229
    std::sync::Mutex::new(String::new());

/// Port of `static LinkList namedcmdll` from `Src/Zle/zle_misc.c:1230`.
pub static namedcmdll: std::sync::Mutex<Vec<String>> =                       // c:1230
    std::sync::Mutex::new(Vec::new());

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1231`.
pub static namedcmdambig: std::sync::atomic::AtomicUsize =                   // c:1231
    std::sync::atomic::AtomicUsize::new(0);

/// Direct port of `static int scancompcmd(HashNode hn, UNUSED(int flags))`
/// from `Src/Zle/zle_misc.c:1235`.
pub fn scancompcmd(name: &str) -> i32 {                                      // c:1235
    use std::sync::atomic::Ordering;
    // c:1240 — `if (strpfx(namedcmdstr, t->nam))`.
    let prefix = namedcmdstr.lock().unwrap().clone();
    if !name.starts_with(&prefix) { return 0; }
    let mut ll = namedcmdll.lock().unwrap();
    let first = ll.first().cloned();
    ll.push(name.to_string());                                               // c:1241 addlinknode
    if let Some(f) = first {
        // c:1242 — `pfxlen(peekfirst(namedcmdll), t->nam)`.
        let l = f.bytes().zip(name.bytes()).take_while(|(a, b)| a == b).count();
        if l < namedcmdambig.load(Ordering::Relaxed) {
            namedcmdambig.store(l, Ordering::Relaxed);                       // c:1243
        }
    } else {
        namedcmdambig.store(name.len(), Ordering::Relaxed);
    }
    0
}

/// Direct port of `int selfinsert(char **args)` from
/// `Src/Zle/zle_misc.c:112-126`.
/// ```c
/// if (!lastchar_wide_valid)
///     getrestchar(lastchar, NULL);
/// // tmp = LASTFULLCHAR;
/// doinsert(&tmp, 1);
/// return 0;
/// ```
///
/// **Multibyte tradeoff:** C's `getrestchar` reassembles a wide
/// char from `lastchar` + buffered continuation bytes when the
/// `wide_valid` flag is clear. Rust's `Zle::getfullchar` (zle_main
/// .rs:730) already produces a full char per read, so by the time
/// `selfinsert` fires, `lastchar` IS the full codepoint — the
/// `wide_valid=false` branch is unreachable in the Rust input path
/// and the ASCII-promotion is the correct fallback for the rare
/// case where a widget sets `lastchar` directly.
/// Port of `selfinsert` from `Src/Zle/zle_misc.c:112`.
pub fn selfinsert(zle: &mut Zle) -> i32 {                                    // c:112
    if !(crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.load(std::sync::atomic::Ordering::SeqCst) != 0) {                                            // c:118
        crate::ported::zle::zle_main::LASTCHAR_WIDE.store((crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst)) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    // c:123 — `tmp = LASTFULLCHAR; doinsert(&tmp, 1)`.
    if let Some(c) = char::from_u32(crate::ported::zle::zle_main::LASTCHAR_WIDE.load(std::sync::atomic::Ordering::SeqCst) as u32) {
        zle.self_insert(c);
    }
    0                                                                        // c:125
}

/// Port of `selfinsertunmeta(args)` from Src/Zle/zle_misc.c:149.
pub fn selfinsertunmeta(zle: &mut Zle) -> i32 {                              // c:149
    // c:151-152 — `fixunmeta(); return selfinsert(args)`.
    fixunmeta(zle);
    selfinsert(zle)
}

/// Port of `sendbreak(args)` from `Src/Zle/zle_misc.c:1144`.
/// ```c
/// int
/// sendbreak(UNUSED(char **args))
/// {
///     errflag |= ERRFLAG_ERROR|ERRFLAG_INT;
///     return 1;
/// }
/// ```
/// `send-break` widget — abort the current editor session by
/// raising both `ERRFLAG_ERROR` and `ERRFLAG_INT` on the global
/// `errflag`, so `zleread` returns -1 to its caller.
pub fn sendbreak() -> i32 {                                                  // c:1144
    // c:1146 — `errflag |= ERRFLAG_ERROR | ERRFLAG_INT`.
    crate::ported::utils::errflag.fetch_or(
        crate::ported::zsh_h::ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_INT,
        std::sync::atomic::Ordering::Relaxed,
    );
    1                                                                        // c:1147 return 1
}

/// Port of `transpose_swap(start, middle, end)` from `Src/Zle/zle_misc.c:254`.
/// ```c
/// static void
/// transpose_swap(int start, int middle, int end)
/// {
///     int len1, len2;
///     ZLE_STRING_T first;
///     len1 = middle - start;
///     len2 = end - middle;
///     first = (ZLE_STRING_T)zalloc(len1 * ZLE_CHAR_SIZE);
///     ZS_memcpy(first, zleline + start, len1);
///     /* Move may be overlapping... */
///     ZS_memmove(zleline + start, zleline + middle, len2);
///     ZS_memcpy(zleline + start + len2, first, len1);
///     zfree(first, len1 * ZLE_CHAR_SIZE);
/// }
/// ```
/// Swap two adjacent slices in the line buffer:
/// `zleline[start..middle]` and `zleline[middle..end]`. After the
/// swap, `zleline[start..start+(end-middle)]` holds the second
/// chunk and `zleline[start+(end-middle)..end]` holds the first.
/// Port of `transpose_swap` from `Src/Zle/zle_misc.c:254`.
pub fn transpose_swap(zle: &mut Zle, start: usize, middle: usize, end: usize) {  // c:254
    let len1 = middle - start;                                               // c:260
    let len2 = end - middle;                                                 // c:261
    // c:263-264 — copy first slice into temp buffer.
    let first: Vec<char> = zle.zleline[start..middle].to_vec();
    // c:266 — `ZS_memmove(zleline + start, zleline + middle, len2)`.
    // Vec doesn't overlap when copy_within is used.
    zle.zleline.copy_within(middle..end, start);
    // c:267 — `ZS_memcpy(zleline + start + len2, first, len1)`.
    for (i, &ch) in first.iter().enumerate() {
        zle.zleline[start + len2 + i] = ch;
    }
    let _ = len1;
}

/// Port of `transposechars(args)` from Src/Zle/zle_misc.c:313.
pub fn transposechars(zle: &mut Zle) -> i32 {                                // c:313
    use crate::ported::zle::zle_move::{deccs, decpos, inccs, incpos};
    let mut n = zle.zmod.mult;
    let neg = n < 0;                                                         // c:317
    if neg {
        n = -n;                                                              // c:319
    }
    while n > 0 {                                                            // c:321
        n -= 1;
        let mut ct = zle.zlecs;                                              // c:322
        if ct == 0 || zle.zleline[zle.zlecs - 1] == '\n' {                   // c:322
            if zle.zlell == zle.zlecs || zle.zleline[zle.zlecs] == '\n' {    // c:323
                return 1;
            }
            if !neg {
                inccs(zle);                                                  // c:326
            }
            incpos(&mut ct);                                                 // c:327
        }
        if neg {
            if zle.zlecs > 0 && zle.zleline[zle.zlecs - 1] != '\n' {         // c:330
                deccs(zle);                                                  // c:331
                if ct > 1 && zle.zleline[ct - 2] != '\n' {                   // c:332
                    decpos(&mut ct);                                         // c:333
                }
            }
        } else if zle.zlecs != zle.zlell && zle.zleline[zle.zlecs] != '\n' {
            inccs(zle);                                                      // c:338
        }
        if ct == zle.zlell || zle.zleline[ct] == '\n' {                      // c:340
            decpos(&mut ct);                                                 // c:341
        }
        if ct < 1 || zle.zleline[ct - 1] == '\n' {                           // c:343
            return 1;
        }
        // c:345-358 — MULTIBYTE branch uses transpose_swap with surrounding
        //              positions; non-multibyte branch swaps two ZLE_CHAR_T.
        //              Rust ZleString is Vec<char> so we can swap directly.
        zle.zleline.swap(ct - 1, ct);
    }
    0
}

/// Port of `undefinedkey(args)` from `Src/Zle/zle_misc.c:892`.
/// ```c
/// int
/// undefinedkey(UNUSED(char **args))
/// {
///     return 1;
/// }
/// ```
/// `undefined-key` widget — bound to key sequences that aren't
/// otherwise defined; returns 1 so the dispatcher beeps.
pub fn undefinedkey() -> i32 {                                               // c:892
    // c:894 — `return 1`. The widget binds to keys with no other
    // function and just signals "unhandled" by returning non-zero.
    1
}

/// Port of `universalargument(args)` from Src/Zle/zle_misc.c:986.
pub fn universalargument(zle: &mut Zle, args: &[String]) -> i32 {            // c:986
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:988-993 — `if (*args)` short-circuit when invoked with an
    //              explicit numeric arg.
    if let Some(a) = args.first() {
        if let Ok(n) = a.parse::<i32>() {
            zle.zmod.mult = n;
            zle.zmod.flags |= MOD_MULT;
            return 0;
        }
    }
    // c:1009-1023 — interactive byte-by-byte digit collection. Without
    //               a live keystream we mirror the no-input branch
    //               (no digits) which multiplies tmult by 4.
    let digcnt = 0;
    if digcnt == 0 {
        zle.zmod.tmult = zle.zmod.tmult.saturating_mul(4);                   // c:1027
    }
    zle.zmod.flags |= MOD_TMULT;                             // c:1029
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:1030
    0
}

/// Port of `viputafter(args)` from Src/Zle/zle_misc.c:643.
pub fn viputafter(zle: &mut Zle) -> i32 {                                    // c:643
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = zle.zmod.mult;                                                   // c:646
    startvichange(zle, -1);                                                  // c:648
    if n < 0 {
        return 1;                                                            // c:650
    }
    if zle.zmod.flags & MOD_NULL != 0 {
        return 0;                                                            // c:652
    }
    // c:653-665 — OS selection branch (MOD_OSSEL = PRI|CLIP). Without
    //              system_clipget we fall through to the cut-buffer path.
    let buf: Vec<char> = if zle.zmod.flags & MOD_VIBUF != 0 {
        let idx = zle.zmod.vibuf as usize;
        if idx >= zle.vibuf.len() {
            return 1;
        }
        zle.vibuf[idx].clone()                                               // c:667
    } else {
        zle.killring.front().cloned().unwrap_or_default()                                                   // c:669
    };
    if buf.is_empty() {
        return 1;                                                            // c:671
    }
    pastebuf(zle, &buf, n, 1)                                                // c:675
}

/// Port of `viputbefore(args)` from Src/Zle/zle_misc.c:607.
pub fn viputbefore(zle: &mut Zle) -> i32 {                                   // c:607
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = zle.zmod.mult;                                                   // c:610
    startvichange(zle, -1);                                                  // c:612
    if n < 0 {
        return 1;                                                            // c:614
    }
    if zle.zmod.flags & MOD_NULL != 0 {
        return 0;                                                            // c:616
    }
    let buf: Vec<char> = if zle.zmod.flags & MOD_VIBUF != 0 {
        let idx = zle.zmod.vibuf as usize;
        if idx >= zle.vibuf.len() {
            return 1;
        }
        zle.vibuf[idx].clone()                                               // c:631
    } else {
        zle.killring.front().cloned().unwrap_or_default()                                                   // c:633
    };
    if buf.is_empty() {
        return 1;                                                            // c:635
    }
    pastebuf(zle, &buf, n, 0)                                                // c:639
}

/// Port of `whatcursorposition(args)` from Src/Zle/zle_misc.c:850.
pub fn whatcursorposition(zle: &mut Zle) -> i32 {                            // c:850
    use crate::ported::zle::zle_utils::findbol;
    let bol = findbol(zle);                                                  // c:855
    let mut msg = String::with_capacity(100);
    if zle.zlecs == zle.zlell {                                              // c:858
        msg.push_str("EOF");                                                 // c:859
    } else {
        msg.push_str("Char: ");                                              // c:861
        let c = zle.zleline[zle.zlecs];                                      // c:856
        match c {
            ' ' => msg.push_str("SPC"),                                      // c:864
            '\t' => msg.push_str("TAB"),                                     // c:867
            '\n' => msg.push_str("LFD"),                                     // c:870
            _ => msg.push(c),                                                // c:878
        }
        let cu = c as u32;
        msg.push_str(&format!(" (0{:o}, {}, 0x{:x})", cu, cu, cu));          // c:881
    }
    let pct = if zle.zlell > 0 { 100 * zle.zlecs / zle.zlell } else { 0 };
    msg.push_str(&format!(
        "  point {} of {}({}%)  column {}",
        zle.zlecs + 1,
        zle.zlell + 1,
        pct,
        zle.zlecs - bol,
    ));                                                                      // c:884
    tracing::info!(target: "zle", "{}", msg);                                // c:887 — showmsg
    0
}

/// Port of `yankpop(args)` from Src/Zle/zle_misc.c:728.
pub fn yankpop(zle: &mut Zle) -> i32 {                                       // c:728
    use crate::ported::zle::widget::WidgetFlags;
    // c:730-735 — `if (!(lastcmd & ZLE_YANK) || !kring || !kctbuf)
    //               return 1`.
    let last = WidgetFlags::from_bits_truncate(
        crate::ported::zle::zle_main::LASTCMD.load(std::sync::atomic::Ordering::SeqCst),
    );
    if !last.contains(WidgetFlags::YANK) || zle.killring.is_empty() {
        return 1;
    }
    // C body cycles the kill ring index `kct` and re-inserts the
    // previous yank. zshrs uses VecDeque<ZleString> with the rotation
    // index `yank_ring_idx`. Simplified: rotate front entry to back,
    // delete previous yank text from line, insert new front.
    let prev_start = zle.yank_start;
    let prev_end   = zle.yank_end;
    if prev_end > prev_start && prev_end <= zle.zleline.len() {
        zle.zleline.drain(prev_start..prev_end);
        zle.zlell -= prev_end - prev_start;
        zle.zlecs = prev_start;
    }
    if let Some(top) = zle.killring.pop_front() {
        zle.killring.push_back(top);
    }
    if let Some(next) = zle.killring.front().cloned() {
        for (i, &c) in next.iter().enumerate() {
            zle.zleline.insert(zle.zlecs + i, c);
        }
        zle.yank_start = zle.zlecs;
        zle.zlecs += next.len();
        zle.zlell += next.len();
        zle.yank_end = zle.zlecs;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn acceptline_sets_done() {
        // c:401-404 — `done = 1; return 0`.
        DONE.store(0, Ordering::SeqCst);
        let r = acceptline();
        assert_eq!(r, 0);
        assert_eq!(DONE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn undefinedkey_returns_one() {
        // c:892-894 — single `return 1` body.
        assert_eq!(undefinedkey(), 1);
    }

    #[test]
    fn sendbreak_sets_errflag_and_returns_one() {
        use crate::ported::zsh_h::{ERRFLAG_ERROR, ERRFLAG_INT};
        use std::sync::atomic::Ordering;
        // Reset errflag so the OR-set is observable.
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
        let r = sendbreak();
        // c:1147 — return 1.
        assert_eq!(r, 1);
        // c:1146 — both ERRFLAG_ERROR | ERRFLAG_INT set.
        let f = crate::ported::utils::errflag.load(Ordering::Relaxed);
        assert!(f & ERRFLAG_ERROR != 0);
        assert!(f & ERRFLAG_INT != 0);
        // Reset for other tests.
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
    }

    #[test]
    fn sendbreak_preserves_existing_errflag_bits() {
        use std::sync::atomic::Ordering;
        // c:1146 — `errflag |= ...` (OR-equal, not assign).
        crate::ported::utils::errflag.store(0x1000, Ordering::Relaxed); // pretend bit 12 was set
        sendbreak();
        let f = crate::ported::utils::errflag.load(Ordering::Relaxed);
        // Pre-existing bit preserved.
        assert!(f & 0x1000 != 0);
        // New bits also set.
        assert!(f & crate::ported::zsh_h::ERRFLAG_ERROR != 0);
        assert!(f & crate::ported::zsh_h::ERRFLAG_INT != 0);
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
    }

    // ---------- negargument / overwritemode real-port tests ----------

    #[test]
    fn negargument_sets_tmult_neg_prefix() {
        // c:976-981 — sets tmult=-1 + TMULT|NEG flags + prefixflag.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        // Ensure clean modifier state.
        z.zmod.tmult = 1;
        z.zmod.flags = 0;
        crate::ported::zle::zle_main::PREFIXFLAG.store(0, std::sync::atomic::Ordering::SeqCst);
        let r = negargument(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.tmult, -1);
        assert!(z.zmod.flags & MOD_TMULT != 0);
        assert!(z.zmod.flags & MOD_NEG != 0);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
    }

    #[test]
    fn negargument_refuses_when_tmult_in_flight() {
        // c:976-977 — if MOD_TMULT already set → return 1.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        z.zmod.flags |= MOD_TMULT;
        z.zmod.tmult = 7; // some pre-existing value
        let r = negargument(&mut z);
        assert_eq!(r, 1);
        // tmult NOT clobbered (early return).
        assert_eq!(z.zmod.tmult, 7);
    }

    #[test]
    fn overwritemode_toggles_insmode() {
        // c:845 — `insmode ^= 1`.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
        overwritemode(&mut z);
        assert_eq!(crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst), 0);
        overwritemode(&mut z);
        assert_eq!(crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // ---------- argumentbase real-port tests ----------

    #[test]
    fn argumentbase_with_arg_sets_base() {
        // c:1043 — parse arg, c:1050 set zmod.base.
        let mut z = Zle::new();
        let r = argumentbase(&mut z, &["8".to_string()]);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.base, 8);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
        // c:1053-1056 — modifier reset.
        assert_eq!(z.zmod.mult, 1);
        assert_eq!(z.zmod.tmult, 1);
        assert_eq!(z.zmod.vibuf, 0);
    }

    #[test]
    fn argumentbase_no_arg_uses_zmod_mult() {
        // c:1045 — fallback to zmod.mult when no arg.
        let mut z = Zle::new();
        z.zmod.mult = 16;
        argumentbase(&mut z, &[]);
        assert_eq!(z.zmod.base, 16);
    }

    #[test]
    fn argumentbase_rejects_below_two() {
        // c:1047-1048 — base < 2 → return 1, state unchanged.
        let mut z = Zle::new();
        z.zmod.base = 10;
        let r = argumentbase(&mut z, &["1".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(z.zmod.base, 10); // unchanged
    }

    #[test]
    fn argumentbase_rejects_above_36() {
        // c:1047-1048 — base > 36 → return 1.
        let mut z = Zle::new();
        z.zmod.base = 10;
        let r = argumentbase(&mut z, &["100".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(z.zmod.base, 10);
    }

    #[test]
    fn argumentbase_hex_prefix() {
        // c:1043 — `zstrtol(s, NULL, 0)`: '0x10' → 16.
        let mut z = Zle::new();
        argumentbase(&mut z, &["0x10".to_string()]);
        assert_eq!(z.zmod.base, 16);
    }

    #[test]
    fn argumentbase_octal_prefix() {
        // c:1043 — '010' → octal 8.
        let mut z = Zle::new();
        argumentbase(&mut z, &["010".to_string()]);
        assert_eq!(z.zmod.base, 8);
    }

    // ---------- parsedigit real-port tests ----------

    #[test]
    fn parsedigit_decimal_base() {
        // c:1092 — base=10, '0'..'9' → 0..9.
        let mut z = Zle::new();
        z.zmod.base = 10;
        assert_eq!(parsedigit(&z, b'0' as i32), 0);
        assert_eq!(parsedigit(&z, b'5' as i32), 5);
        assert_eq!(parsedigit(&z, b'9' as i32), 9);
        // Out of range for base 10
        assert_eq!(parsedigit(&z, b'a' as i32), -1);
    }

    #[test]
    fn parsedigit_octal_base() {
        // c:1092 — base=8, '0'..'7'.
        let mut z = Zle::new();
        z.zmod.base = 8;
        assert_eq!(parsedigit(&z, b'7' as i32), 7);
        // '8' rejected (out of range for octal).
        assert_eq!(parsedigit(&z, b'8' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_lowercase() {
        // c:1083 — base=16, 'a'..'f' → 10..15.
        let mut z = Zle::new();
        z.zmod.base = 16;
        assert_eq!(parsedigit(&z, b'a' as i32), 10);
        assert_eq!(parsedigit(&z, b'f' as i32), 15);
        // 'g' out of range (only a..f for base 16).
        assert_eq!(parsedigit(&z, b'g' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_uppercase() {
        // c:1085 — base=16, 'A'..'F' → 10..15.
        let mut z = Zle::new();
        z.zmod.base = 16;
        assert_eq!(parsedigit(&z, b'A' as i32), 10);
        assert_eq!(parsedigit(&z, b'F' as i32), 15);
    }

    #[test]
    fn parsedigit_hex_digits_still_work() {
        // c:1087 — base > 10 still accepts '0'..'9' via idigit branch.
        let mut z = Zle::new();
        z.zmod.base = 16;
        assert_eq!(parsedigit(&z, b'7' as i32), 7);
    }

    #[test]
    fn parsedigit_strips_meta_bit() {
        // c:1077 — `inkey &= 0x7f`. 0xb5 = '5' | 0x80 → strips to '5'.
        let mut z = Zle::new();
        z.zmod.base = 10;
        assert_eq!(parsedigit(&z, 0x80 | (b'5' as i32)), 5);
    }

    // ---------- digitargument real-port tests ----------

    #[test]
    fn digitargument_first_digit_no_tmult() {
        // c:1050-1051 — `if (!TMULT) tmult = 0`. First digit: tmult=0
        // then tmult = 0*10 + 1*5 = 5.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        z.zmod.flags = 0;
        z.zmod.base = 10;
        z.zmod.mult = 1; // sign = 1
        crate::ported::zle::compcore::LASTCHAR.store((b'5' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        let r = digitargument(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.tmult, 5);
        assert!(z.zmod.flags & MOD_TMULT != 0);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
    }

    #[test]
    fn digitargument_second_digit_accumulates() {
        // c:1058 — second digit: tmult = 5*10 + 1*7 = 57.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        z.zmod.flags = MOD_TMULT;
        z.zmod.tmult = 5;
        z.zmod.base = 10;
        z.zmod.mult = 1; // sign = 1
        crate::ported::zle::compcore::LASTCHAR.store((b'7' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        digitargument(&mut z);
        assert_eq!(z.zmod.tmult, 57);
    }

    #[test]
    fn digitargument_invalid_returns_one() {
        // c:1047-1048 — parsedigit < 0 → return 1.
        let mut z = Zle::new();
        z.zmod.base = 10;
        crate::ported::zle::compcore::LASTCHAR.store((b'a' as i32) as i32, std::sync::atomic::Ordering::SeqCst); // not a decimal digit
        assert_eq!(digitargument(&mut z), 1);
    }

    #[test]
    fn digitargument_neg_flag_replaces_tmult() {
        // c:1054-1056 — MOD_NEG: tmult = sign * newdigit, NEG cleared.
        // sign = -1 (zmult<0); first digit '3' → tmult = -1*3 = -3.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        z.zmod.flags = MOD_TMULT | MOD_NEG;
        z.zmod.tmult = -1;  // set by negargument
        z.zmod.base = 10;
        z.zmod.mult = -1;   // negative → sign = -1
        crate::ported::zle::compcore::LASTCHAR.store((b'3' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        digitargument(&mut z);
        assert_eq!(z.zmod.tmult, -3);
        // NEG cleared.
        assert!(!z.zmod.flags & MOD_NEG != 0);
        assert!(z.zmod.flags & MOD_TMULT != 0);
    }

    // ---------- transpose_swap real-port tests ----------

    #[test]
    fn transpose_swap_equal_halves() {
        // c:254 — swap two equal-length adjacent slices.
        let mut z = Zle::new();
        z.zleline = "abcdef".chars().collect();
        z.zlell = 6;
        // Swap [0..2]="ab" with [2..4]="cd" → "cdabef".
        transpose_swap(&mut z, 0, 2, 4);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "cdabef");
    }

    #[test]
    fn transpose_swap_unequal_halves() {
        // First chunk len 1, second len 3.
        let mut z = Zle::new();
        z.zleline = "abcdef".chars().collect();
        z.zlell = 6;
        // Swap [0..1]="a" with [1..4]="bcd" → "bcdaef".
        transpose_swap(&mut z, 0, 1, 4);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "bcdaef");
    }

    #[test]
    fn transpose_swap_first_longer() {
        // First chunk len 3, second len 1.
        let mut z = Zle::new();
        z.zleline = "abcdef".chars().collect();
        z.zlell = 6;
        // Swap [0..3]="abc" with [3..4]="d" → "dabcef".
        transpose_swap(&mut z, 0, 3, 4);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "dabcef");
    }

    #[test]
    fn transpose_swap_mid_buffer() {
        // Swap not at the start.
        let mut z = Zle::new();
        z.zleline = "0123456789".chars().collect();
        z.zlell = 10;
        // Swap [3..5]="34" with [5..7]="56" → "0125634789".
        transpose_swap(&mut z, 3, 5, 7);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "0125634789");
    }

    // ---------- Batch tests for fixunmeta/selfinsert/deletechar/etc ----------

    #[test]
    fn fixunmeta_strips_meta_and_normalizes_cr() {
        let mut z = Zle::new();
        crate::ported::zle::compcore::LASTCHAR.store((0x80 | b'a' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        fixunmeta(&mut z);
        assert_eq!(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst), b'a' as i32);
        crate::ported::zle::compcore::LASTCHAR.store((b'\r' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        fixunmeta(&mut z);
        assert_eq!(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst), b'\n' as i32);
    }

    #[test]
    fn selfinsert_inserts_lastchar() {
        let mut z = Zle::new();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        z.zlecs = 1;
        crate::ported::zle::compcore::LASTCHAR.store((b'X' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(0, std::sync::atomic::Ordering::SeqCst);
        selfinsert(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "aXbc");
    }

    #[test]
    fn selfinsertunmeta_chains_fixunmeta_and_selfinsert() {
        let mut z = Zle::new();
        z.zleline = "ab".chars().collect();
        z.zlell = 2;
        z.zlecs = 1;
        crate::ported::zle::compcore::LASTCHAR.store((0x80 | b'X' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(0, std::sync::atomic::Ordering::SeqCst);
        selfinsertunmeta(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "aXb");
    }

    #[test]
    fn deletechar_removes_n_chars() {
        let mut z = Zle::new();
        z.zleline = "hello".chars().collect();
        z.zlell = 5;
        z.zlecs = 0;
        z.zmod.mult = 2;
        let r = deletechar(&mut z);
        assert_eq!(r, 0);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "llo");
    }

    #[test]
    fn deletechar_returns_one_at_eol() {
        let mut z = Zle::new();
        z.zleline = "ab".chars().collect();
        z.zlell = 2;
        z.zlecs = 2;
        z.zmod.mult = 1;
        assert_eq!(deletechar(&mut z), 1);
    }

    #[test]
    fn backwarddeletechar_clamps_to_zlecs() {
        let mut z = Zle::new();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        z.zlecs = 2;
        z.zmod.mult = 99;
        backwarddeletechar(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "c");
        assert_eq!(z.zlecs, 0);
    }

    #[test]
    fn killline_kills_to_eol_and_pushes_killring() {
        let mut z = Zle::new();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 6;
        z.zmod.mult = 1;
        killline(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "hello ");
        assert_eq!(z.zlecs, 6);
        assert_eq!(z.killring.front().map(|v| v.iter().collect::<String>()),
                   Some("world".to_string()));
    }

    #[test]
    fn killbuffer_clears_and_pushes() {
        let mut z = Zle::new();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        z.zlecs = 2;
        killbuffer(&mut z);
        assert!(z.zleline.is_empty());
        assert_eq!(z.zlell, 0);
        assert_eq!(z.zlecs, 0);
        assert_eq!(z.killring.front().map(|v| v.iter().collect::<String>()),
                   Some("abc".to_string()));
    }

    #[test]
    fn killwholeline_drops_one_line() {
        let mut z = Zle::new();
        z.zleline = "abc\ndef\nghi".chars().collect();
        z.zlell = 11;
        z.zlecs = 5; // 'e' in 'def'
        z.zmod.mult = 1;
        killwholeline(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "abc\nghi");
    }

    #[test]
    fn copyregionaskill_copies_between_point_mark() {
        let mut z = Zle::new();
        z.zleline = "hello".chars().collect();
        z.zlell = 5;
        z.zlecs = 0;
        z.mark = 3;
        copyregionaskill(&mut z, &[]);
        assert_eq!(z.killring.front().map(|v| v.iter().collect::<String>()),
                   Some("hel".to_string()));
        // Buffer unchanged
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "hello");
    }

    #[test]
    fn regionlines_returns_bol_eol_around_region() {
        let mut z = Zle::new();
        z.zleline = "abc\ndef\nghi".chars().collect();
        z.zlell = 11;
        z.zlecs = 1;
        z.mark = 5;
        let (start, end) = regionlines(&mut z);
        // mark > zlecs branch: start=findbol(zlecs)=0, end=findeol(mark)=7
        assert_eq!(start, 0);
        assert_eq!(end, 7);
    }

    #[test]
    fn killregion_drains_between_mark_and_cursor() {
        let mut z = Zle::new();
        z.zleline = "abcdef".chars().collect();
        z.zlell = 6;
        z.zlecs = 1;
        z.mark = 4;
        killregion(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "aef");
        assert_eq!(z.killring.front().map(|v| v.iter().collect::<String>()),
                   Some("bcd".to_string()));
    }

    #[test]
    fn quoteline_wraps_in_single_quotes() {
        let mut z = Zle::new();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        quoteline(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "'abc'");
    }

    #[test]
    fn quoteline_escapes_internal_quote() {
        let mut z = Zle::new();
        z.zleline = "it's".chars().collect();
        z.zlell = 4;
        quoteline(&mut z);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "'it'\\''s'");
    }

    #[test]
    fn makequote_handles_no_quotes() {
        let s: Vec<char> = "abc".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'abc'");
    }

    #[test]
    fn makequote_escapes_quotes() {
        let s: Vec<char> = "a'b".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'a'\\''b'");
    }

    #[test]
    fn pastebuf_inserts_at_cursor_position_zero() {
        let mut z = Zle::new();
        z.zleline = "foo".chars().collect();
        z.zlell = 3;
        z.zlecs = 1;
        let buf: Vec<char> = "XX".chars().collect();
        pastebuf(&mut z, &buf, 1, 0);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "fXXoo");
    }

    #[test]
    fn pastebuf_inserts_after_cursor_position_one() {
        let mut z = Zle::new();
        z.zleline = "foo".chars().collect();
        z.zlell = 3;
        z.zlecs = 1;
        let buf: Vec<char> = "XX".chars().collect();
        pastebuf(&mut z, &buf, 1, 1);
        // position=1 → INCCS first → insert at zlecs+1
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "foXXo");
    }

    #[test]
    fn yankpop_returns_one_when_lastcmd_not_yank() {
        let mut z = Zle::new();
        // Default lastcmd = empty (no YANK flag).
        assert_eq!(yankpop(&mut z), 1);
    }

    #[test]
    fn zle_usable_when_active_and_no_compfunc() {
        use crate::ported::builtins::sched::zleactive;
        use crate::ported::zle::complete::INCOMPFUNC;
        use std::sync::atomic::Ordering;
        zleactive.store(1, Ordering::SeqCst);
        INCOMPFUNC.store(0, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 1);
        // With incompfunc set → 0
        INCOMPFUNC.store(1, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
        // Reset
        INCOMPFUNC.store(0, Ordering::SeqCst);
        zleactive.store(0, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
    }
}
