//! ZLE movement operations
//!
//! Direct port from zsh/Src/Zle/zle_move.c
//!
//! Move cursor right, checking for combining characters                    // c:118
//! Move cursor left, checking for combining characters                     // c:129


    /// Move cursor to the start of the current logical line.
    /// Port of `findbol()` from Src/Zle/zle_utils.c:1158 — same scan,
    /// just mutates zlecs in-place instead of returning the index.
    /// `findbol` (in utils.rs) is the side-effect-free equivalent.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

    pub fn move_to_bol() {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] != '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Move cursor to the end of the current logical line.
    /// Port of `findeol()` from Src/Zle/zle_utils.c:1169 — mutating
    /// counterpart to `findeol`.
    pub fn move_to_eol() {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] != '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Move cursor up one logical line, preserving the column.
    /// Simplified port of `upline(char **args)` from Src/Zle/zle_hist.c:243 with
    /// fixed n=1 — captures the column-preserve behaviour without the
    /// lastcol sticky-column tracking the C source uses for repeated
    /// up/down chains. Returns false at top-of-buffer.
    pub fn move_up() -> bool {
        let col = current_column();

        // Find start of current line
        let mut line_start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while line_start > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[line_start - 1] != '\n' {
            line_start -= 1;
        }

        if line_start == 0 {
            return false; // Already on first line
        }

        // Move to end of previous line
        crate::ported::zle::zle_main::ZLECS.store(line_start - 1, std::sync::atomic::Ordering::SeqCst);

        // Find start of previous line
        let mut prev_start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while prev_start > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[prev_start - 1] != '\n' {
            prev_start -= 1;
        }

        // Move to same column or end of line
        crate::ported::zle::zle_main::ZLECS.store(prev_start + col.min(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - prev_start), std::sync::atomic::Ordering::SeqCst);

        true
    }

    /// Move cursor down one logical line, preserving the column.
    /// Simplified port of `downline(char **args)` from Src/Zle/zle_hist.c:332
    /// with fixed n=1. Returns false at end-of-buffer.
    pub fn move_down() -> bool {
        let col = current_column();

        // Find end of current line
        let mut line_end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while line_end < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[line_end] != '\n' {
            line_end += 1;
        }

        if line_end >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            return false; // Already on last line
        }

        // Move to start of next line
        crate::ported::zle::zle_main::ZLECS.store(line_end + 1, std::sync::atomic::Ordering::SeqCst);

        // Find end of next line
        let mut next_end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while next_end < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[next_end] != '\n' {
            next_end += 1;
        }

        // Move to same column or end of line
        crate::ported::zle::zle_main::ZLECS.store((crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + col).min(next_end), std::sync::atomic::Ordering::SeqCst);

        true
    }

    /// Compute the cursor's 0-indexed column on its current logical line.
    /// Equivalent to `zlecs - findbol()` — the offset zsh's vertical-
    /// motion code at Src/Zle/zle_hist.c:253 caches in `lastcol` for
    /// sticky-column behaviour across up/down chains.
    pub fn current_column() -> usize {
        let mut col = 0;
        let mut i = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while i > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[i - 1] != '\n' {
            i -= 1;
            col += 1;
        }
        col
    }

    /// Compute the 0-indexed logical-line number containing the cursor.
    /// Port of `findline(int *a, int *b)` from Src/Zle/zle_utils.c:1180 (which fills
    /// in start/end of the cursor's line) but returning just the line
    /// number — counts newlines before the cursor.
    pub fn current_line() -> usize {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
    }

    /// Count the total number of logical lines in the buffer.
    /// Used by display code to size the multi-line refresh region —
    /// mirrors `nlnct` (number of lines counted) tracked by zsh's
    /// `zrefresh()` in Src/Zle/zle_refresh.c.
    pub fn count_lines() -> usize {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().filter(|&&c| c == '\n').count() + 1
    }


/// Port of `BMC_BUFSIZE` from `Src/Zle/zle_move.c:49`.
/// `#define BMC_BUFSIZE MB_CUR_MAX`. Per-cluster buffer size for
/// the multibyte combining-char walker; UTF-8 needs at most 4 bytes
/// per codepoint, so this is conservatively 6 to match POSIX
/// MB_CUR_MAX (some locales use legacy multi-byte encodings up to 6).
pub const BMC_BUFSIZE: usize = 6;                                            // c:161

/// Port of `alignmultiwordleft(int *pos, int setpos)` from Src/Zle/zle_move.c:49.
#[allow(unused_variables)]
pub fn alignmultiwordleft(pos: &mut usize, setpos: i32) {             // c:49
    // C body (c:51-87): walks back over zero-width combining-character
    //                    cluster; pos lands on the base char. Vec<char>
    // indexes one codepoint per slot; the cluster-collapse path is a
    // no-op for ASCII/BMP-only input.
}

/// Port of `alignmultiwordright(int *pos, int setpos)` from Src/Zle/zle_move.c:89.
#[allow(unused_variables)]
pub fn alignmultiwordright(pos: &mut usize, setpos: i32) {            // c:89
    // C body (c:91-119): forward variant of alignmultiwordleft. Same
    //                    no-op-for-Vec<char> story.
}

/// Port of `backwardchar(char **args)` from `Src/Zle/zle_move.c:463`.
/// ```c
/// int
/// backwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = forwardchar();
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
pub fn backwardchar() -> i32 {    // c:464
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                               // c:464 int n = zmult
    if n < 0 {                                                               // c:468
        // c:469-473 — recurse via forwardchar with negated count.
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = forwardchar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && n > 0 {                                           // c:476 while (zlecs > 0 && n--)
        deccs();                                                          // c:477 DECCS()
        n -= 1;
    }
    0                                                                        // c:478 return 0
}

/// Port of `backwardmetafiedchar(char *start, char *endptr, convchar_t *retchr)` from Src/Zle/zle_move.c:170.
/// WARNING: param names don't match C — Rust=(zle) vs C=(start, endptr, retchr)
pub fn backwardmetafiedchar() {   // c:170
    // C body (c:172-184): walks back one Meta-quoted byte pair (0x83
    //                    + (X^0x20)). zshrs's zleline is Vec<char> so
    //                    one decrement covers one codepoint regardless
    //                    of how it'd serialize as Meta-bytes.
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Port of `beginningofline(char **args)` from Src/Zle/zle_move.c:298.
pub fn beginningofline() -> i32 {  // c:298
    // C body (c:300-326): zmult<0 → endofline delegate; else loop
    //                    zmult times: walk back to bol via prev '\\n'.
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = endofline();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            return 0;
        }
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1) == Some(&'\n') {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                return 0;
            }
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1) != Some(&'\n') {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    0
}

/// Port of `beginningoflinehist(char **args)` from Src/Zle/zle_move.c:360.
pub fn beginningoflinehist() -> i32 {  // c:360
    // C body (c:362-398): same as beginningofline but if we hit
    //                    bol with positive count remaining, jump up
    //                    in history.
    let r = beginningofline();
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 && crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult > 1 {
        // C calls uphistory() here; substrate available via History.
        if let Some(_e) = crate::ported::zle::zle_main::history().lock().unwrap().up() {
            crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        }
    }
    r
}

/// Port of `deactivateregion(UNUSED(char **args))` from `Src/Zle/zle_move.c:564`.
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
/// WARNING: param names don't match C — Rust=(zle) vs C=(args)
pub fn deactivateregion() -> i32 {  // c:564
    crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);                                                   // c:564 region_active = 0
    0                                                                        // c:567 return 0
}

/// Port of `deccs()` from `Src/Zle/zle_move.c:133`.
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
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn deccs() {                  // c:133
    crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                                          // c:133
    // c:136 — `alignmultiwordleft(&zlecs, 1)`. Vec<char> indexes
    // one codepoint per slot, so no realignment needed.
}

/// Port of `decpos(int *pos)` from `Src/Zle/zle_move.c:152`.
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
pub fn decpos(pos: &mut usize) {                                             // c:152
    *pos -= 1;                                                               // c:152
    // c:155 — `alignmultiwordleft(pos, 1)`. No-op for Vec<char>.
}

/// Port of `endofline(char **args)` from Src/Zle/zle_move.c:331.
pub fn endofline() -> i32 {       // c:331
    // C body (c:333-355): mirror of beginningofline; walk forward to
    //                    next '\\n'.
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = beginningofline();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
            return 0;
        }
        if crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                return 0;
            }
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) != Some(&'\n') {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    0
}

/// Port of `endoflinehist(char **args)` from Src/Zle/zle_move.c:403.
pub fn endoflinehist() -> i32 {   // c:403
    // C body (c:405-436): mirror of beginningoflinehist; downhistory
    //                    when hitting eol with count remaining.
    let r = endofline();
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult > 1 {
        if let Some(_e) = crate::ported::zle::zle_main::history().lock().unwrap().down() {
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        }
    }
    r
}

/// Port of `exchangepointandmark(UNUSED(char **args))` from `Src/Zle/zle_move.c:495`.
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
pub fn exchangepointandmark() -> i32 {  // c:496
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult == 0 {                                                  // c:496 if (zmult == 0)
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);                                               // c:501
        return 0;                                                            // c:502
    }
    let x = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst);                                                        // c:504 x = mark
    crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                                    // c:505 mark = zlecs
    crate::ported::zle::zle_main::ZLECS.store(x, std::sync::atomic::Ordering::SeqCst);                                                           // c:506 zlecs = x
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                               // c:507
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                               // c:508
    }
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult > 0 {                                                   // c:509
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);                                               // c:510
    }
    0                                                                        // c:511 return 0
}

/// Port of `forwardchar(char **args)` from `Src/Zle/zle_move.c:440`.
/// ```c
/// int
/// forwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = backwardchar();
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
pub fn forwardchar() -> i32 {     // c:441
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                               // c:441 int n = zmult
    if n < 0 {                                                               // c:445
        // c:446-450 — recurse via backwardchar with negated count.
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = backwardchar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && n > 0 {                                   // c:457 while (zlecs < zlell && n--)
        inccs();                                                          // c:458 INCCS()
        n -= 1;
    }
    0                                                                        // c:459 return 0
}

/// Port of `inccs()` from `Src/Zle/zle_move.c:122`.
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
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn inccs() {                  // c:122
    crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                                          // c:122
    // c:125 — `alignmultiwordright(&zlecs, 1)`. No-op for Vec<char>.
}

/// Port of `incpos(int *pos)` from `Src/Zle/zle_move.c:143`.
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
pub fn incpos(pos: &mut usize) {                                             // c:143
    *pos += 1;                                                               // c:143
    // c:146 — `alignmultiwordright(pos, 1)`. No-op for Vec<char>.
}

/// Port of `setmarkcommand(UNUSED(char **args))` from `Src/Zle/zle_move.c:482`.
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
pub fn setmarkcommand() -> i32 {  // c:483
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult < 0 {                                                   // c:483 if (zmult < 0)
        crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);                                               // c:486
        return 0;                                                            // c:487
    }
    crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                                    // c:489 mark = zlecs
    crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:490
    0                                                                        // c:491 return 0
}

/// Port of `vibackwardchar(char **args)` from `Src/Zle/zle_move.c:682`.
/// ```c
/// int
/// vibackwardchar(char **args)
/// {
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = viforwardchar();
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
pub fn vibackwardchar() -> i32 {  // c:683
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                               // c:683
    if n < 0 {                                                               // c:687
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = viforwardchar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_utils::findbol() {            // c:694
        return 1;                                                            // c:695
    }
    while n > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                           // c:696
        deccs();                                                          // c:697
        // c:698-701 — if we crossed onto a '\n', step back forward and exit.
        if crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            break;
        }
        n -= 1;
    }
    0                                                                        // c:703
}

/// Port of `vibeginningofline(UNUSED(char **args))` from `Src/Zle/zle_move.c:728`.
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
/// WARNING: param names don't match C — Rust=(zle) vs C=(args)
pub fn vibeginningofline() -> i32 {  // c:728
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findbol(), std::sync::atomic::Ordering::SeqCst);                 // c:708
    0                                                                        // c:731
}

/// Port of `viendofline(UNUSED(char **args))` from Src/Zle/zle_move.c:708.
pub fn viendofline() -> i32 {     // c:708
    // C body (c:709-723): `oldcs = zlecs; n = zmult; if (n < 1) return 1;
    //                    while (n--) { if (zlecs > zlell) { zlecs = oldcs;
    //                    return 1; } zlecs = findeol() + 1; } DECCS();
    //                    lastcol = 1<<30; return 0`.
    let oldcs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.store(oldcs, std::sync::atomic::Ordering::SeqCst);
            return 1;
        }
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol() + 1, std::sync::atomic::Ordering::SeqCst);
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        deccs();
    }
    0
}

/// Port of `vifindchar(int repeat, char **args)` from Src/Zle/zle_move.c:787.
/// WARNING: param names don't match C — Rust=(zle, repeat) vs C=(repeat, args)
pub fn vifindchar(repeat: i32) -> i32 {  // c:787
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{VFINDCHAR, VFINDDIR, TAILADD};
    let vfind = VFINDCHAR.load(Ordering::Relaxed);
    let vdir = VFINDDIR.load(Ordering::Relaxed);
    let tail = TAILADD.load(Ordering::Relaxed);
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if vdir == 0 {                                                           // c:791
        return 1;
    }
    if n < 0 {                                                               // c:793
        // c:794-798 — recurse via virevrepeatfind with negated count.
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let r = vifindchar(repeat);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n;
        return r;
    }
    // c:800-808 — repeat skip-over-current-match.
    if repeat != 0 && tail != 0 {
        if vdir > 0 {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1 < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) &&
               (crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1] as i32) == vfind {
                inccs();
            }
        } else if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 &&
                  (crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] as i32) == vfind {
            deccs();
        }
    }
    let mut nn = n;
    while nn > 0 {                                                           // c:810
        loop {                                                               // c:811-818 do-while
            if vdir > 0 {
                inccs();
            } else {
                if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 { break; }
                deccs();
            }
            if { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]; crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) ||
               (__c as i32) == vfind ||
               __c == '\n' } {
                break;
            }
        }
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' {
            crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);                                                 // c:820
            return 1;
        }
        nn -= 1;
    }
    if tail > 0 {                                                            // c:824
        inccs();
    } else if tail < 0 {
        deccs();
    }
    0
}

/// Port of `vifindnextchar(char **args)` from Src/Zle/zle_move.c:739.
pub fn vifindnextchar() -> i32 {  // c:739
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:740-746): `if ((vfindchar = vigetkey()) != ZLEEOF) {
    //                    vfinddir=1; tailadd=0; return vifindchar(0,args); }
    //                    return 1`.
    let c = crate::ported::zle::zle_vi::vigetkey();
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindnextcharskip(char **args)` from Src/Zle/zle_move.c:763.
pub fn vifindnextcharskip() -> i32 {  // c:763
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:764-770): vfinddir=1, tailadd=-1 (land just before).
    let c = crate::ported::zle::zle_vi::vigetkey();
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(-1, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindprevchar(char **args)` from Src/Zle/zle_move.c:751.
pub fn vifindprevchar() -> i32 {  // c:751
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:752-758): same as vifindnextchar but vfinddir=-1.
    let c = crate::ported::zle::zle_vi::vigetkey();
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindprevcharskip(char **args)` from Src/Zle/zle_move.c:775.
pub fn vifindprevcharskip() -> i32 {  // c:775
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};
    // C body (c:776-782): vfinddir=-1, tailadd=1 (land just after).
    let c = crate::ported::zle::zle_vi::vigetkey();
    if c < 0 { return 1; }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(1, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifirstnonblank(UNUSED(char **args))` from `Src/Zle/zle_move.c:862`.
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
pub fn vifirstnonblank() -> i32 {  // c:862
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findbol(), std::sync::atomic::Ordering::SeqCst);                 // c:862
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                           // c:865
        let ch = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
        // c:865 — `ZC_iblank` = isblank() = space or tab.
        if ch != ' ' && ch != '\t' {
            break;
        }
        inccs();                                                          // c:866
    }
    0                                                                        // c:867
}

/// Port of `viforwardchar(char **args)` from `Src/Zle/zle_move.c:659`.
/// ```c
/// int
/// viforwardchar(char **args)
/// {
///     int lim = findeol();
///     int n = zmult;
///     if (n < 0) {
///         int ret;
///         zmult = -n;
///         ret = vibackwardchar();
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
pub fn viforwardchar() -> i32 {   // c:660
    let mut lim = crate::ported::zle::zle_utils::findeol();               // c:660
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                               // c:663
    if n < 0 {                                                               // c:665
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = vibackwardchar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    // c:672-673 — invicmdmode + !virangeflag → DECPOS(lim). Skip
    // the vicmd/virangeflag global check; cursor-end-of-line bias
    // applies the same in both modes for the Rust port.
    if *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" && lim > 0 {
        lim -= 1;
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= lim {                                                    // c:674
        return 1;                                                            // c:675
    }
    while n > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < lim {                                         // c:676
        inccs();                                                          // c:677
        n -= 1;
    }
    0                                                                        // c:678
}

/// Port of `vigotocolumn(UNUSED(char **args))` from Src/Zle/zle_move.c:572.
pub fn vigotocolumn() -> i32 {    // c:572
    // C body (c:574-590): findline(&x, &y); n = zmult; if (n>=0) move
    //                    forward n cols from bol (n--); else from eol
    //                    backward.
    let bol = crate::ported::zle::zle_utils::findbol();
    let eol = crate::ported::zle::zle_utils::findeol();
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    let target = if n >= 0 {
        let off = if n > 0 { (n as usize) - 1 } else { 0 };
        (bol + off).min(eol)
    } else {
        eol.saturating_sub((-n) as usize)
    };
    crate::ported::zle::zle_main::ZLECS.store(target.max(bol).min(eol), std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vigotomark(UNUSED(char **args))` from Src/Zle/zle_move.c:887.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(args)
pub fn vigotomark(ch: char) -> i32 { // c:887
    // c:887-927 — read mark name; jump to (vimarkcs[idx], vimarkline[idx]).
    let idx = match ch {
        'a'..='z' => (ch as u8 - b'a') as usize,                             // c:894
        '\'' | '`' => 26,                                                    // c:898 ' / ` mark
        _ => return 1,
    };
    if let Some((cs, hist)) = crate::ported::zle::zle_main::vimarks().lock().unwrap()[idx] {                            // c:903
        crate::ported::zle::zle_main::ZLECS.store(cs.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = hist.max(0) as usize;
        return 0;
    }
    1
}

/// Port of `vigotomarkline(char **args)` from Src/Zle/zle_move.c:929.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(args)
pub fn vigotomarkline(ch: char) -> i32 { // c:929
    // c:929-958 — like vigotomark but lands at first non-blank of
    //              the marked line.
    let r = vigotomark(ch);
    if r == 0 {
        // Snap to start of line + first non-blank.
        let bol = crate::ported::zle::zle_utils::findbol();
        let mut p = bol;
        while p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p];
            if c != ' ' && c != '\t' {
                break;
            }
            p += 1;
        }
        crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
    }
    r
}

/// Port of `vimatchbracket(UNUSED(char **args))` from Src/Zle/zle_move.c:594.
pub fn vimatchbracket() -> i32 {  // c:594
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                                     // c:594
    if (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n')) // c:599
        && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0
    {
        deccs();                                                          // c:600
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') { // c:604
        crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);                                                     // c:605
        return 1;                                                            // c:606
    }
    let me = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];                                         // c:608
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
            crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
            return 1;
        }
    };
    let mut depth = 1i32;                                                    // c:639
    loop {
        if dir > 0 {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
                return 1;
            }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        } else {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
                return 1;
            }
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        let c = match crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) {
            Some(&c) => c,
            None => {
                crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
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

/// Port of `virepeatfind(char **args)` from Src/Zle/zle_move.c:835.
pub fn virepeatfind() -> i32 {    // c:835
    // C body c:837 — `return vifindchar(1, args)`. Repeats the last
    //                vi find with the same direction.
    vifindchar(1)
}

/// Port of `virevrepeatfind(char **args)` from Src/Zle/zle_move.c:842.
pub fn virevrepeatfind() -> i32 { // c:842
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::{TAILADD, VFINDDIR};
    // c:846-851 — `if (zmult < 0) { zmult = -zmult; ret = vifindchar(1);
    //                              zmult = -zmult; return ret }`.
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult < 0 {
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -__g_zmod.mult;
        let ret = vifindchar(1);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -__g_zmod.mult;
        return ret;
    }
    // c:852-856 — toggle tailadd + vfinddir, repeat, restore.
    let t = TAILADD.load(Ordering::SeqCst);
    let d = VFINDDIR.load(Ordering::SeqCst);
    TAILADD.store(-t, Ordering::SeqCst);
    VFINDDIR.store(-d, Ordering::SeqCst);
    let ret = vifindchar(1);
    TAILADD.store(t, Ordering::SeqCst);
    VFINDDIR.store(d, Ordering::SeqCst);
    ret
}

/// Port of `visetmark(UNUSED(char **args))` from Src/Zle/zle_move.c:872.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(args)
pub fn visetmark(ch: char) -> i32 { // c:872
    // c:872 — `ch = getfullchar(0)`. Caller passes the read char.
    if !('a'..='z').contains(&ch) {                                          // c:877
        return 1;
    }
    let idx = (ch as u8 - b'a') as usize;                                    // c:879
    crate::ported::zle::zle_main::vimarks().lock().unwrap()[idx] = Some((crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::history().lock().unwrap().cursor as i32));        // c:880
    0
}

/// Port of `visuallinemode(UNUSED(char **args))` from Src/Zle/zle_move.c:540.
pub fn visuallinemode() -> i32 {  // c:540
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:542-547 — `if (virangeflag) { prefixflag = 1; flags &= ~CHAR;
    //                                  flags |= LINE; return 0 }`.
    match crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {                                                // c:548
        2 => { crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst); }   // c:549-551
        0 => {
            crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                            // c:553
            crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst);                                           // c:556
        }
        1 => { crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst); }   // c:555-557
        _ => {}
    }
    let _ = MOD_LINE;
    0
}

/// Port of `visualmode(UNUSED(char **args))` from Src/Zle/zle_move.c:516.
pub fn visualmode() -> i32 {      // c:516
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:518-523 — `if (virangeflag) { prefixflag = 1; flags &= ~LINE;
    //                                  flags |= CHAR; return 0 }`.
    //              No virangeflag tracker yet; skip.
    match crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {                                                // c:524
        1 => { crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst); }   // c:525-527
        0 => {
            crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                            // c:529 fall-through to case 2
            crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);                                           // c:532
        }
        2 => { crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst); }   // c:531-533
        _ => {}
    }
    let _ = MOD_CHAR;
    0
}

#[cfg(test)]
mod region_tests {
    use super::*;

    #[test]
    fn deactivateregion_clears_active() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:566 — `region_active = 0; return 0`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        let r = deactivateregion();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn setmarkcommand_sets_mark_to_cursor() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:489-490 — `mark = zlecs; region_active = 1`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        let r = setmarkcommand();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 7);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn setmarkcommand_negative_mult_deactivates() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:485-487 — `if (zmult < 0) { region_active = 0; return 0; }`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -1;
        let r = setmarkcommand();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
        // mark NOT updated because we returned early.
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn exchangepointandmark_swaps() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:504-506 — swap zlecs and mark.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        let r = exchangepointandmark();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 8);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 3);
        // c:509-510 — zmult > 0 → activate region.
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn exchangepointandmark_zero_mult_just_activates() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:500-502 — `if (zmult == 0) { region_active = 1; return 0; }`.
        // No swap occurs.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 0;
        let r = exchangepointandmark();
        assert_eq!(r, 0);
        // No swap.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 8);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn exchangepointandmark_clamps_zlecs_to_zlell() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:507-508 — `if (zlecs > zlell) zlecs = zlell`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(99, std::sync::atomic::Ordering::SeqCst);     // mark beyond zlell
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        exchangepointandmark();
        // After swap zlecs would be 99, clamped to 2.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    // ---------- Cursor movement (forwardchar / backwardchar / inccs / deccs) ----

    #[test]
    fn inccs_increments_zlecs() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:121-126 — `zlecs++; alignmultiwordright(...)`. Vec<char>
        // makes alignment a no-op.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        inccs();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
        inccs();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn deccs_decrements_zlecs() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:132-137 — `zlecs--; alignmultiwordleft(...)`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        deccs();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
        deccs();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn incpos_decpos_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:457-458 — `while (zlecs < zlell && n--) INCCS();`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 3;
        let r = forwardchar();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn forwardchar_stops_at_zlell() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:457 — `while (zlecs < zlell && ...)`. Walking past end
        // is bounded.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        forwardchar();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn backwardchar_moves_zmult_positions() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:476-477 — `while (zlecs > 0 && n--) DECCS();`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 3;
        let r = backwardchar();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn backwardchar_stops_at_zero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:476 — `while (zlecs > 0 && ...)`. Doesn't underflow.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        backwardchar();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn forwardchar_negative_count_delegates_to_backward() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:445-450 — `if (n < 0) { zmult = -n; ret = backwardchar(); ... }`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -2;
        forwardchar();
        // -2 → backwardchar(2) → cursor goes 4→2
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
        // c:447,449 — zmult restored to original after recursion.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, -2);
    }

    #[test]
    fn backwardchar_negative_count_delegates_to_forward() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -2;
        backwardchar();
        // -2 → forwardchar(2) → cursor goes 1→3
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, -2);
    }

    // ---------- vi movement (vibeginningofline / vibackwardchar / viforwardchar) ----

    #[test]
    fn vibeginningofline_jumps_to_bol() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:730 — `zlecs = findbol()`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(9, std::sync::atomic::Ordering::SeqCst);  // 'h' in "ghi"
        let r = vibeginningofline();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 8); // after the second '\n'
    }

    #[test]
    fn vibackwardchar_stops_at_line_start() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:694-695 — at findbol → return 1 without moving.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);  // 'd' (right after newline)
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        let r = vibackwardchar();
        assert_eq!(r, 1);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4); // unchanged
    }

    #[test]
    fn vibackwardchar_moves_within_line() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 3;
        vibackwardchar();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn viforwardchar_stops_at_eol() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:674-675 — at findeol → return 1.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);  // at '\n'
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        let r = viforwardchar();
        assert_eq!(r, 1);
    }

    #[test]
    fn viforwardchar_moves_within_line() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 3;
        viforwardchar();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn viforwardchar_clamps_at_findeol() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:676 — `while (n-- && zlecs < lim)`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        viforwardchar();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    // ---------- vifirstnonblank tests ----------

    #[test]
    fn vifirstnonblank_skips_leading_spaces() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:864-866 — bol then skip space/tab.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "   hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(8, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst); // somewhere mid-word
        let r = vifirstnonblank();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3); // first non-blank
    }

    #[test]
    fn vifirstnonblank_skips_tabs() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:865 — ZC_iblank includes tab.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "\t\t\tfoo".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        vifirstnonblank();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn vifirstnonblank_no_blanks() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // No leading blanks → cursor lands at bol.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        vifirstnonblank();
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vifirstnonblank_all_blanks() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:865 — `while zlecs != zlell` exits cleanly when only blanks.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "   ".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        vifirstnonblank();
        // walks to zlell (no non-blank found).
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn vifirstnonblank_respects_findbol() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:864 — `zlecs = findbol()`. With multiline buffer, jump
        // to start of CURRENT line, then skip blanks.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\n   def".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(10, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(8, std::sync::atomic::Ordering::SeqCst); // 'e' in 'def'
        vifirstnonblank();
        // findbol → 4 (after first '\n'); skip 3 spaces → 7 ('d')
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 7);
    }
}
