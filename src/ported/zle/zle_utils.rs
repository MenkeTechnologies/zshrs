//! ZLE utility functions
//!
//! Direct port from zsh/Src/Zle/zle_utils.c
//!
//! Primary cut buffer                                                        // c:33
//! Emacs-style kill buffer ring                                              // c:38
//! the line before last mod (for undo purposes)                              // c:51
//! make sure that the line buffer has at least sz chars                      // c:63
//! undo system                                                               // c:1421
//! head of the undo list, and the current position                           // c:1424
//!
//! Implements:
//! - Line manipulation: setline, sizeline, spaceinline, shiftchars
//! - Undo: initundo, freeundo, handleundo, mkundoent, undo, redo
//! - Cut/paste: cut, cuttext, foredel, backdel, forekill, backkill
//! - Cursor: findbol, findeol, findline
//! - Conversion: zlelineasstring, stringaszleline, zlecharasstring
//! - Display: showmsg, printbind, handlefeep
//! - Position save/restore: zle_save_positions, zle_restore_positions

use super::zle_main::{Zle, ZleChar, ZleString};

impl Zle {
    /// Insert string at cursor position
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
            self.zlell += 1;
        }
        self.resetneeded = true;
    }

    /// Insert chars at cursor position
    pub fn insert_chars(&mut self, chars: &[ZleChar]) {
        for &c in chars {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
            self.zlell += 1;
        }
        self.resetneeded = true;
    }

    /// Delete n characters at cursor position
    pub fn delete_chars(&mut self, n: usize) {
        let n = n.min(self.zlell - self.zlecs);
        for _ in 0..n {
            if self.zlecs < self.zlell {
                self.zleline.remove(self.zlecs);
                self.zlell -= 1;
            }
        }
        self.resetneeded = true;
    }

    /// Delete n characters before cursor
    pub fn backspace_chars(&mut self, n: usize) {
        let n = n.min(self.zlecs);
        for _ in 0..n {
            if self.zlecs > 0 {
                self.zlecs -= 1;
                self.zleline.remove(self.zlecs);
                self.zlell -= 1;
            }
        }
        self.resetneeded = true;
    }

    /// Get the line as a string
    pub fn get_line(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Set the line from a string while preserving the current cursor
    /// position (clamped to the new length).
    /// Port of `setline()` from Src/Zle/zle_utils.c:1129 with the
    /// `ZSL_NOCURSOR` flag set. Used by widget bodies that swap in a
    /// fresh line (history navigation, isearch hit) but want to keep
    /// the cursor where it was.
    pub fn set_line_keep_cursor(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlecs.min(self.zlell);
        self.resetneeded = true;
    }

    /// Clear the line
    pub fn clear_line(&mut self) {
        self.zleline.clear();
        self.zlell = 0;
        self.zlecs = 0;
        self.mark = 0;
        self.resetneeded = true;
    }

    /// Get region between point and mark
    pub fn get_region(&self) -> &[ZleChar] {
        let (start, end) = if self.zlecs < self.mark {
            (self.zlecs, self.mark)
        } else {
            (self.mark, self.zlecs)
        };
        &self.zleline[start..end]
    }

    /// Cut to named buffer
    pub fn cut_to_buffer(&mut self, buf: usize, append: bool) {
        if buf < self.vibuf.len() {
            let (start, end) = if self.zlecs < self.mark {
                (self.zlecs, self.mark)
            } else {
                (self.mark, self.zlecs)
            };

            let text: ZleString = self.zleline[start..end].to_vec();

            if append {
                self.vibuf[buf].extend(text);
            } else {
                self.vibuf[buf] = text;
            }
        }
    }

    /// Paste from a named vi cut buffer.
    /// Port of `pastebuf()` from Src/Zle/zle_misc.c:558. The C source
    /// looks up `vibuf[zmod.vibuf]` (the vi `"a..z` register table),
    /// uses `cutbuf` for the unnamed buffer, and inserts at zlecs (or
    /// zlecs+1 for `after=true`). zshrs models the 36-slot vibuf array
    /// directly on Zle::vibuf.
    pub fn paste_from_buffer(&mut self, buf: usize, after: bool) {
        if buf < self.vibuf.len() {
            let text = self.vibuf[buf].clone();
            if !text.is_empty() {
                if after && self.zlecs < self.zlell {
                    self.zlecs += 1;
                }
                self.insert_chars(&text);
            }
        }
    }
}

/// Metafication helpers (for compatibility with zsh's metafied strings)
pub fn pastebuf(s: &str) -> String {
    // In zsh, Meta (0x83) is used to escape special bytes
    // For Rust we typically don't need this, but provide for compatibility
    s.to_string()
}

pub fn unmetafy(s: &str) -> String {
    s.to_string()
}

// Note: dead `UndoEntry`/`UndoState`/`apply_undo_entry` aggregates
// removed per PORT_PLAN Phase 2. They were a Rust-only invention
// with zero references across the codebase. The canonical undo
// machinery lives directly on `Zle` (`undo_stack: Vec<change>`,
// `changeno`, `cur_change`, `undo_changeno`, `undo_limitno` —
// declared in zle_main.rs) and the canonical port methods are:
//
//   Zle::mkundoent       — port of mkundoent (zle_utils.c)
//   Zle::apply_change    — port of applychange (zle_utils.c:1633)
//   Zle::unapply_change  — port of unapplychange (zle_utils.c:1677)
//
// C source's bag-of-statics that the canonical methods touch:
//
//   struct change *curchange;             // line 1427
//   static struct change *changes;        // line 1429
//   static struct change *nextchanges, *endnextchanges;  // line 1433
//   static zlong undo_limitno;            // line 1442
//   static struct zle_position *zle_positions;  // line 608
//
// These are file-scope (some `extern`-visible from zle_main.c), so
// they're PORT_PLAN Phase 3 bucket-2 (Arc<RwLock>) work, not the
// Phase 2 bucket-1 (thread_local!) wave. The dissolution noted here
// is structural cleanup (remove dead aggregate); the bucket-2 wiring
// of these globals onto Zle is already done in zle_main.rs.

impl Zle {
    /// Find beginning of line from position
    /// Port of findbol() from zle_utils.c
    pub fn find_bol(&self, pos: usize) -> usize {                            // c:1158
        let mut p = pos;
        while p > 0 && self.zleline.get(p - 1) != Some(&'\n') {
            p -= 1;
        }
        p
    }

    /// Find end of line from position
    /// Port of findeol() from zle_utils.c
    pub fn find_eol(&self, pos: usize) -> usize {                            // c:1169
        let mut p = pos;
        while p < self.zlell && self.zleline.get(p) != Some(&'\n') {
            p += 1;
        }
        p
    }

    /// Find line number for position
    /// Port of findline() from zle_utils.c
    pub fn find_line(&self, pos: usize) -> usize {
        self.zleline[..pos].iter().filter(|&&c| c == '\n').count()
    }

    // make sure that the line buffer has at least sz chars               // c:63
    /// Ensure line has enough space
    /// Port of sizeline() from zle_utils.c
    pub fn size_line(&mut self, needed: usize) {                            // c:67
        if self.zleline.capacity() < needed {
            self.zleline.reserve(needed - self.zleline.len());
        }
    }

    // insert space for ct chars at cursor position                        // c:773
    /// Make space in line at position
    /// Port of spaceinline() from zle_utils.c
    pub fn space_in_line(&mut self, pos: usize, count: usize) {             // c:777
        for _ in 0..count {
            self.zleline.insert(pos, ' ');
        }
        self.zlell += count;
        if self.zlecs >= pos {
            self.zlecs += count;
        }
    }

    /// Shift characters in line
    /// Port of shiftchars() from zle_utils.c
    pub fn shift_chars(&mut self, from: usize, count: i32) {                // c:846
        if count > 0 {
            for _ in 0..count {
                self.zleline.insert(from, ' ');
            }
            self.zlell += count as usize;
        } else if count < 0 {
            let to_remove = (-count) as usize;
            for _ in 0..to_remove.min(self.zlell - from) {
                self.zleline.remove(from);
            }
            self.zlell = self.zleline.len();
        }
    }

    /// Delete forward
    /// Port of foredel() from zle_utils.c
    pub fn fore_del(&mut self, count: usize, flags: CutFlags) {             // c:1105
        let count = count.min(self.zlell - self.zlecs);
        if count == 0 {
            return;
        }

        // Save to kill ring if requested
        if flags.contains(CutFlags::KILL) {
            let text: ZleString = self.zleline[self.zlecs..self.zlecs + count].to_vec();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
        }

        // Delete
        for _ in 0..count {
            self.zleline.remove(self.zlecs);
        }
        self.zlell -= count;
        self.resetneeded = true;
    }

    /// Delete backward
    /// Port of backdel() from zle_utils.c
    pub fn back_del(&mut self, count: usize, flags: CutFlags) {             // c:1084
        let count = count.min(self.zlecs);
        if count == 0 {
            return;
        }

        // Save to kill ring if requested
        if flags.contains(CutFlags::KILL) {
            let text: ZleString = self.zleline[self.zlecs - count..self.zlecs].to_vec();
            self.killring.push_front(text);
            if self.killring.len() > self.killringmax {
                self.killring.pop_back();
            }
        }

        // Delete
        self.zlecs -= count;
        for _ in 0..count {
            self.zleline.remove(self.zlecs);
        }
        self.zlell -= count;
        self.resetneeded = true;
    }

    /// Kill forward
    /// Port of forekill() from zle_utils.c
    pub fn fore_kill(&mut self, count: usize, append: bool) {               // c:1064
        let count = count.min(self.zlell - self.zlecs);
        if count == 0 {
            return;
        }

        let text: ZleString = self.zleline[self.zlecs..self.zlecs + count].to_vec();

        if append {
            if let Some(front) = self.killring.front_mut() {
                front.extend(text);
            } else {
                self.killring.push_front(text);
            }
        } else {
            self.killring.push_front(text);
        }

        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }

        for _ in 0..count {
            self.zleline.remove(self.zlecs);
        }
        self.zlell -= count;
        self.resetneeded = true;
    }

    /// Kill backward
    /// Port of backkill() from zle_utils.c
    pub fn back_kill(&mut self, count: usize, append: bool) {               // c:1045
        let count = count.min(self.zlecs);
        if count == 0 {
            return;
        }

        let text: ZleString = self.zleline[self.zlecs - count..self.zlecs].to_vec();

        if append {
            if let Some(front) = self.killring.front_mut() {
                let mut new_text = text;
                new_text.extend(front.iter());
                *front = new_text;
            } else {
                self.killring.push_front(text);
            }
        } else {
            self.killring.push_front(text);
        }

        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }

        self.zlecs -= count;
        for _ in 0..count {
            self.zleline.remove(self.zlecs);
        }
        self.zlell -= count;
        self.resetneeded = true;
    }

    /// Cut text to buffer
    /// Port of cut() / cuttext() from zle_utils.c
    pub fn cut_text(&mut self, start: usize, end: usize, dir: CutDirection) { // c:946
        if start >= end || end > self.zlell {
            return;
        }

        let text: ZleString = self.zleline[start..end].to_vec();

        match dir {
            CutDirection::Front => {
                self.killring.push_front(text);
            }
            CutDirection::Back => {
                if let Some(front) = self.killring.front_mut() {
                    front.extend(text);
                } else {
                    self.killring.push_front(text);
                }
            }
        }

        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }
    }

    /// Snapshot the current line into `last_line` for the undo system.
    /// Port of `setlastline()` from Src/Zle/zle_utils.c:1587. Routes to
    /// the canonical `setlastline` method below — kept under the
    /// snake-case name so older callers compile.
    pub fn set_last_line(&mut self) {
        self.setlastline();
    }

    /// Show a message
    /// Port of showmsg() from zle_utils.c
    pub fn show_msg(&self, msg: &str) {
        eprintln!("{}", msg);
    }

    /// Handle a feep (beep/error)
    /// Port of handlefeep() from zle_utils.c
    pub fn handle_feep(&self) {
        print!("\x07"); // Bell
    }

    /// Add text to line at position
    /// Port of zleaddtoline() from zle_utils.c
    pub fn add_to_line(&mut self, pos: usize, text: &str) {
        for (i, c) in text.chars().enumerate() {
            self.zleline.insert(pos + i, c);
        }
        self.zlell += text.chars().count();
        if self.zlecs >= pos {
            self.zlecs += text.chars().count();
        }
        self.resetneeded = true;
    }

    /// Get line as string
    /// Port of zlelineasstring() from zle_utils.c
    pub fn line_as_string(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Set line from string
    /// Port of stringaszleline() from zle_utils.c
    pub fn string_as_line(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        if self.zlecs > self.zlell {
            self.zlecs = self.zlell;
        }
        self.resetneeded = true;
    }

    /// Get ZLE line
    /// Port of zlegetline() from zle_utils.c
    pub fn get_zle_line(&self) -> &[ZleChar] {
        &self.zleline
    }

    /// Read a y/n response from input.
    /// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197. The C source
    /// reads one key, treats Tab as 'y', any control char or EOF as 'n',
    /// and otherwise tolowers the input. Echoes the response and returns
    /// true iff the user pressed 'y'. Used by completion-listing prompts
    /// like "show all 200 matches?".
    pub fn get_zle_query(&mut self) -> bool {
        let c = match self.getfullchar(false) {
            Some(c) => c,
            None => return false, // EOF → 'n'
        };
        let resolved = if c == '\t' {
            'y'
        } else if c.is_control() {
            'n'
        } else {
            c.to_ascii_lowercase()
        };
        // Echo the response (mirrors zwcputc at zle_utils.c:1229).
        if resolved != '\n' {
            print!("{}", resolved);
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        resolved == 'y'
    }

    /// Handle the auto-removable completion suffix.
    /// Port of `handlesuffix()` from Src/Zle/zle_utils.c:1415. The C
    /// source clears or retains the pending suffix depending on the
    /// invoking widget's flags; without compsys integration in this
    /// crate, we surface a hook so the host can update its compsys
    /// state at the right moment.
    pub fn handle_suffix(&mut self) {
        self.call_hook("handle-suffix", None);
    }

    /// Set the editor line from a string.
    /// Port of `setline()` from Src/Zle/zle_utils.c:1129. The C source
    /// converts the metafied input back to a wide-char buffer; in Rust
    /// we just collect chars into the line buffer and reset the cursor.
    pub fn set_line(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlell;
        self.resetneeded = true;
    }
}

/// Saved position state
#[derive(Debug, Clone)]
pub struct SavedPositions {
    pub zlecs: usize,
    pub zlell: usize,
    pub mark: usize,
}

/// Position save/restore
/// Port of zle_save_positions() / zle_restore_positions() from zle_utils.c
impl Zle {
    pub fn save_positions(&self) -> SavedPositions {
        SavedPositions {
            zlecs: self.zlecs,
            zlell: self.zlell,
            mark: self.mark,
        }
    }

    pub fn restore_positions(&mut self, saved: &SavedPositions) {
        self.zlecs = saved.zlecs.min(self.zlell);
        self.mark = saved.mark.min(self.zlell);
    }
}

bitflags::bitflags! {
    /// Flags for cut operations
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CutFlags: u32 {
        const KILL = 1 << 0;   // Add to kill ring
        const COPY = 1 << 1;   // Don't delete, just copy
        const APPEND = 1 << 2; // Append to kill ring
    }
}

/// Direction for cut operations
#[derive(Debug, Clone, Copy)]
pub enum CutDirection {
    Front,
    Back,
}

/// Format a key sequence for `bindkey -L` listing.
/// Port of `bindztrdup()` from Src/Zle/zle_utils.c:1238. Produces the
/// dquoted-friendly form (`\C-a`, `\M-x`, escaped backslashes/carets)
/// that the bindkey command uses for round-trippable output —
/// distinct from `printbind` below which uses the human-readable
/// `^A` / `^[X` form printed in describe-key-briefly etc.
pub fn bindztrdup(seq: &[u8]) -> String {
    let mut buf = String::new();
    for &b in seq {
        // Meta bit handling: zsh metafies bytes >= 0x80 by inserting
        // 0x83 (Meta) before a (b ^ 0x20) byte. The C source unwinds
        // that here; in our Rust model we don't pastebuf in storage, so
        // we treat any byte >= 0x80 as already a M- target.
        let mut c = b;
        if c & 0x80 != 0 {
            buf.push('\\');
            buf.push('M');
            buf.push('-');
            c &= 0x7f;
        }
        if c < 32 || c == 0x7f {
            buf.push('^');
            c ^= 64;
        }
        if c == b'\\' || c == b'^' {
            buf.push('\\');
        }
        buf.push(c as char);
    }
    buf
}

/// Print a key binding for display
/// Port of printbind() from zle_utils.c
pub fn printbind(seq: &[u8]) -> String {
    let mut result = String::new();

    for &b in seq {
        match b {
            0x1b => result.push_str("^["),
            0..=31 => {
                result.push('^');
                result.push((b + 64) as char);
            }
            127 => result.push_str("^?"),
            128..=159 => {
                result.push_str("^[^");
                result.push((b - 64) as char);
            }
            _ => result.push(b as char),
        }
    }

    result
}

impl Zle {
    /// Queue a hook for the host to dispatch.
    /// Port of `zlecallhook()` from Src/Zle/zle_utils.c:1755 — the C source
    /// resolves the widget via `rthingy_nocreate` and runs it inline via
    /// `execzlefunc(thingy, args, 1, 0)`. The Rust port can't reach the
    /// executor from this crate, so it appends to `pending_hooks`; the
    /// host (the binary owning a `ShellExecutor`) drains the list after
    /// each ZLE call and runs each named widget against its current
    /// dispatch table — matching the same order zsh would have run them
    /// in. `errflag` / `retflag` save/restore (zle_utils.c:1766/1775) is
    /// the host's responsibility.
    pub fn call_hook(&mut self, name: &str, arg: Option<&str>) {
        self.pending_hooks
            .push((name.to_string(), arg.map(|s| s.to_string())));
    }

    /// Drain the queued hook calls. Returns the list and resets the queue.
    /// Mirrors zsh's pattern of clearing pending hooks after dispatch
    /// (see the implicit reset by `unrefthingy` plus the per-call save
    /// of errflag/retflag in zle_utils.c:1766-1776).
    pub fn drain_hooks(&mut self) -> Vec<(String, Option<String>)> {
        std::mem::take(&mut self.pending_hooks)
    }
}

#[cfg(test)]
mod tests_hooks {
    use super::Zle;

    #[test]
    fn call_hook_queues_for_host_dispatch() {
        let mut zle = Zle::new();
        zle.call_hook("zle-line-init", None);
        zle.call_hook("zle-keymap-select", Some("vicmd"));
        let drained = zle.drain_hooks();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], ("zle-line-init".to_string(), None));
        assert_eq!(
            drained[1],
            ("zle-keymap-select".to_string(), Some("vicmd".to_string()))
        );
        // Buffer is empty after drain.
        assert!(zle.drain_hooks().is_empty());
    }

    #[test]
    fn redrawhook_queues_pre_redraw_hook() {
        let mut zle = Zle::new();
        zle.redrawhook();
        let drained = zle.drain_hooks();
        assert_eq!(drained, vec![("zle-line-pre-redraw".to_string(), None)]);
    }

    #[test]
    fn reexpandprompt_re_runs_expansion_against_raw_templates() {
        let mut zle = Zle::new();
        // Set raw templates that don't reference dynamic state, so the
        // expansion is idempotent and easy to assert. %% expands to a
        // single literal '%' per zsh prompt rules.
        zle.lprompt_raw = "%% > ".to_string();
        zle.rprompt_raw = "[%%]".to_string();
        zle.reexpandprompt();
        assert_eq!(zle.prompt(), "% > ");
        assert_eq!(zle.rprompt(), "[%]");
    }
}

#[cfg(test)]
mod tests_bindkey_format {
    use super::bindztrdup;
    use super::printbind;

    #[test]
    fn bind_ztrdup_emits_caret_form_for_control_chars() {
        // Ctrl-A → "^A". Mirrors zsh's bindkey -L line for `bindkey '^A'`.
        assert_eq!(bindztrdup(b"\x01"), "^A");
        // Ctrl-_ → "^_".
        assert_eq!(bindztrdup(b"\x1f"), "^_");
        // DEL (0x7f) → "^?".
        assert_eq!(bindztrdup(b"\x7f"), "^?");
    }

    #[test]
    fn bind_ztrdup_escapes_backslash_and_caret() {
        // '\\' → "\\\\" (escaped per C source's `c == '\\'` branch).
        assert_eq!(bindztrdup(b"\\"), "\\\\");
        // '^' → "\\^".
        assert_eq!(bindztrdup(b"^"), "\\^");
    }

    #[test]
    fn bind_ztrdup_handles_high_bit_as_meta() {
        // Byte with bit-7 set → "\\M-X" prefix. \\xC1 = M-A.
        assert_eq!(bindztrdup(b"\xC1"), "\\M-A");
    }

    #[test]
    fn printbind_caret_form_matches_describe_key_output() {
        // `^A`-style display form (distinct from bindkey's escape form).
        assert_eq!(printbind(b"\x01"), "^A");
        assert_eq!(printbind(b"\x1b"), "^[");
    }
}

impl Zle {
    /// Snapshot the current line into `last_line` so the next `mkundoent`
    /// can diff against it. Port of `setlastline` (zle_utils.c:1587).
    pub fn setlastline(&mut self) {
        self.last_line.clear();
        self.last_line.extend_from_slice(&self.zleline);
        self.last_ll = self.zlell;
        self.last_cs = self.zlecs;
    }

    // add an entry to the undo system, if anything has changed              // c:1528
    /// If the line changed since the last snapshot, append a Change record
    /// describing the diff. Port of `mkundoent` (zle_utils.c:1532).
    pub fn mkundoent(&mut self) {                                             // c:1532
        if self.last_ll == self.zlell && self.last_line[..self.last_ll] == self.zleline[..self.zlell]
        {
            self.last_cs = self.zlecs;
            return;
        }
        let sh = self.last_ll.min(self.zlell);
        let mut pre = 0usize;
        while pre < sh && self.zleline[pre] == self.last_line[pre] {
            pre += 1;
        }
        let mut suf = 0usize;
        while suf < sh - pre
            && self.zleline[self.zlell - 1 - suf] == self.last_line[self.last_ll - 1 - suf]
        {
            suf += 1;
        }
        let del: ZleString = if suf + pre == self.last_ll {
            Vec::new()
        } else {
            self.last_line[pre..self.last_ll - suf].to_vec()
        };
        let ins: ZleString = if suf + pre == self.zlell {
            Vec::new()
        } else {
            self.zleline[pre..self.zlell - suf].to_vec()
        };
        self.undo_changeno += 1;
        let ch = super::zle_main::change {
            flags: 0,
            hist: self.history.cursor as i32,
            off: pre,
            del,
            ins,
            old_cs: self.last_cs,
            new_cs: self.zlecs,
            changeno: self.undo_changeno,
        };
        // Drop any forward redo history past the cursor before pushing.
        self.undo_stack.truncate(self.cur_change);
        self.undo_stack.push(ch);
        self.cur_change = self.undo_stack.len();
    }

    // register pending changes in the undo system                            // c:1484
    /// Pre-widget hook. Port of `handleundo` (zle_utils.c) — currently a thin
    /// stub since `mkundoent` runs after each widget; the C version uses it to
    /// flush in-flight `nextchanges` chains, which our one-change-per-widget
    /// model doesn't need.
    pub fn handleundo(&mut self) {                                            // c:1488
        self.setlastline();
    }

    /// Reverse the change at `idx` (move zleline back to its pre-change state).
    /// Returns true on success.
    /// Port of `unapplychange` (zle_utils.c:1633).
    pub fn unapply_change(&mut self, idx: usize) -> bool {
        if idx >= self.undo_stack.len() {
            return false;
        }
        // Borrow check: clone the small fields we need.
        let (off, dell, insl, old_cs);
        let del_vec;
        let ins_len;
        {
            let ch = &self.undo_stack[idx];
            off = ch.off;
            dell = ch.del.len();
            insl = ch.ins.len();
            ins_len = ch.ins.len();
            old_cs = ch.old_cs;
            del_vec = ch.del.clone();
        }
        let _ = ins_len;
        self.zlecs = off;
        if insl > 0 {
            // Remove the inserted text.
            self.zleline.drain(off..off + insl);
        }
        if dell > 0 {
            // Re-insert the deleted text.
            for (i, c) in del_vec.into_iter().enumerate() {
                self.zleline.insert(off + i, c);
            }
        }
        self.zlell = self.zleline.len();
        self.zlecs = old_cs.min(self.zlell);
        self.resetneeded = true;
        true
    }

    /// Replay the change at `idx`. Port of `applychange` (zle_utils.c:1677).
    pub fn apply_change(&mut self, idx: usize) -> bool {
        if idx >= self.undo_stack.len() {
            return false;
        }
        let (off, dell, insl, new_cs);
        let ins_vec;
        {
            let ch = &self.undo_stack[idx];
            off = ch.off;
            dell = ch.del.len();
            insl = ch.ins.len();
            new_cs = ch.new_cs;
            ins_vec = ch.ins.clone();
        }
        self.zlecs = off;
        if dell > 0 {
            self.zleline.drain(off..off + dell);
        }
        if insl > 0 {
            for (i, c) in ins_vec.into_iter().enumerate() {
                self.zleline.insert(off + i, c);
            }
        }
        self.zlell = self.zleline.len();
        self.zlecs = new_cs.min(self.zlell);
        self.resetneeded = true;
        true
    }

    // move backwards through the change list                                 // c:1597
    /// Walk back one Change. Port of `undo` (zle_utils.c:1601).
    pub fn undo_widget(&mut self) -> i32 {                                    // c:1601
        // Capture any in-flight edits into a Change before stepping back.
        self.mkundoent();
        if self.cur_change == 0 {
            return 1;
        }
        let prev_idx = self.cur_change - 1;
        if self.undo_stack[prev_idx].changeno <= self.undo_limitno {
            return 1;
        }
        if self.unapply_change(prev_idx) {
            self.cur_change = prev_idx;
        }
        self.setlastline();
        0
    }

    // move forwards through the change list                                  // c:1657
    /// Walk forward one Change. Port of `redo` (zle_utils.c:1661).
    pub fn redo_widget(&mut self) -> i32 {                                    // c:1661
        self.mkundoent();
        if self.cur_change >= self.undo_stack.len() {
            return 1;
        }
        if self.apply_change(self.cur_change) {
            self.cur_change += 1;
        }
        self.setlastline();
        0
    }
}

/// Direct port of `static int applychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1678-1730`. Applies one Change record from
/// the undo stack: deletes `ch->del` characters at `ch->off`, then
/// inserts `ch->ins` at the same position, and updates `zlecs`.
/// Returns 1 if there are more changes to apply (CH_NEXT), else 0.
pub fn applychange(zle: &mut crate::ported::zle::zle_main::Zle, ch: i32) -> i32 { // c:1678
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let idx = ch as usize;
    if idx >= zle.undo_stack.len() { return 0; }
    let change = zle.undo_stack[idx].clone();
    // c:1683-1696 — apply del then ins at change.off.
    let off = change.off;
    let del_n = change.del.len();
    if off + del_n <= zle.zleline.len() {
        zle.zleline.drain(off..off + del_n);                                 // c:1690 delete
    }
    // c:1700 — insert change.ins at off.
    for (i, c) in change.ins.iter().enumerate() {
        if off + i <= zle.zleline.len() {
            zle.zleline.insert(off + i, *c);
        } else {
            zle.zleline.push(*c);
        }
    }
    zle.zlecs = change.new_cs;                                               // c:1718
    zle.zlell = zle.zleline.len();
    // c:1721 — return 1 if CH_NEXT, else 0.
    if change.flags & CH_NEXT != 0 { 1 } else { 0 }
}

/// Port of `backdel()` from `Src/Zle/zle_utils.c:1084`. Removes `ct`
/// characters BACKWARD from the cursor (i.e. drops `[zlecs-ct,
/// zlecs)` from the line) without pushing to the kill-ring.
///
/// C signature: `void backdel(int ct, int flags)`. The Rust port
/// takes `&mut Zle` so `zlecs`/`zlell`/`zleline` mutations stay on
/// the typed shell state. The non-RAW path's `DECCS` multibyte
/// adjustment loop (c:1093-1098) collapses to a plain decrement
/// since zshrs treats the buffer as `Vec<char>`.
pub fn backdel(zle: &mut crate::ported::zle::zle_main::Zle, ct: i32, _flags: i32) {  // c:1084
    let ct = ct as usize;
    if ct == 0 || zle.zlecs == 0 { return; }
    let take_n = ct.min(zle.zlecs);
    let start = zle.zlecs - take_n;
    zle.zleline.drain(start..zle.zlecs);                                 // c:1090 shiftchars
    zle.zlell = zle.zleline.len();
    zle.zlecs = start;
    zle.resetneeded = true;                                              // c:1091 CCRIGHT
}

/// Port of `backkill()` from `Src/Zle/zle_utils.c:1045`. Cuts `ct`
/// characters BACKWARD from the cursor (i.e. removes `[zlecs-ct,
/// zlecs)` and pushes them onto the kill-ring head). C: `void
/// backkill(int ct, int flags)`. Rust port takes `&mut Zle` so the
/// killring + zlecs/zlell mutations stay on the typed shell state.
/// `flags` is the `CUT_*` bitmask — `CUT_RAW` skips the multibyte
/// DECCS adjustment loop the non-RAW path uses.
pub fn backkill(zle: &mut crate::ported::zle::zle_main::Zle, ct: i32, flags: i32) {  // c:1045
    let ct = ct as usize;
    if ct == 0 || zle.zlecs == 0 { return; }
    let _ = flags; // CUT_RAW path: no DECCS multibyte adjustment.
    let take_n = ct.min(zle.zlecs);
    let start = zle.zlecs - take_n;
    let cut_chars: Vec<char> = zle.zleline.drain(start..zle.zlecs).collect();   // c:1057 cut + shiftchars
    zle.zlell = zle.zleline.len();
    zle.zlecs = start;
    zle.killring.push_front(cut_chars);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    zle.resetneeded = true;                                              // c:1059 CCRIGHT
}

/// Port of `cut()` from Src/Zle/zle_utils.c:935.
/// `i` is the start byte offset; `ct` is the count to cut; `dir` is
/// the cut direction flag (0=after, non-zero=before).
pub fn cut(zle: &mut crate::ported::zle::zle_main::Zle, i: i32,              // c:935
           ct: i32, dir: i32) -> i32 {
    // C body c:937-944 — `cuttext(zleline+i, ct, dir)`. Fold to a
    //                    single helper that pushes a slice into the
    //                    kill ring (or vibuf when MOD_VIBUF is set).
    if ct <= 0 || i < 0 {
        return 0;
    }
    let start = i as usize;
    let end = (start + ct as usize).min(zle.zleline.len());
    if start >= end {
        return 0;
    }
    let chunk: Vec<char> = zle.zleline[start..end].to_vec();
    cuttext(zle, &chunk, dir);
    0
}

/// Port of `cuttext()` from Src/Zle/zle_utils.c:946.
pub fn cuttext(zle: &mut crate::ported::zle::zle_main::Zle, txt: &[char],    // c:946
               dir: i32) {
    use crate::ported::zle::zle_main::ModifierFlags;
    // C body c:948-1043 — pushes `txt` into vibuf[zmod.vibuf] when
    //                     MOD_VIBUF is set, else front of killring.
    //                     CUT_APPEND/CUT_REPLACE flag handling skipped
    //                     in this distilled body.
    let chars: Vec<char> = txt.to_vec();
    if zle.zmod.flags.contains(ModifierFlags::VIBUF) {                       // c:961
        let idx = zle.zmod.vibuf as usize;
        if idx < zle.vibuf.len() {
            if dir != 0 {
                zle.vibuf[idx] = chars;
            } else {
                zle.vibuf[idx].extend(chars);
            }
        }
    } else {
        zle.killring.push_front(chars);                                      // c:996
        let max = zle.killringmax;
        if zle.killring.len() > max {
            zle.killring.pop_back();
        }
    }
}

/// Port of `findbol()` from `Src/Zle/zle_utils.c:1157`.
/// ```c
/// int
/// findbol(void)
/// {
///     int x = zlecs;
///     while (x > 0 && zleline[x - 1] != ZWC('\n'))
///         x--;
///     return x;
/// }
/// ```
/// Walk backward from the cursor to the start of the current line
/// (or the start of the buffer if there's no preceding newline).
/// Returns the byte offset.
pub fn findbol(zle: &crate::ported::zle::zle_main::Zle) -> usize {           // c:1157
    let mut x = zle.zlecs;                                                   // c:1160 int x = zlecs
    while x > 0 && zle.zleline.get(x - 1) != Some(&'\n') {                   // c:1162
        x -= 1;                                                              // c:1163 x--
    }
    x                                                                        // c:1164 return x
}

/// Port of `findeol()` from `Src/Zle/zle_utils.c:1168`.
/// ```c
/// int
/// findeol(void)
/// {
///     int x = zlecs;
///     while (x != zlell && zleline[x] != ZWC('\n'))
///         x++;
///     return x;
/// }
/// ```
/// Walk forward from the cursor to the next newline (or end of
/// buffer). Returns the byte offset.
pub fn findeol(zle: &crate::ported::zle::zle_main::Zle) -> usize {           // c:1168
    let mut x = zle.zlecs;                                                   // c:1171 int x = zlecs
    while x != zle.zlell && zle.zleline.get(x) != Some(&'\n') {              // c:1173
        x += 1;                                                              // c:1174 x++
    }
    x                                                                        // c:1175 return x
}

/// Port of `findline()` from `Src/Zle/zle_utils.c:1179`.
/// ```c
/// void
/// findline(int *a, int *b)
/// {
///     *a = findbol();
///     *b = findeol();
/// }
/// ```
/// Returns `(bol, eol)` for the current line.
pub fn findline(zle: &crate::ported::zle::zle_main::Zle) -> (usize, usize) {  // c:1179
    (findbol(zle), findeol(zle))                                             // c:1182-1183
}

/// Port of `foredel()` from `Src/Zle/zle_utils.c:1105`. Removes `ct`
/// characters FORWARD from the cursor (i.e. drops `[zlecs, zlecs+ct)`
/// from the line) without pushing to the kill-ring.
///
/// C signature: `void foredel(int ct, int flags)`. Rust port takes
/// `&mut Zle`. The non-RAW path's `INCCS` multibyte adjustment loop
/// (c:1115+) collapses to plain `Vec<char>::drain`.
pub fn foredel(zle: &mut crate::ported::zle::zle_main::Zle, ct: i32, _flags: i32) {  // c:1105
    let ct = ct as usize;
    if ct == 0 || zle.zlecs >= zle.zlell { return; }
    let take_n = ct.min(zle.zlell - zle.zlecs);
    let i = zle.zlecs;
    zle.zleline.drain(i..i + take_n);                                    // c:1111 shiftchars
    zle.zlell = zle.zleline.len();
    zle.resetneeded = true;                                              // c:1112 CCRIGHT
}

/// Port of `forekill()` from `Src/Zle/zle_utils.c:1064`. Cuts `ct`
/// characters FORWARD from the cursor (i.e. removes `[zlecs,
/// zlecs+ct)` and pushes them onto the kill-ring head). C: `void
/// forekill(int ct, int flags)`. Rust port takes `&mut Zle`. The
/// `CUT_RAW` path (matching the C `flags & CUT_RAW` arm at
/// zle_utils.c:1069) skips the multibyte INCCS adjustment loop —
/// zshrs treats the buffer as `Vec<char>` and never needs that
/// re-walk.
pub fn forekill(zle: &mut crate::ported::zle::zle_main::Zle, ct: i32, flags: i32) {  // c:1064
    let ct = ct as usize;
    if ct == 0 || zle.zlecs >= zle.zlell { return; }
    let _ = flags; // CUT_RAW path: no INCCS multibyte adjustment.
    let take_n = ct.min(zle.zlell - zle.zlecs);
    let i = zle.zlecs;
    let cut_chars: Vec<char> = zle.zleline.drain(i..i + take_n).collect();      // c:1077 cut + shiftchars
    zle.zlell = zle.zleline.len();
    zle.killring.push_front(cut_chars);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    zle.resetneeded = true;                                              // c:1079 CCRIGHT
}

/// Port of `free_region_highlights_memos()` from Src/Zle/zle_utils.c:567.
pub fn free_region_highlights_memos() {                                      // c:567
    // C body c:569-580 — walks region_highlights_memos free list,
    //                    calls zfree on each. Drop covers it; no-op.
}

/// Port of `freechanges()` from Src/Zle/zle_utils.c:1472.
pub fn freechanges() {                                                       // c:1472
    // C body c:1474-1484 — walks Change linked list, frees del/ins
    //                      strings + the Change node. Drop covers it.
}

/// Port of `freeundo()` from Src/Zle/zle_utils.c:1461.
pub fn freeundo() {                                                          // c:1461
    // C body c:1463-1470 — `freechanges(curchange); freechanges(...)
    //                      etc. for the whole undo chain`. Drop covers.
}

/// Port of `get_undo_current_change()` from Src/Zle/zle_utils.c:1785.
pub fn get_undo_current_change() -> i64 {                                    // c:1785
    // C body c:1787-1810 — `if (!curchange) return -1; return curchange->changeno`.
    //                      Without curchange tracker: -1 (no change).
    -1
}

/// Port of `get_undo_limit_change()` from Src/Zle/zle_utils.c:1812.
pub fn get_undo_limit_change() -> i64 {                                      // c:1812
    // C body c:1814-1817 — `return undo_limit_change`. Returns the
    //                      undo-limit anchor change number.
    -1
}

/// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197.
pub fn getzlequery() -> i32 {                                                // c:1197
    // C body c:1199-1300 — reads y/n response from terminal interactive
    //                      prompt. Without a live tty read we report
    //                      cancel (-1).
    -1
}

/// Port of `handlefeep()` from `Src/Zle/zle_utils.c:1404`.
/// ```c
/// int
/// handlefeep(UNUSED(char **args))
/// {
///     zbeep();
///     return 0;
/// }
/// ```
/// `beep` widget — fires the terminal bell via `zbeep`.
pub fn handlefeep() -> i32 {                                                 // c:1404
    crate::ported::utils::zbeep();                                           // c:1407 zbeep()
    0                                                                        // c:1408 return 0
}

/// Port of `handlesuffix()` from Src/Zle/zle_utils.c:1415.
pub fn handlesuffix(zle: &mut crate::ported::zle::zle_main::Zle, c: i32) -> i32 { // c:1415
    // C body c:1417-1444 — peeks the next byte; if SUFFIXLEN is set
    //                      and the byte is in the suffix's noinsert
    //                      set, drop the suffix; else keep + insert.
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::SUFFIXLEN;
    let _ = (c, zle);
    let len = SUFFIXLEN.load(Ordering::SeqCst);
    if len > 0 {
        SUFFIXLEN.store(0, Ordering::SeqCst);
    }
    0
}

/// Port of `initundo()` from Src/Zle/zle_utils.c:1446.
pub fn initundo() {                                                          // c:1446
    // C body c:1448-1459 — `nextchanges = endnextchanges = NULL;
    //                       lastline = ...; freeundo()`.
    //                      Undo chain isn't a Rust struct yet; no-op.
    freeundo();
}

/// Direct port of `void mergeundo(void)` from
/// `Src/Zle/zle_utils.c:1733-1745`. Walks the undo stack backward
/// from `cur_change` chaining CH_PREV/CH_NEXT flags so the changes
/// since `vistartchange+1` form a single undo step (atomic vi
/// insert-mode group). Resets `vistartchange = u64::MAX` (C's -1).
pub fn mergeundo(zle: &mut crate::ported::zle::zle_main::Zle) {              // c:1733
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    // c:1735-1742 — walk current->prev while changeno > vistartchange+1.
    if zle.cur_change == 0 { return; }
    let mut current = zle.cur_change - 1;                                    // c:1735 prev
    while current > 0
        && zle.undo_stack[current].changeno > zle.vistartchange + 1
    {
        zle.undo_stack[current].flags |= CH_PREV;                  // c:1740
        zle.undo_stack[current - 1].flags |= CH_NEXT;              // c:1741
        current -= 1;
    }
    zle.vistartchange = u64::MAX;                                            // c:1744 = -1
}

/// Direct port of `int redo(UNUSED(char **args))` from
/// `Src/Zle/zle_utils.c:1661-1675`. Walks the undo stack forward
/// from `zle.cur_change` calling `applychange` on each; returns 0
/// on success, 1 when nothing to redo.
pub fn redo(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {            // c:1661
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    loop {
        if zle.cur_change >= zle.undo_stack.len() { return 1; }              // c:1664
        let cur_idx = zle.cur_change;
        if applychange(zle, cur_idx as i32) == 0 { break; }                  // c:1668
        zle.cur_change = cur_idx + 1;
        let has_next = zle.undo_stack.get(cur_idx)
            .map(|c| c.flags & CH_NEXT != 0)
            .unwrap_or(false);
        if !has_next { break; }                                              // c:1670
    }
    zle.cur_change += 1;                                                     // c:1672 advance past applied
    0                                                                        // c:1674
}

/// Port of `set_undo_limit_change()` from Src/Zle/zle_utils.c:1819.
pub fn set_undo_limit_change(_n: i64) -> i32 {                               // c:1819
    // C body c:1821-1825 — `undo_limit_change = n; return 0`.
    //                      Without undo_limit_change global: 0.
    0
}

/// Port of `setline()` from Src/Zle/zle_utils.c:1129.
pub fn setline(zle: &mut crate::ported::zle::zle_main::Zle, s: &str,         // c:1129
               flags: i32) {
    // C body c:1131-1156 — replaces zleline with `s`; if !ZSL_KEEPCS
    //                      reset zlecs to 0 or len(s). flags bit
    //                      ZSL_KEEPCS = 1.
    zle.zleline.clear();
    zle.zleline.extend(s.chars());
    zle.zlell = zle.zleline.len();
    if flags & 1 == 0 {
        zle.zlecs = zle.zlell;                                               // c:1145
    }
    zle.resetneeded = true;
}

/// Port of `shiftchars()` from Src/Zle/zle_utils.c:846.
pub fn shiftchars(zle: &mut crate::ported::zle::zle_main::Zle, to: i32, cnt: i32) { // c:846
    // C body c:848-865 — `if (to + cnt < zlell) memmove(line+to,
    //                     line+to+cnt, (zlell-(to+cnt)) * char_t);
    //                     zlell -= cnt`. Pure shift-left of `cnt`
    //                     chars at offset `to`.
    let to = to as usize;
    let cnt = cnt as usize;
    if to + cnt > zle.zleline.len() {
        return;
    }
    zle.zleline.drain(to..to + cnt);
    zle.zlell = zle.zleline.len();
}

/// Port of `showmsg()` from Src/Zle/zle_utils.c:1303.
pub fn showmsg(msg: &str) {                                                  // c:1303
    // C body c:1305-1402 — prints msg below the prompt with cursor
    //                      position save/restore. Without curses
    //                      substrate we emit via tracing.
    tracing::info!(target: "zle", "{}", msg);
}

/// Port of `sizeline()` from Src/Zle/zle_utils.c:67.
pub fn sizeline(zle: &mut crate::ported::zle::zle_main::Zle, sz: usize) {    // c:67
    // C body c:69-87 — `if (sz > linesz) { linesz = sz + 256; line =
    //                  zrealloc(line, (linesz+1) * char_t) }`. Vec
    //                  grows on demand; just reserve.
    if sz > zle.zleline.len() {
        zle.zleline.reserve(sz - zle.zleline.len() + 256);
    }
}

/// Port of `spaceinline()` from Src/Zle/zle_utils.c:777.
pub fn spaceinline(zle: &mut crate::ported::zle::zle_main::Zle, ct: i32) {   // c:777
    // C body c:779-844 — opens `ct` chars of space at zlecs by
    //                    moving zleline[zlecs..zlell] forward `ct`,
    //                    growing buffer if needed. zlell += ct.
    if ct <= 0 {
        return;
    }
    let ct = ct as usize;
    for _ in 0..ct {
        zle.zleline.insert(zle.zlecs, '\0');
    }
    zle.zlell = zle.zleline.len();
}

/// Direct port of `int splitundo(char **args)` from
/// `Src/Zle/zle_utils.c:1721-1731`.
/// ```c
/// if (vistartchange >= 0) {
///     mergeundo();
///     vistartchange = undo_changeno;
/// }
/// handleundo();
/// return 0;
/// ```
pub fn splitundo(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:1721
    // C uses signed `vistartchange`; Rust uses u64 with u64::MAX as
    // the "-1 / inactive" sentinel.
    if zle.vistartchange != u64::MAX {                                       // c:1723 >= 0
        mergeundo(zle);                                                      // c:1725
        zle.vistartchange = zle.undo_changeno;                               // c:1726
    }
    zle.handleundo();                                                        // c:1728
    0                                                                        // c:1730
}

/// Port of `stringaszleline()` from Src/Zle/zle_utils.c:375.
pub fn stringaszleline(s: &str) -> Vec<char> {                               // c:375
    // C body c:377-580 — converts a metafied string into ZLE_CHAR_T
    //                    array (multibyte decode + meta unescape).
    //                    Vec<char> is already wide-char; demeta and
    //                    return.
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x83 && i + 1 < bytes.len() {                                // Meta byte
            i += 1;
            out.push((bytes[i] ^ 32) as char);
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

/// Port of `unapplychange()` from Src/Zle/zle_utils.c:1634.
/// Direct port of `static int unapplychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1634-1652`. Reverse of applychange: deletes
/// `ch->ins` at `ch->off` and re-inserts `ch->del`.
pub fn unapplychange(zle: &mut crate::ported::zle::zle_main::Zle, ch: i32) -> i32 { // c:1634
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let idx = ch as usize;
    if idx >= zle.undo_stack.len() { return 0; }
    let change = zle.undo_stack[idx].clone();
    let off = change.off;
    // c:1638-1644 — delete what was inserted.
    let ins_n = change.ins.len();
    if off + ins_n <= zle.zleline.len() {
        zle.zleline.drain(off..off + ins_n);                                 // c:1640
    }
    // c:1646 — re-insert the deleted chars.
    for (i, c) in change.del.iter().enumerate() {
        if off + i <= zle.zleline.len() {
            zle.zleline.insert(off + i, *c);
        } else {
            zle.zleline.push(*c);
        }
    }
    zle.zlecs = change.old_cs;                                               // c:1649
    zle.zlell = zle.zleline.len();
    // c:1651 — return 1 if CH_PREV, else 0.
    if change.flags & CH_PREV != 0 { 1 } else { 0 }
}

/// Direct port of `int undo(char **args)` from
/// `Src/Zle/zle_utils.c:1601-1632`. Walks the undo stack backward
/// from `zle.cur_change` calling `unapplychange` on each; stops at
/// `last_change` (parsed from `args[0]` if provided, else -1 for
/// "single step") or at `undo_limitno`. Returns 0 on success,
/// 1 when nothing left to undo.
pub fn undo(zle: &mut crate::ported::zle::zle_main::Zle, args: &[String]) -> i32 { // c:1601
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let last_change: i64 = if !args.is_empty() {                             // c:1605
        args[0].parse().unwrap_or(-1)
    } else {
        -1
    };

    loop {
        // c:1614 — `prev = curchange->prev`; in Rust we step the
        // index down.
        if zle.cur_change == 0 { return 1; }                                 // c:1615
        let prev_idx = zle.cur_change - 1;
        let prev_chno = zle.undo_stack[prev_idx].changeno as i64;
        if prev_chno <= last_change { break; }                               // c:1618
        if (prev_chno as u64) <= zle.undo_limitno && args.is_empty() {       // c:1619
            return 1;
        }
        if unapplychange(zle, prev_idx as i32) == 0 {                        // c:1621
            if last_change >= 0 {
                unapplychange(zle, prev_idx as i32);                         // c:1623
                zle.cur_change = prev_idx;                                   // c:1624
            }
        } else {
            zle.cur_change = prev_idx;                                       // c:1627
        }
        let has_prev = zle.undo_stack.get(prev_idx)
            .map(|c| c.flags & CH_PREV != 0)
            .unwrap_or(false);
        if !(last_change >= 0 || has_prev) { break; }                        // c:1630
    }
    0                                                                        // c:1631
}

/// Direct port of `int viundochange(char **args)` from
/// `Src/Zle/zle_utils.c:1705-1719`.
/// ```c
/// handleundo();
/// if (curchange->next) {
///     do { applychange(curchange); curchange = curchange->next; }
///     while(curchange->next);
///     setlastline();
///     return 0;
/// } else return undo(args);
/// ```
pub fn viundochange(zle: &mut crate::ported::zle::zle_main::Zle,             // c:1705
                    args: &[String]) -> i32 {
    zle.handleundo();                                                        // c:1707
    if zle.cur_change < zle.undo_stack.len() {                               // c:1708 curchange->next
        // Re-apply all forward changes (collapses an undo chain back
        // to current state).
        while zle.cur_change < zle.undo_stack.len() {                        // c:1710
            let idx = zle.cur_change;
            applychange(zle, idx as i32);                                    // c:1711
            zle.cur_change = idx + 1;                                        // c:1712
        }
        0                                                                    // c:1715
    } else {
        undo(zle, args)                                                      // c:1717
    }
}

/// Port of `struct zle_position` from Src/Zle/zle_utils.c:594.
/// Saved (cs, mark, ll) for a stacked position.
#[derive(Debug, Clone)]
pub struct ZlePosition {                                                     // c:594
    /// Cursor position.
    pub cs: usize,                                                           // c:599
    /// Mark.
    pub mk: usize,                                                           // c:601
    /// Line length.
    pub ll: usize,                                                           // c:603
    // c:604 region_highlights chain — region-highlight system not yet
    // a static-link Rust struct; saved positions don't carry region
    // state until that lands.
}

/// Port of `static struct zle_position *zle_positions` from
/// Src/Zle/zle_utils.c:608. LIFO stack of saved positions.
pub static ZLE_POSITIONS: std::sync::Mutex<Vec<ZlePosition>> =               // c:608
    std::sync::Mutex::new(Vec::new());

/// Port of `mod_export void zle_save_positions(void)` from
/// Src/Zle/zle_utils.c:619.
///
/// "Save positions including cursor, end-of-line and (non-special)
/// region highlighting. Must be matched by a subsequent
/// `zle_restore_positions()`."
pub fn zle_save_positions(zle: &crate::ported::zle::zle_main::Zle) {         // c:619
    let pos = ZlePosition {                                                  // c:625 newpos = zalloc
        mk: zle.mark,                                                        // c:627
        cs: zle.zlecs,                                                       // c:634 (no zlemetaline branch)
        ll: zle.zlell,                                                       // c:635
    };
    if let Ok(mut s) = ZLE_POSITIONS.lock() {                                // c:665 push
        s.push(pos);
    }
}

/// Port of `mod_export void zle_restore_positions(void)` from
/// Src/Zle/zle_utils.c:677. Pops the last saved (cs, mark, ll).
pub fn zle_restore_positions(zle: &mut crate::ported::zle::zle_main::Zle) {  // c:677
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        if let Some(oldpos) = s.pop() {                                      // c:679-684
            zle.mark = oldpos.mk;                                            // c:686
            zle.zlecs = oldpos.cs.min(zle.zlell);                            // c:693
            zle.zlell = oldpos.ll;                                           // c:694
        }
    }
}

/// Port of `mod_export void zle_free_positions(void)` from
/// Src/Zle/zle_utils.c:747. Discards the top of stack without
/// applying it.
pub fn zle_free_positions() {                                                // c:747
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        s.pop();                                                             // c:749 oldpos = zle_positions; zle_positions = next
    }
}

/// Port of `zleaddtoline()` from Src/Zle/zle_utils.c:102.
pub fn zleaddtoline(zle: &mut crate::ported::zle::zle_main::Zle, ch: i32) {  // c:102
    // C body c:104-115 — `sizeline(zlell+1); zleline[zlell] = ch;
    //                    zleline[++zlell] = '\\0'`.
    zle.zleline.push(ch as u8 as char);
    zle.zlell = zle.zleline.len();
}

/// Port of `zlecallhook()` from Src/Zle/zle_utils.c:1755.
pub fn zlecallhook(name: &str, arg: Option<&str>) {                          // c:1755
    // C body c:1757-1840 — looks up shfunc `name` and dispatches via
    //                      execzlefunc. Without exec hook we record
    //                      via tracing.
    tracing::debug!(target: "zle", "zlecallhook({}, {:?})", name, arg);
}

/// Port of `zlecharasstring()` from Src/Zle/zle_utils.c:117.
pub fn zlecharasstring(c: char, buf: &mut String) -> i32 {                   // c:117
    // C body c:119-145 — converts a ZLE_CHAR_T to its display form
    //                    (UTF-8 multibyte if MULTIBYTE_SUPPORT, else
    //                    raw byte). Vec<char> is wide-char already;
    //                    just append.
    let start = buf.len();
    buf.push(c);
    (buf.len() - start) as i32
}

/// Port of `zlegetline()` from Src/Zle/zle_utils.c:547.
pub fn zlegetline(zle: &crate::ported::zle::zle_main::Zle,                   // c:148
                  ll: &mut usize, cs: &mut usize) -> Vec<char> {
    // C body c:150-200 — `if (zlemetaline) { *ll=zlemetall; *cs=zlemetacs;
    //                     return ztrdup(zlemetaline) } else
    //                     return zlelineasstring(...)`. Snapshot of the
    //                     current line + cursor.
    *ll = zle.zlell;
    *cs = zle.zlecs;
    zle.zleline.clone()
}

/// Port of `zlelineasstring()` from Src/Zle/zle_utils.c:192.
pub fn zlelineasstring(line: &[char], ll: usize, _flags: i32) -> String {    // c:282
    // C body c:284-373 — encodes ZLE_CHAR_T array to a metafied
    //                    multibyte string. Vec<char> → String is
    //                    direct; meta encoding skipped (we don't run
    //                    through zsh's parser path).
    line.iter().take(ll).collect()
}

#[cfg(test)]
mod findbol_findeol_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;

    fn zle_with(line: &str, cs: usize) -> Zle {
        let mut z = Zle::default();
        z.zleline = line.chars().collect();
        z.zlell = z.zleline.len();
        z.zlecs = cs;
        z
    }

    #[test]
    fn findbol_no_newline_returns_zero() {
        // c:1162 — walks back to start when no '\n' encountered.
        let z = zle_with("hello world", 7);
        assert_eq!(findbol(&z), 0);
    }

    #[test]
    fn findbol_finds_preceding_newline() {
        // c:1162 — `zleline[x-1] != '\n'` exits loop when prev char IS '\n'.
        // For "abc\ndef\nghi" with cursor at 9 (the 'h' in 'ghi'):
        // walks back to 8 (after the second '\n'), returns 8.
        let z = zle_with("abc\ndef\nghi", 9);
        assert_eq!(findbol(&z), 8);
    }

    #[test]
    fn findbol_at_start_returns_zero() {
        let z = zle_with("anything", 0);
        assert_eq!(findbol(&z), 0);
    }

    #[test]
    fn findeol_no_newline_returns_end() {
        // c:1173 — walks forward to zlell when no '\n' encountered.
        let z = zle_with("hello world", 0);
        assert_eq!(findeol(&z), 11);
    }

    #[test]
    fn findeol_finds_next_newline() {
        // c:1173 — `zleline[x] != '\n'` exits when current char IS '\n'.
        // For "abc\ndef" cursor at 0: walks 0→1→2→3 (which is '\n'), returns 3.
        let z = zle_with("abc\ndef", 0);
        assert_eq!(findeol(&z), 3);
    }

    #[test]
    fn findeol_at_end_returns_zlell() {
        let z = zle_with("hello", 5);
        assert_eq!(findeol(&z), 5);
    }

    #[test]
    fn findline_returns_bol_eol_pair() {
        // c:1182-1183 — both findbol and findeol from the same cursor.
        // "abc\ndef\nghi" cursor at 5 (the 'e' in 'def'):
        //   findbol → 4 (after first '\n')
        //   findeol → 7 (the second '\n')
        let z = zle_with("abc\ndef\nghi", 5);
        let (bol, eol) = findline(&z);
        assert_eq!(bol, 4);
        assert_eq!(eol, 7);
    }
}
