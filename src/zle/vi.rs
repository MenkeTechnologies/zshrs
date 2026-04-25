//! ZLE vi mode operations
//!
//! Direct port from zsh/Src/Zle/zle_vi.c

use super::main::{ModifierFlags, Zle};

/// Vi operation pending
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViPendingOp {
    None,
    Delete,
    Change,
    Yank,
    ShiftLeft,
    ShiftRight,
    Filter,
    Case,
}

/// Vi state
#[derive(Debug, Default)]
pub struct ViState {
    /// Pending operator
    pub pending_op: Option<ViPendingOp>,
    /// Character to find
    pub find_char: Option<char>,
    /// Find direction (true = forward)
    pub find_forward: bool,
    /// Find skip (t/T vs f/F)
    pub find_skip: bool,
    /// Last change for dot repeat
    pub last_change: Option<ViChange>,
    /// Numeric argument being built
    pub arg: Option<i32>,
}

/// A recorded vi change for repeat
#[derive(Debug, Clone)]
pub struct ViChange {
    /// Keys that made up the change
    pub keys: Vec<u8>,
    /// Starting cursor position
    pub start_cs: usize,
}

impl Zle {
    /// Get numeric argument (mult)
    pub fn vi_get_arg(&self) -> i32 {
        if self.zmod.flags.contains(ModifierFlags::MULT) {
            self.zmod.mult
        } else {
            1
        }
    }

    /// Handle vi find character (f/F/t/T)
    pub fn vi_find_char(&mut self, forward: bool, skip: bool) {
        // Read the character to find
        let c = match self.getfullchar(true) {
            Some(c) => c,
            None => return,
        };

        let count = self.vi_get_arg();

        for _ in 0..count {
            if forward {
                // Search forward
                let mut pos = self.zlecs + 1;
                while pos < self.zlell {
                    if self.zleline[pos] == c {
                        self.zlecs = if skip { pos - 1 } else { pos };
                        break;
                    }
                    pos += 1;
                }
            } else {
                // Search backward
                if self.zlecs > 0 {
                    let mut pos = self.zlecs - 1;
                    loop {
                        if self.zleline[pos] == c {
                            self.zlecs = if skip { pos + 1 } else { pos };
                            break;
                        }
                        if pos == 0 {
                            break;
                        }
                        pos -= 1;
                    }
                }
            }
        }

        self.resetneeded = true;
    }

    /// Vi percent match (find matching bracket)
    pub fn vi_match_bracket(&mut self) {
        let c = if self.zlecs < self.zlell {
            self.zleline[self.zlecs]
        } else {
            return;
        };

        let (target, forward) = match c {
            '(' => (')', true),
            ')' => ('(', false),
            '[' => (']', true),
            ']' => ('[', false),
            '{' => ('}', true),
            '}' => ('{', false),
            '<' => ('>', true),
            '>' => ('<', false),
            _ => return,
        };

        let mut depth = 1;
        let mut pos = self.zlecs;

        if forward {
            pos += 1;
            while pos < self.zlell && depth > 0 {
                if self.zleline[pos] == c {
                    depth += 1;
                } else if self.zleline[pos] == target {
                    depth -= 1;
                }
                if depth > 0 {
                    pos += 1;
                }
            }
        } else {
            if pos > 0 {
                pos -= 1;
                loop {
                    if self.zleline[pos] == c {
                        depth += 1;
                    } else if self.zleline[pos] == target {
                        depth -= 1;
                    }
                    if depth == 0 || pos == 0 {
                        break;
                    }
                    pos -= 1;
                }
            }
        }

        if depth == 0 {
            self.zlecs = pos;
            self.resetneeded = true;
        }
    }

    /// Vi replace mode (R command)
    pub fn vi_replace_mode(&mut self) {
        self.keymaps.select("viins");
        self.insmode = false; // Overwrite mode
    }

    /// Vi swap case
    pub fn vi_swap_case(&mut self) {
        let count = self.vi_get_arg() as usize;

        for _ in 0..count {
            if self.zlecs < self.zlell {
                let c = self.zleline[self.zlecs];
                self.zleline[self.zlecs] = if c.is_uppercase() {
                    c.to_lowercase().next().unwrap_or(c)
                } else if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
                self.zlecs += 1;
            }
        }

        // Move back one if we went past end
        if self.zlecs > 0 && self.zlecs == self.zlell {
            self.zlecs -= 1;
        }

        self.resetneeded = true;
    }

    /// Vi undo (u command)
    pub fn vi_undo(&mut self) {
        // TODO: implement full undo
    }

    /// Vi visual mode
    pub fn vi_visual_mode(&mut self) {
        self.mark = self.zlecs;
        // TODO: implement visual mode state
    }

    /// Vi visual line mode
    pub fn vi_visual_line_mode(&mut self) {
        self.mark = self.zlecs;
        // TODO: implement visual line mode
    }

    /// Vi visual block mode
    pub fn vi_visual_block_mode(&mut self) {
        self.mark = self.zlecs;
        // TODO: implement visual block mode
    }

    /// Vi set mark
    pub fn vi_set_mark(&mut self, name: char) {
        // TODO: implement named marks
        let _ = name;
        self.mark = self.zlecs;
    }

    /// Vi goto mark
    pub fn vi_goto_mark(&mut self, name: char) {
        // TODO: implement named marks
        let _ = name;
    }

    /// Record keys for vi repeat
    pub fn vi_record_change(&mut self, key: u8) {
        // TODO: implement change recording
        let _ = key;
    }

    /// Replay last change (dot command)
    pub fn vi_repeat_change(&mut self) {
        // TODO: implement change replay
    }
}
