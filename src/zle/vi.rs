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

    /// Read the next keystroke and treat it as a vi motion to define an
    /// operator range. Returns `Some((start, end, line_mode))` where the
    /// operator should act on `[start, end)`, or `None` if the motion was
    /// unknown / canceled / a no-op.
    ///
    /// Port of `getvirange()` from `Src/Zle/zle_vi.c:172`. The full C
    /// implementation runs the next bound widget under `virangeflag = 1`
    /// using the operator-pending keymap. This Rust port short-circuits by
    /// dispatching a fixed set of common motions inline rather than going
    /// through the keymap — covering the daily-driver subset (`w`/`W`,
    /// `b`/`B`, `e`/`E`, `0`, `^`, `$`, `h`, `l`, `j`, `k`, `f`/`F`/`t`/`T`)
    /// plus the doubled-letter line-mode pattern (`dd`, `cc`, `yy` etc.).
    /// Text objects (`iw`, `aw`, `i"`, `a"`, …) and arbitrary user-bound
    /// motions in the operator-pending map are not yet wired through.
    ///
    /// `op_char` is the operator that triggered the call (`d` / `c` / `y`)
    /// — used to recognise the doubled form for line mode.
    pub fn vi_get_range(&mut self, op_char: char) -> Option<(usize, usize, bool)> {
        let pos = self.zlecs;
        let n = self.vi_get_arg().max(1);
        let motion = self.getfullchar(false)?;

        // Doubled letter (e.g. `dd`, `cc`, `yy`) → entire current line(s).
        // Mirrors the `MOD_LINE` branch of `getvirange()` in zle_vi.c:281
        // but invoked directly when the user repeats the operator letter.
        if motion == op_char {
            let bol = self.find_bol(pos);
            let mut eol = self.find_eol(bol);
            // Extend by `n - 1` more lines forward to honour the count
            // (vi `3dd` deletes 3 lines).
            for _ in 1..n {
                if eol >= self.zlell {
                    break;
                }
                eol = self.find_eol(eol + 1);
            }
            // Include the trailing newline in the range when there is one,
            // so the operator pulls the whole line including its terminator.
            let end = if eol < self.zlell { eol + 1 } else { eol };
            return Some((bol, end, true));
        }

        let other = match motion {
            // Word motions — `w` / `b` / `e` use the WordStyle::Vi class,
            // `W` / `B` / `E` use blank-delimited (matches zsh's WORDFLAG_W
            // distinction between iword and ialnum classes).
            'w' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_end(super::word::WordStyle::Vi);
                    self.zlecs = saved_cs;
                }
                p
            }
            'W' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_end(super::word::WordStyle::BlankDelimited);
                    self.zlecs = saved_cs;
                }
                p
            }
            'b' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_start(super::word::WordStyle::Vi);
                    self.zlecs = saved_cs;
                }
                p
            }
            'B' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_start(super::word::WordStyle::BlankDelimited);
                    self.zlecs = saved_cs;
                }
                p
            }
            'e' => {
                // `e` is end-of-word inclusive; the C path (`viendword`)
                // lands on the last char of the word. For our range it
                // becomes start..=word_end which is start..(word_end+1).
                let saved_cs = self.zlecs;
                self.zlecs = pos;
                let mut p = self.find_word_end(super::word::WordStyle::Vi);
                self.zlecs = saved_cs;
                if p < self.zlell {
                    p += 1;
                }
                p
            }
            'E' => {
                let saved_cs = self.zlecs;
                self.zlecs = pos;
                let mut p = self.find_word_end(super::word::WordStyle::BlankDelimited);
                self.zlecs = saved_cs;
                if p < self.zlell {
                    p += 1;
                }
                p
            }
            // Line-internal motions.
            '0' => self.find_bol(pos),
            '^' => {
                // First non-blank — `vifirstnonblank` in zle_move.c:862.
                let bol = self.find_bol(pos);
                let mut p = bol;
                while p < self.zlell && self.zleline[p].is_whitespace()
                    && self.zleline[p] != '\n'
                {
                    p += 1;
                }
                p
            }
            '$' => self.find_eol(pos),
            'h' => pos.saturating_sub(n as usize),
            'l' => (pos + n as usize).min(self.zlell),
            // Line mode for j/k — extend the range across `n` lines.
            'j' => {
                let mut p = self.find_eol(pos);
                for _ in 0..n {
                    if p >= self.zlell {
                        break;
                    }
                    p = self.find_eol(p + 1);
                }
                let bol = self.find_bol(pos);
                let end = if p < self.zlell { p + 1 } else { p };
                return Some((bol, end, true));
            }
            'k' => {
                let mut bol = self.find_bol(pos);
                for _ in 0..n {
                    if bol == 0 {
                        break;
                    }
                    bol = self.find_bol(bol - 1);
                }
                let eol = self.find_eol(pos);
                let end = if eol < self.zlell { eol + 1 } else { eol };
                return Some((bol, end, true));
            }
            // Find-char motions delegate to vi_find_char_inner which already
            // honours t/T tail-skip and the count via `mult`. We push the
            // motion char as the find-char target.
            'f' | 'F' | 't' | 'T' => {
                let next = self.getfullchar(false)?;
                self.vi_last_find_char = Some(next);
                self.vi_last_find_dir = if motion == 'f' || motion == 't' { 1 } else { -1 };
                self.vi_last_find_tail = match motion {
                    'f' | 'F' => 0,
                    't' => -1,
                    'T' => 1,
                    _ => 0,
                };
                let saved_mult = self.zmod.mult;
                self.zmod.mult = n;
                let ok = self.vi_find_char_inner(false) == 0;
                self.zmod.mult = saved_mult;
                if !ok {
                    return None;
                }
                // For `f`/`t` (forward), include the landed-on char in the
                // range — match C's `if (vfinddir == 1 && virangeflag) INCCS();`
                // at zle_move.c:828.
                let mut p = self.zlecs;
                if (motion == 'f' || motion == 't') && p < self.zlell {
                    p += 1;
                }
                self.zlecs = pos;
                p
            }
            _ => return None,
        };

        if other == pos {
            return None;
        }
        let (start, end) = if other > pos { (pos, other) } else { (other, pos) };
        Some((start, end, false))
    }

    /// Push `n` chars from `start` onto the kill ring (front).
    /// Helper used by the operator ports below — equivalent to C zsh's
    /// `cut(start, n, CUT_RAW)` / `forekill(n, CUT_RAW)` but operating
    /// directly on our `Vec<char>` buffer.
    fn vi_cut_into_killring(&mut self, start: usize, end: usize) {
        if end <= start || end > self.zleline.len() {
            return;
        }
        let killed: Vec<char> = self.zleline[start..end].to_vec();
        self.killring.push_front(killed);
        if self.killring.len() > self.killringmax {
            self.killring.pop_back();
        }
    }

    /// `d{motion}` — vi delete operator.
    /// Port of `videlete()` from `Src/Zle/zle_vi.c:384`.
    pub fn vi_delete_op(&mut self) -> i32 {
        let (start, end, line_mode) = match self.vi_get_range('d') {
            Some(r) => r,
            None => return 1,
        };
        self.vi_cut_into_killring(start, end);
        let drained = end - start;
        self.zleline.drain(start..end);
        self.zlell = self.zleline.len();
        self.zlecs = start.min(self.zlell);
        if line_mode && self.zlell > 0 {
            // C zle_vi.c:392-397 — for line ranges, also pull the trailing
            // \n if the cursor now sits past the buffer end, then jump to
            // the first non-blank of the surviving line.
            self.lastcol = -1;
            let bol = self.find_bol(self.zlecs);
            let mut p = bol;
            while p < self.zlell && self.zleline[p].is_whitespace() && self.zleline[p] != '\n' {
                p += 1;
            }
            self.zlecs = p;
        }
        let _ = drained;
        self.resetneeded = true;
        0
    }

    /// `c{motion}` — vi change operator.
    /// Port of `vichange()` from `Src/Zle/zle_vi.c:438`. After deleting the
    /// range, switches the keymap to insert mode (`startvitext`) — the C
    /// path also sets `viinsbegin = zlecs; vistartchange = undo_changeno`,
    /// which we mirror so a future `.` repeat can replay correctly.
    pub fn vi_change_op(&mut self) -> i32 {
        let (start, end, _) = match self.vi_get_range('c') {
            Some(r) => r,
            None => return 1,
        };
        self.vi_cut_into_killring(start, end);
        self.zleline.drain(start..end);
        self.zlell = self.zleline.len();
        self.zlecs = start.min(self.zlell);
        self.vistartchange = self.undo_changeno;
        self.keymaps.select("main");
        self.resetneeded = true;
        0
    }

    /// `y{motion}` — vi yank operator.
    /// Port of `viyank()` from `Src/Zle/zle_vi.c:507`. Copies the range to
    /// the kill ring without removing it; cursor lands at the start of the
    /// yanked region.
    pub fn vi_yank_op(&mut self) -> i32 {
        let saved_lastcol = self.lastcol;
        let (start, end, line_mode) = match self.vi_get_range('y') {
            Some(r) => r,
            None => return 1,
        };
        self.vi_cut_into_killring(start, end);
        self.zlecs = start;
        if line_mode && saved_lastcol != -1 {
            // zle_vi.c:518-531 — for line yanks, restore the column on the
            // current line (clamped to its end-of-line).
            let eol = self.find_eol(self.zlecs);
            self.zlecs += saved_lastcol as usize;
            if self.zlecs >= eol {
                self.zlecs = eol;
            }
            self.lastcol = -1;
        }
        self.resetneeded = true;
        0
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

    fn feed(zle: &mut Zle, s: &str) {
        // Pre-feed bytes into the unget buffer so getfullchar() returns
        // them without blocking on stdin. Used by the operator tests below
        // to drive vi_get_range's next-keystroke read.
        zle.ungetbytes(s.as_bytes());
    }

    #[test]
    fn vi_get_range_dd_selects_whole_current_line() {
        let mut zle = zle_with("aaa\nbbb\nccc", 4); // cursor on 'b' line
        feed(&mut zle, "d");
        let (s, e, line) = zle.vi_get_range('d').expect("range");
        assert!(line);
        assert_eq!(s, 4);
        assert_eq!(e, 8); // up to and including the trailing '\n'
    }

    #[test]
    fn vi_get_range_dw_selects_to_word_end() {
        let mut zle = zle_with("hello world", 0);
        feed(&mut zle, "w");
        let (s, e, line) = zle.vi_get_range('d').expect("range");
        assert!(!line);
        assert_eq!(s, 0);
        // find_word_end on "hello world" at pos 0 (Vi style) skips through
        // "hello" plus trailing whitespace, landing at 6 ("world" start).
        assert_eq!(e, 6);
    }

    #[test]
    fn vi_get_range_d_dollar_selects_to_eol() {
        let mut zle = zle_with("foo bar baz", 4);
        feed(&mut zle, "$");
        let (s, e, _) = zle.vi_get_range('d').expect("range");
        assert_eq!(s, 4);
        assert_eq!(e, 11);
    }

    #[test]
    fn vi_delete_op_dw_removes_first_word() {
        let mut zle = zle_with("hello world", 0);
        feed(&mut zle, "w");
        assert_eq!(zle.vi_delete_op(), 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "world");
        // Killed text landed on the kill ring.
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("hello ".to_string())
        );
    }

    #[test]
    fn vi_yank_op_y_dollar_copies_without_removing() {
        let mut zle = zle_with("foo bar", 4);
        feed(&mut zle, "$");
        assert_eq!(zle.vi_yank_op(), 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "foo bar");
        assert_eq!(
            zle.killring.front().map(|v| v.iter().collect::<String>()),
            Some("bar".to_string())
        );
        // Cursor lands at start of the yanked range.
        assert_eq!(zle.zlecs, 4);
    }

    #[test]
    fn vi_change_op_cw_removes_word_and_clears_pending_change() {
        let mut zle = zle_with("foo bar", 0);
        feed(&mut zle, "w");
        assert_eq!(zle.vi_change_op(), 0);
        assert_eq!(zle.zleline.iter().collect::<String>(), "bar");
        assert_eq!(zle.zlecs, 0);
        // vistartchange records the change number we entered insert mode at;
        // it should now equal undo_changeno (zero in this fresh zle).
        assert_eq!(zle.vistartchange, zle.undo_changeno);
    }

    #[test]
    fn vi_get_range_unknown_motion_returns_none() {
        let mut zle = zle_with("abc", 0);
        feed(&mut zle, "Z"); // no motion mapped to Z
        assert!(zle.vi_get_range('d').is_none());
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
