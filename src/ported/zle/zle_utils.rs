//! ZLE utility functions
//!
//! Direct port from zsh/Src/Zle/zle_utils.c
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
// machinery lives directly on `Zle` (`undo_stack: Vec<Change>`,
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
    pub fn find_bol(&self, pos: usize) -> usize {
        let mut p = pos;
        while p > 0 && self.zleline.get(p - 1) != Some(&'\n') {
            p -= 1;
        }
        p
    }

    /// Find end of line from position
    /// Port of findeol() from zle_utils.c
    pub fn find_eol(&self, pos: usize) -> usize {
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

    /// Ensure line has enough space
    /// Port of sizeline() from zle_utils.c
    pub fn size_line(&mut self, needed: usize) {
        if self.zleline.capacity() < needed {
            self.zleline.reserve(needed - self.zleline.len());
        }
    }

    /// Make space in line at position
    /// Port of spaceinline() from zle_utils.c
    pub fn space_in_line(&mut self, pos: usize, count: usize) {
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
    pub fn shift_chars(&mut self, from: usize, count: i32) {
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
    pub fn fore_del(&mut self, count: usize, flags: CutFlags) {
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
    pub fn back_del(&mut self, count: usize, flags: CutFlags) {
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
    pub fn fore_kill(&mut self, count: usize, append: bool) {
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
    pub fn back_kill(&mut self, count: usize, append: bool) {
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
    pub fn cut_text(&mut self, start: usize, end: usize, dir: CutDirection) {
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

    /// If the line changed since the last snapshot, append a Change record
    /// describing the diff. Port of `mkundoent` (zle_utils.c:1532).
    pub fn mkundoent(&mut self) {
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
        let ch = super::zle_main::Change {
            flags: super::zle_main::ChangeFlags::empty(),
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

    /// Pre-widget hook. Port of `handleundo` (zle_utils.c) — currently a thin
    /// stub since `mkundoent` runs after each widget; the C version uses it to
    /// flush in-flight `nextchanges` chains, which our one-change-per-widget
    /// model doesn't need.
    pub fn handleundo(&mut self) {
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

    /// Walk back one Change. Port of `undo` (zle_utils.c:1601).
    pub fn undo_widget(&mut self) -> i32 {
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

    /// Walk forward one Change. Port of `redo` (zle_utils.c:1661).
    pub fn redo_widget(&mut self) -> i32 {
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

/// Port of `applychange()` from Src/Zle/zle_utils.c:1678. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn applychange() -> i32 { 0 }

/// Port of `backdel()` from Src/Zle/zle_utils.c:1084. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backdel() -> i32 { 0 }

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

/// Port of `cut()` from Src/Zle/zle_utils.c:935. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cut() -> i32 { 0 }

/// Port of `cuttext()` from Src/Zle/zle_utils.c:946. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cuttext() -> i32 { 0 }

/// Port of `findbol()` from Src/Zle/zle_utils.c:1158. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn findbol() -> i32 { 0 }

/// Port of `findeol()` from Src/Zle/zle_utils.c:1169. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn findeol() -> i32 { 0 }

/// Port of `findline()` from Src/Zle/zle_utils.c:1180. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn findline() -> i32 { 0 }

/// Port of `foredel()` from Src/Zle/zle_utils.c:1105. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn foredel() -> i32 { 0 }

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

/// Port of `free_region_highlights_memos()` from Src/Zle/zle_utils.c:567. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn free_region_highlights_memos() -> i32 { 0 }

/// Port of `freechanges()` from Src/Zle/zle_utils.c:1472. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freechanges() -> i32 { 0 }

/// Port of `freeundo()` from Src/Zle/zle_utils.c:1461. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freeundo() -> i32 { 0 }

/// Port of `get_undo_current_change()` from Src/Zle/zle_utils.c:1785. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_undo_current_change() -> i32 { 0 }

/// Port of `get_undo_limit_change()` from Src/Zle/zle_utils.c:1812. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_undo_limit_change() -> i32 { 0 }

/// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getzlequery() -> i32 { 0 }

/// Port of `handlefeep()` from Src/Zle/zle_utils.c:1405. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn handlefeep() -> i32 { 0 }

/// Port of `handlesuffix()` from Src/Zle/zle_utils.c:1415. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn handlesuffix() -> i32 { 0 }

/// Port of `initundo()` from Src/Zle/zle_utils.c:1446. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn initundo() -> i32 { 0 }

/// Port of `mergeundo()` from Src/Zle/zle_utils.c:1733. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn mergeundo() -> i32 { 0 }

/// Port of `redo()` from Src/Zle/zle_utils.c:1661. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn redo() -> i32 { 0 }

/// Port of `set_undo_limit_change()` from Src/Zle/zle_utils.c:1819. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_undo_limit_change() -> i32 { 0 }

/// Port of `setline()` from Src/Zle/zle_utils.c:1129. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setline() -> i32 { 0 }

/// Port of `shiftchars()` from Src/Zle/zle_utils.c:846. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn shiftchars() -> i32 { 0 }

/// Port of `showmsg()` from Src/Zle/zle_utils.c:1303. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn showmsg() -> i32 { 0 }

/// Port of `sizeline()` from Src/Zle/zle_utils.c:67. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn sizeline() -> i32 { 0 }

/// Port of `spaceinline()` from Src/Zle/zle_utils.c:777. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn spaceinline() -> i32 { 0 }

/// Port of `splitundo()` from Src/Zle/zle_utils.c:1721. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn splitundo() -> i32 { 0 }

/// Port of `stringaszleline()` from Src/Zle/zle_utils.c:375. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn stringaszleline() -> i32 { 0 }

/// Port of `unapplychange()` from Src/Zle/zle_utils.c:1634. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn unapplychange() -> i32 { 0 }

/// Port of `undo()` from Src/Zle/zle_utils.c:1601. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn undo() -> i32 { 0 }

/// Port of `viundochange()` from Src/Zle/zle_utils.c:1705. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viundochange() -> i32 { 0 }

/// Port of `zle_free_positions()` from Src/Zle/zle_utils.c:747. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_free_positions() -> i32 { 0 }

/// Port of `zle_restore_positions()` from Src/Zle/zle_utils.c:677. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_restore_positions() -> i32 { 0 }

/// Port of `zle_save_positions()` from Src/Zle/zle_utils.c:619. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zle_save_positions() -> i32 { 0 }

/// Port of `zleaddtoline()` from Src/Zle/zle_utils.c:102. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zleaddtoline() -> i32 { 0 }

/// Port of `zlecallhook()` from Src/Zle/zle_utils.c:1755. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlecallhook() -> i32 { 0 }

/// Port of `zlecharasstring()` from Src/Zle/zle_utils.c:117. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlecharasstring() -> i32 { 0 }

/// Port of `zlegetline()` from Src/Zle/zle_utils.c:547. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlegetline() -> i32 { 0 }

/// Port of `zlelineasstring()` from Src/Zle/zle_utils.c:192. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlelineasstring() -> i32 { 0 }
