//! ZLE parameters
//!
//! Direct port from zsh/Src/Zle/zle_params.c
//!
//! Special parameters that expose ZLE state to shell scripts

use super::main::Zle;

/// ZLE parameter names
pub mod names {
    pub const BUFFER: &str = "BUFFER";
    pub const CURSOR: &str = "CURSOR";
    pub const LBUFFER: &str = "LBUFFER";
    pub const RBUFFER: &str = "RBUFFER";
    pub const PREBUFFER: &str = "PREBUFFER";
    pub const WIDGET: &str = "WIDGET";
    pub const LASTWIDGET: &str = "LASTWIDGET";
    pub const KEYMAP: &str = "KEYMAP";
    pub const KEYS: &str = "KEYS";
    pub const NUMERIC: &str = "NUMERIC";
    pub const HISTNO: &str = "HISTNO";
    pub const BUFFERLINES: &str = "BUFFERLINES";
    pub const PENDING: &str = "PENDING";
    pub const CUTBUFFER: &str = "CUTBUFFER";
    pub const KILLRING: &str = "killring";
    pub const MARK: &str = "MARK";
    pub const REGION_ACTIVE: &str = "REGION_ACTIVE";
    pub const ZLE_STATE: &str = "ZLE_STATE";
}

impl Zle {
    /// Get BUFFER parameter
    pub fn get_buffer(&self) -> String {
        self.zleline.iter().collect()
    }

    /// Set BUFFER parameter
    pub fn set_buffer(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlecs.min(self.zlell);
        self.resetneeded = true;
    }

    /// Get CURSOR parameter
    pub fn get_cursor(&self) -> usize {
        self.zlecs
    }

    /// Set CURSOR parameter
    pub fn set_cursor(&mut self, pos: usize) {
        self.zlecs = pos.min(self.zlell);
        self.resetneeded = true;
    }

    /// Get LBUFFER (text before cursor)
    pub fn get_lbuffer(&self) -> String {
        self.zleline[..self.zlecs].iter().collect()
    }

    /// Set LBUFFER
    pub fn set_lbuffer(&mut self, s: &str) {
        let rbuf: String = self.zleline[self.zlecs..].iter().collect();
        self.zleline = s.chars().chain(rbuf.chars()).collect();
        self.zlell = self.zleline.len();
        self.zlecs = s.chars().count();
        self.resetneeded = true;
    }

    /// Get RBUFFER (text after cursor)
    pub fn get_rbuffer(&self) -> String {
        self.zleline[self.zlecs..].iter().collect()
    }

    /// Set RBUFFER
    pub fn set_rbuffer(&mut self, s: &str) {
        let lbuf: String = self.zleline[..self.zlecs].iter().collect();
        self.zleline = lbuf.chars().chain(s.chars()).collect();
        self.zlell = self.zleline.len();
        self.resetneeded = true;
    }

    /// Get CUTBUFFER (kill ring top)
    pub fn get_cutbuffer(&self) -> String {
        self.killring
            .front()
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Set CUTBUFFER
    pub fn set_cutbuffer(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        if self.killring.is_empty() {
            self.killring.push_front(chars);
        } else {
            self.killring[0] = chars;
        }
    }

    /// Get MARK parameter
    pub fn get_mark(&self) -> usize {
        self.mark
    }

    /// Set MARK parameter
    pub fn set_mark(&mut self, pos: usize) {
        self.mark = pos.min(self.zlell);
    }

    /// Get BUFFERLINES (number of lines)
    pub fn get_bufferlines(&self) -> usize {
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// Get PENDING (number of bytes waiting)
    pub fn get_pending(&self) -> usize {
        // unget_buf is private, return 0 for now
        0
    }

    /// Get current keymap name
    pub fn get_keymap(&self) -> &str {
        &self.keymaps.current_name
    }

    /// Get NUMERIC (numeric argument if set)
    pub fn get_numeric(&self) -> Option<i32> {
        if self.zmod.flags.contains(super::main::ModifierFlags::MULT) {
            Some(self.zmod.mult)
        } else {
            None
        }
    }

    /// Check if in insert mode
    pub fn is_insert_mode(&self) -> bool {
        self.insmode
    }

    /// Check if region is active
    pub fn is_region_active(&self) -> bool {
        // Region is "active" if mark != cursor (simplified)
        self.mark != self.zlecs
    }

    /// Get ZLE_STATE string
    pub fn get_zle_state(&self) -> String {
        let mut state = String::new();

        if self.insmode {
            state.push_str("insert");
        } else {
            state.push_str("overwrite");
        }

        // Add keymap info
        state.push(':');
        state.push_str(&self.keymaps.current_name);

        state
    }
}
