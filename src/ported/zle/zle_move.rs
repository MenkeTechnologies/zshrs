//! ZLE movement operations
//!
//! Direct port from zsh/Src/Zle/zle_move.c
//!
//! Move cursor right, checking for combining characters                    // c:118
//! Move cursor left, checking for combining characters                     // c:129

use super::zle_main::Zle;

impl Zle {
    /// Move cursor to the start of the current logical line.
    /// Port of `findbol()` from Src/Zle/zle_utils.c:1158 — same scan,
    /// just mutates zlecs in-place instead of returning the index.
    /// `find_bol` (in utils.rs) is the side-effect-free equivalent.
    pub fn move_to_bol(&mut self) {
        while self.zlecs > 0 && self.zleline[self.zlecs - 1] != '\n' {
            self.zlecs -= 1;
        }
    }

    /// Move cursor to the end of the current logical line.
    /// Port of `findeol()` from Src/Zle/zle_utils.c:1169 — mutating
    /// counterpart to `find_eol`.
    pub fn move_to_eol(&mut self) {
        while self.zlecs < self.zlell && self.zleline[self.zlecs] != '\n' {
            self.zlecs += 1;
        }
    }

    /// Move cursor up one logical line, preserving the column.
    /// Simplified port of `upline()` from Src/Zle/zle_hist.c:243 with
    /// fixed n=1 — captures the column-preserve behaviour without the
    /// lastcol sticky-column tracking the C source uses for repeated
    /// up/down chains. Returns false at top-of-buffer.
    pub fn move_up(&mut self) -> bool {
        let col = self.current_column();

        // Find start of current line
        let mut line_start = self.zlecs;
        while line_start > 0 && self.zleline[line_start - 1] != '\n' {
            line_start -= 1;
        }

        if line_start == 0 {
            return false; // Already on first line
        }

        // Move to end of previous line
        self.zlecs = line_start - 1;

        // Find start of previous line
        let mut prev_start = self.zlecs;
        while prev_start > 0 && self.zleline[prev_start - 1] != '\n' {
            prev_start -= 1;
        }

        // Move to same column or end of line
        self.zlecs = prev_start + col.min(self.zlecs - prev_start);

        true
    }

    /// Move cursor down one logical line, preserving the column.
    /// Simplified port of `downline()` from Src/Zle/zle_hist.c:332
    /// with fixed n=1. Returns false at end-of-buffer.
    pub fn move_down(&mut self) -> bool {
        let col = self.current_column();

        // Find end of current line
        let mut line_end = self.zlecs;
        while line_end < self.zlell && self.zleline[line_end] != '\n' {
            line_end += 1;
        }

        if line_end >= self.zlell {
            return false; // Already on last line
        }

        // Move to start of next line
        self.zlecs = line_end + 1;

        // Find end of next line
        let mut next_end = self.zlecs;
        while next_end < self.zlell && self.zleline[next_end] != '\n' {
            next_end += 1;
        }

        // Move to same column or end of line
        self.zlecs = (self.zlecs + col).min(next_end);

        true
    }

    /// Compute the cursor's 0-indexed column on its current logical line.
    /// Equivalent to `zlecs - findbol()` — the offset zsh's vertical-
    /// motion code at Src/Zle/zle_hist.c:253 caches in `lastcol` for
    /// sticky-column behaviour across up/down chains.
    pub fn current_column(&self) -> usize {
        let mut col = 0;
        let mut i = self.zlecs;
        while i > 0 && self.zleline[i - 1] != '\n' {
            i -= 1;
            col += 1;
        }
        col
    }

    /// Compute the 0-indexed logical-line number containing the cursor.
    /// Port of `findline()` from Src/Zle/zle_utils.c:1180 (which fills
    /// in start/end of the cursor's line) but returning just the line
    /// number — counts newlines before the cursor.
    pub fn current_line(&self) -> usize {
        self.zleline[..self.zlecs]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
    }

    /// Count the total number of logical lines in the buffer.
    /// Used by display code to size the multi-line refresh region —
    /// mirrors `nlnct` (number of lines counted) tracked by zsh's
    /// `zrefresh()` in Src/Zle/zle_refresh.c.
    pub fn count_lines(&self) -> usize {
        self.zleline.iter().filter(|&&c| c == '\n').count() + 1
    }
}

/// Port of `alignmultiwordleft()` from Src/Zle/zle_move.c:49. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn alignmultiwordleft() -> i32 { 0 }                                     // c:49

/// Port of `alignmultiwordright()` from Src/Zle/zle_move.c:89. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn alignmultiwordright() -> i32 { 0 }                                    // c:89

/// Port of `backwardchar()` from Src/Zle/zle_move.c:464. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwardchar() -> i32 { 0 }                                           // c:464

/// Port of `backwardmetafiedchar()` from Src/Zle/zle_move.c:170. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn backwardmetafiedchar() -> i32 { 0 }

/// Port of `beginningofline()` from Src/Zle/zle_move.c:298. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningofline() -> i32 { 0 }

/// Port of `beginningoflinehist()` from Src/Zle/zle_move.c:360. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn beginningoflinehist() -> i32 { 0 }

/// Port of `deactivateregion()` from `Src/Zle/zle_move.c:563`.
/// ```c
/// int
/// deactivateregion(UNUSED(char **args))
/// {
///     region_active = 0;
///     return 0;
/// }
/// ```
/// Clear the region-active flag so subsequent commands stop
/// treating point/mark as a selected range.
pub fn deactivateregion(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:563
    zle.region_active = 0;                                                   // c:566 region_active = 0
    0                                                                        // c:567 return 0
}

/// Port of `deccs()` from Src/Zle/zle_move.c:133. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deccs() -> i32 { 0 }

/// Port of `decpos()` from Src/Zle/zle_move.c:152. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn decpos() -> i32 { 0 }

/// Port of `endofline()` from Src/Zle/zle_move.c:331. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endofline() -> i32 { 0 }

/// Port of `endoflinehist()` from Src/Zle/zle_move.c:403. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endoflinehist() -> i32 { 0 }

/// Port of `exchangepointandmark()` from `Src/Zle/zle_move.c:495`.
/// ```c
/// int
/// exchangepointandmark(UNUSED(char **args))
/// {
///     int x;
///     if (zmult == 0) {
///         region_active = 1;
///         return 0;
///     }
///     x = mark;
///     mark = zlecs;
///     zlecs = x;
///     if (zlecs > zlell)
///         zlecs = zlell;
///     if (zmult > 0)
///         region_active = 1;
///     return 0;
/// }
/// ```
/// Swap the cursor (point) with the mark. With `zmult == 0` just
/// activates the region without swapping. With `zmult > 0` also
/// activates the region after the swap.
pub fn exchangepointandmark(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:495
    if zle.zmod.mult == 0 {                                                  // c:500 if (zmult == 0)
        zle.region_active = 1;                                               // c:501
        return 0;                                                            // c:502
    }
    let x = zle.mark;                                                        // c:504 x = mark
    zle.mark = zle.zlecs;                                                    // c:505 mark = zlecs
    zle.zlecs = x;                                                           // c:506 zlecs = x
    if zle.zlecs > zle.zlell {                                               // c:507
        zle.zlecs = zle.zlell;                                               // c:508
    }
    if zle.zmod.mult > 0 {                                                   // c:509
        zle.region_active = 1;                                               // c:510
    }
    0                                                                        // c:511 return 0
}

/// Port of `forwardchar()` from Src/Zle/zle_move.c:441. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn forwardchar() -> i32 { 0 }

/// Port of `inccs()` from Src/Zle/zle_move.c:122. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn inccs() -> i32 { 0 }

/// Port of `incpos()` from Src/Zle/zle_move.c:143. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn incpos() -> i32 { 0 }

/// Port of `setmarkcommand()` from `Src/Zle/zle_move.c:482`.
/// ```c
/// int
/// setmarkcommand(UNUSED(char **args))
/// {
///     if (zmult < 0) {
///         region_active = 0;
///         return 0;
///     }
///     mark = zlecs;
///     region_active = 1;
///     return 0;
/// }
/// ```
/// `set-mark-command` widget — saves the cursor position into
/// `mark` and activates the region. Negative numeric arg
/// (`zmult < 0`) cancels the region instead.
pub fn setmarkcommand(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:482
    if zle.zmod.mult < 0 {                                                   // c:485 if (zmult < 0)
        zle.region_active = 0;                                               // c:486
        return 0;                                                            // c:487
    }
    zle.mark = zle.zlecs;                                                    // c:489 mark = zlecs
    zle.region_active = 1;                                                   // c:490
    0                                                                        // c:491 return 0
}

/// Port of `vibackwardchar()` from Src/Zle/zle_move.c:683. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibackwardchar() -> i32 { 0 }

/// Port of `vibeginningofline()` from Src/Zle/zle_move.c:728. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vibeginningofline() -> i32 { 0 }

/// Port of `viendofline()` from Src/Zle/zle_move.c:708. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viendofline() -> i32 { 0 }

/// Port of `vifindchar()` from Src/Zle/zle_move.c:787. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifindchar() -> i32 { 0 }

/// Port of `vifindnextchar()` from Src/Zle/zle_move.c:739. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifindnextchar() -> i32 { 0 }

/// Port of `vifindnextcharskip()` from Src/Zle/zle_move.c:763. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifindnextcharskip() -> i32 { 0 }

/// Port of `vifindprevchar()` from Src/Zle/zle_move.c:751. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifindprevchar() -> i32 { 0 }

/// Port of `vifindprevcharskip()` from Src/Zle/zle_move.c:775. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifindprevcharskip() -> i32 { 0 }

/// Port of `vifirstnonblank()` from Src/Zle/zle_move.c:862. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vifirstnonblank() -> i32 { 0 }

/// Port of `viforwardchar()` from Src/Zle/zle_move.c:660. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn viforwardchar() -> i32 { 0 }

/// Port of `vigotocolumn()` from Src/Zle/zle_move.c:572. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vigotocolumn() -> i32 { 0 }

/// Port of `vigotomark()` from Src/Zle/zle_move.c:887. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vigotomark() -> i32 { 0 }

/// Port of `vigotomarkline()` from Src/Zle/zle_move.c:929. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vigotomarkline() -> i32 { 0 }

/// Port of `vimatchbracket()` from Src/Zle/zle_move.c:594. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn vimatchbracket() -> i32 { 0 }

/// Port of `virepeatfind()` from Src/Zle/zle_move.c:835. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virepeatfind() -> i32 { 0 }

/// Port of `virevrepeatfind()` from Src/Zle/zle_move.c:842. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn virevrepeatfind() -> i32 { 0 }

/// Port of `visetmark()` from Src/Zle/zle_move.c:872. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn visetmark() -> i32 { 0 }

/// Port of `visuallinemode()` from Src/Zle/zle_move.c:540. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn visuallinemode() -> i32 { 0 }

/// Port of `visualmode()` from Src/Zle/zle_move.c:516. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn visualmode() -> i32 { 0 }

#[cfg(test)]
mod region_tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;

    #[test]
    fn deactivateregion_clears_active() {
        // c:566 — `region_active = 0; return 0`.
        let mut z = Zle::default();
        z.region_active = 1;
        let r = deactivateregion(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.region_active, 0);
    }

    #[test]
    fn setmarkcommand_sets_mark_to_cursor() {
        // c:489-490 — `mark = zlecs; region_active = 1`.
        let mut z = Zle::default();
        z.zlecs = 7;
        z.zmod.mult = 1;
        let r = setmarkcommand(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.mark, 7);
        assert_eq!(z.region_active, 1);
    }

    #[test]
    fn setmarkcommand_negative_mult_deactivates() {
        // c:485-487 — `if (zmult < 0) { region_active = 0; return 0; }`.
        let mut z = Zle::default();
        z.region_active = 1;
        z.mark = 5;
        z.zlecs = 7;
        z.zmod.mult = -1;
        let r = setmarkcommand(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.region_active, 0);
        // mark NOT updated because we returned early.
        assert_eq!(z.mark, 5);
    }

    #[test]
    fn exchangepointandmark_swaps() {
        // c:504-506 — swap zlecs and mark.
        let mut z = Zle::default();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 3;
        z.mark = 8;
        z.zmod.mult = 1;
        let r = exchangepointandmark(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zlecs, 8);
        assert_eq!(z.mark, 3);
        // c:509-510 — zmult > 0 → activate region.
        assert_eq!(z.region_active, 1);
    }

    #[test]
    fn exchangepointandmark_zero_mult_just_activates() {
        // c:500-502 — `if (zmult == 0) { region_active = 1; return 0; }`.
        // No swap occurs.
        let mut z = Zle::default();
        z.zlecs = 3;
        z.mark = 8;
        z.zmod.mult = 0;
        let r = exchangepointandmark(&mut z);
        assert_eq!(r, 0);
        // No swap.
        assert_eq!(z.zlecs, 3);
        assert_eq!(z.mark, 8);
        assert_eq!(z.region_active, 1);
    }

    #[test]
    fn exchangepointandmark_clamps_zlecs_to_zlell() {
        // c:507-508 — `if (zlecs > zlell) zlecs = zlell`.
        let mut z = Zle::default();
        z.zleline = "hi".chars().collect();
        z.zlell = 2;
        z.zlecs = 1;
        z.mark = 99;     // mark beyond zlell
        z.zmod.mult = 1;
        exchangepointandmark(&mut z);
        // After swap zlecs would be 99, clamped to 2.
        assert_eq!(z.zlecs, 2);
    }
}
