//! ZLE vi mode operations
//!
//! Direct port from zsh/Src/Zle/zle_vi.c

use super::zle_main::{ModifierFlags, Zle};

// Note: dead `ViState` / `ViChange` / `ViPendingOp` aggregates
// removed per PORT_PLAN Phase 2. They had zero references across the
// codebase. The actual zsh-side state lives in C file-scope globals
// declared in `Src/Zle/zle_vi.c`:
//
//     int virangeflag;       // line 36
// kludge to get cw and dw to work right                                   // c:38
//     int wordflag;          // line 41
//     int vilinerange;       // line 46
//     struct vichange lastvichg, curvichg;  // line 54
//     int vichgflag;         // line 65
//     int viinrepeat;        // line 73
// point where vi insert mode was last entered                             // c:75
//     int viinsbegin;        // line 78
//
// These are cross-compilation-unit (referenced from textobjects.c
// and zle_move.c), so they belong to PORT_PLAN Phase 3 bucket-2
// (Arc<RwLock>), not the Phase 2 bucket-1 (thread_local!) wave.

impl Zle {
    /// Read the active numeric multiplier.
    /// Port of `zmult` macro at Src/Zle/zle.h:267 (`#define zmult
    /// (zmod.mult)`). Returns the explicit MULT prefix when set,
    /// otherwise 1 — the default-1 fall-through that initmodifier
    /// installs (zle_main.c:1604).
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
    pub fn vi_repeat_find(&mut self) -> i32 {                                // c:835
        self.vi_find_char_inner(true)
    }

    /// `,` — repeat last find in reverse direction.
    /// Port of virevrepeatfind() from Src/Zle/zle_move.c:842.
    pub fn vi_rev_repeat_find(&mut self) -> i32 {                            // c:842
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

    /// Jump to the bracket matching the one under the cursor.
    /// Port of `vimatchbracket()` from Src/Zle/zle_misc.c. Vim's `%`
    /// motion — recognises (), [], {}, <>; walks forward or backward
    /// honouring nesting depth.
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

    /// Enter overwrite mode (vim's `R` command).
    /// Port of `vireplace()` from Src/Zle/zle_vi.c. Switches to the
    /// insert keymap with `insmode = false` so subsequent self-inserts
    /// overwrite existing chars instead of pushing them right.
    pub fn vi_replace_mode(&mut self) {
        self.keymaps.select("viins");
        self.insmode = false; // Overwrite mode
    }

    /// Toggle the case of the character under the cursor and advance.
    /// Port of `viswapcase()` from Src/Zle/zle_vi.c (vim's `~`).
    /// Uppercase letters become lowercase and vice versa; non-letters
    /// pass through untouched. Cursor advances one position post-swap.
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

    /// Vi visual mode (`v` in command mode).
    /// Port of visualmode() from Src/Zle/zle_move.c:516. Toggles
    /// `region_active` between 0 (off), 1 (charwise), and 2 (linewise) per
    /// the C switch: from inactive enters charwise (sets mark first); from
    /// charwise turns off; from linewise switches to charwise.
    pub fn vi_visual_mode(&mut self) {
        match self.region_active {
            1 => {
                self.region_active = 0;
            }
            0 => {
                self.mark = self.zlecs;
                self.region_active = 1;
            }
            2 => {
                self.region_active = 1;
            }
            _ => {}
        }
    }

    /// Vi visual line mode (`V` in command mode).
    /// Port of visuallinemode() from Src/Zle/zle_move.c:540. Same toggle
    /// shape as visualmode but the "active" target is 2 (linewise).
    pub fn vi_visual_line_mode(&mut self) {
        match self.region_active {
            2 => {
                self.region_active = 0;
            }
            0 => {
                self.mark = self.zlecs;
                self.region_active = 2;
            }
            1 => {
                self.region_active = 2;
            }
            _ => {}
        }
    }

    /// Vi visual block mode — Rust-side extension; zsh has no built-in
    /// visual-block widget (not in iwidgets.list). Treat as charwise so the
    /// caller still gets a usable selection.
    /// Reference: zsh has `visualmode` (charwise) and `visuallinemode`
    /// (linewise) only — see Src/Zle/iwidgets.list. This is a behavioural
    /// extension, not a port.
    pub fn vi_visual_block_mode(&mut self) {
        if self.region_active == 0 {
            self.mark = self.zlecs;
        }
        self.region_active = 1;
    }

    /// Deactivate the visual region (`Esc` from visual mode).
    /// Port of deactivateregion() from Src/Zle/zle_move.c:564.
    pub fn vi_deactivate_region(&mut self) {
        self.region_active = 0;
    }

    /// Vi set mark (`m{a-z}` in command mode). Port of visetmark() from
    /// Src/Zle/zle_move.c:872. Stores the current cursor and history line in
    /// the named slot; non-letter names are rejected.
    pub fn vi_set_mark(&mut self, name: char) {
        // Set the historical mark (mirror with self.mark for emacs compat).
        self.mark = self.zlecs;
        if let Some(idx) = viyank(name) {
            self.vi_marks[idx] = Some((self.zlecs, self.history.cursor as i32));
        }
    }

    /// Vi goto mark (`'a` / `` `a `` in command mode). Port of vigotomark()
    /// from zle_move.c:887. ASCII letters jump to the saved location;
    /// `'` / `` ` `` jumps to the implicit "last position" mark; other
    /// characters are rejected.
    pub fn vi_goto_mark(&mut self, name: char) {
        let idx = match viyank(name) {
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

    /// Append `key` to the vi change-replay buffer.
    /// Port of the recording side of `virepeatchange()` machinery from
    /// Src/Zle/zle_vi.c — C zsh tracks this via `vichgflag` + `vichgbuf`
    /// in zle_main.c, capturing every byte fed during a `c` / `d` / `y`
    /// operator, between `startvichange()` and the operator completion.
    /// Callers (the operator entry/exit points) gate when recording is
    /// active; this method just appends. The buffer is consumed by
    /// `widget_vi_repeat_change` via `ungetbytes`.
    pub fn vi_record_change(&mut self, key: u8) {
        self.vi_chg_buf.push(key);
    }

    /// Reset the change-replay buffer to start a fresh recording session.
    /// Mirrors C zsh's `vichgflag = 1; freevichg(); vichgbuf = ...` setup
    /// inside `startvichange()` (zle_vi.c).
    pub fn vi_start_change_recording(&mut self) {
        self.vi_chg_buf.clear();
    }

    /// Replay the last vi change ("." in command mode).
    /// Port of `virepeatchange()` from Src/Zle/zle_vi.c — re-feeds the
    /// recorded `vi_chg_buf` via `ungetbytes` so the next `zlecore`
    /// iteration re-runs the captured operator + motion. With nothing
    /// recorded yet (operator entry/exit don't gate `vi_record_change`
    /// in this build), the buffer is empty and replay is a no-op,
    /// matching zsh's behaviour pre-first-change.
    pub fn vi_repeat_change(&mut self) {
        if self.vi_chg_buf.is_empty() {
            return;
        }
        let bytes = self.vi_chg_buf.clone();
        self.ungetbytes(&bytes);
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
                    p = self.find_word_end(super::widget::WordStyle::Vi);
                    self.zlecs = saved_cs;
                }
                p
            }
            'W' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_end(super::widget::WordStyle::BlankDelimited);
                    self.zlecs = saved_cs;
                }
                p
            }
            'b' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_start(super::widget::WordStyle::Vi);
                    self.zlecs = saved_cs;
                }
                p
            }
            'B' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = self.zlecs;
                    self.zlecs = p;
                    p = self.find_word_start(super::widget::WordStyle::BlankDelimited);
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
                let mut p = self.find_word_end(super::widget::WordStyle::Vi);
                self.zlecs = saved_cs;
                if p < self.zlell {
                    p += 1;
                }
                p
            }
            'E' => {
                let saved_cs = self.zlecs;
                self.zlecs = pos;
                let mut p = self.find_word_end(super::widget::WordStyle::BlankDelimited);
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
fn viyank(name: char) -> Option<usize> {
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
    fn vi_visual_mode_toggles_charwise() {
        let mut zle = zle_with("abcd", 2);
        assert_eq!(zle.region_active, 0);
        zle.vi_visual_mode();
        assert_eq!(zle.region_active, 1);
        assert_eq!(zle.mark, 2);
        zle.vi_visual_mode();
        assert_eq!(zle.region_active, 0);
    }

    #[test]
    fn vi_visual_line_mode_toggles_linewise_and_swaps_with_charwise() {
        let mut zle = zle_with("abcd", 0);
        zle.vi_visual_line_mode();
        assert_eq!(zle.region_active, 2);
        // In linewise → charwise via vi_visual_mode().
        zle.vi_visual_mode();
        assert_eq!(zle.region_active, 1);
        // Charwise → linewise via vi_visual_line_mode().
        zle.vi_visual_line_mode();
        assert_eq!(zle.region_active, 2);
        // Linewise → off via vi_visual_line_mode().
        zle.vi_visual_line_mode();
        assert_eq!(zle.region_active, 0);
    }

    #[test]
    fn vi_deactivate_region_clears_active_state() {
        let mut zle = zle_with("abcd", 0);
        zle.region_active = 2;
        zle.vi_deactivate_region();
        assert_eq!(zle.region_active, 0);
    }

    #[test]
    fn vi_record_change_appends_to_replay_buffer() {
        let mut zle = zle_with("", 0);
        zle.vi_start_change_recording();
        zle.vi_record_change(b'd');
        zle.vi_record_change(b'w');
        assert_eq!(zle.vi_chg_buf, vec![b'd', b'w']);
        zle.vi_start_change_recording();
        assert!(zle.vi_chg_buf.is_empty());
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

/// Port of `dovilinerange()` from Src/Zle/zle_vi.c:302.
pub fn dovilinerange(zle: &mut crate::ported::zle::zle_main::Zle) -> (usize, usize) {  // c:302
    // C body (c:304-333): expands the current vi range to whole lines
    //                    (includes leading/trailing newlines). Returns
    //                    a [start, end) byte pair.
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    // Include the trailing newline if present.
    let end = if eol < zle.zlell { eol + 1 } else { eol };
    (bol, end)
}

/// Port of `getvirange()` from Src/Zle/zle_vi.c:172.
pub fn getvirange(zle: &mut crate::ported::zle::zle_main::Zle, _wf: i32) -> i32 {  // c:172
    // C body (c:172-300): drives the vi-range read by interpreting
    //                    a follow-up keystroke (motion command),
    //                    invoking it with virangeflag set, and
    //                    returning the resulting cursor position.
    // Substrate (virangeflag + execzlefunc dispatch + motion read)
    // deferred. Returns the current cursor as a no-motion fallback.
    zle.zlecs as i32
}

/// Port of `startvichange()` from Src/Zle/zle_vi.c:90.
pub fn startvichange(zle: &mut crate::ported::zle::zle_main::Zle, im: i32) {  // c:88
    // C body (c:90-113): `if (im > -1) insmode = im; if (viinrepeat
    //                    && im != -2) { zmod = lastvichg.mod;
    //                    vichgflag = 0; } else if (!vichgflag) {
    //                    curvichg.buf = ... }`. Substrate (viinrepeat,
    // lastvichg, vichgflag, curvichg) deferred. Faithful: insmode set.
    if im > -1 {
        zle.insmode = im != 0;
    }
}

/// Direct port of `static void startvitext(int im)` from
/// `Src/Zle/zle_vi.c:118-124`.
/// ```c
/// startvitext(int im) {
///     startvichange(im);
///     selectkeymap("main", 1);
///     vistartchange = undo_changeno;
///     viinsbegin = zlecs;
/// }
/// ```
pub fn startvitext(zle: &mut crate::ported::zle::zle_main::Zle, im: i32) {   // c:118
    startvichange(zle, im);                                                  // c:120
    crate::ported::zle::zle_keymap::selectkeymap("main", 1);                 // c:121
    zle.vistartchange = zle.undo_changeno;                                   // c:122
    zle.viinsbegin    = zle.zlecs;                                           // c:123
}

/// Port of `viaddeol()` from Src/Zle/zle_vi.c:346.
pub fn viaddeol(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:345
    // C body (c:347-350): `zlecs = findeol(); startvitext(1); return 0`.
    zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
    startvitext(zle, 1);
    0
}

/// Port of `viaddnext()` from Src/Zle/zle_vi.c:336.
pub fn viaddnext(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:335
    // C body (c:337-341): `if (zlecs != findeol()) INCCS();
    //                     startvitext(1); return 0`.
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    if zle.zlecs != eol {
        crate::ported::zle::zle_move::inccs(zle);
    }
    startvitext(zle, 1);
    0
}

/// Port of `vibackwarddeletechar()` from Src/Zle/zle_vi.c:888.
pub fn vibackwarddeletechar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:888
    // C body (c:892-911): `startvichange(-1); if (zmult < 0) {...
    //                     deletechar(args)... } if (zlecs == bol)
    //                     return 1; backdel(...)`. Without zmult<0 path
    // we approximate: startvichange + backwarddeletechar.
    startvichange(zle, -1);
    crate::ported::zle::zle_misc::backwarddeletechar(zle)
}

/// Direct port of `int vicapslockpanic(char **args)` from
/// `Src/Zle/zle_vi.c:1002-1014`.
/// ```c
/// int vicapslockpanic(char **args) {
///     clearlist = 1;
///     zbeep();
///     statusline = "press a lowercase key to continue";
///     zrefresh();
///     while (!ZC_ilower(getfullchar(0))) ;
///     statusline = NULL;
///     return 0;
/// }
/// ```
pub fn vicapslockpanic() -> i32 {                                            // c:1002
    use std::sync::atomic::Ordering;
    // c:1004 — clearlist = 1.
    crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed);
    // c:1005 — zbeep().
    crate::ported::utils::zbeep();
    // c:1006 — statusline = "press a lowercase key to continue".
    // The statusline lives on the active Zle struct; without a handle
    // we mirror to the paramtab so the prompt drawer picks it up.
    let _ = crate::ported::params::setsparam(
        "STATUSLINE", "press a lowercase key to continue",
    );
    // c:1007 — zrefresh() — flushes paramtab/buffer state to the
    // refresh layer; deferred to live ZLE draw. The CLEARLIST flag is
    // the trigger the draw path watches.
    // c:1008-1009 — `while (!ZC_ilower(getfullchar(0))) ;`.
    // Without a live key-read loop we cannot block here; the live
    // ZLE input path (Zle::getfullchar) does the wait. The flag
    // state above triggers the correct draw, and the live read
    // continues normally.
    // c:1010 — clear statusline.
    let _ = crate::ported::params::setsparam("STATUSLINE", "");
    0                                                                        // c:1011
}

/// Port of `vichange()` from Src/Zle/zle_vi.c:438.
pub fn vichange(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:438
    // C body (c:440-453): `startvichange(1); if ((c2 = getvirange(0))
    //                     != -1) { forekill(c2-zlecs, CUT_RAW); ret = 0;
    //                     startvitext(1); }`. Without getvirange, fall
    //                     through to startvitext.
    startvichange(zle, 1);
    startvitext(zle, 1);
    0
}

/// Port of `vichangeeol()` from Src/Zle/zle_vi.c:482.
pub fn vichangeeol(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {     // c:481
    // C body (c:483-498): `if (region_active) { regionlines(...);
    //                     zlecs = a; region_active = 0; ... } else
    //                     forekill(findeol() - zlecs, CUT_RAW);
    //                     startvitext(1); return 0`.
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    if eol > zle.zlecs {
        let text: Vec<char> = zle.zleline.drain(zle.zlecs..eol).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= eol - zle.zlecs;
    }
    startvitext(zle, 1);
    0
}

/// Port of `vichangewholeline()` from Src/Zle/zle_vi.c:499.
pub fn vichangewholeline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:498
    // C body (c:500-503): `vifirstnonblank(args); return vichangeeol(...)`.
    crate::ported::zle::zle_move::vifirstnonblank(zle);
    vichangeeol(zle)
}

/// Port of `vicmdmode()` from Src/Zle/zle_vi.c:677.
pub fn vicmdmode(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:676
    // C body (c:678-694): `if (invicmdmode() || selectkeymap("vicmd",
    //                     0)) return 1; mergeundo(); insmode = unset(
    //                     OVERSTRIKE); ...; if (zlecs != findbol())
    //                     DECCS()`.
    if zle.keymaps.current_name == "vicmd" {
        return 1;
    }
    if crate::ported::zle::zle_keymap::selectkeymap("vicmd", 0) != 0 {
        return 1;
    }
    zle.keymaps.current_name = "vicmd".to_string();
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    if zle.zlecs != bol {
        crate::ported::zle::zle_move::deccs(zle);
    }
    0
}

/// Port of `videlete()` from Src/Zle/zle_vi.c:384.
pub fn videlete(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:383
    // C body (c:385-400): `startvichange(1); if ((c2 = getvirange(0))
    //                     != -1) { forekill(c2 - zlecs, CUT_RAW); ret = 0;
    //                     ... } return ret`. Without getvirange we
    //                     can't determine the range; approximate by
    //                     using current cursor as no-op range.
    startvichange(zle, 1);
    1                                                                        // c:386 ret = 1
}

/// Port of `videletechar()` from Src/Zle/zle_vi.c:405.
pub fn videletechar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {    // c:404
    // C body (c:406-433): `startvichange(-1); n = zmult; ... if (zlecs
    //                     == zlell || zleline[zlecs] == '\\n') return 1;
    //                     forekill(n, ...)`. Approximation: startvichange
    //                     + deletechar with EOL check.
    startvichange(zle, -1);
    if zle.zlecs == zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n') {
        return 1;                                                            // c:421-422
    }
    crate::ported::zle::zle_misc::deletechar(zle)
}

/// Port of `vidigitorbeginningofline()` from Src/Zle/zle_vi.c:1129.
pub fn vidigitorbeginningofline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:vidigitorbeginningofline
    // C body: `if (zmod.flags & MOD_TMULT) return digitargument(args);
    //          else { removesuffix(); invalidatelist();
    //                 return vibeginningofline(args); }`.
    use crate::ported::zle::zle_main::ModifierFlags;
    if zle.zmod.flags.contains(ModifierFlags::TMULT) {
        return crate::ported::zle::zle_misc::digitargument(zle);
    }
    crate::ported::zle::zle_move::vibeginningofline(zle)
}

/// Port of `vidowncase()` from Src/Zle/zle_vi.c:773.
pub fn vidowncase(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:773
    // C body (c:775-794): startvichange(1); if ((c2 = getvirange(0))
    //                    != -1) { lowercase all letters in [zlecs, c2);
    //                    return 0; } else return 1.
    // Without getvirange we use [zlecs, eol) as the implicit range.
    startvichange(zle, 1);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    for i in zle.zlecs..eol {
        zle.zleline[i] = zle.zleline[i].to_ascii_lowercase();
    }
    zle.resetneeded = true;
    0
}

/// Port of `vigetkey()` from Src/Zle/zle_vi.c:128.
pub fn vigetkey(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:127
    // C body (c:128-170): `mn = openkeymap("main"); ... if (getbyte(0,
    //                    NULL, 1) == EOF) return ZLEEOF; ... resolve
    //                    Thingy via main keymap; if self-insert, return
    //                    the byte; else return -1`.
    // Without getbyte interactive read, drain unget_buf; -1 if empty.
    if let Some(b) = zle.unget_buf.pop_front() {
        b as i32
    } else {
        -1                                                                   // c:138 ZLEEOF
    }
}

/// Port of `viindent()` from Src/Zle/zle_vi.c:820.
pub fn viindent(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:820
    // C body (c:822-855): startvichange(1); insert SHIFTWIDTH spaces
    //                    at start of each line in range. Default
    //                    SHIFTWIDTH = 4 (per zsh's iwidgets.list).
    startvichange(zle, 1);
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    for _ in 0..4 {
        zle.zleline.insert(bol, ' ');
        zle.zlell += 1;
    }
    if zle.zlecs >= bol {
        zle.zlecs += 4;
    }
    zle.resetneeded = true;
    0
}

/// Port of `viinsert()` from Src/Zle/zle_vi.c:355.
pub fn viinsert(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:354
    // C body (c:356-358): `startvitext(1); return 0`.
    startvitext(zle, 1);
    0
}

/// Port of `viinsert_init()` from Src/Zle/zle_vi.c:368.
pub fn viinsert_init(zle: &mut crate::ported::zle::zle_main::Zle) {          // c:367
    // C body (c:369-371): `startvitext(-2)`. Special init flag for
    // first-time vi insert mode entry from zle session start.
    startvitext(zle, -2);
}

/// Port of `viinsertbol()` from Src/Zle/zle_vi.c:375.
pub fn viinsertbol(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {     // c:374
    // C body (c:376-379): `vifirstnonblank(zlenoargs); startvitext(1);
    //                     return 0`.
    crate::ported::zle::zle_move::vifirstnonblank(zle);
    startvitext(zle, 1);
    0
}

/// Port of `vijoin()` from Src/Zle/zle_vi.c:933.
pub fn vijoin(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {          // c:vijoin
    // C body: replace next '\\n' with ' ', skipping leading whitespace
    //         on the joined line. Repeat zmult times.
    startvichange(zle, -1);
    let n = zle.zmod.mult.max(1);
    for _ in 0..n {
        let eol = crate::ported::zle::zle_utils::findeol(zle);
        if eol >= zle.zlell || zle.zleline.get(eol) != Some(&'\n') {
            return 1;
        }
        zle.zleline[eol] = ' ';
        // Strip leading whitespace on the joined-in line.
        let mut p = eol + 1;
        while p < zle.zleline.len() && zle.zleline[p].is_whitespace() {
            zle.zleline.remove(p);
            zle.zlell -= 1;
        }
        let _ = p;
        zle.zlecs = eol;
    }
    zle.resetneeded = true;
    0
}

/// Port of `vikilleol()` from Src/Zle/zle_vi.c:1056.
pub fn vikilleol(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:vikilleol
    // C body: kill from cursor to eol; start vi cmd-mode change.
    startvichange(zle, 1);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    if eol > zle.zlecs {
        let text: Vec<char> = zle.zleline.drain(zle.zlecs..eol).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= eol - zle.zlecs;
    }
    zle.resetneeded = true;
    0
}

/// Port of `vikillline()` from Src/Zle/zle_vi.c:923.
pub fn vikillline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:vikillline
    // C body: kill from cursor back to bol.
    startvichange(zle, 1);
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    if zle.zlecs > bol {
        let text: Vec<char> = zle.zleline.drain(bol..zle.zlecs).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= zle.zlecs - bol;
        zle.zlecs = bol;
    }
    zle.resetneeded = true;
    0
}

/// Port of `viopenlineabove()` from Src/Zle/zle_vi.c:711.
pub fn viopenlineabove(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:710
    // C body (c:712-718): `zlecs = findbol(); spaceinline(1);
    //                     zleline[zlecs] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    zle.zlecs = crate::ported::zle::zle_utils::findbol(zle);
    zle.zleline.insert(zle.zlecs, '\n');
    zle.zlell += 1;
    startvitext(zle, 1);
    zle.resetneeded = true;
    0
}

/// Port of `viopenlinebelow()` from Src/Zle/zle_vi.c:699.
pub fn viopenlinebelow(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:698
    // C body (c:700-707): `zlecs = findeol(); spaceinline(1);
    //                     zleline[zlecs++] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    zle.zlecs = crate::ported::zle::zle_utils::findeol(zle);
    zle.zleline.insert(zle.zlecs, '\n');
    zle.zlecs += 1;
    zle.zlell += 1;
    startvitext(zle, 1);
    zle.resetneeded = true;
    0
}

/// Port of `vioperswapcase()` from Src/Zle/zle_vi.c:723.
pub fn vioperswapcase(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:723
    // C body (c:725-746): startvichange(1); if (getvirange(0) != -1)
    //                    swap case in range. Without getvirange, use
    //                    [zlecs, eol) as implicit range.
    startvichange(zle, 1);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    let oldcs = zle.zlecs;
    while zle.zlecs < eol {
        let c = zle.zleline[zle.zlecs];
        zle.zleline[zle.zlecs] = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        zle.zlecs += 1;
    }
    zle.zlecs = oldcs;
    zle.resetneeded = true;
    0
}

/// Port of `vipoundinsert()` from Src/Zle/zle_vi.c:1072.
pub fn vipoundinsert(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {   // c:vipoundinsert
    // C body: same as poundinsert (toggle # comment) but in vi cmdmode.
    crate::ported::zle::zle_misc::poundinsert(zle)
}

/// Port of `viquotedinsert()` from Src/Zle/zle_vi.c:1099.
pub fn viquotedinsert(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:viquotedinsert
    // C body: same as quotedinsert with vi insmode setup.
    startvichange(zle, -1);
    crate::ported::zle::zle_misc::quotedinsert(zle)
}

/// Port of `virepeatchange()` from Src/Zle/zle_vi.c:795.
pub fn virepeatchange() -> i32 {                                             // c:795
    // C body: replays lastvichg.buf via ungetbytes; sets viinrepeat
    //         flag. Substrate (lastvichg + viinrepeat + ungetbytes
    //         interaction with execzlefunc) deferred. Returns 1 to
    //         signal "no change to repeat" until lastvichg is wired.
    1
}

/// Port of `vireplace()` from Src/Zle/zle_vi.c:574.
pub fn vireplace(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:573
    // C body (c:575-577): `startvitext(0); return 0`. Enter overwrite-
    // style insert mode (insmode=0).
    startvitext(zle, 0);
    0
}

/// Port of `vireplacechars()` from Src/Zle/zle_vi.c:594.
pub fn vireplacechars(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:594
    // C body (c:596-675): read one char (vigetkey), replace next zmult
    //                    chars with it (clamped to eol). Without
    //                    vigetkey reading from terminal, use lastchar
    //                    as the replacement source.
    startvichange(zle, 1);
    let n = zle.zmod.mult.max(1) as usize;
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    let avail = eol.saturating_sub(zle.zlecs);
    if n > avail {
        return 1;                                                            // not enough chars
    }
    if let Some(c) = char::from_u32(zle.lastchar as u32) {
        for i in 0..n {
            zle.zleline[zle.zlecs + i] = c;
        }
        zle.resetneeded = true;
    }
    0
}

/// Port of `visetbuffer()` from Src/Zle/zle_vi.c:1015.
pub fn visetbuffer(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {     // c:visetbuffer
    // C body: read one char as the vi buffer name (a-z or 1-9 or '"');
    //         set zmod.vibuf for the next yank/cut. Without vigetkey
    //         interactive read, use lastchar.
    use crate::ported::zle::zle_main::ModifierFlags;
    let c = (zle.lastchar & 0xff) as u8;
    let idx: i32 = if c.is_ascii_digit() {
        (c - b'0') as i32 + 26
    } else if c.is_ascii_lowercase() {
        (c - b'a') as i32
    } else if c.is_ascii_uppercase() {
        // uppercase = append to register
        zle.zmod.flags.insert(ModifierFlags::VIAPP);
        (c - b'A') as i32
    } else {
        return 1;
    };
    zle.zmod.vibuf = idx;
    zle.zmod.flags.insert(ModifierFlags::VIBUF);
    zle.prefixflag = true;
    0
}

/// Port of `visubstitute()` from Src/Zle/zle_vi.c:455.
pub fn visubstitute(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {    // c:455
    // C body (c:457-475): startvichange(1); n=zmult; if(n<0) return 1;
    //                    error if at eol; forekill(n, CUT_RAW);
    //                    startvitext(1); return 0.
    startvichange(zle, 1);
    let n = zle.zmod.mult;
    if n < 0 {
        return 1;
    }
    if zle.zlecs == zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n') {
        return 1;
    }
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    let count = (n as usize).min(eol - zle.zlecs);
    if count > 0 {
        let text: Vec<char> = zle.zleline.drain(zle.zlecs..zle.zlecs + count).collect();
        zle.killring.push_front(text);
        if zle.killring.len() > zle.killringmax {
            zle.killring.pop_back();
        }
        zle.zlell -= count;
    }
    startvitext(zle, 1);
    0
}

/// Port of `viswapcase()` from Src/Zle/zle_vi.c:977.
pub fn viswapcase(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:viswapcase
    // C body: walk zmult chars, swap case of each; advance cursor.
    startvichange(zle, -1);
    let n = zle.zmod.mult;
    if n < 1 {
        return 1;
    }
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    for _ in 0..n {
        if zle.zlecs >= eol {
            break;
        }
        let c = zle.zleline[zle.zlecs];
        let swapped = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        zle.zleline[zle.zlecs] = swapped;
        zle.zlecs += 1;
    }
    zle.resetneeded = true;
    0
}

/// Port of `viunindent()` from Src/Zle/zle_vi.c:856.
pub fn viunindent(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:856
    // C body: remove up to SHIFTWIDTH (4) leading spaces from each
    //         line in range.
    startvichange(zle, 1);
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    let mut removed = 0;
    while removed < 4 && bol < zle.zleline.len() && zle.zleline[bol] == ' ' {
        zle.zleline.remove(bol);
        zle.zlell -= 1;
        removed += 1;
    }
    if zle.zlecs >= bol + removed {
        zle.zlecs -= removed;
    }
    zle.resetneeded = true;
    0
}

/// Port of `viupcase()` from Src/Zle/zle_vi.c:751.
pub fn viupcase(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {        // c:751
    // C body (c:753-771): same as vidowncase but uppercase.
    startvichange(zle, 1);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    for i in zle.zlecs..eol {
        zle.zleline[i] = zle.zleline[i].to_ascii_uppercase();
    }
    zle.resetneeded = true;
    0
}

/// Port of `viyankeol()` from Src/Zle/zle_vi.c:537.
pub fn viyankeol(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:537
    // C body (c:539-547): `x = findeol(); startvichange(-1); if (x ==
    //                     zlecs) return 1; cut(zlecs, x-zlecs, CUT_YANK);
    //                     return 0`.
    let x = crate::ported::zle::zle_utils::findeol(zle);
    startvichange(zle, -1);
    if x == zle.zlecs {
        return 1;                                                            // c:543
    }
    let text: Vec<char> = zle.zleline[zle.zlecs..x].to_vec();
    zle.killring.push_front(text);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    0                                                                        // c:546
}

/// Port of `viyankwholeline()` from Src/Zle/zle_vi.c:550.
pub fn viyankwholeline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:550
    // C body (c:553-572): `bol = findbol(); startvichange(-1); n = zmult;
    //                     if (n < 1) return 1; for (i=n; i--; ) zlecs =
    //                     findeol() + 1; if (zlecs > zlell) zlecs = zlell;
    //                     cut(bol, zlecs - bol, CUT_YANK); zlecs = bol +
    //                     oldcs - bol; return 0`.
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    let oldcs = zle.zlecs;
    startvichange(zle, -1);
    let n = zle.zmod.mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        zle.zlecs = crate::ported::zle::zle_utils::findeol(zle) + 1;
        if zle.zlecs > zle.zlell {
            zle.zlecs = zle.zlell;
        }
    }
    let end = zle.zlecs;
    let text: Vec<char> = zle.zleline[bol..end].to_vec();
    zle.killring.push_front(text);
    if zle.killring.len() > zle.killringmax {
        zle.killring.pop_back();
    }
    zle.zlecs = oldcs;
    0
}
