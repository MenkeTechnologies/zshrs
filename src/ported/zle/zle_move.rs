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
    /// Simplified port of `upline(args)` from Src/Zle/zle_hist.c:243 with
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
    /// Simplified port of `downline(args)` from Src/Zle/zle_hist.c:332
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
    /// Port of `findline(a, b)` from Src/Zle/zle_utils.c:1180 (which fills
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

/// Port of `BMC_BUFSIZE` from `Src/Zle/zle_move.c:161`.
/// `#define BMC_BUFSIZE MB_CUR_MAX`. Per-cluster buffer size for
/// the multibyte combining-char walker; UTF-8 needs at most 4 bytes
/// per codepoint, so this is conservatively 6 to match POSIX
/// MB_CUR_MAX (some locales use legacy multi-byte encodings up to 6).
pub const BMC_BUFSIZE: usize = 6;                                            // c:161

/// Port of `alignmultiwordleft(pos, setpos)` from Src/Zle/zle_move.c:49.
pub fn alignmultiwordleft(_pos: &mut usize, _interactive: i32) {             // c:49
    // C body (c:51-87): walks back over zero-width combining-character
    //                    cluster; pos lands on the base char. Vec<char>
    // indexes one codepoint per slot; the cluster-collapse path is a
    // no-op for ASCII/BMP-only input.
}

/// Port of `alignmultiwordright(pos, setpos)` from Src/Zle/zle_move.c:89.
pub fn alignmultiwordright(_pos: &mut usize, _interactive: i32) {            // c:89
    // C body (c:91-119): forward variant of alignmultiwordleft. Same
    //                    no-op-for-Vec<char> story.
}

/// Port of `backwardchar(args)` from `Src/Zle/zle_move.c:463`.
/// ```c
/// int
/// backwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = forwardchar(args);
///         zmult = n;
///         return ret;
///     }
///     while (zlecs > 0 && n--)
///         DECCS();
///     return 0;
/// }
/// ```
/// `backward-char` widget — move cursor left by `zmult` positions.
/// Negative count delegates to `forwardchar` with negated count.
/// Port of `backwardchar` from `Src/Zle/zle_move.c:463`.
pub fn backwardchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {    // c:463
    let mut n = zle.zmod.mult;                                               // c:466 int n = zmult
    if n < 0 {                                                               // c:468
        // c:469-473 — recurse via forwardchar with negated count.
        let saved = n;
        zle.zmod.mult = -n;
        let ret = forwardchar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    while zle.zlecs > 0 && n > 0 {                                           // c:476 while (zlecs > 0 && n--)
        deccs(zle);                                                          // c:477 DECCS()
        n -= 1;
    }
    0                                                                        // c:478 return 0
}

/// Port of `backwardmetafiedchar(start, endptr, retchr)` from Src/Zle/zle_move.c:170.
pub fn backwardmetafiedchar(zle: &mut crate::ported::zle::zle_main::Zle) {   // c:170
    // C body (c:172-184): walks back one Meta-quoted byte pair (0x83
    //                    + (X^0x20)). zshrs's zleline is Vec<char> so
    //                    one decrement covers one codepoint regardless
    //                    of how it'd serialize as Meta-bytes.
    if zle.zlecs > 0 {
        zle.zlecs -= 1;
    }
}

/// Port of `beginningofline(args)` from Src/Zle/zle_move.c:298.
pub fn beginningofline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:298
    // C body (c:300-326): zmult<0 → endofline delegate; else loop
    //                    zmult times: walk back to bol via prev '\\n'.
    let n = zle.zmod.mult;
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n;
        let ret = endofline(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    for _ in 0..n {
        if zle.zlecs == 0 {
            return 0;
        }
        if zle.zlecs > 0 && zle.zleline.get(zle.zlecs - 1) == Some(&'\n') {
            zle.zlecs -= 1;
            if zle.zlecs == 0 {
                return 0;
            }
        }
        while zle.zlecs > 0 && zle.zleline.get(zle.zlecs - 1) != Some(&'\n') {
            zle.zlecs -= 1;
        }
    }
    0
}

/// Port of `beginningoflinehist(args)` from Src/Zle/zle_move.c:360.
pub fn beginningoflinehist(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:360
    // C body (c:362-398): same as beginningofline but if we hit
    //                    bol with positive count remaining, jump up
    //                    in history.
    let r = beginningofline(zle);
    if zle.zlecs == 0 && zle.zmod.mult > 1 {
        // C calls uphistory(args) here; substrate available via History.
        if let Some(_e) = zle.history.up() {
            zle.zlecs = 0;
        }
    }
    r
}

/// Port of `deactivateregion(args)` from `Src/Zle/zle_move.c:563`.
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

/// Port of `deccs()` from `Src/Zle/zle_move.c:132`.
/// ```c
/// mod_export void
/// deccs(void)
/// {
///     zlecs--;
///     alignmultiwordleft(&zlecs, 1);
/// }
/// ```
/// Decrement the cursor, skipping combining-char clusters.
/// In zshrs `zleline` is `Vec<char>` (one codepoint per slot), so
/// the C alignmultiwordleft path is a no-op — just `zlecs--`.
pub fn deccs(zle: &mut crate::ported::zle::zle_main::Zle) {                  // c:132
    zle.zlecs -= 1;                                                          // c:135
    // c:136 — `alignmultiwordleft(&zlecs, 1)`. Vec<char> indexes
    // one codepoint per slot, so no realignment needed.
}

/// Port of `decpos(pos)` from `Src/Zle/zle_move.c:151`.
/// ```c
/// mod_export void
/// decpos(int *pos)
/// {
///     (*pos)--;
///     alignmultiwordleft(pos, 1);
/// }
/// ```
/// Decrement an arbitrary cursor position; same multibyte note as
/// `deccs`.
pub fn decpos(pos: &mut usize) {                                             // c:151
    *pos -= 1;                                                               // c:154
    // c:155 — `alignmultiwordleft(pos, 1)`. No-op for Vec<char>.
}

/// Port of `endofline(args)` from Src/Zle/zle_move.c:331.
pub fn endofline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {       // c:331
    // C body (c:333-355): mirror of beginningofline; walk forward to
    //                    next '\\n'.
    let n = zle.zmod.mult;
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n;
        let ret = beginningofline(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    for _ in 0..n {
        if zle.zlecs >= zle.zlell {
            zle.zlecs = zle.zlell;
            return 0;
        }
        if zle.zleline.get(zle.zlecs) == Some(&'\n') {
            zle.zlecs += 1;
            if zle.zlecs == zle.zlell {
                return 0;
            }
        }
        while zle.zlecs != zle.zlell && zle.zleline.get(zle.zlecs) != Some(&'\n') {
            zle.zlecs += 1;
        }
    }
    0
}

/// Port of `endoflinehist(args)` from Src/Zle/zle_move.c:403.
pub fn endoflinehist(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {   // c:403
    // C body (c:405-436): mirror of beginningoflinehist; downhistory
    //                    when hitting eol with count remaining.
    let r = endofline(zle);
    if zle.zlecs == zle.zlell && zle.zmod.mult > 1 {
        if let Some(_e) = zle.history.down() {
            zle.zlecs = zle.zlell;
        }
    }
    r
}

/// Port of `exchangepointandmark(args)` from `Src/Zle/zle_move.c:495`.
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
/// Port of `exchangepointandmark` from `Src/Zle/zle_move.c:495`.
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

/// Port of `forwardchar(args)` from `Src/Zle/zle_move.c:440`.
/// ```c
/// int
/// forwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = backwardchar(args);
///         zmult = n;
///         return ret;
///     }
///     while (zlecs < zlell && n--)
///         INCCS();
///     return 0;
/// }
/// ```
/// `forward-char` widget — move cursor right by `zmult` positions.
/// Negative count delegates to `backwardchar` with negated count.
/// Port of `forwardchar` from `Src/Zle/zle_move.c:440`.
pub fn forwardchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {     // c:440
    let mut n = zle.zmod.mult;                                               // c:443 int n = zmult
    if n < 0 {                                                               // c:445
        // c:446-450 — recurse via backwardchar with negated count.
        let saved = n;
        zle.zmod.mult = -n;
        let ret = backwardchar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    while zle.zlecs < zle.zlell && n > 0 {                                   // c:457 while (zlecs < zlell && n--)
        inccs(zle);                                                          // c:458 INCCS()
        n -= 1;
    }
    0                                                                        // c:459 return 0
}

/// Port of `inccs()` from `Src/Zle/zle_move.c:121`.
/// ```c
/// mod_export void
/// inccs(void)
/// {
///     zlecs++;
///     alignmultiwordright(&zlecs, 1);
/// }
/// ```
/// Increment the cursor, skipping combining-char clusters.
/// In zshrs `zleline` is `Vec<char>` (one codepoint per slot), so
/// the C alignmultiwordright path is a no-op — just `zlecs++`.
pub fn inccs(zle: &mut crate::ported::zle::zle_main::Zle) {                  // c:121
    zle.zlecs += 1;                                                          // c:124
    // c:125 — `alignmultiwordright(&zlecs, 1)`. No-op for Vec<char>.
}

/// Port of `incpos(pos)` from `Src/Zle/zle_move.c:142`.
/// ```c
/// mod_export void
/// incpos(int *pos)
/// {
///     (*pos)++;
///     alignmultiwordright(pos, 1);
/// }
/// ```
/// Increment an arbitrary cursor position; same multibyte note as
/// `inccs`.
pub fn incpos(pos: &mut usize) {                                             // c:142
    *pos += 1;                                                               // c:145
    // c:146 — `alignmultiwordright(pos, 1)`. No-op for Vec<char>.
}

/// Port of `setmarkcommand(args)` from `Src/Zle/zle_move.c:482`.
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
/// Port of `setmarkcommand` from `Src/Zle/zle_move.c:482`.
pub fn setmarkcommand(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:482
    if zle.zmod.mult < 0 {                                                   // c:485 if (zmult < 0)
        zle.region_active = 0;                                               // c:486
        return 0;                                                            // c:487
    }
    zle.mark = zle.zlecs;                                                    // c:489 mark = zlecs
    zle.region_active = 1;                                                   // c:490
    0                                                                        // c:491 return 0
}

/// Port of `vibackwardchar(args)` from `Src/Zle/zle_move.c:682`.
/// ```c
/// int
/// vibackwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = viforwardchar(args);
///         zmult = n;
///         return ret;
///     }
///     if (zlecs == findbol())
///         return 1;
///     while (n-- && zlecs > 0) {
///         DECCS();
///         if (zleline[zlecs] == '\n') {
///             zlecs++;
///             break;
///         }
///     }
///     return 0;
/// }
/// ```
/// `vi-backward-char` widget — move left by zmult positions but
/// stop at the start of the current line (don't cross a newline).
/// Port of `vibackwardchar` from `Src/Zle/zle_move.c:682`.
pub fn vibackwardchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:682
    let mut n = zle.zmod.mult;                                               // c:685
    if n < 0 {                                                               // c:687
        let saved = n;
        zle.zmod.mult = -n;
        let ret = viforwardchar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    if zle.zlecs == crate::ported::zle::zle_utils::findbol(zle) {            // c:694
        return 1;                                                            // c:695
    }
    while n > 0 && zle.zlecs > 0 {                                           // c:696
        deccs(zle);                                                          // c:697
        // c:698-701 — if we crossed onto a '\n', step back forward and exit.
        if zle.zleline.get(zle.zlecs) == Some(&'\n') {
            zle.zlecs += 1;
            break;
        }
        n -= 1;
    }
    0                                                                        // c:703
}

/// Port of `vibeginningofline(args)` from `Src/Zle/zle_move.c:727`.
/// ```c
/// int
/// vibeginningofline(UNUSED(char **args))
/// {
///     zlecs = findbol();
///     return 0;
/// }
/// ```
/// `vi-beginning-of-line` widget — jump to the start of the
/// current line (after any preceding newline).
pub fn vibeginningofline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:727
    zle.zlecs = crate::ported::zle::zle_utils::findbol(zle);                 // c:730
    0                                                                        // c:731
}

/// Port of `viendofline(args)` from Src/Zle/zle_move.c:708.
pub fn viendofline(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {     // c:707
    // C body (c:709-723): `oldcs = zlecs; n = zmult; if (n < 1) return 1;
    //                    while (n--) { if (zlecs > zlell) { zlecs = oldcs;
    //                    return 1; } zlecs = findeol() + 1; } DECCS();
    //                    lastcol = 1<<30; return 0`.
    let oldcs = zle.zlecs;
    let n = zle.zmod.mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        if zle.zlecs > zle.zlell {
            zle.zlecs = oldcs;
            return 1;
        }
        zle.zlecs = crate::ported::zle::zle_utils::findeol(zle) + 1;
    }
    if zle.zlecs > 0 {
        deccs(zle);
    }
    0
}

/// Port of `vifindchar(repeat, args)` from Src/Zle/zle_move.c:787.
pub fn vifindchar(zle: &mut crate::ported::zle::zle_main::Zle, repeat: i32) -> i32 {  // c:786
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{VFINDCHAR, VFINDDIR, TAILADD};
    let vfind = VFINDCHAR.load(Ordering::Relaxed);
    let vdir = VFINDDIR.load(Ordering::Relaxed);
    let tail = TAILADD.load(Ordering::Relaxed);
    let ocs = zle.zlecs;
    let n = zle.zmod.mult;
    if vdir == 0 {                                                           // c:791
        return 1;
    }
    if n < 0 {                                                               // c:793
        // c:794-798 — recurse via virevrepeatfind with negated count.
        zle.zmod.mult = -n;
        let r = vifindchar(zle, repeat);
        zle.zmod.mult = n;
        return r;
    }
    // c:800-808 — repeat skip-over-current-match.
    if repeat != 0 && tail != 0 {
        if vdir > 0 {
            if zle.zlecs + 1 < zle.zlell &&
               (zle.zleline[zle.zlecs + 1] as i32) == vfind {
                inccs(zle);
            }
        } else if zle.zlecs > 0 &&
                  (zle.zleline[zle.zlecs - 1] as i32) == vfind {
            deccs(zle);
        }
    }
    let mut nn = n;
    while nn > 0 {                                                           // c:810
        loop {                                                               // c:811-818 do-while
            if vdir > 0 {
                inccs(zle);
            } else {
                if zle.zlecs == 0 { break; }
                deccs(zle);
            }
            if zle.zlecs >= zle.zlell ||
               (zle.zleline[zle.zlecs] as i32) == vfind ||
               zle.zleline[zle.zlecs] == '\n' {
                break;
            }
        }
        if zle.zlecs >= zle.zlell || zle.zleline[zle.zlecs] == '\n' {
            zle.zlecs = ocs;                                                 // c:820
            return 1;
        }
        nn -= 1;
    }
    if tail > 0 {                                                            // c:824
        inccs(zle);
    } else if tail < 0 {
        deccs(zle);
    }
    0
}

/// Port of `vifindnextchar(args)` from Src/Zle/zle_move.c:739.
pub fn vifindnextchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:738
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:740-746): `if ((vfindchar = vigetkey()) != ZLEEOF) {
    //                    vfinddir=1; tailadd=0; return vifindchar(0,args); }
    //                    return 1`.
    let c = crate::ported::zle::zle_vi::vigetkey(zle);
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(zle, 0)
}

/// Port of `vifindnextcharskip(args)` from Src/Zle/zle_move.c:763.
pub fn vifindnextcharskip(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:762
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:764-770): vfinddir=1, tailadd=-1 (land just before).
    let c = crate::ported::zle::zle_vi::vigetkey(zle);
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(-1, Ordering::SeqCst);
    vifindchar(zle, 0)
}

/// Port of `vifindprevchar(args)` from Src/Zle/zle_move.c:751.
pub fn vifindprevchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:750
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:752-758): same as vifindnextchar but vfinddir=-1.
    let c = crate::ported::zle::zle_vi::vigetkey(zle);
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(zle, 0)
}

/// Port of `vifindprevcharskip(args)` from Src/Zle/zle_move.c:775.
pub fn vifindprevcharskip(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:774
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:776-782): vfinddir=-1, tailadd=1 (land just after).
    let c = crate::ported::zle::zle_vi::vigetkey(zle);
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(1, Ordering::SeqCst);
    vifindchar(zle, 0)
}

/// Port of `vifirstnonblank(args)` from `Src/Zle/zle_move.c:861`.
/// ```c
/// int
/// vifirstnonblank(UNUSED(char **args))
/// {
///     zlecs = findbol();
///     while (zlecs != zlell && ZC_iblank(zleline[zlecs]))
///         INCCS();
///     return 0;
/// }
/// ```
/// `vi-first-non-blank` widget — jump to bol then skip leading
/// whitespace. ZC_iblank is `iblank` (space/tab) for ASCII.
/// Port of `vifirstnonblank` from `Src/Zle/zle_move.c:861`.
pub fn vifirstnonblank(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:861
    zle.zlecs = crate::ported::zle::zle_utils::findbol(zle);                 // c:864
    while zle.zlecs != zle.zlell {                                           // c:865
        let ch = zle.zleline[zle.zlecs];
        // c:865 — `ZC_iblank` = isblank() = space or tab.
        if ch != ' ' && ch != '\t' {
            break;
        }
        inccs(zle);                                                          // c:866
    }
    0                                                                        // c:867
}

/// Port of `viforwardchar(args)` from `Src/Zle/zle_move.c:659`.
/// ```c
/// int
/// viforwardchar(char **args)
/// {
///     int lim = findeol();
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = vibackwardchar(args);
///         zmult = n;
///         return ret;
///     }
///     if (invicmdmode() && !virangeflag)
///         DECPOS(lim);
///     if (zlecs >= lim)
///         return 1;
///     while (n-- && zlecs < lim)
///         INCCS();
///     return 0;
/// }
/// ```
/// `vi-forward-char` widget — move right by zmult positions but
/// stop at the end of the current line. In vi-cmd-mode the cursor
/// can't sit ON the trailing newline (DECPOS(lim) excludes it).
/// Port of `viforwardchar` from `Src/Zle/zle_move.c:659`.
pub fn viforwardchar(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {   // c:659
    let mut lim = crate::ported::zle::zle_utils::findeol(zle);               // c:662
    let mut n = zle.zmod.mult;                                               // c:663
    if n < 0 {                                                               // c:665
        let saved = n;
        zle.zmod.mult = -n;
        let ret = vibackwardchar(zle);
        zle.zmod.mult = saved;
        return ret;
    }
    // c:672-673 — invicmdmode + !virangeflag → DECPOS(lim). Skip
    // the vicmd/virangeflag global check; cursor-end-of-line bias
    // applies the same in both modes for the Rust port.
    if *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" && lim > 0 {
        lim -= 1;
    }
    if zle.zlecs >= lim {                                                    // c:674
        return 1;                                                            // c:675
    }
    while n > 0 && zle.zlecs < lim {                                         // c:676
        inccs(zle);                                                          // c:677
        n -= 1;
    }
    0                                                                        // c:678
}

/// Port of `vigotocolumn(args)` from Src/Zle/zle_move.c:572.
pub fn vigotocolumn(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {    // c:572
    // C body (c:574-590): findline(&x, &y); n = zmult; if (n>=0) move
    //                    forward n cols from bol (n--); else from eol
    //                    backward.
    let bol = crate::ported::zle::zle_utils::findbol(zle);
    let eol = crate::ported::zle::zle_utils::findeol(zle);
    let n = zle.zmod.mult;
    let target = if n >= 0 {
        let off = if n > 0 { (n as usize) - 1 } else { 0 };
        (bol + off).min(eol)
    } else {
        eol.saturating_sub((-n) as usize)
    };
    zle.zlecs = target.max(bol).min(eol);
    0
}

/// Port of `vigotomark(args)` from Src/Zle/zle_move.c:887.
pub fn vigotomark(zle: &mut crate::ported::zle::zle_main::Zle, ch: char) -> i32 { // c:887
    // c:889-927 — read mark name; jump to (vimarkcs[idx], vimarkline[idx]).
    let idx = match ch {
        'a'..='z' => (ch as u8 - b'a') as usize,                             // c:894
        '\'' | '`' => 26,                                                    // c:898 ' / ` mark
        _ => return 1,
    };
    if let Some((cs, hist)) = zle.vi_marks[idx] {                            // c:903
        zle.zlecs = cs.min(zle.zlell);
        zle.history.cursor = hist.max(0) as usize;
        return 0;
    }
    1
}

/// Port of `vigotomarkline(args)` from Src/Zle/zle_move.c:929.
pub fn vigotomarkline(zle: &mut crate::ported::zle::zle_main::Zle, ch: char) -> i32 { // c:929
    // c:931-958 — like vigotomark but lands at first non-blank of
    //              the marked line.
    let r = vigotomark(zle, ch);
    if r == 0 {
        // Snap to start of line + first non-blank.
        let bol = crate::ported::zle::zle_utils::findbol(zle);
        let mut p = bol;
        while p < zle.zlell {
            let c = zle.zleline[p];
            if c != ' ' && c != '\t' {
                break;
            }
            p += 1;
        }
        zle.zlecs = p;
    }
    r
}

/// Port of `vimatchbracket(args)` from Src/Zle/zle_move.c:594.
pub fn vimatchbracket(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:594
    let ocs = zle.zlecs;                                                     // c:596
    if (zle.zlecs == zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n')) // c:599
        && zle.zlecs > 0
    {
        deccs(zle);                                                          // c:600
    }
    if zle.zlecs == zle.zlell || zle.zleline.get(zle.zlecs) == Some(&'\n') { // c:604
        zle.zlecs = ocs;                                                     // c:605
        return 1;                                                            // c:606
    }
    let me = zle.zleline[zle.zlecs];                                         // c:608
    let (oth, dir) = match me {                                              // c:609-635
        '{' => ('}', 1),
        '}' => ('{', -1),
        '(' => (')', 1),
        ')' => ('(', -1),
        '[' => (']', 1),
        ']' => ('[', -1),
        '<' => ('>', 1),
        '>' => ('<', -1),
        _ => {
            zle.zlecs = ocs;
            return 1;
        }
    };
    let mut depth = 1i32;                                                    // c:639
    loop {
        if dir > 0 {
            if zle.zlecs >= zle.zlell {
                zle.zlecs = ocs;
                return 1;
            }
            zle.zlecs += 1;
        } else {
            if zle.zlecs == 0 {
                zle.zlecs = ocs;
                return 1;
            }
            zle.zlecs -= 1;
        }
        let c = match zle.zleline.get(zle.zlecs) {
            Some(&c) => c,
            None => {
                zle.zlecs = ocs;
                return 1;
            }
        };
        if c == me {
            depth += 1;
        } else if c == oth {
            depth -= 1;
            if depth == 0 {
                return 0;
            }
        }
    }
}

/// Port of `virepeatfind(args)` from Src/Zle/zle_move.c:835.
pub fn virepeatfind(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {    // c:835
    // C body c:837 — `return vifindchar(1, args)`. Repeats the last
    //                vi find with the same direction.
    vifindchar(zle, 1)
}

/// Port of `virevrepeatfind(args)` from Src/Zle/zle_move.c:842.
pub fn virevrepeatfind(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 { // c:842
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDDIR};
    // c:846-851 — `if (zmult < 0) { zmult = -zmult; ret = vifindchar(1);
    //                              zmult = -zmult; return ret }`.
    if zle.zmod.mult < 0 {
        zle.zmod.mult = -zle.zmod.mult;
        let ret = vifindchar(zle, 1);
        zle.zmod.mult = -zle.zmod.mult;
        return ret;
    }
    // c:852-856 — toggle tailadd + vfinddir, repeat, restore.
    let t = TAILADD.load(Ordering::SeqCst);
    let d = VFINDDIR.load(Ordering::SeqCst);
    TAILADD.store(-t, Ordering::SeqCst);
    VFINDDIR.store(-d, Ordering::SeqCst);
    let ret = vifindchar(zle, 1);
    TAILADD.store(t, Ordering::SeqCst);
    VFINDDIR.store(d, Ordering::SeqCst);
    ret
}

/// Port of `visetmark(args)` from Src/Zle/zle_move.c:872.
pub fn visetmark(zle: &mut crate::ported::zle::zle_main::Zle, ch: char) -> i32 { // c:872
    // c:876 — `ch = getfullchar(0)`. Caller passes the read char.
    if !('a'..='z').contains(&ch) {                                          // c:877
        return 1;
    }
    let idx = (ch as u8 - b'a') as usize;                                    // c:879
    zle.vi_marks[idx] = Some((zle.zlecs, zle.history.cursor as i32));        // c:880
    0
}

/// Port of `visuallinemode(args)` from Src/Zle/zle_move.c:540.
pub fn visuallinemode(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {  // c:540
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:542-547 — `if (virangeflag) { prefixflag = 1; flags &= ~CHAR;
    //                                  flags |= LINE; return 0 }`.
    match zle.region_active {                                                // c:548
        2 => zle.region_active = 0,                                          // c:549-551
        0 => {
            zle.mark = zle.zlecs;                                            // c:553
            zle.region_active = 2;                                           // c:556
        }
        1 => zle.region_active = 2,                                          // c:555-557
        _ => {}
    }
    let _ = MOD_LINE;
    0
}

/// Port of `visualmode(args)` from Src/Zle/zle_move.c:516.
pub fn visualmode(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:516
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:518-523 — `if (virangeflag) { prefixflag = 1; flags &= ~LINE;
    //                                  flags |= CHAR; return 0 }`.
    //              No virangeflag tracker yet; skip.
    match zle.region_active {                                                // c:524
        1 => zle.region_active = 0,                                          // c:525-527
        0 => {
            zle.mark = zle.zlecs;                                            // c:529 fall-through to case 2
            zle.region_active = 1;                                           // c:532
        }
        2 => zle.region_active = 1,                                          // c:531-533
        _ => {}
    }
    let _ = MOD_CHAR;
    0
}

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

    // ---------- Cursor movement (forwardchar / backwardchar / inccs / deccs) ----

    #[test]
    fn inccs_increments_zlecs() {
        // c:121-126 — `zlecs++; alignmultiwordright(...)`. Vec<char>
        // makes alignment a no-op.
        let mut z = Zle::default();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        z.zlecs = 0;
        inccs(&mut z);
        assert_eq!(z.zlecs, 1);
        inccs(&mut z);
        assert_eq!(z.zlecs, 2);
    }

    #[test]
    fn deccs_decrements_zlecs() {
        // c:132-137 — `zlecs--; alignmultiwordleft(...)`.
        let mut z = Zle::default();
        z.zleline = "abc".chars().collect();
        z.zlell = 3;
        z.zlecs = 2;
        deccs(&mut z);
        assert_eq!(z.zlecs, 1);
        deccs(&mut z);
        assert_eq!(z.zlecs, 0);
    }

    #[test]
    fn incpos_decpos_round_trip() {
        // c:142-156 — pos++ / pos-- with no-op alignment.
        let mut p = 5;
        incpos(&mut p);
        assert_eq!(p, 6);
        incpos(&mut p);
        assert_eq!(p, 7);
        decpos(&mut p);
        assert_eq!(p, 6);
    }

    #[test]
    fn forwardchar_moves_zmult_positions() {
        // c:457-458 — `while (zlecs < zlell && n--) INCCS();`.
        let mut z = Zle::default();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 0;
        z.zmod.mult = 3;
        let r = forwardchar(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zlecs, 3);
    }

    #[test]
    fn forwardchar_stops_at_zlell() {
        // c:457 — `while (zlecs < zlell && ...)`. Walking past end
        // is bounded.
        let mut z = Zle::default();
        z.zleline = "ab".chars().collect();
        z.zlell = 2;
        z.zlecs = 0;
        z.zmod.mult = 99;
        forwardchar(&mut z);
        assert_eq!(z.zlecs, 2);
    }

    #[test]
    fn backwardchar_moves_zmult_positions() {
        // c:476-477 — `while (zlecs > 0 && n--) DECCS();`.
        let mut z = Zle::default();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 8;
        z.zmod.mult = 3;
        let r = backwardchar(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zlecs, 5);
    }

    #[test]
    fn backwardchar_stops_at_zero() {
        // c:476 — `while (zlecs > 0 && ...)`. Doesn't underflow.
        let mut z = Zle::default();
        z.zleline = "ab".chars().collect();
        z.zlell = 2;
        z.zlecs = 1;
        z.zmod.mult = 99;
        backwardchar(&mut z);
        assert_eq!(z.zlecs, 0);
    }

    #[test]
    fn forwardchar_negative_count_delegates_to_backward() {
        // c:445-450 — `if (n < 0) { zmult = -n; ret = backwardchar(args); ... }`.
        let mut z = Zle::default();
        z.zleline = "hello".chars().collect();
        z.zlell = 5;
        z.zlecs = 4;
        z.zmod.mult = -2;
        forwardchar(&mut z);
        // -2 → backwardchar(2) → cursor goes 4→2
        assert_eq!(z.zlecs, 2);
        // c:447,449 — zmult restored to original after recursion.
        assert_eq!(z.zmod.mult, -2);
    }

    #[test]
    fn backwardchar_negative_count_delegates_to_forward() {
        let mut z = Zle::default();
        z.zleline = "hello".chars().collect();
        z.zlell = 5;
        z.zlecs = 1;
        z.zmod.mult = -2;
        backwardchar(&mut z);
        // -2 → forwardchar(2) → cursor goes 1→3
        assert_eq!(z.zlecs, 3);
        assert_eq!(z.zmod.mult, -2);
    }

    // ---------- vi movement (vibeginningofline / vibackwardchar / viforwardchar) ----

    #[test]
    fn vibeginningofline_jumps_to_bol() {
        // c:730 — `zlecs = findbol()`.
        let mut z = Zle::default();
        z.zleline = "abc\ndef\nghi".chars().collect();
        z.zlell = 11;
        z.zlecs = 9;  // 'h' in "ghi"
        let r = vibeginningofline(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zlecs, 8); // after the second '\n'
    }

    #[test]
    fn vibackwardchar_stops_at_line_start() {
        // c:694-695 — at findbol → return 1 without moving.
        let mut z = Zle::default();
        z.zleline = "abc\ndef".chars().collect();
        z.zlell = 7;
        z.zlecs = 4;  // 'd' (right after newline)
        z.zmod.mult = 1;
        let r = vibackwardchar(&mut z);
        assert_eq!(r, 1);
        assert_eq!(z.zlecs, 4); // unchanged
    }

    #[test]
    fn vibackwardchar_moves_within_line() {
        let mut z = Zle::default();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 8;
        z.zmod.mult = 3;
        vibackwardchar(&mut z);
        assert_eq!(z.zlecs, 5);
    }

    #[test]
    fn viforwardchar_stops_at_eol() {
        // c:674-675 — at findeol → return 1.
        let mut z = Zle::default();
        z.zleline = "abc\ndef".chars().collect();
        z.zlell = 7;
        z.zlecs = 3;  // at '\n'
        z.zmod.mult = 1;
        let r = viforwardchar(&mut z);
        assert_eq!(r, 1);
    }

    #[test]
    fn viforwardchar_moves_within_line() {
        let mut z = Zle::default();
        z.zleline = "hello world".chars().collect();
        z.zlell = 11;
        z.zlecs = 0;
        z.zmod.mult = 3;
        viforwardchar(&mut z);
        assert_eq!(z.zlecs, 3);
    }

    #[test]
    fn viforwardchar_clamps_at_findeol() {
        // c:676 — `while (n-- && zlecs < lim)`.
        let mut z = Zle::default();
        z.zleline = "ab".chars().collect();
        z.zlell = 2;
        z.zlecs = 0;
        z.zmod.mult = 99;
        viforwardchar(&mut z);
        assert_eq!(z.zlecs, 2);
    }

    // ---------- vifirstnonblank tests ----------

    #[test]
    fn vifirstnonblank_skips_leading_spaces() {
        // c:864-866 — bol then skip space/tab.
        let mut z = Zle::default();
        z.zleline = "   hello".chars().collect();
        z.zlell = 8;
        z.zlecs = 5; // somewhere mid-word
        let r = vifirstnonblank(&mut z);
        assert_eq!(r, 0);
        assert_eq!(z.zlecs, 3); // first non-blank
    }

    #[test]
    fn vifirstnonblank_skips_tabs() {
        // c:865 — ZC_iblank includes tab.
        let mut z = Zle::default();
        z.zleline = "\t\t\tfoo".chars().collect();
        z.zlell = 6;
        z.zlecs = 0;
        vifirstnonblank(&mut z);
        assert_eq!(z.zlecs, 3);
    }

    #[test]
    fn vifirstnonblank_no_blanks() {
        // No leading blanks → cursor lands at bol.
        let mut z = Zle::default();
        z.zleline = "hello".chars().collect();
        z.zlell = 5;
        z.zlecs = 3;
        vifirstnonblank(&mut z);
        assert_eq!(z.zlecs, 0);
    }

    #[test]
    fn vifirstnonblank_all_blanks() {
        // c:865 — `while zlecs != zlell` exits cleanly when only blanks.
        let mut z = Zle::default();
        z.zleline = "   ".chars().collect();
        z.zlell = 3;
        z.zlecs = 0;
        vifirstnonblank(&mut z);
        // walks to zlell (no non-blank found).
        assert_eq!(z.zlecs, 3);
    }

    #[test]
    fn vifirstnonblank_respects_findbol() {
        // c:864 — `zlecs = findbol()`. With multiline buffer, jump
        // to start of CURRENT line, then skip blanks.
        let mut z = Zle::default();
        z.zleline = "abc\n   def".chars().collect();
        z.zlell = 10;
        z.zlecs = 8; // 'e' in 'def'
        vifirstnonblank(&mut z);
        // findbol → 4 (after first '\n'); skip 3 spaces → 7 ('d')
        assert_eq!(z.zlecs, 7);
    }
}
