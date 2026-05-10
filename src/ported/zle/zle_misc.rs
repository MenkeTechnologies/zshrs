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

/// Port of `int suffixnoinsrem` from `Src/Zle/zle_misc.c:1549`.
/// Suppresses inserted-character suffix removal when set.
pub static SUFFIXNOINSREM: AtomicI32 = AtomicI32::new(0);                    // c:1549

/// Clipboard/paste buffer for yank operations
// yankb, yanke; mark the start and end of last yank in editing buffer.    // c:526
// The original cutbuffer, either cutbuf or one of the vi buffers.         // c:528
#[derive(Debug, Default)]
pub struct PasteBuffer {
    pub content: Vec<char>,
}

impl Zle {
    // insert a zle string, with repetition and suffix removal              // c:33

    /// Self insert - insert the typed character
    /// Port of selfinsert() from zle_misc.c
    pub fn self_insert(&mut self, c: char) {                                 // c:113
        self.zleline.insert(self.zlecs, c);
        self.zlecs += 1;
        self.zlell += 1;
        self.resetneeded = true;
    }

    /// Self insert unmeta - insert character with meta bit stripped
    /// Port of selfinsertunmeta() from zle_misc.c
    pub fn self_insert_unmeta(&mut self, c: char) {                          // c:149
        let unmetaed = if (c as u32) >= 0x80 && (c as u32) < 0x100 {
            char::from_u32((c as u32) & 0x7f).unwrap_or(c)
        } else {
            c
        };
        self.self_insert(unmetaed);
    }

    /// Accept line - return the current line for execution
    /// Port of acceptline() from zle_misc.c
    pub fn accept_line(&self) -> String {                                    // c:401
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
    pub fn delete_char(&mut self) {                                          // c:157
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
    pub fn kill_line(&mut self) {                                            // c:419
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

    /// Swap cursor and mark.
    /// Port of `exchangepointandmark()` from Src/Zle/zle_move.c:496. The
    /// C source has additional zmult-based behaviour (zmult==0 just
    /// activates the region without swapping; zmult>0 also activates).
    /// This bare method only swaps; the widget-level
    /// `widget_exchange_point_and_mark` honours the count semantics.
    pub fn exchange_point_and_mark(&mut self) {
        std::mem::swap(&mut self.zlecs, &mut self.mark);
        self.resetneeded = true;
    }

    /// Set mark at the current cursor position.
    /// Port of `setmarkcommand()` from Src/Zle/zle_move.c:483 with the
    /// activate-region branch elided. The widget-level
    /// `widget_set_mark_command` covers the negative-count
    /// deactivate path that the bare C source supports.
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

    /// Capitalize the next word: title-case the first letter, lowercase
    /// the rest of the word.
    /// Port of `capitalizeword()` from Src/Zle/zle_word.c (the C source
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

        self.resetneeded = true;
    }

    /// Lowercase the next word.
    /// Port of `downcaseword()` from Src/Zle/zle_word.c — calls
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

        self.resetneeded = true;
    }

    /// Uppercase the next word.
    /// Port of `upcaseword()` from Src/Zle/zle_word.c — calls
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
            (self.zlecs * 100).checked_div(self.zlell).unwrap_or(0)
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

/// Port of `acceptandhold()` from Src/Zle/zle_misc.c:409. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn acceptandhold() -> i32 { 0 }

/// Port of `acceptline()` from `Src/Zle/zle_misc.c:401`.
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
/// Port of `addsuffix()` from Src/Zle/zle_misc.c:1558. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addsuffix() -> i32 { 0 }                                              // c:1558

/// Port of `addsuffixstring()` from Src/Zle/zle_misc.c:1580. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addsuffixstring() -> i32 { 0 }

/// Port of `argumentbase()` from `Src/Zle/zle_misc.c:1037`.
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
pub fn argumentbase(zle: &mut Zle, args: &[String]) -> i32 {                 // c:1037
    use super::zle_main::ModifierFlags;
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
    zle.zmod.flags = ModifierFlags::empty();
    zle.zmod.mult = 1;
    zle.zmod.tmult = 1;
    zle.zmod.vibuf = 0;
    // c:1059 — still operating on prefix arg.
    zle.prefixflag = true;
    0                                                                        // c:1061 return 0
}

/// Port of `backwarddeletechar()` from Src/Zle/zle_misc.c:180. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwarddeletechar() -> i32 { 0 }

/// Port of `backwardkillline()` from Src/Zle/zle_misc.c:225. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwardkillline() -> i32 { 0 }

/// Port of `bracketedpaste()` from Src/Zle/zle_misc.c:814. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bracketedpaste() -> i32 { 0 }

/// Port of `bracketedstring()` from Src/Zle/zle_misc.c:784. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bracketedstring() -> i32 { 0 }

/// Port of `copyprevshellword()` from Src/Zle/zle_misc.c:1108. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn copyprevshellword() -> i32 { 0 }

/// Port of `copyprevword()` from Src/Zle/zle_misc.c:1066. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn copyprevword() -> i32 { 0 }

/// Port of `copyregionaskill()` from Src/Zle/zle_misc.c:494. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn copyregionaskill() -> i32 { 0 }

/// Port of `deletechar()` from Src/Zle/zle_misc.c:157. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deletechar() -> i32 { 0 }

/// Port of `digitargument()` from Src/Zle/zle_misc.c:950. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn digitargument(zle: &mut Zle) -> i32 {                                 // c:1042
    use super::zle_main::ModifierFlags;
    // c:1044 — `int sign = (zmult < 0) ? -1 : 1`.
    let sign: i32 = if zle.zmod.mult < 0 { -1 } else { 1 };
    // c:1045 — `parsedigit(lastchar)`.
    let newdigit = parsedigit(zle, zle.lastchar);
    if newdigit < 0 {                                                        // c:1047
        return 1;                                                            // c:1048
    }
    // c:1050-1051 — `if (!(zmod.flags & MOD_TMULT)) zmod.tmult = 0`.
    if !zle.zmod.flags.contains(ModifierFlags::TMULT) {
        zle.zmod.tmult = 0;
    }
    // c:1052-1057 — MOD_NEG path: replace tmult with sign*newdigit.
    if zle.zmod.flags.contains(ModifierFlags::NEG) {
        zle.zmod.tmult = sign * newdigit;
        zle.zmod.flags.remove(ModifierFlags::NEG);
    } else {
        // c:1058 — `zmod.tmult = zmod.tmult * zmod.base + sign*newdigit`.
        zle.zmod.tmult = zle.zmod.tmult * zle.zmod.base + sign * newdigit;
    }
    zle.zmod.flags.insert(ModifierFlags::TMULT);                             // c:1059
    zle.prefixflag = true;                                                   // c:1060
    0                                                                        // c:1061
}

/// Port of `doinsert()` from Src/Zle/zle_misc.c:37. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doinsert() -> i32 { 0 }

/// Port of `executenamedcommand()` from Src/Zle/zle_misc.c:1261. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn executenamedcommand() -> i32 { 0 }

// Fix the suffix in place, if there is one, making it non-removable.      // c:1820
/// Port of `fixsuffix()` from Src/Zle/zle_misc.c:1824. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn fixsuffix() -> i32 { 0 }                                              // c:1824

/// Port of `fixunmeta()` from Src/Zle/zle_misc.c:130. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn fixunmeta() -> i32 { 0 }

/// Port of `gosmacstransposechars()` from Src/Zle/zle_misc.c:274. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn gosmacstransposechars() -> i32 { 0 }

// Remove suffix, if there is one, when inserting character c.             // c:1695
/// Port of `iremovesuffix()` from Src/Zle/zle_misc.c:1699. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn iremovesuffix() -> i32 { 0 }                                          // c:1699

/// Port of `killbuffer()` from Src/Zle/zle_misc.c:215. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn killbuffer() -> i32 { 0 }

/// Port of `killline()` from Src/Zle/zle_misc.c:419. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn killline() -> i32 { 0 }

/// Port of `killregion()` from Src/Zle/zle_misc.c:463. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn killregion() -> i32 { 0 }

/// Port of `killwholeline()` from Src/Zle/zle_misc.c:195. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn killwholeline() -> i32 { 0 }

/// Port of `makeparamsuffix()` from Src/Zle/zle_misc.c:1623. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makeparamsuffix() -> i32 { 0 }

/// Port of `makequote()` from Src/Zle/zle_misc.c:1201. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makequote() -> i32 { 0 }

/// Port of `makesuffix()` from Src/Zle/zle_misc.c:1598. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makesuffix() -> i32 { 0 }

/// Port of `makesuffixstr()` from Src/Zle/zle_misc.c:1642. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makesuffixstr() -> i32 { 0 }

/// Port of `negargument()` from `Src/Zle/zle_misc.c:974`.
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
pub fn negargument(zle: &mut Zle) -> i32 {                                   // c:974
    use super::zle_main::ModifierFlags;
    if zle.zmod.flags.contains(ModifierFlags::TMULT) {                       // c:976
        return 1;                                                            // c:977
    }
    zle.zmod.tmult = -1;                                                     // c:978
    zle.zmod.flags.insert(ModifierFlags::TMULT | ModifierFlags::NEG);        // c:979
    zle.prefixflag = true;                                                   // c:980
    0                                                                        // c:981 return 0
}

/// Port of `overwritemode()` from `Src/Zle/zle_misc.c:842`.
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
    zle.insmode = !zle.insmode;                                              // c:845 insmode ^= 1
    0                                                                        // c:846 return 0
}

/// Port of `parsedigit()` from Src/Zle/zle_misc.c:919. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
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

/// Port of `pastebuf()` from Src/Zle/zle_misc.c:558. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pastebuf() -> i32 { 0 }

/// Port of `poundinsert()` from Src/Zle/zle_misc.c:369. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn poundinsert() -> i32 { 0 }

/// Port of `putreplaceselection()` from Src/Zle/zle_misc.c:680. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn putreplaceselection() -> i32 { 0 }

/// Port of `quotedinsert()` from Src/Zle/zle_misc.c:899. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn quotedinsert() -> i32 { 0 }

/// Port of `quoteline()` from Src/Zle/zle_misc.c:1187. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn quoteline() -> i32 { 0 }

/// Port of `quoteregion()` from Src/Zle/zle_misc.c:1152. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn quoteregion() -> i32 { 0 }

/// Port of `regionlines()` from Src/Zle/zle_misc.c:444. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn regionlines() -> i32 { 0 }

/// Port of `scancompcmd()` from Src/Zle/zle_misc.c:1235. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn scancompcmd() -> i32 { 0 }

/// Port of `selfinsert()` from Src/Zle/zle_misc.c:113. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn selfinsert() -> i32 { 0 }

/// Port of `selfinsertunmeta()` from Src/Zle/zle_misc.c:149. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn selfinsertunmeta() -> i32 { 0 }

/// Port of `sendbreak()` from `Src/Zle/zle_misc.c:1144`.
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
    let cur = crate::ported::utils::errflag();
    crate::ported::utils::set_errflag(
        cur | crate::ported::zsh_h::ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_INT,
    );
    1                                                                        // c:1147 return 1
}

/// Port of `transpose_swap()` from `Src/Zle/zle_misc.c:254`.
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

/// Port of `transposechars()` from Src/Zle/zle_misc.c:313. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn transposechars() -> i32 { 0 }

/// Port of `undefinedkey()` from `Src/Zle/zle_misc.c:892`.
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

/// Port of `universalargument()` from Src/Zle/zle_misc.c:986. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn universalargument() -> i32 { 0 }

/// Port of `viputafter()` from Src/Zle/zle_misc.c:644. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viputafter() -> i32 { 0 }

/// Port of `viputbefore()` from Src/Zle/zle_misc.c:608. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viputbefore() -> i32 { 0 }

/// Port of `whatcursorposition()` from Src/Zle/zle_misc.c:851. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn whatcursorposition() -> i32 { 0 }

/// Port of `yankpop()` from Src/Zle/zle_misc.c:728. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn yankpop() -> i32 { 0 }

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
        // Reset errflag so the OR-set is observable.
        crate::ported::utils::set_errflag(0);
        let r = sendbreak();
        // c:1147 — return 1.
        assert_eq!(r, 1);
        // c:1146 — both ERRFLAG_ERROR | ERRFLAG_INT set.
        let f = crate::ported::utils::errflag();
        assert!(f & ERRFLAG_ERROR != 0);
        assert!(f & ERRFLAG_INT != 0);
        // Reset for other tests.
        crate::ported::utils::set_errflag(0);
    }

    #[test]
    fn sendbreak_preserves_existing_errflag_bits() {
        // c:1146 — `errflag |= ...` (OR-equal, not assign).
        crate::ported::utils::set_errflag(0x1000); // pretend bit 12 was set
        sendbreak();
        let f = crate::ported::utils::errflag();
        // Pre-existing bit preserved.
        assert!(f & 0x1000 != 0);
        // New bits also set.
        assert!(f & crate::ported::zsh_h::ERRFLAG_ERROR != 0);
        assert!(f & crate::ported::zsh_h::ERRFLAG_INT != 0);
        crate::ported::utils::set_errflag(0);
    }

    // ---------- negargument / overwritemode real-port tests ----------

    #[test]
    fn negargument_sets_tmult_neg_prefix() {
        // c:976-981 — sets tmult=-1 + TMULT|NEG flags + prefixflag.
        use super::super::zle_main::ModifierFlags;
        let mut z = Zle::new();
        // Ensure clean modifier state.
        z.zmod.tmult = 1;
        z.zmod.flags = ModifierFlags::empty();
        z.prefixflag = false;
        let r = negargument(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.tmult, -1);
        assert!(z.zmod.flags.contains(ModifierFlags::TMULT));
        assert!(z.zmod.flags.contains(ModifierFlags::NEG));
        assert!(z.prefixflag);
    }

    #[test]
    fn negargument_refuses_when_tmult_in_flight() {
        // c:976-977 — if MOD_TMULT already set → return 1.
        use super::super::zle_main::ModifierFlags;
        let mut z = Zle::new();
        z.zmod.flags.insert(ModifierFlags::TMULT);
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
        z.insmode = true;
        overwritemode(&mut z);
        assert!(!z.insmode);
        overwritemode(&mut z);
        assert!(z.insmode);
    }

    // ---------- argumentbase real-port tests ----------

    #[test]
    fn argumentbase_with_arg_sets_base() {
        // c:1043 — parse arg, c:1050 set zmod.base.
        let mut z = Zle::new();
        let r = argumentbase(&mut z, &["8".to_string()]);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.base, 8);
        assert!(z.prefixflag);
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
        use super::super::zle_main::ModifierFlags;
        let mut z = Zle::new();
        z.zmod.flags = ModifierFlags::empty();
        z.zmod.base = 10;
        z.zmod.mult = 1; // sign = 1
        z.lastchar = b'5' as i32;
        let r = digitargument(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zmod.tmult, 5);
        assert!(z.zmod.flags.contains(ModifierFlags::TMULT));
        assert!(z.prefixflag);
    }

    #[test]
    fn digitargument_second_digit_accumulates() {
        // c:1058 — second digit: tmult = 5*10 + 1*7 = 57.
        use super::super::zle_main::ModifierFlags;
        let mut z = Zle::new();
        z.zmod.flags = ModifierFlags::TMULT;
        z.zmod.tmult = 5;
        z.zmod.base = 10;
        z.zmod.mult = 1; // sign = 1
        z.lastchar = b'7' as i32;
        digitargument(&mut z);
        assert_eq!(z.zmod.tmult, 57);
    }

    #[test]
    fn digitargument_invalid_returns_one() {
        // c:1047-1048 — parsedigit < 0 → return 1.
        let mut z = Zle::new();
        z.zmod.base = 10;
        z.lastchar = b'a' as i32; // not a decimal digit
        assert_eq!(digitargument(&mut z), 1);
    }

    #[test]
    fn digitargument_neg_flag_replaces_tmult() {
        // c:1054-1056 — MOD_NEG: tmult = sign * newdigit, NEG cleared.
        // sign = -1 (zmult<0); first digit '3' → tmult = -1*3 = -3.
        use super::super::zle_main::ModifierFlags;
        let mut z = Zle::new();
        z.zmod.flags = ModifierFlags::TMULT | ModifierFlags::NEG;
        z.zmod.tmult = -1;  // set by negargument
        z.zmod.base = 10;
        z.zmod.mult = -1;   // negative → sign = -1
        z.lastchar = b'3' as i32;
        digitargument(&mut z);
        assert_eq!(z.zmod.tmult, -3);
        // NEG cleared.
        assert!(!z.zmod.flags.contains(ModifierFlags::NEG));
        assert!(z.zmod.flags.contains(ModifierFlags::TMULT));
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
}
