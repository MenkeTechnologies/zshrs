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

    /// Read the next char from input and run a vi find-char.
    /// `forward`: true for f/t (forward), false for F/T (backward).
    /// `skip`: true for t/T (stop one short), false for f/F (land on the char).
    /// Port of vifindnextchar/vifindprevchar/vifindnextcharskip/vifindprevcharskip
    /// from Src/Zle/zle_move.c:739-783 — which all set state and call `vifindchar(0)`.
    pub fn vi_find_char(&mut self, forward: bool, skip: bool) {
        let c = match self.getfullchar(true) {
            Some(c) => c,
            None => return,
        };
        self.vi_last_find_char = Some(c);
        self.vi_last_find_dir = if forward { 1 } else { -1 };
        // tailadd: f/F → 0; t → -1; T → +1.
        self.vi_last_find_tail = match (forward, skip) {
            (_, false) => 0,
            (true, true) => -1,
            (false, true) => 1,
        };
        let _ = self.vi_find_char_inner(false);
    }

    /// Inner find-char routine. `repeat` distinguishes the user-typed call
    /// from `;` / `,` re-runs.
    /// Port of `vifindchar(int repeat, ...)` from Src/Zle/zle_move.c:787.
    pub fn vi_find_char_inner(&mut self, repeat: bool) -> i32 {
        let target = match self.vi_last_find_char {
            Some(c) => c,
            None => return 1,
        };
        if self.vi_last_find_dir == 0 {
            return 1;
        }
        let ocs = self.zlecs;
        let mut n = self.vi_get_arg();
        if n < 0 {
            // Negative count flips direction; faithful to C virevrepeatfind path.
            n = -n;
            self.vi_last_find_dir = -self.vi_last_find_dir;
            self.vi_last_find_tail = -self.vi_last_find_tail;
            let saved_mult = self.zmod.mult;
            self.zmod.mult = n;
            let ret = self.vi_find_char_inner(repeat);
            self.zmod.mult = saved_mult;
            self.vi_last_find_dir = -self.vi_last_find_dir;
            self.vi_last_find_tail = -self.vi_last_find_tail;
            return ret;
        }
        // On `;` (repeat) with t/T, step over the immediately-adjacent match
        // so we don't get stuck on the same char.
        if repeat && self.vi_last_find_tail != 0 {
            if self.vi_last_find_dir > 0 {
                if self.zlecs < self.zlell
                    && self.zlecs + 1 < self.zlell
                    && self.zleline[self.zlecs + 1] == target
                {
                    self.zlecs += 1;
                }
            } else if self.zlecs > 0 && self.zleline[self.zlecs - 1] == target {
                self.zlecs -= 1;
            }
        }
        let dir = self.vi_last_find_dir;
        for _ in 0..n {
            // Step at least once, then keep stepping until we land on the char,
            // hit a newline, or run off the end.
            let found = if dir > 0 {
                let mut p = self.zlecs + 1;
                let mut hit = None;
                while p < self.zlell {
                    let ch = self.zleline[p];
                    if ch == '\n' {
                        break;
                    }
                    if ch == target {
                        hit = Some(p);
                        break;
                    }
                    p += 1;
                }
                hit
            } else {
                if self.zlecs == 0 {
                    None
                } else {
                    let mut p = self.zlecs - 1;
                    let mut hit = None;
                    loop {
                        let ch = self.zleline[p];
                        if ch == '\n' {
                            break;
                        }
                        if ch == target {
                            hit = Some(p);
                            break;
                        }
                        if p == 0 {
                            break;
                        }
                        p -= 1;
                    }
                    hit
                }
            };
            match found {
                Some(p) => self.zlecs = p,
                None => {
                    self.zlecs = ocs;
                    return 1;
                }
            }
        }
        // Apply the t/T adjustment after the final landing.
        if self.vi_last_find_tail > 0 && self.zlecs < self.zlell {
            self.zlecs += 1;
        } else if self.vi_last_find_tail < 0 && self.zlecs > 0 {
            self.zlecs -= 1;
        }
        self.resetneeded = true;
        0
    }

    /// `;` — repeat last find in same direction.
    /// Port of virepeatfind() from Src/Zle/zle_move.c:835.
    pub fn vi_repeat_find(&mut self) -> i32 {
        self.vi_find_char_inner(true)
    }

    /// `,` — repeat last find in reverse direction.
    /// Port of virevrepeatfind() from Src/Zle/zle_move.c:842.
    pub fn vi_rev_repeat_find(&mut self) -> i32 {
        let n = self.vi_get_arg();
        if n < 0 {
            return self.vi_find_char_inner(true);
        }
        self.vi_last_find_tail = -self.vi_last_find_tail;
        self.vi_last_find_dir = -self.vi_last_find_dir;
        let ret = self.vi_find_char_inner(true);
        self.vi_last_find_dir = -self.vi_last_find_dir;
        self.vi_last_find_tail = -self.vi_last_find_tail;
        ret
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

    /// Vi undo (`u` in command mode). Port of viundo() — which in C zsh just
    /// dispatches to undo() (zle_utils.c:1601). Routes through our index-based
    /// undo_widget() that mirrors that implementation.
    pub fn vi_undo(&mut self) {
        let _ = self.undo_widget();
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

    /// Vi set mark (`m{a-z}` in command mode). Port of visetmark() from
    /// Src/Zle/zle_move.c:872. Stores the current cursor and history line in
    /// the named slot; non-letter names are rejected.
    pub fn vi_set_mark(&mut self, name: char) {
        // Set the historical mark (mirror with self.mark for emacs compat).
        self.mark = self.zlecs;
        if let Some(idx) = vi_mark_index(name) {
            self.vi_marks[idx] = Some((self.zlecs, self.history.cursor as i32));
        }
    }

    /// Vi goto mark (`'a` / `` `a `` in command mode). Port of vigotomark()
    /// from zle_move.c:887. ASCII letters jump to the saved location;
    /// `'` / `` ` `` jumps to the implicit "last position" mark; other
    /// characters are rejected.
    pub fn vi_goto_mark(&mut self, name: char) {
        let idx = match vi_mark_index(name) {
            Some(i) => i,
            None => return,
        };
        let (cs, hist) = match self.vi_marks[idx] {
            Some(s) => s,
            None => return,
        };
        // Save the pre-jump position into the implicit mark (slot 26) so the
        // user can return to it with `''`.
        self.vi_marks[26] = Some((self.zlecs, self.history.cursor as i32));
        if hist >= 0 && (hist as usize) < self.history.entries.len() {
            // Cross-history jumps need to load that entry.
            let target = hist as usize;
            if target != self.history.cursor {
                self.history.cursor = target;
                self.zleline = self.history.entries[target].line.chars().collect();
                self.zlell = self.zleline.len();
            }
        }
        self.zlecs = cs.min(self.zlell);
        self.resetneeded = true;
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

/// Map a vi mark name to its slot index in `Zle::vi_marks`.
/// `a..z` → 0..25; `'` / `` ` `` → 26 (the implicit last-position mark).
fn vi_mark_index(name: char) -> Option<usize> {
    if name.is_ascii_lowercase() {
        Some(name as usize - 'a' as usize)
    } else if name == '\'' || name == '`' {
        Some(26)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zle_with(line: &str, cs: usize) -> Zle {
        let mut zle = Zle::new();
        zle.zleline = line.chars().collect();
        zle.zlell = zle.zleline.len();
        zle.zlecs = cs;
        zle
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_forward() {
        let mut zle = zle_with("abcdef", 0);
        zle.vi_last_find_char = Some('d');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = 0;
        assert_eq!(zle.vi_find_char_inner(false), 0);
        assert_eq!(zle.zlecs, 3);
    }

    #[test]
    fn vi_find_char_inner_skip_stops_one_short_forward() {
        let mut zle = zle_with("abcdef", 0);
        zle.vi_last_find_char = Some('d');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = -1; // t = forward skip
        assert_eq!(zle.vi_find_char_inner(false), 0);
        assert_eq!(zle.zlecs, 2);
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_backward() {
        let mut zle = zle_with("abcdef", 5);
        zle.vi_last_find_char = Some('b');
        zle.vi_last_find_dir = -1;
        zle.vi_last_find_tail = 0;
        assert_eq!(zle.vi_find_char_inner(false), 0);
        assert_eq!(zle.zlecs, 1);
    }

    #[test]
    fn vi_find_char_inner_returns_1_and_restores_when_missing() {
        let mut zle = zle_with("abcdef", 0);
        zle.vi_last_find_char = Some('z');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = 0;
        assert_eq!(zle.vi_find_char_inner(false), 1);
        assert_eq!(zle.zlecs, 0);
    }

    #[test]
    fn vi_find_char_inner_stops_at_newline() {
        let mut zle = zle_with("abc\ndef", 0);
        zle.vi_last_find_char = Some('e');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = 0;
        // 'e' is past the \n on the next line; vi find must not cross it.
        assert_eq!(zle.vi_find_char_inner(false), 1);
        assert_eq!(zle.zlecs, 0);
    }

    #[test]
    fn vi_repeat_find_walks_to_next_match_in_same_direction() {
        let mut zle = zle_with("a-b-c-d", 0);
        zle.vi_last_find_char = Some('-');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = 0;
        // Initial find lands on first '-'.
        assert_eq!(zle.vi_find_char_inner(false), 0);
        assert_eq!(zle.zlecs, 1);
        // Repeat-find advances to the next '-'.
        assert_eq!(zle.vi_repeat_find(), 0);
        assert_eq!(zle.zlecs, 3);
        // And the next.
        assert_eq!(zle.vi_repeat_find(), 0);
        assert_eq!(zle.zlecs, 5);
    }

    #[test]
    fn vi_set_and_goto_named_mark_round_trip() {
        let mut zle = zle_with("hello world", 6);
        zle.vi_set_mark('a');
        zle.zlecs = 0;
        zle.vi_goto_mark('a');
        assert_eq!(zle.zlecs, 6);
    }

    #[test]
    fn vi_goto_mark_records_implicit_last_position() {
        let mut zle = zle_with("0123456789", 4);
        zle.vi_set_mark('m');
        zle.zlecs = 9;
        zle.vi_goto_mark('m'); // jump back; 26th slot now holds 9
        assert_eq!(zle.zlecs, 4);
        zle.vi_goto_mark('\''); // jump to implicit last position
        assert_eq!(zle.zlecs, 9);
    }

    #[test]
    fn vi_set_mark_ignores_invalid_names() {
        let mut zle = zle_with("abc", 1);
        zle.vi_set_mark('A'); // uppercase not allowed
        zle.vi_set_mark('1'); // digit not allowed
        assert!(zle.vi_marks.iter().all(|m| m.is_none()));
    }

    #[test]
    fn vi_undo_reverses_a_recorded_change() {
        let mut zle = zle_with("", 0);
        zle.setlastline();
        zle.zleline = "abc".chars().collect();
        zle.zlell = 3;
        zle.zlecs = 3;
        zle.mkundoent();
        zle.vi_undo();
        assert_eq!(zle.zleline.iter().collect::<String>(), "");
    }

    #[test]
    fn vi_rev_repeat_find_walks_back() {
        let mut zle = zle_with("a-b-c-d", 0);
        zle.vi_last_find_char = Some('-');
        zle.vi_last_find_dir = 1;
        zle.vi_last_find_tail = 0;
        // Forward to first '-' at index 1.
        assert_eq!(zle.vi_find_char_inner(false), 0);
        assert_eq!(zle.zlecs, 1);
        // Forward again to '-' at 3.
        assert_eq!(zle.vi_repeat_find(), 0);
        assert_eq!(zle.zlecs, 3);
        // Reverse repeat — back to index 1.
        assert_eq!(zle.vi_rev_repeat_find(), 0);
        assert_eq!(zle.zlecs, 1);
    }
}
