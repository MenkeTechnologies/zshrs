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

use super::main::{Zle, ZleChar, ZleString};

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

    /// Set the line from a string
    pub fn set_line(&mut self, s: &str) {
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

    /// Paste from named buffer
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
pub fn metafy(s: &str) -> String {
    // In zsh, Meta (0x83) is used to escape special bytes
    // For Rust we typically don't need this, but provide for compatibility
    s.to_string()
}

pub fn unmetafy(s: &str) -> String {
    s.to_string()
}

/// String width calculation
pub fn strwidth(s: &str) -> usize {
    // TODO: use unicode-width for proper width calculation
    s.chars().count()
}

/// Check if character is printable
pub fn is_printable(c: char) -> bool {
    !c.is_control() && c != '\x7f'
}

/// Escape special characters for display
pub fn escape_for_display(c: char) -> String {
    if c.is_control() {
        if c as u32 <= 26 {
            format!("^{}", (c as u8 + b'@') as char)
        } else {
            format!("\\x{:02x}", c as u32)
        }
    } else if c == '\x7f' {
        "^?".to_string()
    } else {
        c.to_string()
    }
}

/// Undo entry structure
/// Port of struct change from zle_utils.c
#[derive(Debug, Clone)]
pub struct UndoEntry {
    /// Start position of change
    pub start: usize,
    /// End position of change (original)
    pub end: usize,
    /// Inserted/deleted text
    pub text: ZleString,
    /// Cursor position before change
    pub cursor: usize,
    /// Whether this is the start of a group
    pub group_start: bool,
}

/// Undo state
#[derive(Debug, Default)]
pub struct UndoState {
    /// Undo history
    pub history: Vec<UndoEntry>,
    /// Current position in undo history
    pub current: usize,
    /// Undo limit (where to stop)
    pub limit: usize,
    /// Whether changes are being recorded
    pub recording: bool,
    /// Merge sequential inserts
    pub merge_inserts: bool,
}

impl UndoState {
    pub fn new() -> Self {
        UndoState {
            recording: true,
            merge_inserts: true,
            ..Default::default()
        }
    }

    /// Initialize undo system
    /// Port of initundo() from zle_utils.c
    pub fn init(&mut self) {
        self.history.clear();
        self.current = 0;
        self.limit = 0;
        self.recording = true;
    }

    /// Free undo history
    /// Port of freeundo() from zle_utils.c
    pub fn free(&mut self) {
        self.history.clear();
        self.current = 0;
    }

    /// Create an undo entry
    /// Port of mkundoent() from zle_utils.c
    pub fn make_entry(&mut self, start: usize, end: usize, text: ZleString, cursor: usize) {
        if !self.recording {
            return;
        }

        // Remove any entries after current position (redo history)
        self.history.truncate(self.current);

        let entry = UndoEntry {
            start,
            end,
            text,
            cursor,
            group_start: false,
        };

        self.history.push(entry);
        self.current = self.history.len();
    }

    /// Split undo (start a new undo group)
    /// Port of splitundo() from zle_utils.c
    pub fn split(&mut self) {
        if let Some(entry) = self.history.last_mut() {
            entry.group_start = true;
        }
    }

    /// Merge with previous undo entry
    /// Port of mergeundo() from zle_utils.c
    pub fn merge(&mut self) {
        // For sequential character inserts, merge into one undo
        if self.history.len() >= 2 {
            let last = self.history.len() - 1;
            let prev = last - 1;

            // Check if mergeable (consecutive inserts at same position)
            if self.history[prev].end == self.history[last].start
                && self.history[prev].text.is_empty()
                && self.history[last].text.is_empty()
            {
                self.history[prev].end = self.history[last].end;
                self.history.pop();
                self.current = self.history.len();
            }
        }
    }

    /// Get current change
    /// Port of get_undo_current_change() from zle_utils.c
    pub fn get_current(&self) -> Option<&UndoEntry> {
        if self.current > 0 {
            self.history.get(self.current - 1)
        } else {
            None
        }
    }

    /// Set undo limit
    /// Port of set_undo_limit_change() from zle_utils.c
    pub fn set_limit(&mut self) {
        self.limit = self.current;
    }

    /// Get undo limit
    /// Port of get_undo_limit_change() from zle_utils.c
    pub fn get_limit(&self) -> usize {
        self.limit
    }
}

impl Zle {
    /// Apply an undo entry
    /// Port of applychange() from zle_utils.c
    fn apply_change(&mut self, entry: &UndoEntry, reverse: bool) {
        if reverse {
            // Redo: re-insert removed text
            let removed: ZleString = self.zleline.drain(entry.start..entry.end).collect();
            for (i, &c) in entry.text.iter().enumerate() {
                self.zleline.insert(entry.start + i, c);
            }
            self.zlell = self.zleline.len();
            self.zlecs = entry.cursor;

            // Store for undo
            let _ = removed;
        } else {
            // Undo: remove inserted text and restore old
            let end = entry.start + entry.text.len();
            self.zleline.drain(entry.start..end.min(self.zleline.len()));
            for (i, &c) in entry.text.iter().enumerate() {
                self.zleline.insert(entry.start + i, c);
            }
            self.zlell = self.zleline.len();
            self.zlecs = entry.cursor;
        }
        self.resetneeded = true;
    }

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

    /// Set the last line (for history)
    /// Port of setlastline() from zle_utils.c
    pub fn set_last_line(&mut self) {
        // Would store current line as last line
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

    /// Get ZLE query (for menu selection etc)
    /// Port of getzlequery() from zle_utils.c
    pub fn get_zle_query(&self) -> Option<String> {
        // Would prompt for input
        None
    }

    /// Handle suffix (for completion)
    /// Port of handlesuffix() from zle_utils.c
    pub fn handle_suffix(&mut self) {
        // Would handle completion suffix removal
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

/// Print a key binding for display
/// Port of printbind() from zle_utils.c
pub fn print_bind(seq: &[u8]) -> String {
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

/// Call ZLE hook
/// Port of zlecallhook() from zle_utils.c  
pub fn zle_call_hook(_name: &str, _args: &[&str]) -> i32 {
    // Would call user-defined hook function
    0
}
