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

// Each accessor below corresponds to one of the special parameters
// zsh exposes via Src/Zle/zle_params.c. The C source registers them
// through the `zleparams[]` table at zle_params.c:38; widget bodies
// (and shell scripts running inside ZLE) read or assign to them
// through the parameter system.
impl Zle {
    /// `$BUFFER` accessor — full edited line as a String.
    /// Port of `get_buffer()` from Src/Zle/zle_params.c (the
    /// `BUFFER` getfn entry in `zleparams[]`).
    pub fn get_buffer(&self) -> String {
        self.zleline.iter().collect()
    }

    /// `$BUFFER=s` setter — replace the full edited line.
    /// Port of `set_buffer()` from Src/Zle/zle_params.c (the
    /// `BUFFER` setfn entry); zsh clamps the cursor to the new
    /// length, mirrored here.
    pub fn set_buffer(&mut self, s: &str) {
        self.zleline = s.chars().collect();
        self.zlell = self.zleline.len();
        self.zlecs = self.zlecs.min(self.zlell);
        self.resetneeded = true;
    }

    /// `$CURSOR` accessor — current cursor position (0-indexed).
    /// Port of `get_cursor()` from Src/Zle/zle_params.c.
    pub fn get_cursor(&self) -> usize {
        self.zlecs
    }

    /// `$CURSOR=pos` setter — clamped to buffer length.
    /// Port of `set_cursor()` from Src/Zle/zle_params.c.
    pub fn set_cursor(&mut self, pos: usize) {
        self.zlecs = pos.min(self.zlell);
        self.resetneeded = true;
    }

    /// `$LBUFFER` accessor — text before the cursor.
    /// Port of `get_lbuffer()` from Src/Zle/zle_params.c.
    pub fn get_lbuffer(&self) -> String {
        self.zleline[..self.zlecs].iter().collect()
    }

    /// `$LBUFFER=s` setter — replace text before the cursor; cursor
    /// lands at the new lbuffer's end.
    /// Port of `set_lbuffer()` from Src/Zle/zle_params.c.
    pub fn set_lbuffer(&mut self, s: &str) {
        let rbuf: String = self.zleline[self.zlecs..].iter().collect();
        self.zleline = s.chars().chain(rbuf.chars()).collect();
        self.zlell = self.zleline.len();
        self.zlecs = s.chars().count();
        self.resetneeded = true;
    }

    /// `$RBUFFER` accessor — text after the cursor.
    /// Port of `get_rbuffer()` from Src/Zle/zle_params.c.
    pub fn get_rbuffer(&self) -> String {
        self.zleline[self.zlecs..].iter().collect()
    }

    /// `$RBUFFER=s` setter — replace text after the cursor.
    /// Port of `set_rbuffer()` from Src/Zle/zle_params.c.
    pub fn set_rbuffer(&mut self, s: &str) {
        let lbuf: String = self.zleline[..self.zlecs].iter().collect();
        self.zleline = lbuf.chars().chain(s.chars()).collect();
        self.zlell = self.zleline.len();
        self.resetneeded = true;
    }

    /// `$CUTBUFFER` accessor — most-recent kill-ring entry.
    /// Port of `get_cutbuffer()` from Src/Zle/zle_params.c which
    /// reads `cutbuf` (the unnamed kill register).
    pub fn get_cutbuffer(&self) -> String {
        self.killring
            .front()
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// `$CUTBUFFER=s` setter — overwrite the front of the kill ring.
    /// Port of `set_cutbuffer()` from Src/Zle/zle_params.c.
    pub fn set_cutbuffer(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        if self.killring.is_empty() {
            self.killring.push_front(chars);
        } else {
            self.killring[0] = chars;
        }
    }

    /// `$MARK` accessor — current mark position.
    /// Port of `get_mark()` from Src/Zle/zle_params.c.
    pub fn get_mark(&self) -> usize {
        self.mark
    }

    /// `$MARK=pos` setter — clamp to buffer length.
    /// Port of `set_mark()` from Src/Zle/zle_params.c.
    pub fn set_mark(&mut self, pos: usize) {
        self.mark = pos.min(self.zlell);
    }

    /// `$BUFFERLINES` accessor — number of newline-separated lines.
    /// Port of `get_bufferlines()` from Src/Zle/zle_params.c.
    pub fn get_bufferlines(&self) -> usize {
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// `$PENDING` accessor — bytes waiting in the input queue.
    /// Port of `get_pending()` from Src/Zle/zle_params.c which
    /// returns `kungetct` (the unget-buffer fill).
    pub fn get_pending(&self) -> usize {
        0 // unget_buf is private; future expansion can expose its len
    }

    /// `$KEYMAP` accessor — currently-active keymap name.
    /// Port of `get_keymap()` from Src/Zle/zle_params.c.
    pub fn get_keymap(&self) -> &str {
        &self.keymaps.current_name
    }

    /// `$NUMERIC` accessor — numeric prefix when set.
    /// Port of `get_numeric()` from Src/Zle/zle_params.c which
    /// returns `zmod.mult` only when `MOD_MULT` is set, otherwise
    /// the parameter is unset.
    pub fn get_numeric(&self) -> Option<i32> {
        if self.zmod.flags.contains(super::main::ModifierFlags::MULT) {
            Some(self.zmod.mult)
        } else {
            None
        }
    }

    /// `$ZLE_STATE` insert/overwrite component — true for insert.
    /// Sub-port of `get_zle_state()` (Src/Zle/zle_params.c) which
    /// emits "insert" / "overwrite" + " " + "vicmd" / "main".
    pub fn is_insert_mode(&self) -> bool {
        self.insmode
    }

    /// `$REGION_ACTIVE` accessor — non-zero when a visual selection
    /// is active.
    /// Port of `get_region_active()` from Src/Zle/zle_params.c. The
    /// C source returns 1/2 (charwise/linewise); our simplified
    /// boolean compares mark vs cursor.
    pub fn is_region_active(&self) -> bool {
        self.mark != self.zlecs
    }

    /// `$ZLE_STATE` accessor — "insert"|"overwrite" + ":" + keymap.
    /// Port of `get_zle_state()` from Src/Zle/zle_params.c. The C
    /// source emits a space-separated list of state words; our
    /// minimal version covers the two most-consulted fields.
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
