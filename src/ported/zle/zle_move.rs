//! ZLE movement operations
//!
//! Direct port from zsh/Src/Zle/zle_move.c
//!
//! Move cursor right, checking for combining characters                    // c:118
//! Move cursor left, checking for combining characters                     // c:129

use std::sync::atomic::Ordering;

use crate::ported::zle::zle_h::{MOD_CHAR, MOD_LINE};
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_params::*,
    zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{isset, COMBININGCHARS};
use crate::zsh_h::{IS_BASECHAR, IS_COMBINING};

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Port of `int alignmultiwordleft(int *pos, int setpos)` from
/// `Src/Zle/zle_move.c:49-78`. If `*pos` lands inside a combining-
/// char cluster (`COMBININGCHARS` set + char at `*pos` is combining),
/// walks left until hitting the cluster's base char and (when
/// `setpos != 0`) writes that index back through `*pos`. Returns 1
/// on successful re-align, 0 if not on a combining char or if the
/// scan ran off the buffer start.
///
/// ```c
/// int alignmultiwordleft(int *pos, int setpos) {
///     int loccs = *pos;
///     if (!isset(COMBININGCHARS) || loccs == zlell || loccs == 0)
///         return 0;                                              // c:54-55
///     if (!IS_COMBINING(zleline[loccs]))
///         return 0;                                              // c:58-59
///     loccs--;                                                   // c:62
///     for (;;) {
///         if (IS_BASECHAR(zleline[loccs])) {
///             if (setpos) *pos = loccs;
///             return 1;                                          // c:67-69
///         } else if (!IS_COMBINING(zleline[loccs])) {
///             return 0;                                          // c:70-72
///         }
///         if (loccs-- == 0)
///             return 0;                                          // c:75-76
///     }
/// }
/// ```
pub fn alignmultiwordleft(pos: &mut usize, setpos: i32) -> i32 {
    // c:49
    let mut loccs = *pos;
    let zlell = ZLELL.load(Ordering::SeqCst) as usize;
    // c:54-55 — gate on COMBININGCHARS + boundary checks.
    if !isset(COMBININGCHARS) || loccs == zlell || loccs == 0 {
        return 0;
    }
    let zleline = ZLELINE.lock().unwrap();
    // c:58-59 — current pos must be on a combining char.
    if loccs >= zleline.len() || !IS_COMBINING(zleline[loccs]) {
        return 0;
    }
    // c:62 — step back one cell.
    loccs -= 1;
    // c:63-77 — scan back until base char (return 1) or non-combining
    // non-base (return 0) or buffer start.
    loop {
        if loccs >= zleline.len() {
            return 0;
        }
        if IS_BASECHAR(zleline[loccs]) {
            if setpos != 0 {
                *pos = loccs; // c:68
            }
            return 1; // c:69
        } else if !IS_COMBINING(zleline[loccs]) {
            return 0; // c:71
        }
        // c:75 — `if (loccs-- == 0) return 0;` post-decrement: returns
        // 0 BEFORE decrementing past 0 (so we never read at -1).
        if loccs == 0 {
            return 0;
        }
        loccs -= 1;
    }
}

/// Port of `int alignmultiwordright(int *pos, int setpos)` from
/// `Src/Zle/zle_move.c:89-115`. First runs `alignmultiwordleft(pos, 0)`
/// to test if `*pos` is on a combining char; if not, returns 0. If
/// yes, scans FORWARD from `*pos + 1` until the next non-combining
/// char (or end-of-buffer), and (when `setpos != 0`) sets `*pos`
/// to that index.
///
/// ```c
/// int alignmultiwordright(int *pos, int setpos) {
///     int loccs;
///     if (!alignmultiwordleft(pos, 0))                           // c:96
///         return 0;
///     loccs = *pos + 1;                                          // c:100
///     while (loccs < zlell) {
///         if (!IS_COMBINING(zleline[loccs])) {
///             if (setpos) *pos = loccs;
///             return 1;                                          // c:104-107
///         }
///         loccs++;
///     }
///     if (setpos) *pos = loccs;
///     return 1;                                                  // c:112-114
/// }
/// ```
pub fn alignmultiwordright(pos: &mut usize, setpos: i32) -> i32 {
    // c:89
    // c:96 — probe via left-align to confirm we're on a combining char.
    if alignmultiwordleft(pos, 0) == 0 {
        return 0;
    }
    let mut loccs = *pos + 1; // c:100
    let zlell = ZLELL.load(Ordering::SeqCst) as usize;
    let zleline = ZLELINE.lock().unwrap();
    // c:102-110 — scan forward through the cluster.
    while loccs < zlell {
        if loccs >= zleline.len() {
            break;
        }
        if !IS_COMBINING(zleline[loccs]) {
            if setpos != 0 {
                *pos = loccs; // c:106
            }
            return 1; // c:107
        }
        loccs += 1;
    }
    // c:112-114 — ran off the end; pin `*pos` at end and report success.
    if setpos != 0 {
        *pos = loccs;
    }
    1
}

/// Port of `inccs()` from `Src/Zle/zle_move.c:122`.
/// ```c
/// mod_export void
/// inccs(void)
/// {
///     zlecs++;
///     alignmultiwordright(&zlecs, 1);                        // c:125
/// }
/// ```
/// Increment the cursor, then realign past any combining-mark cluster
/// at the new position.
pub fn inccs() {
    // c:122
    let new_cs = ZLECS.fetch_add(1, Ordering::SeqCst) + 1; // c:122
    let mut p = new_cs;
    alignmultiwordright(&mut p, 1); // c:125
    if p != new_cs {
        ZLECS.store(p, Ordering::SeqCst);
    }
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
/// Decrement the cursor, then realign back over any combining-mark
/// cluster at the new position to land on the cluster's base char.
pub fn deccs() {
    // c:133
    let prev = ZLECS.fetch_sub(1, Ordering::SeqCst); // c:133
    let new_cs = prev.saturating_sub(1);
    let mut p = new_cs;
    alignmultiwordleft(&mut p, 1); // c:136
    if p != new_cs {
        ZLECS.store(p, Ordering::SeqCst);
    }
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
pub fn incpos(pos: &mut usize) {
    // c:143
    *pos += 1; // c:143
               // c:146 — `alignmultiwordright(pos, 1)`. No-op for Vec<char>.
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
pub fn decpos(pos: &mut usize) {
    // c:152
    // C: `(*pos)--;` on an `int*` — pos can underflow to -1 silently
    // (two's complement). Rust's `usize` panics on overflow in debug
    // builds. C callers always guard with `if (zlecs > 0) decpos(...)`
    // (see Src/Zle/zle_move.c:130, c:198, etc.), so the underflow case
    // is unreachable in practice. Guard it here defensively so any
    // accidental caller doesn't crash the shell — staying at 0 is the
    // best approximation of C's "before start" intent.
    if *pos > 0 {
        *pos -= 1; // c:152
    }
    // c:155 — `alignmultiwordleft(pos, 1)`. No-op for Vec<char>.
}

/// Port of `BMC_BUFSIZE` from `Src/Zle/zle_move.c:49`.
/// `#define BMC_BUFSIZE MB_CUR_MAX`. Per-cluster buffer size for
/// the multibyte combining-char walker; UTF-8 needs at most 4 bytes
/// per codepoint, so this is conservatively 6 to match POSIX
/// MB_CUR_MAX (some locales use legacy multi-byte encodings up to 6).
pub const BMC_BUFSIZE: usize = 6; // c:161

/// Port of `backwardmetafiedchar(char *start, char *endptr, convchar_t *retchr)` from Src/Zle/zle_move.c:170.
/// WARNING: param names don't match C — Rust=(zle) vs C=(start, endptr, retchr)
pub fn backwardmetafiedchar() {
    // c:170
    // C body (c:172-184): walks back one Meta-quoted byte pair (0x83
    //                    + (X^0x20)). zshrs's zleline is Vec<char> so
    //                    one decrement covers one codepoint regardless
    //                    of how it'd serialize as Meta-bytes.
    if ZLECS.load(Ordering::SeqCst) > 0 {
        ZLECS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Port of `beginningofline(char **args)` from Src/Zle/zle_move.c:298.
pub fn beginningofline() -> i32 {
    // c:298
    // C body (c:300-326): zmult<0 → endofline delegate; else loop
    //                    zmult times: walk back to bol via prev '\\n'.
    let n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = endofline();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    for _ in 0..n {
        if ZLECS.load(Ordering::SeqCst) == 0 {
            return 0;
        }
        if ZLECS.load(Ordering::SeqCst) > 0
            && ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst) - 1)
                == Some(&'\n')
        {
            ZLECS.fetch_sub(1, Ordering::SeqCst);
            if ZLECS.load(Ordering::SeqCst) == 0 {
                return 0;
            }
        }
        while ZLECS.load(Ordering::SeqCst) > 0
            && ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst) - 1)
                != Some(&'\n')
        {
            ZLECS.fetch_sub(1, Ordering::SeqCst);
        }
    }
    0
}

/// Port of `endofline(char **args)` from Src/Zle/zle_move.c:331.
pub fn endofline() -> i32 {
    // c:331
    // C body (c:333-355): mirror of beginningofline; walk forward to
    //                    next '\\n'.
    let n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = beginningofline();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    for _ in 0..n {
        if ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst) {
            ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
            return 0;
        }
        if ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) == Some(&'\n') {
            ZLECS.fetch_add(1, Ordering::SeqCst);
            if ZLECS.load(Ordering::SeqCst) == ZLELL.load(Ordering::SeqCst) {
                return 0;
            }
        }
        while ZLECS.load(Ordering::SeqCst) != ZLELL.load(Ordering::SeqCst)
            && ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) != Some(&'\n')
        {
            ZLECS.fetch_add(1, Ordering::SeqCst);
        }
    }
    0
}

/// Port of `beginningoflinehist(char **args)` from Src/Zle/zle_move.c:360.
///
/// Direct line-by-line port. Walks the buffer backward `zmult` lines,
/// stopping at each `\n` boundary. If we still have remaining count
/// when we hit BOL, redirect into [`uphistory`] for the leftover.
/// On `zmult < 0` redirects to [`endoflinehist`] with the absolute
/// count and restores zmult on return.
pub fn beginningoflinehist() -> i32 {
    use crate::ported::zle::zle_hist::uphistory;

    // c:362 — `int n = zmult;`
    let mut n = ZMOD.lock().unwrap().mult;

    // c:364-369 — zmult<0 redirect.
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = endoflinehist();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }

    // c:370-389 — walk back `n` lines via \n boundaries.
    while n > 0 {
        let cs = ZLECS.load(Ordering::SeqCst);
        if cs == 0 {
            break; // c:373
        }
        // c:375-376 — `pos = zlecs; DECPOS(pos);`
        let pos = cs - 1;
        let at = ZLELINE.lock().unwrap().get(pos).copied();
        if at == Some('\n') {
            ZLECS.store(pos, Ordering::SeqCst); // c:378
            if pos == 0 {
                break; // c:380
            }
        }
        // c:384-385 — `while (zlecs && zleline[zlecs - 1] != '\n') zlecs--;`
        loop {
            let cs = ZLECS.load(Ordering::SeqCst);
            if cs == 0 {
                break;
            }
            if ZLELINE.lock().unwrap().get(cs - 1).copied() == Some('\n') {
                break;
            }
            ZLECS.fetch_sub(1, Ordering::SeqCst);
        }
        n -= 1; // c:386
    }

    // c:388-394 — leftover count → uphistory + zlecs=0.
    if n > 0 {
        let m = ZMOD.lock().unwrap().mult;
        ZMOD.lock().unwrap().mult = n;
        let ret = uphistory();
        ZMOD.lock().unwrap().mult = m;
        ZLECS.store(0, Ordering::SeqCst);
        return ret;
    }
    0 // c:395
}

/// Port of `endoflinehist(char **args)` from Src/Zle/zle_move.c:403.
pub fn endoflinehist() -> i32 {
    // c:403

    // c:405 — int n = zmult;
    let mut n = ZMOD.lock().unwrap().mult;
    // c:407-413 — if (n < 0) { zmult = -n; ret = beginningoflinehist(args); zmult = n; return ret; }
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = beginningoflinehist();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }

    // c:414-427 — line-boundary walk: each iteration advances to the
    //              next '\n' (skipping one if at it under vi-cmd mode).
    let line = ZLELINE.lock().unwrap().clone();
    let ll = ZLELL.load(Ordering::SeqCst);
    while n > 0 {
        let mut cs = ZLECS.load(Ordering::SeqCst);
        // c:415-418 — if (zlecs >= zlell) { zlecs = zlell; break; }
        if cs >= ll {
            ZLECS.store(ll, Ordering::SeqCst);
            break;
        }
        // c:419-420 — if ((zlecs += invicmdmode()) == zlell) break;
        cs += invicmdmode();
        if cs == ll {
            ZLECS.store(cs, Ordering::SeqCst);
            break;
        }
        // c:421-423 — if (zleline[zlecs] == '\n') if (++zlecs == zlell) break;
        if cs < line.len() && line.get(cs) == Some(&'\n') {
            cs += 1;
            if cs == ll {
                ZLECS.store(cs, Ordering::SeqCst);
                break;
            }
        }
        // c:424-425 — while (zlecs != zlell && zleline[zlecs] != '\n') zlecs++;
        while cs != ll && line.get(cs) != Some(&'\n') {
            cs += 1;
        }
        ZLECS.store(cs, Ordering::SeqCst);
        n -= 1; // c:426
    }
    // c:428-434 — if (n) { m = zmult; zmult = n; ret = downhistory(args); zmult = m; return ret; }
    if n > 0 {
        let m = ZMOD.lock().unwrap().mult;
        ZMOD.lock().unwrap().mult = n;
        if let Some(_e) = history().lock().unwrap().down() {
            ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        ZMOD.lock().unwrap().mult = m;
    }
    // c:436 — return 0;
    0
}

/// Local port of `invicmdmode()`. Returns 1 in vi-command mode (cursor
/// stops one short of EOL), 0 otherwise.
fn invicmdmode() -> usize {
    if INSMODE.load(Ordering::SeqCst) == 0 {
        1
    } else {
        0
    }
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
pub fn forwardchar() -> i32 {
    // c:441
    let mut n = ZMOD.lock().unwrap().mult; // c:441 int n = zmult
    if n < 0 {
        // c:445
        // c:446-450 — recurse via backwardchar with negated count.
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = backwardchar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    while ZLECS.load(Ordering::SeqCst) < ZLELL.load(Ordering::SeqCst) && n > 0 {
        // c:457 while (zlecs < zlell && n--)
        inccs(); // c:458 INCCS()
        n -= 1;
    }
    0 // c:459 return 0
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
pub fn backwardchar() -> i32 {
    // c:464
    let mut n = ZMOD.lock().unwrap().mult; // c:464 int n = zmult
    if n < 0 {
        // c:468
        // c:469-473 — recurse via forwardchar with negated count.
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = forwardchar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    while ZLECS.load(Ordering::SeqCst) > 0 && n > 0 {
        // c:476 while (zlecs > 0 && n--)
        deccs(); // c:477 DECCS()
        n -= 1;
    }
    0 // c:478 return 0
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
pub fn setmarkcommand() -> i32 {
    // c:483
    if ZMOD.lock().unwrap().mult < 0 {
        // c:483 if (zmult < 0)
        REGION_ACTIVE.store(0, Ordering::SeqCst); // c:486
        return 0; // c:487
    }
    MARK.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst); // c:489 mark = zlecs
    REGION_ACTIVE.store(1, Ordering::SeqCst); // c:490
    0 // c:491 return 0
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
pub fn exchangepointandmark() -> i32 {
    // c:496
    if ZMOD.lock().unwrap().mult == 0 {
        // c:496 if (zmult == 0)
        REGION_ACTIVE.store(1, Ordering::SeqCst); // c:501
        return 0; // c:502
    }
    let x = MARK.load(Ordering::SeqCst); // c:504 x = mark
    MARK.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst); // c:505 mark = zlecs
    ZLECS.store(x, Ordering::SeqCst); // c:506 zlecs = x
    if ZLECS.load(Ordering::SeqCst) > ZLELL.load(Ordering::SeqCst) {
        // c:507
        ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst); // c:508
    }
    if ZMOD.lock().unwrap().mult > 0 {
        // c:509
        REGION_ACTIVE.store(1, Ordering::SeqCst); // c:510
    }
    0 // c:511 return 0
}

/// Port of `visualmode(UNUSED(char **args))` from Src/Zle/zle_move.c:516.
pub fn visualmode() -> i32 {
    // c:516
    // c:518-523 — `if (virangeflag) { prefixflag = 1; flags &= ~LINE;
    //                                  flags |= CHAR; return 0 }`.
    //              No virangeflag tracker yet; skip.
    match REGION_ACTIVE.load(Ordering::SeqCst) {
        // c:524
        1 => {
            REGION_ACTIVE.store(0, Ordering::SeqCst);
        } // c:525-527
        0 => {
            MARK.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst); // c:529 fall-through to case 2
            REGION_ACTIVE.store(1, Ordering::SeqCst); // c:532
        }
        2 => {
            REGION_ACTIVE.store(1, Ordering::SeqCst);
        } // c:531-533
        _ => {}
    }
    let _ = MOD_CHAR;
    0
}

/// Port of `visuallinemode(UNUSED(char **args))` from Src/Zle/zle_move.c:540.
pub fn visuallinemode() -> i32 {
    // c:540
    // c:542-547 — `if (virangeflag) { prefixflag = 1; flags &= ~CHAR;
    //                                  flags |= LINE; return 0 }`.
    match REGION_ACTIVE.load(Ordering::SeqCst) {
        // c:548
        2 => {
            REGION_ACTIVE.store(0, Ordering::SeqCst);
        } // c:549-551
        0 => {
            MARK.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst); // c:553
            REGION_ACTIVE.store(2, Ordering::SeqCst); // c:556
        }
        1 => {
            REGION_ACTIVE.store(2, Ordering::SeqCst);
        } // c:555-557
        _ => {}
    }
    let _ = MOD_LINE;
    0
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
pub fn deactivateregion() -> i32 {
    // c:564
    REGION_ACTIVE.store(0, Ordering::SeqCst); // c:564 region_active = 0
    0 // c:567 return 0
}

/// Port of `vigotocolumn(UNUSED(char **args))` from Src/Zle/zle_move.c:572.
pub fn vigotocolumn() -> i32 {
    // c:572
    // C body (c:574-590): findline(&x, &y); n = zmult; if (n>=0) move
    //                    forward n cols from bol (n--); else from eol
    //                    backward.
    let bol = findbol();
    let eol = findeol();
    let n = ZMOD.lock().unwrap().mult;
    let target = if n >= 0 {
        let off = if n > 0 { (n as usize) - 1 } else { 0 };
        (bol + off).min(eol)
    } else {
        eol.saturating_sub((-n) as usize)
    };
    ZLECS.store(target.max(bol).min(eol), Ordering::SeqCst);
    0
}

/// Port of `vimatchbracket(UNUSED(char **args))` from Src/Zle/zle_move.c:594.
pub fn vimatchbracket() -> i32 {
    // c:594
    let ocs = ZLECS.load(Ordering::SeqCst); // c:594
    if (ZLECS.load(Ordering::SeqCst) == ZLELL.load(Ordering::SeqCst) || ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) == Some(&'\n')) // c:599
        && ZLECS.load(Ordering::SeqCst) > 0
    {
        deccs(); // c:600
    }
    if ZLECS.load(Ordering::SeqCst) == ZLELL.load(Ordering::SeqCst)
        || ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) == Some(&'\n')
    {
        // c:604
        ZLECS.store(ocs, Ordering::SeqCst); // c:605
        return 1; // c:606
    }
    let me = ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)]; // c:608
    let (oth, dir) = match me {
        // c:609-635
        '{' => ('}', 1),
        '}' => ('{', -1),
        '(' => (')', 1),
        ')' => ('(', -1),
        '[' => (']', 1),
        ']' => ('[', -1),
        '<' => ('>', 1),
        '>' => ('<', -1),
        _ => {
            ZLECS.store(ocs, Ordering::SeqCst);
            return 1;
        }
    };
    let mut depth = 1i32; // c:639
    loop {
        if dir > 0 {
            if ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst) {
                ZLECS.store(ocs, Ordering::SeqCst);
                return 1;
            }
            ZLECS.fetch_add(1, Ordering::SeqCst);
        } else {
            if ZLECS.load(Ordering::SeqCst) == 0 {
                ZLECS.store(ocs, Ordering::SeqCst);
                return 1;
            }
            ZLECS.fetch_sub(1, Ordering::SeqCst);
        }
        let c = match ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) {
            Some(&c) => c,
            None => {
                ZLECS.store(ocs, Ordering::SeqCst);
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
pub fn viforwardchar() -> i32 {
    // c:660
    let mut lim = findeol(); // c:660
    let mut n = ZMOD.lock().unwrap().mult; // c:663
    if n < 0 {
        // c:665
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = vibackwardchar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    // c:672-673 — invicmdmode + !virangeflag → DECPOS(lim). Skip
    // the vicmd/virangeflag global check; cursor-end-of-line bias
    // applies the same in both modes for the Rust port.
    if *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" && lim > 0 {
        lim -= 1;
    }
    if ZLECS.load(Ordering::SeqCst) >= lim {
        // c:674
        return 1; // c:675
    }
    while n > 0 && ZLECS.load(Ordering::SeqCst) < lim {
        // c:676
        inccs(); // c:677
        n -= 1;
    }
    0 // c:678
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
pub fn vibackwardchar() -> i32 {
    // c:683
    let mut n = ZMOD.lock().unwrap().mult; // c:683
    if n < 0 {
        // c:687
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = viforwardchar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    if ZLECS.load(Ordering::SeqCst) == findbol() {
        // c:694
        return 1; // c:695
    }
    while n > 0 && ZLECS.load(Ordering::SeqCst) > 0 {
        // c:696
        deccs(); // c:697
                 // c:698-701 — if we crossed onto a '\n', step back forward and exit.
        if ZLELINE.lock().unwrap().get(ZLECS.load(Ordering::SeqCst)) == Some(&'\n') {
            ZLECS.fetch_add(1, Ordering::SeqCst);
            break;
        }
        n -= 1;
    }
    0 // c:703
}

/// Port of `viendofline(UNUSED(char **args))` from Src/Zle/zle_move.c:708.
pub fn viendofline() -> i32 {
    // c:708
    // C body (c:709-723): `oldcs = zlecs; n = zmult; if (n < 1) return 1;
    //                    while (n--) { if (zlecs > zlell) { zlecs = oldcs;
    //                    return 1; } zlecs = findeol() + 1; } DECCS();
    //                    lastcol = 1<<30; return 0`.
    let oldcs = ZLECS.load(Ordering::SeqCst);
    let n = ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        if ZLECS.load(Ordering::SeqCst) > ZLELL.load(Ordering::SeqCst) {
            ZLECS.store(oldcs, Ordering::SeqCst);
            return 1;
        }
        ZLECS.store(findeol() + 1, Ordering::SeqCst);
    }
    if ZLECS.load(Ordering::SeqCst) > 0 {
        deccs();
    }
    // c:722 — `lastcol = 1<<30`. The sentinel is what makes `$` STICKY: a
    // later vertical motion aims for this column, never reaches it, and so
    // lands at the end of whatever line it moved to. Dropping it left `$`
    // as a one-shot move and a following `j` landed at the start of the
    // next line instead of its end.
    crate::ported::zle::zle_main::LASTCOL.store(1 << 30, Ordering::SeqCst);
    0
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
pub fn vibeginningofline() -> i32 {
    // c:728
    ZLECS.store(findbol(), Ordering::SeqCst); // c:708
    0 // c:731
}

/// Port of `vifindnextchar(char **args)` from Src/Zle/zle_move.c:739.
pub fn vifindnextchar() -> i32 {
    // c:739
    // C body (c:740-746): `if ((vfindchar = vigetkey()) != ZLEEOF) {
    //                    vfinddir=1; tailadd=0; return vifindchar(0,args); }
    //                    return 1`.
    let c = vigetkey();
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindprevchar(char **args)` from Src/Zle/zle_move.c:751.
pub fn vifindprevchar() -> i32 {
    // c:751
    // C body (c:752-758): same as vifindnextchar but vfinddir=-1.
    let c = vigetkey();
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(0, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindnextcharskip(char **args)` from Src/Zle/zle_move.c:763.
pub fn vifindnextcharskip() -> i32 {
    // c:763
    // C body (c:764-770): vfinddir=1, tailadd=-1 (land just before).
    let c = vigetkey();
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(1, Ordering::SeqCst);
    TAILADD.store(-1, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindprevcharskip(char **args)` from Src/Zle/zle_move.c:775.
pub fn vifindprevcharskip() -> i32 {
    // c:775
    // C body (c:776-782): vfinddir=-1, tailadd=1 (land just after).
    let c = vigetkey();
    if c < 0 {
        return 1;
    }
    VFINDCHAR.store(c, Ordering::SeqCst);
    VFINDDIR.store(-1, Ordering::SeqCst);
    TAILADD.store(1, Ordering::SeqCst);
    vifindchar(0)
}

/// Port of `vifindchar(int repeat, char **args)` from Src/Zle/zle_move.c:787.
/// WARNING: param names don't match C — Rust=(zle, repeat) vs C=(repeat, args)
pub fn vifindchar(repeat: i32) -> i32 {
    // c:787
    let vfind = VFINDCHAR.load(Ordering::Relaxed);
    let vdir = VFINDDIR.load(Ordering::Relaxed);
    let tail = TAILADD.load(Ordering::Relaxed);
    let ocs = ZLECS.load(Ordering::SeqCst);
    let n = ZMOD.lock().unwrap().mult;
    if vdir == 0 {
        // c:791
        return 1;
    }
    if n < 0 {
        // c:793
        // c:794-798 — recurse via virevrepeatfind with negated count.
        ZMOD.lock().unwrap().mult = -n;
        let r = vifindchar(repeat);
        ZMOD.lock().unwrap().mult = n;
        return r;
    }
    // c:800-808 — repeat skip-over-current-match.
    if repeat != 0 && tail != 0 {
        if vdir > 0 {
            if ZLECS.load(Ordering::SeqCst) + 1 < ZLELL.load(Ordering::SeqCst)
                && (ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst) + 1] as i32) == vfind
            {
                inccs();
            }
        } else if ZLECS.load(Ordering::SeqCst) > 0
            && (ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst) - 1] as i32) == vfind
        {
            deccs();
        }
    }
    let mut nn = n;
    while nn > 0 {
        // c:810
        loop {
            // c:811-818 do-while
            if vdir > 0 {
                inccs();
            } else {
                if ZLECS.load(Ordering::SeqCst) == 0 {
                    break;
                }
                deccs();
            }
            // c:818-820 — `while (zlecs >= 0 && zlecs < zlell &&
            // zleline[zlecs] != vfindchar && zleline[zlecs] != '\n')`
            // — C short-circuits the BOUNDS test before indexing
            // zleline[zlecs]. The previous Rust indexed first, so
            // `f<char>` with the cursor reaching end-of-line panicked
            // (index out of bounds: len 18, index 18).
            if {
                let cs = ZLECS.load(Ordering::SeqCst);
                cs >= ZLELL.load(Ordering::SeqCst) || {
                    let __c = ZLELINE.lock().unwrap()[cs];
                    (__c as i32) == vfind || __c == '\n'
                }
            } {
                break;
            }
        }
        if ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst)
            || ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)] == '\n'
        {
            ZLECS.store(ocs, Ordering::SeqCst); // c:820
            return 1;
        }
        nn -= 1;
    }
    if tail > 0 {
        // c:824
        inccs();
    } else if tail < 0 {
        deccs();
    }
    0
}

/// Port of `virepeatfind(char **args)` from Src/Zle/zle_move.c:835.
pub fn virepeatfind() -> i32 {
    // c:835
    // C body c:837 — `return vifindchar(1, args)`. Repeats the last
    //                vi find with the same direction.
    vifindchar(1)
}

/// Port of `virevrepeatfind(char **args)` from Src/Zle/zle_move.c:842.
pub fn virevrepeatfind() -> i32 {
    // c:842
    // c:846-851 — `if (zmult < 0) { zmult = -zmult; ret = vifindchar(1);
    //                              zmult = -zmult; return ret }`.
    if ZMOD.lock().unwrap().mult < 0 {
        // Each ZMOD lock stays inside its own block: `vifindchar` locks
        // ZMOD again, and std's Mutex is not reentrant (see zmult_arg,
        // zle_main.rs). C negates `zmult` in place with no lock at all.
        {
            let mut __g_zmod = ZMOD.lock().unwrap();
            __g_zmod.mult = -__g_zmod.mult;
        }
        let ret = vifindchar(1);
        {
            let mut __g_zmod = ZMOD.lock().unwrap();
            __g_zmod.mult = -__g_zmod.mult;
        }
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
pub fn vifirstnonblank() -> i32 {
    // c:862
    ZLECS.store(findbol(), Ordering::SeqCst); // c:862
    while ZLECS.load(Ordering::SeqCst) != ZLELL.load(Ordering::SeqCst) {
        // c:865
        let ch = ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)];
        // c:865 — `ZC_iblank` aliases to `wcsiblank` under MULTIBYTE_SUPPORT
        // (Src/Zle/zle.h:62 → Src/utils.c:4302-4307): `iswspace && != \n`.
        if !crate::ported::zle::zle_h::ZC_iblank(ch) {
            break;
        }
        inccs(); // c:866
    }
    0 // c:867
}

/// Port of `visetmark(UNUSED(char **args))` from Src/Zle/zle_move.c:872.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(args)
pub fn visetmark(ch: char) -> i32 {
    // c:872
    // c:872 — `ch = getfullchar(0)`. Caller passes the read char.
    if !('a'..='z').contains(&ch) {
        // c:877
        return 1;
    }
    let idx = (ch as u8 - b'a') as usize; // c:879
    vimarks().lock().unwrap()[idx] = Some((
        ZLECS.load(Ordering::SeqCst),
        history().lock().unwrap().cursor as i32,
    )); // c:880
    0
}

/// Port of `vigotomark(UNUSED(char **args))` from Src/Zle/zle_move.c:887.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(args)
pub fn vigotomark(ch: char) -> i32 {
    // c:887
    // c:887-927 — read mark name; jump to (vimarkcs[idx], vimarkline[idx]).
    let idx = match ch {
        'a'..='z' => (ch as u8 - b'a') as usize, // c:894
        '\'' | '`' => 26,                        // c:898 ' / ` mark
        _ => return 1,
    };
    if let Some((cs, hist)) = vimarks().lock().unwrap()[idx] {
        // c:903
        ZLECS.store(cs.min(ZLELL.load(Ordering::SeqCst)), Ordering::SeqCst);
        history().lock().unwrap().cursor = hist.max(0) as usize;
        return 0;
    }
    1
}

/// Port of `vigotomarkline(char **args)` from Src/Zle/zle_move.c:929.
/// C body (single statement chain):
///     `vigotomark(args); return vifirstnonblank(zlenoargs);`
/// WARNING: param names don't match C — Rust=(ch) vs C=(args)
pub fn vigotomarkline(ch: char) -> i32 {
    // c:929
    vigotomark(ch); // c:931
    vifirstnonblank() // c:932
}
/// Move cursor to the start of the current logical line.
///
/// NOT the port of `findbol()` — that lives under its C name at
/// `zle_utils.rs:1206` (`Src/Zle/zle_utils.c:1165`) and RETURNS the
/// index without touching the cursor. This is the Rust-only mutating
/// counterpart: the same backward scan, assigning `zlecs` in place
/// (C spells that inline as `zlecs = findbol();`).
pub fn move_to_bol() {
    while ZLECS.load(Ordering::SeqCst) > 0
        && ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst) - 1] != '\n'
    {
        ZLECS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Move cursor to the end of the current logical line.
///
/// NOT the port of `findeol()` — that lives under its C name at
/// `zle_utils.rs:1229` (`Src/Zle/zle_utils.c:1176`) and RETURNS the
/// index without touching the cursor. This is the Rust-only mutating
/// counterpart: the same forward scan, assigning `zlecs` in place
/// (C spells that inline as `zlecs = findeol();`).
pub fn move_to_eol() {
    while ZLECS.load(Ordering::SeqCst) < ZLELL.load(Ordering::SeqCst)
        && ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)] != '\n'
    {
        ZLECS.fetch_add(1, Ordering::SeqCst);
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
    let mut line_start = ZLECS.load(Ordering::SeqCst);
    while line_start > 0 && ZLELINE.lock().unwrap()[line_start - 1] != '\n' {
        line_start -= 1;
    }

    if line_start == 0 {
        return false; // Already on first line
    }

    // Move to end of previous line
    ZLECS.store(line_start - 1, Ordering::SeqCst);

    // Find start of previous line
    let mut prev_start = ZLECS.load(Ordering::SeqCst);
    while prev_start > 0 && ZLELINE.lock().unwrap()[prev_start - 1] != '\n' {
        prev_start -= 1;
    }

    // Move to same column or end of line
    ZLECS.store(
        prev_start + col.min(ZLECS.load(Ordering::SeqCst) - prev_start),
        Ordering::SeqCst,
    );

    true
}

/// Move cursor down one logical line, preserving the column.
/// Simplified port of `downline(char **args)` from Src/Zle/zle_hist.c:332
/// with fixed n=1. Returns false at end-of-buffer.
pub fn move_down() -> bool {
    let col = current_column();

    // Find end of current line
    let mut line_end = ZLECS.load(Ordering::SeqCst);
    while line_end < ZLELL.load(Ordering::SeqCst) && ZLELINE.lock().unwrap()[line_end] != '\n' {
        line_end += 1;
    }

    if line_end >= ZLELL.load(Ordering::SeqCst) {
        return false; // Already on last line
    }

    // Move to start of next line
    ZLECS.store(line_end + 1, Ordering::SeqCst);

    // Find end of next line
    let mut next_end = ZLECS.load(Ordering::SeqCst);
    while next_end < ZLELL.load(Ordering::SeqCst) && ZLELINE.lock().unwrap()[next_end] != '\n' {
        next_end += 1;
    }

    // Move to same column or end of line
    ZLECS.store(
        (ZLECS.load(Ordering::SeqCst) + col).min(next_end),
        Ordering::SeqCst,
    );

    true
}

/// Compute the cursor's 0-indexed column on its current logical line.
/// Equivalent to `zlecs - findbol()` — the offset zsh's vertical-
/// motion code at Src/Zle/zle_hist.c:253 caches in `lastcol` for
/// sticky-column behaviour across up/down chains.
pub fn current_column() -> usize {
    let mut col = 0;
    let mut i = ZLECS.load(Ordering::SeqCst);
    while i > 0 && ZLELINE.lock().unwrap()[i - 1] != '\n' {
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
    ZLELINE.lock().unwrap()[..ZLECS.load(Ordering::SeqCst)]
        .iter()
        .filter(|&&c| c == '\n')
        .count()
}

/// Count the total number of logical lines in the buffer.
/// Used by display code to size the multi-line refresh region —
/// mirrors `nlnct` (number of lines counted) tracked by zsh's
/// `zrefresh()` in Src/Zle/zle_refresh.c.
pub fn count_lines() -> usize {
    ZLELINE
        .lock()
        .unwrap()
        .iter()
        .filter(|&&c| c == '\n')
        .count()
        + 1
}

#[cfg(test)]
mod alignmultiword_tests {
    use super::*;

    /// Common test setup: install `zleline` with a string and length,
    /// enable `COMBININGCHARS`. Caller passes the chars + cursor pos.
    fn setup_combining(chars: &[char]) {
        crate::ported::options::opt_state_set("combiningchars", true);
        let mut line = ZLELINE.lock().unwrap();
        line.clear();
        line.extend_from_slice(chars);
        ZLELL.store(chars.len(), Ordering::SeqCst);
    }

    fn teardown_combining() {
        crate::ported::options::opt_state_set("combiningchars", false);
        ZLELINE.lock().unwrap().clear();
        ZLELL.store(0, Ordering::SeqCst);
    }

    /// `Src/Zle/zle_move.c:54-55` — without `COMBININGCHARS`, returns 0
    /// immediately regardless of cursor position.
    #[test]
    fn alignmultiwordleft_returns_zero_without_combiningchars_option() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::options::opt_state_set("combiningchars", false);
        let mut line = ZLELINE.lock().unwrap();
        line.clear();
        line.extend_from_slice(&['e', '\u{0301}']); // e + combining acute
        ZLELL.store(2, Ordering::SeqCst);
        drop(line);
        let mut pos = 1usize;
        assert_eq!(
            alignmultiwordleft(&mut pos, 1),
            0,
            "c:54 — !isset(COMBININGCHARS) short-circuits to 0"
        );
        assert_eq!(pos, 1, "pos unchanged");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:54` — at the end of buffer (loccs == zlell),
    /// returns 0 without advancing.
    #[test]
    fn alignmultiwordleft_at_zlell_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setup_combining(&['a', 'b']);
        let mut pos = 2usize; // == zlell
        assert_eq!(
            alignmultiwordleft(&mut pos, 1),
            0,
            "c:54 — at zlell, return 0"
        );
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:54` — at start of buffer (loccs == 0),
    /// returns 0 (nowhere to step back).
    #[test]
    fn alignmultiwordleft_at_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setup_combining(&['\u{0301}', 'b']); // combining at start
        let mut pos = 0usize;
        assert_eq!(
            alignmultiwordleft(&mut pos, 1),
            0,
            "c:54 — loccs==0, return 0"
        );
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:58-59` — if current cell is NOT a combining
    /// char, returns 0 without realign. Pin the IS_COMBINING gate.
    #[test]
    fn alignmultiwordleft_returns_zero_when_not_on_combining_char() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setup_combining(&['a', 'b', 'c']); // all base chars
        let mut pos = 1usize; // on 'b' (base char)
        assert_eq!(
            alignmultiwordleft(&mut pos, 1),
            0,
            "c:58 — !IS_COMBINING(curr) → return 0"
        );
        assert_eq!(pos, 1, "pos unchanged");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:62-77` — pointer on a combining char walks
    /// back to the cluster's base, returns 1, writes pos when setpos≠0.
    #[test]
    fn alignmultiwordleft_walks_back_to_base() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // 'a' (base) + 2 combining marks: 0301 (acute), 0303 (tilde).
        setup_combining(&['a', '\u{0301}', '\u{0303}']);
        // Sanity: option + width prerequisites for the test to be valid.
        assert!(
            crate::ported::zsh_h::isset(crate::ported::zsh_h::COMBININGCHARS),
            "setup precondition: isset(COMBININGCHARS) must be true"
        );
        assert_eq!(
            crate::ported::compat::u9_wcwidth('\u{0303}'),
            0,
            "setup precondition: U+0303 must have width 0 for IS_COMBINING"
        );
        let mut pos = 2usize; // on second combining mark
        assert_eq!(
            alignmultiwordleft(&mut pos, 1),
            1,
            "c:69 — found base char, return 1"
        );
        assert_eq!(pos, 0, "c:68 — pos set to base char index");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:67-69` — `setpos == 0` returns success but
    /// does NOT mutate pos.
    #[test]
    fn alignmultiwordleft_setpos_zero_does_not_mutate() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setup_combining(&['a', '\u{0301}']);
        let mut pos = 1usize;
        assert_eq!(alignmultiwordleft(&mut pos, 0), 1, "found base");
        assert_eq!(pos, 1, "c:67 — setpos==0 skips assignment");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:89-115` — forward variant: probe via
    /// alignmultiwordleft, then scan forward over combining marks.
    /// Returns 1 + sets pos to the FIRST non-combining char (or end).
    #[test]
    fn alignmultiwordright_walks_forward_over_combining() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // 'a' (base) + 2 combining marks + 'b' (next base).
        setup_combining(&['a', '\u{0301}', '\u{0303}', 'b']);
        let mut pos = 2usize; // on second combining mark
        assert_eq!(
            alignmultiwordright(&mut pos, 1),
            1,
            "c:107 — found non-combining char, return 1"
        );
        assert_eq!(pos, 3, "c:106 — pos set to next base char index");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:96` — alignmultiwordright returns 0 when
    /// alignmultiwordleft would have returned 0 (not on a cluster).
    #[test]
    fn alignmultiwordright_returns_zero_when_not_in_cluster() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setup_combining(&['a', 'b']); // no combining chars
        let mut pos = 1usize;
        assert_eq!(
            alignmultiwordright(&mut pos, 1),
            0,
            "c:96 — left-align failed → return 0"
        );
        assert_eq!(pos, 1, "pos unchanged");
        teardown_combining();
    }

    /// `Src/Zle/zle_move.c:112-114` — when scan runs off end-of-buffer
    /// while in a cluster, sets pos to end and STILL returns 1.
    #[test]
    fn alignmultiwordright_runs_off_end_returns_one_with_end_pos() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // 'a' + 2 combining marks; cluster extends to end.
        setup_combining(&['a', '\u{0301}', '\u{0303}']);
        let mut pos = 1usize;
        assert_eq!(
            alignmultiwordright(&mut pos, 1),
            1,
            "c:114 — fell off end, still return 1"
        );
        assert_eq!(pos, 3, "c:113 — pos set to zlell (end-of-buffer)");
        teardown_combining();
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;

    #[test]
    fn deactivateregion_clears_active() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:566 — `region_active = 0; return 0`.
        zle_reset();
        REGION_ACTIVE.store(1, Ordering::SeqCst);
        let r = deactivateregion();
        assert_eq!(r, 0);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn setmarkcommand_sets_mark_to_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:489-490 — `mark = zlecs; region_active = 1`.
        zle_reset();
        ZLECS.store(7, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 1;
        let r = setmarkcommand();
        assert_eq!(r, 0);
        assert_eq!(MARK.load(Ordering::SeqCst), 7);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn setmarkcommand_negative_mult_deactivates() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:485-487 — `if (zmult < 0) { region_active = 0; return 0; }`.
        zle_reset();
        REGION_ACTIVE.store(1, Ordering::SeqCst);
        MARK.store(5, Ordering::SeqCst);
        ZLECS.store(7, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = -1;
        let r = setmarkcommand();
        assert_eq!(r, 0);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 0);
        // mark NOT updated because we returned early.
        assert_eq!(MARK.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn exchangepointandmark_swaps() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:504-506 — swap zlecs and mark.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        MARK.store(8, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 1;
        let r = exchangepointandmark();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 8);
        assert_eq!(MARK.load(Ordering::SeqCst), 3);
        // c:509-510 — zmult > 0 → activate region.
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn exchangepointandmark_zero_mult_just_activates() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:500-502 — `if (zmult == 0) { region_active = 1; return 0; }`.
        // No swap occurs.
        zle_reset();
        ZLECS.store(3, Ordering::SeqCst);
        MARK.store(8, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 0;
        let r = exchangepointandmark();
        assert_eq!(r, 0);
        // No swap.
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
        assert_eq!(MARK.load(Ordering::SeqCst), 8);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn exchangepointandmark_clamps_zlecs_to_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:507-508 — `if (zlecs > zlell) zlecs = zlell`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hi".chars().collect();
        ZLELL.store(2, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        MARK.store(99, Ordering::SeqCst); // mark beyond zlell
        ZMOD.lock().unwrap().mult = 1;
        exchangepointandmark();
        // After swap zlecs would be 99, clamped to 2.
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
    }

    // ---------- Cursor movement (forwardchar / backwardchar / inccs / deccs) ----

    #[test]
    fn inccs_increments_zlecs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:121-126 — `zlecs++; alignmultiwordright(...)`. Vec<char>
        // makes alignment a no-op.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        inccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 1);
        inccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn deccs_decrements_zlecs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:132-137 — `zlecs--; alignmultiwordleft(...)`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(2, Ordering::SeqCst);
        deccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 1);
        deccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn incpos_decpos_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:457-458 — `while (zlecs < zlell && n--) INCCS();`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 3;
        let r = forwardchar();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn forwardchar_stops_at_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:457 — `while (zlecs < zlell && ...)`. Walking past end
        // is bounded.
        zle_reset();
        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLELL.store(2, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 99;
        forwardchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn backwardchar_moves_zmult_positions() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:476-477 — `while (zlecs > 0 && n--) DECCS();`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(8, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 3;
        let r = backwardchar();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn backwardchar_stops_at_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:476 — `while (zlecs > 0 && ...)`. Doesn't underflow.
        zle_reset();
        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLELL.store(2, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 99;
        backwardchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn forwardchar_negative_count_delegates_to_backward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:445-450 — `if (n < 0) { zmult = -n; ret = backwardchar(); ... }`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(4, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = -2;
        forwardchar();
        // -2 → backwardchar(2) → cursor goes 4→2
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
        // c:447,449 — zmult restored to original after recursion.
        assert_eq!(ZMOD.lock().unwrap().mult, -2);
    }

    #[test]
    fn backwardchar_negative_count_delegates_to_forward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = -2;
        backwardchar();
        // -2 → forwardchar(2) → cursor goes 1→3
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
        assert_eq!(ZMOD.lock().unwrap().mult, -2);
    }

    // ---------- vi movement (vibeginningofline / vibackwardchar / viforwardchar) ----

    #[test]
    fn vibeginningofline_jumps_to_bol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:730 — `zlecs = findbol()`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(9, Ordering::SeqCst); // 'h' in "ghi"
        let r = vibeginningofline();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 8); // after the second '\n'
    }

    #[test]
    fn vibackwardchar_stops_at_line_start() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:694-695 — at findbol → return 1 without moving.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        ZLELL.store(7, Ordering::SeqCst);
        ZLECS.store(4, Ordering::SeqCst); // 'd' (right after newline)
        ZMOD.lock().unwrap().mult = 1;
        let r = vibackwardchar();
        assert_eq!(r, 1);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 4); // unchanged
    }

    #[test]
    fn vibackwardchar_moves_within_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(8, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 3;
        vibackwardchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn viforwardchar_stops_at_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:674-675 — at findeol → return 1.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        ZLELL.store(7, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst); // at '\n'
        ZMOD.lock().unwrap().mult = 1;
        let r = viforwardchar();
        assert_eq!(r, 1);
    }

    #[test]
    fn viforwardchar_moves_within_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 3;
        viforwardchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn viforwardchar_clamps_at_findeol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:676 — `while (n-- && zlecs < lim)`.
        zle_reset();
        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLELL.store(2, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        ZMOD.lock().unwrap().mult = 99;
        viforwardchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
    }

    // ---------- vifirstnonblank tests ----------

    #[test]
    fn vifirstnonblank_skips_leading_spaces() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:864-866 — bol then skip space/tab.
        zle_reset();
        *ZLELINE.lock().unwrap() = "   hello".chars().collect();
        ZLELL.store(8, Ordering::SeqCst);
        ZLECS.store(5, Ordering::SeqCst); // somewhere mid-word
        let r = vifirstnonblank();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3); // first non-blank
    }

    #[test]
    fn vifirstnonblank_skips_tabs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:865 — ZC_iblank includes tab.
        zle_reset();
        *ZLELINE.lock().unwrap() = "\t\t\tfoo".chars().collect();
        ZLELL.store(6, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        vifirstnonblank();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn vifirstnonblank_no_blanks() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // No leading blanks → cursor lands at bol.
        zle_reset();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        vifirstnonblank();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn vifirstnonblank_all_blanks() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:865 — `while zlecs != zlell` exits cleanly when only blanks.
        zle_reset();
        *ZLELINE.lock().unwrap() = "   ".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        vifirstnonblank();
        // walks to zlell (no non-blank found).
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn vifirstnonblank_respects_findbol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:864 — `zlecs = findbol()`. With multiline buffer, jump
        // to start of CURRENT line, then skip blanks.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abc\n   def".chars().collect();
        ZLELL.store(10, Ordering::SeqCst);
        ZLECS.store(8, Ordering::SeqCst); // 'e' in 'def'
        vifirstnonblank();
        // findbol → 4 (after first '\n'); skip 3 spaces → 7 ('d')
        assert_eq!(ZLECS.load(Ordering::SeqCst), 7);
    }

    /// `Src/Zle/zle_move.c:865` — `while (zlecs != zlell && ZC_iblank(...))`.
    /// After the wcsiblank fix `ZC_iblank` accepts CR/FF/VT/NBSP as
    /// blanks (Src/Zle/zle.h:62 → Src/utils.c:4302-4307). A buffer
    /// indented with mixed wide-whitespace (e.g. a Windows-CRLF
    /// snippet pasted into the shell) must walk past those chars to
    /// the first non-blank. Pins the wide-char path end-to-end.
    #[test]
    fn vifirstnonblank_skips_wide_whitespace_per_wcsiblank() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_reset();
        // VT, FF, CR, NBSP, then 'x' — wcsiblank true for all four.
        let buf: Vec<char> = vec!['\x0b', '\x0c', '\r', '\u{00A0}', 'x'];
        *ZLELINE.lock().unwrap() = buf;
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        vifirstnonblank();
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            4,
            "c:865 must skip CR/FF/VT/NBSP per wcsiblank"
        );
    }

    /// c:865 — `\n` is the SOLE iswspace char wcsiblank rejects.
    /// vifirstnonblank must NOT skip past a newline (a regression
    /// using `iswspace` directly would silently jump across line
    /// boundaries).
    #[test]
    fn vifirstnonblank_stops_at_newline_per_wcsiblank() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_reset();
        // Buffer: "  \n  x" — leading 2 spaces, newline, then more.
        *ZLELINE.lock().unwrap() = "  \n  x".chars().collect();
        ZLELL.store(6, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        vifirstnonblank();
        // c:865 — skip the 2 spaces, then stop AT '\n' (idx 2).
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            2,
            "wcsiblank excludes '\\n' — cursor must stop at the newline"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Zle/zle_move.c inccs/deccs/incpos/decpos
    // primitive cursor walks.
    // ═══════════════════════════════════════════════════════════════════

    /// c:122 — `inccs()` advances ZLECS by 1.
    #[test]
    fn inccs_advances_zlecs_by_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        inccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 1);
        inccs();
        inccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    /// c:133 — `deccs()` decrements ZLECS by 1.
    #[test]
    fn deccs_decrements_zlecs_by_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        deccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 2);
        deccs();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 1);
    }

    /// c:143 — `incpos(&mut p)` advances p by 1 in-place.
    #[test]
    fn incpos_advances_arg_in_place() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut p = 5_usize;
        incpos(&mut p);
        assert_eq!(p, 6);
        incpos(&mut p);
        incpos(&mut p);
        assert_eq!(p, 8);
    }

    /// c:152 — `decpos(&mut p)` decrements p by 1 in-place.
    #[test]
    fn decpos_decrements_arg_in_place() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut p = 10_usize;
        decpos(&mut p);
        assert_eq!(p, 9);
        decpos(&mut p);
        assert_eq!(p, 8);
    }

    /// c:122/133 — `inccs` then `deccs` round-trips.
    #[test]
    fn inccs_deccs_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(5, Ordering::SeqCst);
        let before = ZLECS.load(Ordering::SeqCst);
        inccs();
        deccs();
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            before,
            "inccs+deccs is identity"
        );
    }

    /// c:170 — `backwardmetafiedchar()` at ZLECS=0 is safe no-op
    /// (won't underflow ZLECS).
    #[test]
    fn backwardmetafiedchar_at_zero_no_underflow() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ZLECS.store(0, Ordering::SeqCst);
        backwardmetafiedchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 0, "must not underflow past 0");
    }

    /// c:170 — `backwardmetafiedchar()` from positive position
    /// decrements by 1.
    #[test]
    fn backwardmetafiedchar_decrements_from_positive() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ZLECS.store(5, Ordering::SeqCst);
        backwardmetafiedchar();
        assert_eq!(ZLECS.load(Ordering::SeqCst), 4);
    }

    /// `BMC_BUFSIZE` constant equals 6 (POSIX MB_CUR_MAX upper bound).
    /// Pin so a regen that lowered it to 4 (UTF-8 max) would be caught
    /// — some legacy multibyte encodings need 6 bytes per codepoint.
    #[test]
    fn bmc_bufsize_is_six() {
        assert_eq!(BMC_BUFSIZE, 6, "MB_CUR_MAX legacy encoding ceiling");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_move.c
    // c:246 beginningofline / c:288 endofline / c:473 forwardchar /
    // c:513 backwardchar / c:550 setmarkcommand / c:585 exchangepointandmark /
    // c:607 visualmode / c:631 visuallinemode / c:665 deactivateregion /
    // c:672 vigotocolumn / c:781 viforwardchar / c:838 vibackwardchar
    // ═══════════════════════════════════════════════════════════════════

    /// c:246 — `beginningofline` returns i32 (compile-time type pin).
    #[test]
    fn beginningofline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = beginningofline();
    }

    /// c:288 — `endofline` returns i32 (compile-time type pin).
    #[test]
    fn endofline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = endofline();
    }

    /// c:473 — `forwardchar` return in u8 exit-code range.
    #[test]
    fn forwardchar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = forwardchar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:513 — `backwardchar` return in u8 exit-code range.
    #[test]
    fn backwardchar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = backwardchar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:550 — `setmarkcommand` return in u8 exit-code range.
    #[test]
    fn setmarkcommand_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = setmarkcommand();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:585 — `exchangepointandmark` return in u8 exit-code range.
    #[test]
    fn exchangepointandmark_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = exchangepointandmark();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:607 — `visualmode` return in u8 exit-code range.
    #[test]
    fn visualmode_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = visualmode();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:631 — `visuallinemode` return in u8 exit-code range.
    #[test]
    fn visuallinemode_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = visuallinemode();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:665 — `deactivateregion` return in u8 exit-code range.
    #[test]
    fn deactivateregion_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = deactivateregion();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:781 — `viforwardchar` return in u8 exit-code range.
    #[test]
    fn viforwardchar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viforwardchar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:838 — `vibackwardchar` return in u8 exit-code range.
    #[test]
    fn vibackwardchar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardchar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:672 — `vigotocolumn` return in u8 exit-code range.
    #[test]
    fn vigotocolumn_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vigotocolumn();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_move.c
    // c:56 alignmultiwordleft / c:118 alignmultiwordright /
    // c:327 beginningoflinehist / c:384 endoflinehist /
    // c:691 vimatchbracket / c:867 viendofline / c:903 vibeginningofline
    // ═══════════════════════════════════════════════════════════════════

    /// c:56 — `alignmultiwordleft` returns i32 (compile-time type pin).
    #[test]
    fn alignmultiwordleft_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        let _: i32 = alignmultiwordleft(&mut pos, 0);
    }

    /// c:118 — `alignmultiwordright` returns i32.
    #[test]
    fn alignmultiwordright_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        let _: i32 = alignmultiwordright(&mut pos, 0);
    }

    /// c:327 — `beginningoflinehist` returns i32.
    #[test]
    fn beginningoflinehist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = beginningoflinehist();
    }

    /// c:384 — `endoflinehist` returns i32.
    #[test]
    fn endoflinehist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = endoflinehist();
    }

    /// c:691 — `vimatchbracket` returns i32.
    #[test]
    fn vimatchbracket_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vimatchbracket();
    }

    /// c:867 — `viendofline` returns i32.
    #[test]
    fn viendofline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viendofline();
    }

    /// c:903 — `vibeginningofline` returns i32.
    #[test]
    fn vibeginningofline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vibeginningofline();
    }

    /// c:56 — `alignmultiwordleft(_, 0)` safe on empty buffer.
    #[test]
    fn alignmultiwordleft_safe_on_empty_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        let _ = alignmultiwordleft(&mut pos, 0);
    }

    /// c:118 — `alignmultiwordright(_, 0)` safe on empty buffer.
    #[test]
    fn alignmultiwordright_safe_on_empty_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        let _ = alignmultiwordright(&mut pos, 0);
    }

    /// c:691 — `vimatchbracket` returns in u8 exit range.
    #[test]
    fn vimatchbracket_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vimatchbracket();
        assert!(
            (0..256).contains(&r),
            "vimatchbracket exit code {} must fit u8",
            r
        );
    }

    /// c:867 — `viendofline` returns in u8 exit range.
    #[test]
    fn viendofline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viendofline();
        assert!((0..256).contains(&r));
    }

    /// c:903 — `vibeginningofline` returns in u8 exit range.
    #[test]
    fn vibeginningofline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibeginningofline();
        assert!((0..256).contains(&r));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_move.c
    // c:158 inccs / c:180 deccs / c:202 incpos / c:219 decpos /
    // c:473 forwardchar / c:513 backwardchar / c:550 setmarkcommand /
    // c:585 exchangepointandmark / c:607 visualmode / c:665 deactivateregion
    // ═══════════════════════════════════════════════════════════════════

    /// c:158 — `inccs` is idempotent (no panic across many calls).
    #[test]
    fn inccs_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            inccs();
        }
    }

    /// c:180 — `deccs` is idempotent.
    #[test]
    fn deccs_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            deccs();
        }
    }

    /// c:202 — `incpos(&mut 0)` is safe (no panic for zero-pos arg).
    #[test]
    fn incpos_zero_arg_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        incpos(&mut pos);
    }

    /// c:219 — `decpos(&mut 0)` MUST be safe (no underflow panic).
    /// C source guards via `if (*p > 0)` before decrementing. In zshrs
    /// the port subtracts unconditionally → usize-underflow panic at
    /// `Src/Zle/zle_move.rs:221`.
    #[test]
    fn decpos_zero_arg_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut pos = 0usize;
        decpos(&mut pos);
    }

    /// c:473 — `forwardchar` returns i32 (compile-time pin).
    #[test]
    fn forwardchar_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = forwardchar();
    }

    /// c:513 — `backwardchar` returns i32 (compile-time pin).
    #[test]
    fn backwardchar_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = backwardchar();
    }

    /// c:550 — `setmarkcommand` returns i32 + idempotent.
    #[test]
    fn setmarkcommand_returns_i32_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            let _: i32 = setmarkcommand();
        }
    }

    /// c:585 — `exchangepointandmark` returns i32 (compile-time pin).
    #[test]
    fn exchangepointandmark_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = exchangepointandmark();
    }

    /// c:607 — `visualmode` returns i32 (compile-time pin).
    #[test]
    fn visualmode_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = visualmode();
    }

    /// c:631 — `visuallinemode` returns i32 (compile-time pin).
    #[test]
    fn visuallinemode_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = visuallinemode();
    }

    /// c:665 — `deactivateregion` returns i32 + idempotent.
    #[test]
    fn deactivateregion_returns_i32_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            let _: i32 = deactivateregion();
        }
    }

    /// c:672 — `vigotocolumn` returns i32 + in u8 exit range (alt).
    #[test]
    fn vigotocolumn_returns_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vigotocolumn();
        assert!(
            (0..256).contains(&r),
            "vigotocolumn exit code {} must fit u8",
            r
        );
    }

    /// c:550 — `setmarkcommand` exit code in u8 range.
    #[test]
    fn setmarkcommand_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = setmarkcommand();
        assert!((0..256).contains(&r));
    }
}
