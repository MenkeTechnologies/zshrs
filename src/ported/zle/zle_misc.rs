//! ZLE miscellaneous operations
//!
//! Direct port from zsh/Src/Zle/zle_misc.c
//!
//! Implements misc editing widgets:
//! - self-insert, self-insert-unmeta
//! - accept-line, accept-and-hold
//! - quoted-insert, bracketed-paste
//! - delete-char, backward-delete-char
//! - kill-line, backward-kill-line, kill-buffer, kill-whole-line
//! - copy-region-as-kill, kill-region
//! - yank, yank-pop
//! - transpose-chars, bslashquote-line, bslashquote-region
//! - what-cursor-position, universal-argument, digit-argument
//! - undefined-key, send-break
//! - vi-put-after, vi-put-before, overwrite-mode

// Bulk-import the most-used zle_main statics + helpers so the bodies
// below stay close to the C source visually instead of being drowned
// in fully-qualified `crate::ported::zle::zle_main::*` paths. MARK
// excluded: this file has its own `pub static MARK` at line 1610
// (a duplicate that pre-existed this cleanup — separate fix).
use crate::ported::zle::compcore::{
    LASTCHAR, ZLECS as ZLECS_C, ZLELINE as ZLELINE_C, ZLELL as ZLELL_C,
};
use crate::ported::zle::zle_main::{
    vibuf, zle_reset, INSMODE, KILLRING, KILLRINGMAX, LASTCHAR_WIDE, LASTCHAR_WIDE_VALID, MARK,
    MULT, NEG_ARG, PREFIXFLAG, REGION_ACTIVE, YANKB, YANKE, ZLECS, ZLELINE, ZLELL,
    ZLE_RESET_NEEDED, ZMOD,
};
use std::io::Read;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::ported::builtins::sched::zleactive;
use crate::ported::utils::{errflag, quotestring};
use crate::ported::zle::complete::INCOMPFUNC;
use crate::ported::zle::zle_move::{deccs, decpos, inccs, incpos, vifirstnonblank};
use crate::ported::zle::zle_utils::{findbol, findeol, shiftchars, spaceinline};
use crate::ported::zle::zle_vi::startvichange;
use crate::ported::zsh_h::{isset, ERRFLAG_ERROR, ERRFLAG_INT, KSHARRAYS, QT_SINGLE_OPTIONAL};
use crate::zle::zle_h::{MOD_MULT, MOD_NEG, MOD_NULL, MOD_TMULT};

// =====================================================================
// Globals — `Src/Zle/zle_main.c:79-84` (live in zle_main but consumed
// by widgets in zle_misc).
// =====================================================================

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_move::*, zle_params::*,
    zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Port of `int done` from `Src/Zle/zle_main.c:79`. Non-zero when
/// the editor session should terminate (`accept-line`,
/// `accept-and-hold`, `accept-line-and-down-history`, etc.).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]

/// Port of `doinsert()` from `Src/Zle/zle_misc.c:37`.
/// C decl: `doinsert(ZLE_STRING_T zstr, int len)`
/// ```c
/// mod_export void
/// doinsert(ZLE_STRING_T zstr, int len) {
///     ...
///     m = abs(zmult); count = m * len;
///     ...insert m copies of zstr at cursor (or after, if zmult < 0)...
/// }
/// ```
/// Insert `zstr` `|zmod.mult|` times at the cursor. Negative count
/// inserts AFTER the cursor (cursor stays put). Honors INSMODE
/// (overwrite mode) by replacing existing chars instead of inserting
/// when INSMODE==0 and the cursor isn't on a newline. Fires
/// `iremovesuffix` + `invalidatelist` at entry per C c:47-48.
/// WARNING: param names don't match C — Rust=(zle, zstr) vs C=(zstr, len)
pub fn doinsert(zstr: &[char]) {
    // c:37
    // c:47 — `iremovesuffix(c1, 0)`. Strip pending menu-suffix first.
    if let Some(&c1) = zstr.first() {
        iremovesuffix(c1 as i32, 0);
    }
    // c:48 — `invalidatelist()`.
    invalidatelist();

    let zmult_val = ZMOD.lock().unwrap().mult;
    let neg = zmult_val < 0;
    let m = zmult_val.unsigned_abs() as usize;
    let len = zstr.len();
    let total = m * len;
    let cs = ZLECS.load(SeqCst);
    let insmode = INSMODE.load(SeqCst);
    let at_newline = ZLELINE.lock().unwrap().get(cs).copied() == Some('\n');

    // c:51-101 — overwrite-mode branch: replace existing chars up to
    // the count or next newline; insert any remaining slack.
    let overwrite = insmode == 0 && !at_newline;
    if overwrite {
        // c:82-86 — walk to end pos: cs + count or first newline.
        let mut pos = cs;
        let mut i = total;
        let ll = ZLELL.load(SeqCst);
        while pos < ll && i > 0 {
            let ch = ZLELINE.lock().unwrap().get(pos).copied();
            if ch == Some('\n') {
                break;
            }
            pos += 1;
            i -= 1;
        }
        // c:90-100 — diff = pos - cs - m * len.
        let diff = pos as i32 - cs as i32 - total as i32;
        if diff < 0 {
            // c:92 — `spaceinline(-diff)` — opens slack we still need.
            spaceinline(-diff);
        } else if diff > 0 {
            // c:99 — `shiftchars(zlecs, diff)` — surplus collapses.
            shiftchars(cs as i32, diff);
        }
    } else {
        // c:52 — `spaceinline(m * len)`: pure insert.
        spaceinline(total as i32);
    }
    // c:102-104 — `while (m--) for (s=zstr; count; s++, count--)
    //              zleline[zlecs++] = *s;` — write chars + advance cs.
    {
        let mut line = ZLELINE.lock().unwrap();
        let mut wcs = cs;
        for _ in 0..m {
            for &c in zstr.iter() {
                if wcs < line.len() {
                    line[wcs] = c;
                } else {
                    line.push(c);
                }
                wcs += 1;
            }
        }
        let new_ll = line.len();
        drop(line);
        ZLELL.store(new_ll, SeqCst);
        ZLECS.store(wcs, SeqCst);
    }
    // c:105-106 — `if (neg) zlecs += zmult * len;` (already past
    // inserted span; back up).
    if neg {
        let offset = (zmult_val * len as i32) as i64;
        let new_cs = (ZLECS.load(SeqCst) as i64 + offset).max(0) as usize;
        ZLECS.store(new_cs, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Direct port of `int selfinsert(char **args)` from
/// `Src/Zle/zle_misc.c:112-126`.
/// ```c
/// if (!lastchar_wide_valid)
///     getrestchar(lastchar, NULL);
/// // tmp = LASTFULLCHAR;
/// doinsert(&tmp, 1);
/// return 0;
/// ```
///
/// **Multibyte tradeoff:** C's `getrestchar` reassembles a wide
/// char from `lastchar` + buffered continuation bytes when the
/// `wide_valid` flag is clear. Rust's `getfullchar` (zle_main
/// .rs:730) already produces a full char per read, so by the time
/// `selfinsert` fires, `lastchar` IS the full codepoint — the
/// `wide_valid=false` branch is unreachable in the Rust input path
/// and the ASCII-promotion is the correct fallback for the rare
/// case where a widget sets `lastchar` directly.
/// Port of `selfinsert()` from `Src/Zle/zle_misc.c:113`.
/// C decl: `selfinsert(UNUSED(char **args))`
/// C body (4 lines under MULTIBYTE):
///   `if (!lastchar_wide_valid && getrestchar(lastchar,...) == WEOF) return 1;
///    tmp = LASTFULLCHAR;
///    doinsert(&tmp, 1);
///    return 0;`
pub fn selfinsert(_args: &[String]) -> i32 {
    // c:113
    // c:117-122 — MULTIBYTE_SUPPORT block. When `lastchar_wide_valid`
    // is false, the C code calls `getrestchar(lastchar, NULL, NULL)`
    // to re-assemble a wide char from `lastchar` + buffered
    // continuation bytes; on WEOF (zshrs port: -1), C returns 1
    // immediately. Previous Rust port silently faked this branch by
    // promoting `lastchar` straight into `lastchar_wide` and
    // marking it valid — that suppressed the C error path and
    // left invalid byte sequences as if they'd round-tripped
    // successfully.
    if LASTCHAR_WIDE_VALID.load(SeqCst) == 0 {
        let lc = LASTCHAR.load(SeqCst);
        if crate::ported::zle::zle_main::getrestchar(lc) == -1 {
            // c:121 — `if (getrestchar(...) == WEOF) return 1`.
            return 1;
        }
    }
    // c:123 — `tmp = LASTFULLCHAR;` where
    // `#define LASTFULLCHAR lastchar_wide` (zle.h).
    let tmp = LASTCHAR_WIDE.load(SeqCst);
    // c:124 — `doinsert(&tmp, 1);` insert a single wide char.
    // Rust `doinsert(&[char])` requires a valid `char`; an
    // invalid codepoint (surrogate / > 0x10FFFF) falls back to
    // U+FFFD REPLACEMENT CHARACTER so the insert still happens —
    // C would have written the raw int into the buffer, which
    // Rust strings can't represent.
    let ch = char::from_u32(tmp as u32).unwrap_or('\u{FFFD}');
    doinsert(&[ch]);
    0 // c:125
}

/// Port of `fixunmeta()` from Src/Zle/zle_misc.c:130.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn fixunmeta() {
    // c:130
    // c:130 — `lastchar &= 0x7f`. Strip Meta/high bit.
    LASTCHAR.fetch_and((0x7f) as i32, SeqCst);
    // c:133-134 — `if (lastchar == '\\r') lastchar = '\\n'`.
    if LASTCHAR.load(SeqCst) == b'\r' as i32 {
        LASTCHAR.store((b'\n' as i32) as i32, SeqCst);
    }
    // c:140 — `lastchar_wide = (ZLE_INT_T)lastchar`. Sync wide.
    LASTCHAR_WIDE.store((LASTCHAR.load(SeqCst)) as i32, SeqCst);
    LASTCHAR_WIDE_VALID.store(1, SeqCst);
}

/// Port of `selfinsertunmeta()` from `Src/Zle/zle_misc.c:149`.
/// C decl: `selfinsertunmeta(char **args)`
pub fn selfinsertunmeta(args: &[String]) -> i32 {
    // c:149
    // c:151-152 — `fixunmeta(); return selfinsert(args)`. Args
    // pass through to mirror the C call.
    fixunmeta();
    selfinsert(args)
}

/// Port of `deletechar()` from `Src/Zle/zle_misc.c:157`.
/// C decl: `deletechar(char **args)`
pub fn deletechar() -> i32 {
    // c:157
    // c:157-166 — `if (zmult < 0) { negate, recurse to backward,
    //               restore zmult, return ret }`.
    let mut n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = backwarddeletechar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    n = ZMOD.lock().unwrap().mult;
    // c:169-173 — `while (n--) { if (zlecs == zlell) return 1; INCCS() }`.
    while n > 0 {
        if ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
            return 1;
        }
        inccs();
        n -= 1;
    }
    // c:174 — `backdel(zmult, 0)`. Method deletechar does forward.
    let count = ZMOD.lock().unwrap().mult.max(0) as usize;
    for _ in 0..count {
        if ZLECS.load(SeqCst) > 0 {
            ZLECS.fetch_sub(1, SeqCst);
            if ZLECS.load(SeqCst) < ZLELINE.lock().unwrap().len() {
                ZLELINE.lock().unwrap().remove(ZLECS.load(SeqCst));
                ZLELL.fetch_sub(1, SeqCst);
            }
        }
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:175
}

/// Port of `backwarddeletechar()` from `Src/Zle/zle_misc.c:180`.
/// C decl: `backwarddeletechar(char **args)`
pub fn backwarddeletechar() -> i32 {
    // c:180
    // c:180-188 — `if (zmult < 0) { negate, recurse to forward,
    //               restore zmult, return ret }`.
    let n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = deletechar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    // c:189 — `backdel(zmult > zlecs ? zlecs : zmult, 0)`.
    let count = (n as usize).min(ZLECS.load(SeqCst));
    for _ in 0..count {
        if ZLECS.load(SeqCst) > 0 {
            ZLECS.fetch_sub(1, SeqCst);
            ZLELINE.lock().unwrap().remove(ZLECS.load(SeqCst));
            ZLELL.fetch_sub(1, SeqCst);
        }
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:190
}

/// Port of `killwholeline()` from `Src/Zle/zle_misc.c:195`.
/// C decl: `killwholeline(UNUSED(char **args))`
pub fn killwholeline() -> i32 {
    // c:195
    let mut n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        // c:199
        return 1; // c:200
    }
    while n > 0 {
        // c:201
        // c:202-203 — last-line edge: at zlell with non-empty buffer
        // step back one so the trailing '\n' belongs to this line.
        let _fg = ZLECS.load(SeqCst) > 0 && ZLECS.load(SeqCst) == ZLELL.load(SeqCst);
        if _fg {
            ZLECS.fetch_sub(1, SeqCst);
        }
        // c:204-205 — walk back to bol.
        while ZLECS.load(SeqCst) > 0 && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] != '\n' {
            ZLECS.fetch_sub(1, SeqCst);
        }
        // c:206 — `for (i=zlecs; i!=zlell && zleline[i]!='\n'; i++)`.
        let mut i = ZLECS.load(SeqCst);
        while i != ZLELL.load(SeqCst) && ZLELINE.lock().unwrap()[i] != '\n' {
            i += 1;
        }
        // c:207 — `forekill(i - zlecs + (i != zlell), fg ?
        // (CUT_FRONT|CUT_RAW) : CUT_RAW);` — include the trailing '\n'
        // if there is one; kill routes through cuttext (CUTBUF).
        let drop = i - ZLECS.load(SeqCst) + (if i != ZLELL.load(SeqCst) { 1 } else { 0 });
        forekill(drop as i32, if _fg { CUT_FRONT | CUT_RAW } else { CUT_RAW });
        n -= 1;
    }
    CLEARLIST.store(1, SeqCst); // c:209 `clearlist = 1;`
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:210
}

/// Port of `killbuffer()` from `Src/Zle/zle_misc.c:215`.
/// C decl: `killbuffer(UNUSED(char **args))`
/// C body (4 lines):
///   `zlecs = 0; forekill(zlell, CUT_RAW); clearlist = 1; return 0;`
pub fn killbuffer() -> i32 {
    // c:215
    ZLECS.store(0, SeqCst); // c:217
    let zlell = ZLELL.load(SeqCst) as i32;
    forekill(zlell, CUT_RAW); // c:218
    CLEARLIST.store(1, SeqCst); // c:219
    0 // c:220
}

/// Port of `backwardkillline()` from `Src/Zle/zle_misc.c:225`.
/// C decl: `backwardkillline(char **args)`
pub fn backwardkillline() -> i32 {
    // c:225
    // c:225-234 — `if (n < 0) { negate, recurse killline, restore }`.
    let n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        ZMOD.lock().unwrap().mult = -n;
        let ret = killline();
        ZMOD.lock().unwrap().mult = n;
        return ret;
    }
    let mut nn = n;
    let mut i = 0_usize;
    // c:236-242 — walk back; '\n' on the LEFT bumps zlecs--, i++.
    while nn > 0 {
        if ZLECS.load(SeqCst) > 0 && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] == '\n' {
            ZLECS.fetch_sub(1, SeqCst);
            i += 1;
        } else {
            while ZLECS.load(SeqCst) > 0 && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] != '\n'
            {
                ZLECS.fetch_sub(1, SeqCst);
                i += 1;
            }
        }
        nn -= 1;
    }
    // c:243 — `forekill(i, CUT_FRONT|CUT_RAW);` — kill routes through
    // cuttext (CUTBUF, FRONT semantics prepend to the current cut).
    forekill(i as i32, CUT_FRONT | CUT_RAW);
    CLEARLIST.store(1, SeqCst); // c:244 `clearlist = 1;`
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:245
}

/// Port of `transpose_swap()` from `Src/Zle/zle_misc.c:255`.
/// C decl: `transpose_swap(int start, int middle, int end)`
/// ```c
/// static void
/// transpose_swap(int start, int middle, int end)
/// {
///     int len1, len2;
///     ZLE_STRING_T first;
///     len1 = middle - start;
///     len2 = end - middle;
///     first = (ZLE_STRING_T)zalloc(len1 * ZLE_CHAR_SIZE);
///     ZS_memcpy(first, zleline + start, len1);
///     /* Move may be overlapping... */
///     ZS_memmove(zleline + start, zleline + middle, len2);
///     ZS_memcpy(zleline + start + len2, first, len1);
///     zfree(first, len1 * ZLE_CHAR_SIZE);
/// }
/// ```
/// Swap two adjacent slices in the line buffer:
/// `zleline[start..middle]` and `zleline[middle..end]`. After the
/// swap, `zleline[start..start+(end-middle)]` holds the second
/// chunk and `zleline[start+(end-middle)..end]` holds the first.
/// WARNING: param names don't match C — Rust=(zle, start, middle, end) vs C=(start, middle, end)
pub fn transpose_swap(start: usize, middle: usize, end: usize) {
    // c:255
    let len1 = middle - start; // c:255
    let len2 = end - middle; // c:261
                             // c:263-264 — copy first slice into temp buffer.
    let first: Vec<char> = ZLELINE.lock().unwrap()[start..middle].to_vec();
    // c:266 — `ZS_memmove(zleline + start, zleline + middle, len2)`.
    // Vec doesn't overlap when copy_within is used.
    ZLELINE.lock().unwrap().copy_within(middle..end, start);
    // c:267 — `ZS_memcpy(zleline + start + len2, first, len1)`.
    for (i, &ch) in first.iter().enumerate() {
        ZLELINE.lock().unwrap()[start + len2 + i] = ch;
    }
    let _ = len1;
}

/// Port of `gosmacstransposechars()` from `Src/Zle/zle_misc.c:274`.
/// C decl: `gosmacstransposechars(UNUSED(char **args))`
pub fn gosmacstransposechars() -> i32 {
    // c:274
    // C body (c:276-307): gosmacs-style: transpose char before cursor
    // with char at cursor; advance cursor. Skips through newlines and
    // multi-byte combining chars.
    if ZLECS.load(SeqCst) < 2 || ZLECS.load(SeqCst) > ZLELL.load(SeqCst) {
        // Edge: try to advance past initial newline so we can transpose.
        let twice = ZLECS.load(SeqCst) == 0
            || ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(SeqCst).saturating_sub(1))
                == Some(&'\n');
        if ZLECS.load(SeqCst) >= ZLELL.load(SeqCst)
            || ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)) == Some(&'\n')
        {
            return 1;
        }
        ZLECS.fetch_add(1, SeqCst);
        if twice {
            if ZLECS.load(SeqCst) >= ZLELL.load(SeqCst)
                || ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)) == Some(&'\n')
            {
                return 1;
            }
            ZLECS.fetch_add(1, SeqCst);
        }
    }
    if ZLECS.load(SeqCst) >= 2 && ZLECS.load(SeqCst) <= ZLELINE.lock().unwrap().len() {
        ZLELINE
            .lock()
            .unwrap()
            .swap(ZLECS.load(SeqCst) - 2, ZLECS.load(SeqCst) - 1);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
    0
}

/// Port of `transposechars()` from `Src/Zle/zle_misc.c:313`.
/// C decl: `transposechars(UNUSED(char **args))`
pub fn transposechars() -> i32 {
    // c:313
    let mut n = ZMOD.lock().unwrap().mult;
    let neg = n < 0; // c:317
    if neg {
        n = -n; // c:319
    }
    while n > 0 {
        // c:321
        n -= 1;
        let mut ct = ZLECS.load(SeqCst); // c:322
        if ct == 0 || ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] == '\n' {
            // c:322
            if ZLELL.load(SeqCst) == ZLECS.load(SeqCst)
                || ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] == '\n'
            {
                // c:323
                return 1;
            }
            if !neg {
                inccs(); // c:326
            }
            incpos(&mut ct); // c:327
        }
        if neg {
            if ZLECS.load(SeqCst) > 0 && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] != '\n' {
                // c:330
                deccs(); // c:331
                if ct > 1 && ZLELINE.lock().unwrap()[ct - 2] != '\n' {
                    // c:332
                    decpos(&mut ct); // c:333
                }
            }
        } else if ZLECS.load(SeqCst) != ZLELL.load(SeqCst)
            && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] != '\n'
        {
            inccs(); // c:338
        }
        if ct == ZLELL.load(SeqCst) || ZLELINE.lock().unwrap()[ct] == '\n' {
            // c:340
            decpos(&mut ct); // c:341
        }
        if ct < 1 || ZLELINE.lock().unwrap()[ct - 1] == '\n' {
            // c:343
            return 1;
        }
        // c:345-358 — MULTIBYTE branch uses transpose_swap with surrounding
        //              positions; non-multibyte branch swaps two ZLE_CHAR_T.
        //              Rust Vec<char> is Vec<char> so we can swap directly.
        ZLELINE.lock().unwrap().swap(ct - 1, ct);
    }
    0
}

/// Port of `poundinsert()` from `Src/Zle/zle_misc.c:369`.
/// C decl: `poundinsert(UNUSED(char **args))`
pub fn poundinsert() -> i32 {
    // c:369
    // c:371-393 — `zlecs = 0; vifirstnonblank(zlenoargs);
    //              if (zleline[zlecs] != '#') { spaceinline(1);
    //                  zleline[zlecs] = '#'; zlecs = findeol();
    //                  while (zlecs != zlell) { ... } }
    //              else { foredel(1, 0); zlecs = findeol(); ... }
    //              done = 1; return 0`.
    ZLECS.store(0, SeqCst); // c:371
    vifirstnonblank(); // c:372
    let at_pound = ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)) == Some(&'#');
    if !at_pound {
        // c:374-383 — insert # at this line, advance to next, repeat.
        spaceinline(1); // c:374
        if let Some(slot) = ZLELINE.lock().unwrap().get_mut(ZLECS.load(SeqCst)) {
            *slot = '#'; // c:375
        }
        ZLECS.store(findeol(), SeqCst); // c:376
        while ZLECS.load(SeqCst) != ZLELL.load(SeqCst) {
            ZLECS.fetch_add(1, SeqCst); // c:378
            vifirstnonblank(); // c:379
            spaceinline(1); // c:380
            if let Some(slot) = ZLELINE.lock().unwrap().get_mut(ZLECS.load(SeqCst)) {
                *slot = '#'; // c:381
            }
            ZLECS.store(findeol(), SeqCst); // c:382
        }
    } else {
        // c:384-393 — strip leading # from each line.
        ZLELINE.lock().unwrap().remove(ZLECS.load(SeqCst));
        ZLELL.fetch_sub(1, SeqCst);
        ZLECS.store(findeol(), SeqCst);
        while ZLECS.load(SeqCst) != ZLELL.load(SeqCst) {
            ZLECS.fetch_add(1, SeqCst);
            vifirstnonblank();
            if ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)) == Some(&'#') {
                ZLELINE.lock().unwrap().remove(ZLECS.load(SeqCst));
                ZLELL.fetch_sub(1, SeqCst);
            }
            ZLECS.store(findeol(), SeqCst);
        }
    }
    DONE.store(1, SeqCst); // c:395
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:396
}

/// Port of `acceptline()` from `Src/Zle/zle_misc.c:401`.
/// C decl: `acceptline(UNUSED(char **args))`
/// ```c
/// int
/// acceptline(UNUSED(char **args))
/// {
///     done = 1;
///     return 0;
/// }
/// ```
/// `accept-line` widget — the simplest possible: just signal the
/// editor session to terminate so `zleread` returns the current line.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn acceptline() -> i32 {
    // c:401
    DONE.store(1, SeqCst); // c:403 done = 1
    0 // c:404 return 0
}

/// Port of `acceptandhold()` from `Src/Zle/zle_misc.c:409`.
/// C decl: `acceptandhold(UNUSED(char **args))`
/// C body (4 lines):
///     `zpushnode(bufstack, zlelineasstring(zleline, zlell, 0, NULL, NULL, 0));
///      stackcs = zlecs;
///      done = 1;
///      return 0;`
pub fn acceptandhold() -> i32 {
    // c:409
    let zlell = ZLELL.load(SeqCst);
    let line: String = ZLELINE.lock().unwrap().iter().take(zlell).collect();
    BUFSTACK.lock().unwrap().insert(0, line); // c:411 zpushnode
    STACKCS.store(ZLECS.load(SeqCst), SeqCst); // c:412
    DONE.store(1, SeqCst); // c:413
    0 // c:414
}

/// Port of `killline()` from `Src/Zle/zle_misc.c:419`.
/// C decl: `killline(char **args)`
pub fn killline() -> i32 {
    // c:419
    // c:419-428 — `if (n < 0) { backward delegate w/ negated zmult }`.
    let n_orig = ZMOD.lock().unwrap().mult;
    if n_orig < 0 {
        ZMOD.lock().unwrap().mult = -n_orig;
        let ret = backwardkillline();
        ZMOD.lock().unwrap().mult = n_orig;
        return ret;
    }
    let mut n = n_orig;
    let start = ZLECS.load(SeqCst);
    let mut i = 0_usize;
    // c:430-436 — walk to next newline; skip past existing newline.
    while n > 0 {
        if ZLECS.load(SeqCst) < ZLELINE.lock().unwrap().len()
            && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] == '\n'
        {
            ZLECS.fetch_add(1, SeqCst);
            i += 1;
        } else {
            while ZLECS.load(SeqCst) != ZLELL.load(SeqCst)
                && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] != '\n'
            {
                ZLECS.fetch_add(1, SeqCst);
                i += 1;
            }
        }
        n -= 1;
    }
    // c:437 — `backkill(i, CUT_RAW);` — the walk above advanced zlecs
    // to the range end; backkill rewinds it and routes the kill
    // through cuttext (CUTBUF), so `yank` sees it.
    let _ = start;
    backkill(i as i32, CUT_RAW);
    CLEARLIST.store(1, SeqCst); // c:438 `clearlist = 1;`
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:439
}

/// Port of `regionlines()` from `Src/Zle/zle_misc.c:444`.
/// C decl: `regionlines(int *start, int *end)`
/// WARNING: param names don't match C — Rust=(zle) vs C=(start, end)
pub fn regionlines() -> (usize, usize) {
    // c:444
    // c:446 — `int origcs = zlecs`. Save cursor.
    let origcs = ZLECS.load(SeqCst);
    let start;
    let end;
    if ZLECS.load(SeqCst) < MARK.load(SeqCst) {
        // c:449
        // c:450-452 — start=findbol(); zlecs=min(mark,zlell); end=findeol().
        start = findbol();
        ZLECS.store(
            if MARK.load(SeqCst) > ZLELL.load(SeqCst) {
                ZLELL.load(SeqCst)
            } else {
                MARK.load(SeqCst)
            },
            SeqCst,
        );
        end = findeol();
    } else {
        // c:454-456 — end=findeol(); zlecs=mark; start=findbol().
        end = findeol();
        ZLECS.store(MARK.load(SeqCst), SeqCst);
        start = findbol();
    }
    // c:458 — `zlecs = origcs`. Restore.
    ZLECS.store(origcs, SeqCst);
    (start, end)
}

/// Port of `killregion()` from `Src/Zle/zle_misc.c:463`.
/// C decl: `killregion(UNUSED(char **args))`
pub fn killregion() -> i32 {
    // c:463
    // c:463-466 — `if (mark > zlell) mark = zlell`.
    if MARK.load(SeqCst) > ZLELL.load(SeqCst) {
        MARK.store(ZLELL.load(SeqCst), SeqCst);
    }
    // c:467-479 — region_active==2 (visual-line): whole-line cut.
    if REGION_ACTIVE.load(SeqCst) == 2 {
        let (a, b) = regionlines(); // c:469
        ZLECS.store(a, SeqCst); // c:470 `zlecs = a;`
        REGION_ACTIVE.store(0, SeqCst); // c:471
        let len = b as i32 - a as i32;
        crate::ported::zle::zle_utils::cut(a as i32, len, CUT_RAW); // c:472
        crate::ported::zle::zle_utils::shiftchars(a as i32, len); // c:473
        if ZLELL.load(SeqCst) != 0 {
            // c:474
            if ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
                crate::ported::zle::zle_move::deccs(); // c:475-476
            }
            crate::ported::zle::zle_utils::foredel(1, 0); // c:477
            let _ = crate::ported::zle::zle_move::vifirstnonblank(); // c:478
        }
    } else if MARK.load(SeqCst) > ZLECS.load(SeqCst) {
        // c:480-483 — kill forward to mark; cuttext fills CUTBUF.
        let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
        if crate::ported::zle::zle_h::invicmdmode(&kn) {
            MARK.fetch_add(1, SeqCst); // c:481-482 `INCPOS(mark);`
        }
        let n = MARK.load(SeqCst) as i32 - ZLECS.load(SeqCst) as i32;
        forekill(n, CUT_RAW); // c:483
    } else {
        // c:484-487 — kill backward to mark.
        let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
        if crate::ported::zle::zle_h::invicmdmode(&kn) {
            crate::ported::zle::zle_move::inccs(); // c:485-486 `INCCS();`
        }
        let n = ZLECS.load(SeqCst) as i32 - MARK.load(SeqCst) as i32;
        backkill(n, CUT_FRONT | CUT_RAW); // c:487
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:489
}

/// Port of `copyregionaskill()` from `Src/Zle/zle_misc.c:494`.
/// C decl: `copyregionaskill(char **args)`
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn copyregionaskill(args: &[String]) -> i32 {
    // c:494
    // c:494-501 — `if (*args) { stringaszleline; cuttext(line, len, CUT_REPLACE) }`.
    if let Some(arg) = args.first() {
        // c:499-500 — `line = stringaszleline(*args, 0, &len, NULL,
        // NULL); cuttext(line, len, CUT_REPLACE);`
        let text: Vec<char> =
            crate::ported::zle::zle_utils::stringaszleline(arg, 0, None, None, None);
        crate::ported::zle::zle_utils::cuttext(&text, CUT_REPLACE); // c:500
        return 0;
    }
    // c:503-512 — copy region between point and mark.
    if MARK.load(SeqCst) > ZLELL.load(SeqCst) {
        MARK.store(ZLELL.load(SeqCst), SeqCst);
    }
    let (start, end) = if MARK.load(SeqCst) > ZLECS.load(SeqCst) {
        (ZLECS.load(SeqCst), MARK.load(SeqCst))
    } else {
        (MARK.load(SeqCst), ZLECS.load(SeqCst))
    };
    // c:512-514 — `if (invicmdmode()) INCPOS(end); cut(start, end -
    // start, mark > zlecs ? 0 : CUT_FRONT);` — copy through cuttext
    // (CUTBUF) so `yank` sees it.
    let mut end = end;
    let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
    if crate::ported::zle::zle_h::invicmdmode(&kn) && end < ZLELL.load(SeqCst) {
        end += 1; // c:513 `INCPOS(end);`
    }
    crate::ported::zle::zle_utils::cut(
        start as i32,
        end as i32 - start as i32,
        if MARK.load(SeqCst) > ZLECS.load(SeqCst) {
            0
        } else {
            CUT_FRONT
        },
    ); // c:514
    0 // c:516
}

/// Yank - insert from the cut buffer.
/// Port of `yank()` from `Src/Zle/zle_misc.c:533`.
/// C decl: `yank(UNUSED(char **args))`
pub fn yank() -> i32 {
    // c:533
    let mut n = ZMOD.lock().unwrap().mult; // c:535 `int n = zmult;`
    if n < 0 {
        return 1; // c:537-538
    }
    // c:539-542 — `kctbuf = &vibuf[zmod.vibuf]` / `kctbuf = &cutbuf`.
    if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        KCTBUF_SEL.store(ZMOD.lock().unwrap().vibuf, SeqCst); // c:540
    } else {
        KCTBUF_SEL.store(-1, SeqCst); // c:542
    }
    let text: Vec<char> = match KCTBUF_SEL.load(SeqCst) {
        -1 => crate::ported::zle::zle_main::CUTBUF
            .lock()
            .unwrap()
            .buf
            .chars()
            .collect(),
        idx if idx >= 0 && (idx as usize) < 36 => {
            vibuf().lock().unwrap()[idx as usize].buf.chars().collect()
        }
        _ => Vec::new(),
    };
    if text.is_empty() {
        return 1; // c:543-544 `if (!kctbuf->buf) return 1;`
    }
    // c:545 — `yankb = yankcs = mark = zlecs;`
    let cs0 = ZLECS.load(SeqCst);
    YANKB.store(cs0, SeqCst);
    YANKCS.store(cs0 as i32, SeqCst);
    MARK.store(cs0, SeqCst);
    while n > 0 {
        // c:546 `while (n--)`
        KCT.store(-1, SeqCst); // c:547 `kct = -1;`
        spaceinline(text.len() as i32); // c:548
        let cs = ZLECS.load(SeqCst);
        {
            let mut line = ZLELINE.lock().unwrap();
            line[cs..cs + text.len()].copy_from_slice(&text); // c:549
        }
        ZLECS.store(cs + text.len(), SeqCst); // c:550 `zlecs += kctbuf->len;`
        YANKE.store(ZLECS.load(SeqCst), SeqCst); // c:551 `yanke = zlecs;`
        n -= 1;
    }
    YANKLAST.store(true, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:553
}

/// Port of `pastebuf()` from `Src/Zle/zle_misc.c:558`.
/// C decl: `pastebuf(Cutbuffer buf, int mult, int position)`
/// c:556 — position: 0 is before, 1 after, 2 split the line
pub fn pastebuf(buf: &crate::ported::zle::zle_h::cutbuffer, mult: i32, position: i32) -> i32 {
    // c:558
    use crate::ported::zle::zle_h::CUTBUFFER_LINE;
    use crate::ported::zle::zle_utils::{findbol, findeol};
    let text: Vec<char> = buf.buf.chars().collect();
    let mut position = position;
    if buf.flags & CUTBUFFER_LINE != 0 {
        // c:561 — line-mode buffer: paste whole lines.
        if position == 2 {
            // c:562-567 — split degrades to before/after at the edges.
            if ZLECS.load(SeqCst) == 0 {
                position = 0; // c:563-564
            } else if ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
                position = 1; // c:565-566
            }
        }
        if position == 2 {
            // c:568-575 — split the line at the cursor.
            YANKB.store(ZLECS.load(SeqCst), SeqCst); // c:569 `yankb = zlecs;`
            spaceinline(text.len() as i32 + 2); // c:570
            let mut cs = ZLECS.load(SeqCst);
            {
                let mut line = ZLELINE.lock().unwrap();
                line[cs] = '\n'; // c:571 `zleline[zlecs++] = ZWC('\n');`
                cs += 1;
                line[cs..cs + text.len()].copy_from_slice(&text); // c:572
                cs += text.len(); // c:573
                line[cs] = '\n'; // c:574
            }
            ZLECS.store(cs, SeqCst);
            YANKE.store(cs + 1, SeqCst); // c:575 `yanke = zlecs + 1;`
        } else if position != 0 {
            // c:576-581 — after: open a new line below the current one.
            let mut cs = findeol(); // c:577 `yankb = zlecs = findeol();`
            YANKB.store(cs, SeqCst);
            ZLECS.store(cs, SeqCst);
            spaceinline(text.len() as i32 + 1); // c:578
            {
                let mut line = ZLELINE.lock().unwrap();
                line[cs] = '\n'; // c:579 `zleline[zlecs++] = ZWC('\n');`
                cs += 1;
                line[cs..cs + text.len()].copy_from_slice(&text); // c:581
            }
            ZLECS.store(cs, SeqCst);
            YANKE.store(cs + text.len(), SeqCst); // c:580 `yanke = zlecs + buf->len;`
        } else {
            // c:582-588 — before: open a new line above the current one.
            let cs = findbol(); // c:583 `yankb = zlecs = findbol();`
            YANKB.store(cs, SeqCst);
            ZLECS.store(cs, SeqCst);
            spaceinline(text.len() as i32 + 1); // c:584
            {
                let mut line = ZLELINE.lock().unwrap();
                line[cs..cs + text.len()].copy_from_slice(&text); // c:585
                line[cs + text.len()] = '\n'; // c:587
            }
            YANKE.store(cs + text.len() + 1, SeqCst); // c:586 `yanke = zlecs + buf->len + 1;`
        }
        let _ = crate::ported::zle::zle_move::vifirstnonblank(); // c:589
    } else {
        // c:590-603 — char-wise paste.
        // c:591-592 — `if (position == 1 && zlecs != findeol()) INCCS();`
        if position == 1 && ZLECS.load(SeqCst) != findeol() {
            crate::ported::zle::zle_move::inccs();
        }
        YANKB.store(ZLECS.load(SeqCst), SeqCst); // c:593 `yankb = zlecs;`
        let cc = text.len(); // c:594 `cc = buf->len;`
        let mut mult = mult;
        while mult > 0 {
            // c:595 `while (mult--)`
            spaceinline(cc as i32); // c:596
            let cs = ZLECS.load(SeqCst);
            {
                let mut line = ZLELINE.lock().unwrap();
                line[cs..cs + cc].copy_from_slice(&text); // c:597
            }
            ZLECS.store(cs + cc, SeqCst); // c:598 `zlecs += cc;`
            mult -= 1;
        }
        YANKE.store(ZLECS.load(SeqCst), SeqCst); // c:600 `yanke = zlecs;`
                                                 // c:601-602 — `if (zlecs && invicmdmode()) DECCS();`
        let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
        if ZLECS.load(SeqCst) != 0 && crate::ported::zle::zle_h::invicmdmode(&kn) {
            crate::ported::zle::zle_move::deccs();
        }
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viputbefore()` from `Src/Zle/zle_misc.c:608`.
/// C decl: `viputbefore(UNUSED(char **args))`
pub fn viputbefore() -> i32 {
    // c:608
    let n = ZMOD.lock().unwrap().mult; // c:610
    startvichange(-1); // c:612
    if n < 0 {
        return 1; // c:614
    }
    if ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 0; // c:616
    }
    // c:631-634 — `kctbuf = &vibuf[zmod.vibuf]` / `kctbuf = &cutbuf`.
    // The unnamed register is CUTBUF (the vi cut buffer with its
    // CUTBUFFER_LINE flag), NOT the emacs kill ring — reading the ring
    // lost the line flag, so `yyp` pasted mid-line instead of opening
    // a new line.
    let kctbuf: crate::ported::zle::zle_h::cutbuffer =
        if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
            let idx = ZMOD.lock().unwrap().vibuf as usize;
            if idx >= vibuf().lock().unwrap().len() {
                return 1;
            }
            KCTBUF_SEL.store(idx as i32, SeqCst); // c:632 `kctbuf = &vibuf[zmod.vibuf];`
            vibuf().lock().unwrap()[idx].clone()
        } else {
            KCTBUF_SEL.store(-1, SeqCst); // c:634 `kctbuf = &cutbuf;`
            let cb = crate::ported::zle::zle_main::CUTBUF.lock().unwrap();
            crate::ported::zle::zle_h::cutbuffer {
                buf: cb.buf.clone(),
                len: cb.len,
                flags: cb.flags,
            }
        };
    if kctbuf.buf.is_empty() {
        return 1; // c:635-636 `if (!kctbuf->buf) return 1;`
    }
    KCT.store(-1, SeqCst); // c:637 `kct = -1;`
    YANKCS.store(ZLECS.load(SeqCst) as i32, SeqCst); // c:638 `yankcs = zlecs;`
    pastebuf(&kctbuf, n, 0) // c:639
}

/// Port of `viputafter()` from `Src/Zle/zle_misc.c:644`.
/// C decl: `viputafter(UNUSED(char **args))`
pub fn viputafter() -> i32 {
    // c:644
    let n = ZMOD.lock().unwrap().mult; // c:646
    startvichange(-1); // c:648
    if n < 0 {
        return 1; // c:650
    }
    if ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 0; // c:652
    }
    // c:653-667 — OS selection branch (MOD_OSSEL = PRI|CLIP). Without
    //              system_clipget we fall through to the cut-buffer path.
    // c:668-671 — `kctbuf = &vibuf[zmod.vibuf]` / `kctbuf = &cutbuf`.
    // Same unnamed-register fix as viputbefore: read CUTBUF (flags
    // intact), not the kill ring.
    let kctbuf: crate::ported::zle::zle_h::cutbuffer =
        if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
            let idx = ZMOD.lock().unwrap().vibuf as usize;
            if idx >= vibuf().lock().unwrap().len() {
                return 1;
            }
            KCTBUF_SEL.store(idx as i32, SeqCst); // c:669 `kctbuf = &vibuf[zmod.vibuf];`
            vibuf().lock().unwrap()[idx].clone()
        } else {
            KCTBUF_SEL.store(-1, SeqCst); // c:671 `kctbuf = &cutbuf;`
            let cb = crate::ported::zle::zle_main::CUTBUF.lock().unwrap();
            crate::ported::zle::zle_h::cutbuffer {
                buf: cb.buf.clone(),
                len: cb.len,
                flags: cb.flags,
            }
        };
    if kctbuf.buf.is_empty() {
        return 1; // c:672-673 `if (!kctbuf->buf) return 1;`
    }
    KCT.store(-1, SeqCst); // c:674 `kct = -1;`
    YANKCS.store(ZLECS.load(SeqCst) as i32, SeqCst); // c:675 `yankcs = zlecs;`
    pastebuf(&kctbuf, n, 1) // c:676
}

/// Port of `putreplaceselection()` from `Src/Zle/zle_misc.c:680`.
/// C decl: `putreplaceselection(UNUSED(char **args))`
pub fn putreplaceselection() -> i32 {
    // c:680
    let n = ZMOD.lock().unwrap().mult; // c:682
    let mut pos = 2; // c:686
    startvichange(-1); // c:688
    if n < 0 || ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 1; // c:690
    }
    // c:698-702 — `kctbuf = &vibuf[zmod.vibuf]` / `kctbuf = &cutbuf`.
    let prevbuf: crate::ported::zle::zle_h::cutbuffer =
        if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
            let idx = ZMOD.lock().unwrap().vibuf as usize;
            if idx >= vibuf().lock().unwrap().len() {
                return 1;
            }
            vibuf().lock().unwrap()[idx].clone() // c:700
        } else {
            let cb = crate::ported::zle::zle_main::CUTBUF.lock().unwrap();
            crate::ported::zle::zle_h::cutbuffer {
                buf: cb.buf.clone(),
                len: cb.len,
                flags: cb.flags,
            } // c:702
        };
    if prevbuf.buf.is_empty() {
        return 1; // c:702
    }
    ZMOD.lock().unwrap().flags = 0; // c:712
    if REGION_ACTIVE.load(SeqCst) == 2 {
        // c:713
        // c:714-717 — regionlines split; lines-flag check elided.
        pos = if ZLELL.load(SeqCst) == ZLECS.load(SeqCst) {
            1
        } else {
            0
        };
    }
    let _ = killregion(); // c:719
    pastebuf(&prevbuf, n, pos) // c:721
}

/// Port of `yankpop()` from `Src/Zle/zle_misc.c:728`.
/// C decl: `yankpop(UNUSED(char **args))`
///
/// The C walk cycles `kct` through: original buffer (`-1`, whatever
/// `kctbuf` points at) → kill ring entries newest→oldest → back to the
/// original buffer. zshrs's `KILLRING` deque holds newest at index 0
/// (cuttext pushes front and pins `kringnum = 0`), so the descending
/// C index walk maps to ASCENDING deque indices.
///
/// Substrate note: `KILLRING` entries are `Vec<char>` without the C
/// `struct cutbuffer` flags, so a ring entry pastes char-wise even if
/// its kill was line-mode (`dd` then `M-y M-y`). The original CUTBUF /
/// vibuf (`kct == -1`) keeps its flags. Retyping KILLRING to
/// `VecDeque<cutbuffer>` lifts this.
pub fn yankpop() -> i32 {
    // c:741
    let kctstart = KCT.load(SeqCst); // c:744 `int kctstart = kct;`
                                     // c:747-750 — `if (!(lastcmd & ZLE_YANK) || !kring || !kctbuf)
                                     //              { kctbuf = NULL; return 1; }`
    let last = LASTCMD.load(SeqCst) as i32;
    if (last & ZLE_YANK) == 0
        || KILLRING.lock().unwrap().is_empty()
        || KCTBUF_SEL.load(SeqCst) == -2
    {
        KCTBUF_SEL.store(-2, SeqCst); // c:748 `kctbuf = NULL;`
        return 1;
    }
    let ringlen = KILLRING.lock().unwrap().len() as i32;
    let text: Vec<char>;
    loop {
        // c:751 do
        // c:759-767 — advance kct. C: kct==-1 → kringnum (newest);
        // else step to the next-older entry, wrapping to -1 (the
        // original buffer) after the oldest. Deque mapping: newest is
        // index 0, older is +1.
        let kct = KCT.load(SeqCst);
        let new_kct = if kct == -1 {
            0 // c:760 `kct = kringnum;`
        } else if kct + 1 >= ringlen {
            -1 // c:762-764 — wrapped past the oldest → original buffer
        } else {
            kct + 1 // c:766
        };
        KCT.store(new_kct, SeqCst);
        // c:772-774 — `if (kct == kctstart) return 1;` — full loop.
        if new_kct == kctstart {
            return 1;
        }
        // c:768-771 — resolve the buffer for this kct.
        let candidate: Vec<char> = if new_kct == -1 {
            // c:769 — the original cutbuffer (CUTBUF or a vibuf).
            match KCTBUF_SEL.load(SeqCst) {
                -1 => crate::ported::zle::zle_main::CUTBUF
                    .lock()
                    .unwrap()
                    .buf
                    .chars()
                    .collect(),
                idx if idx >= 0 && (idx as usize) < 36 => {
                    vibuf().lock().unwrap()[idx as usize].buf.chars().collect()
                }
                _ => Vec::new(),
            }
        } else {
            KILLRING
                .lock()
                .unwrap()
                .get(new_kct as usize)
                .cloned()
                .unwrap_or_default()
        };
        // c:787 — `while (!buf->buf || *buf->buf == ZWC('\0'));` —
        // skip unset / zero-length buffers.
        if !candidate.is_empty() {
            text = candidate;
            break;
        }
    }
    // c:789-792 — replace the last yank with this buffer.
    ZLECS.store(YANKB.load(SeqCst), SeqCst); // c:789 `zlecs = yankb;`
    let del = YANKE.load(SeqCst) as i32 - YANKB.load(SeqCst) as i32;
    crate::ported::zle::zle_utils::foredel(del.max(0), crate::ported::zle::zle_h::CUT_RAW); // c:790
    ZLECS.store(YANKCS.load(SeqCst).max(0) as usize, SeqCst); // c:791 `zlecs = yankcs;`
    let pastebuf_arg = crate::ported::zle::zle_h::cutbuffer {
        buf: text.iter().collect(),
        len: text.len(),
        flags: 0,
    };
    // c:792 — `pastebuf(buf, 1, !!(lastcmd & ZLE_YANKAFTER));`
    pastebuf(
        &pastebuf_arg,
        1,
        if last & crate::ported::zle::zle_h::ZLE_YANKAFTER != 0 {
            1
        } else {
            0
        },
    );
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:793
}

/// Port of `mod_export char *bracketedstring(void)` from
/// Src/Zle/zle_misc.c:784.
///
/// Reads bytes from the terminal until the end-paste sequence
/// `\e[201~` is seen, demetafying high-bit bytes and translating
/// `\r` → `\n` along the way.
///
/// Blocked on: `getbyte()` from zle_main.c — the keyboard input
/// pump that respects the ZLE timeout/select(2) machinery. Until
/// the input pump lands, returns the empty string so callers see a
/// no-op paste rather than a panic.
/// Port of `bracketedstring()` from `Src/Zle/zle_misc.c:784`.
/// C decl: `bracketedstring(void)`
/// Reads bytes from the controlling tty
/// looking for the bracketed-paste end sentinel `\033[201~`,
/// translating CR → LF and meta-encoding high-bit bytes along the
/// way. Returns the accumulated payload (without the sentinel).
///
/// C uses `getbyte(1L, &timeout, 1)` which goes through the full
/// ZLE input pump (sets `timeout=1`, blocks ≤1 sec). The Rust port
/// uses a direct `read()` on SHTTY with a 1-second poll budget per
/// byte — enough for paste activity but not enough to wedge an
/// idle session.
pub fn bracketedstring() -> String {
    // c:784

    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if fd < 0 {
        return String::new();
    }

    const ENDESC: &[u8] = b"\x1b[201~"; // c:786
    let mut pbuf: Vec<u8> = Vec::with_capacity(64); // c:789
    let mut endpos: usize = 0; // c:787

    // Read one byte at a time with a 1-second deadline per `getbyte`-
    // equivalent call. Use stdin fd 0 if SHTTY is the controlling tty;
    // otherwise read directly from SHTTY.
    let mut stdin = std::io::stdin();
    let deadline_per_byte = std::time::Duration::from_secs(1);

    while endpos < ENDESC.len() {
        // c:793
        let mut buf = [0u8; 1];
        let start = std::time::Instant::now();
        let next: u8 = loop {
            match stdin.read(&mut buf) {
                Ok(1) => break buf[0],                                       // c:796
                Ok(_) => return String::from_utf8_lossy(&pbuf).into_owned(), // EOF
                Err(_) if start.elapsed() < deadline_per_byte => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(_) => return String::from_utf8_lossy(&pbuf).into_owned(),
            }
        };

        // c:798-799 — sliding match against ENDESC.
        if endpos == 0 || next != ENDESC[endpos] {
            endpos = if next == ENDESC[0] { 1 } else { 0 };
        } else {
            endpos += 1;
        }

        // c:800-806 — meta-encode high-bit bytes, CR→LF, else copy.
        if (next & 0x80) != 0 && next != 0xff {
            // c:800 imeta()
            pbuf.push(0x83); // c:801 Meta
            pbuf.push(next ^ 32); // c:802
        } else if next == b'\r' {
            // c:803
            pbuf.push(b'\n'); // c:804
        } else {
            pbuf.push(next); // c:806
        }
    }
    // c:808 — `pbuf[current-endpos] = '\0';` — trim the sentinel we
    //          appended byte-by-byte off the tail.
    let strip = endpos.min(pbuf.len());
    pbuf.truncate(pbuf.len() - strip);
    String::from_utf8_lossy(&pbuf).into_owned()
}

/// Port of `bracketedpaste()` from `Src/Zle/zle_misc.c:814`.
/// C decl: `bracketedpaste(char **args)`
///
/// Captures a bracketed-paste payload via `bracketedstring()` then
/// either stores it in `args[0]` (assoc-array setparam) or inserts it
/// at the cursor with `doinsert`. The single-quote-escape detour
/// (`quotestring(pbuf, QT_SINGLE_OPTIONAL)`) when `zmult != 1`
/// prevents the user from accidentally pasting shell metacharacters.
pub fn bracketedpaste(args: &[String]) -> i32 {
    // c:814
    let pbuf = bracketedstring(); // c:816
    if let Some(name) = args.first() {
        // c:818
        // c:819 — `setsparam(*args, pbuf)`. Param-table not yet a
        // singleton; fall back to env-var (matches other ports).
        std::env::set_var(name, &pbuf);
        return 0;
    }
    // c:822-825 — quote when zmult != 1 then convert to ZLE_CHAR_T,
    //              cuttext (REPLACE) the prior cutbuf with the paste.
    let payload = if ZMOD.lock().unwrap().mult == 1 {
        // c:823
        pbuf.clone()
    } else {
        quotestring(&pbuf, QT_SINGLE_OPTIONAL) // c:824
    };
    // c:823 — `wpaste = stringaszleline((zmult==1) ? pbuf : quotestr, 0, &n, NULL, NULL);`
    let wpaste: Vec<char> =
        crate::ported::zle::zle_utils::stringaszleline(&payload, 0, None, None, None);
    // c:826-834 — !(zmod.flags & MOD_VIBUF) → reset kct, killregion if
    // region_active, then doinsert(wpaste).
    if !ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        ZMOD.lock().unwrap().mult = 1; // c:829
                                       // c:830-832 — `if (region_active) killregion(...)`.
        if REGION_ACTIVE.load(SeqCst) != 0 {
            let _ = killregion();
        }
        // c:833 — `doinsert(wpaste, n)`. Inline insert at zlecs.
        for c in wpaste.iter().copied() {
            ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst), c);
            ZLECS.fetch_add(1, SeqCst);
            ZLELL.fetch_add(1, SeqCst);
        }
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
    0 // c:838
}

/// Port of `overwritemode()` from `Src/Zle/zle_misc.c:843`.
/// C decl: `overwritemode(UNUSED(char **args))`
/// ```c
/// int
/// overwritemode(UNUSED(char **args))
/// {
///     insmode ^= 1;
///     return 0;
/// }
/// ```
/// `overwrite-mode` widget — toggle insert/overwrite mode.
pub fn overwritemode() -> i32 {
    // c:843
    INSMODE.fetch_xor(1, SeqCst); // c:843 insmode ^= 1
    0 // c:846 return 0
}

/// Port of `whatcursorposition()` from `Src/Zle/zle_misc.c:851`.
/// C decl: `whatcursorposition(UNUSED(char **args))`
pub fn whatcursorposition() -> i32 {
    // c:851
    let bol = findbol(); // c:855
    let mut msg = String::with_capacity(100);
    if ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
        // c:858
        msg.push_str("EOF"); // c:859
    } else {
        msg.push_str("Char: "); // c:861
        let c = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)]; // c:856
        match c {
            ' ' => msg.push_str("SPC"),  // c:864
            '\t' => msg.push_str("TAB"), // c:867
            '\n' => msg.push_str("LFD"), // c:870
            _ => msg.push(c),            // c:878
        }
        let cu = c as u32;
        msg.push_str(&format!(" (0{:o}, {}, 0x{:x})", cu, cu, cu)); // c:881
    }
    let pct = if ZLELL.load(SeqCst) > 0 {
        100 * ZLECS.load(SeqCst) / ZLELL.load(SeqCst)
    } else {
        0
    };
    msg.push_str(&format!(
        "  point {} of {}({}%)  column {}",
        ZLECS.load(SeqCst) + 1,
        ZLELL.load(SeqCst) + 1,
        pct,
        ZLECS.load(SeqCst) - bol,
    )); // c:884
        // c:887 — `showmsg(msg);` Route through the real showmsg which
        //          writes to SHTTY (previously `tracing::info!` only).
    showmsg(&msg);
    0
}

/// Port of `undefinedkey()` from `Src/Zle/zle_misc.c:892`.
/// C decl: `undefinedkey(UNUSED(char **args))`
/// ```c
/// int
/// undefinedkey(UNUSED(char **args))
/// {
///     return 1;
/// }
/// ```
/// `undefined-key` widget — bound to key sequences that aren't
/// otherwise defined; returns 1 so the dispatcher beeps.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn undefinedkey() -> i32 {
    // c:892
    // c:892 — `return 1`. The widget binds to keys with no other
    // function and just signals "unhandled" by returning non-zero.
    1
}

/// Port of `quotedinsert()` from `Src/Zle/zle_misc.c:899`.
/// C decl: `quotedinsert(char **args)`
///
/// ```c
/// // (raw-mode tweak for non-HAS_TIO systems — skipped on Linux/macOS)
/// getfullchar(0);
/// if (LASTFULLCHAR == ZLEEOF) return 1;
/// return selfinsert();
/// ```
/// HAS_TIO is set everywhere zshrs builds (Linux/macOS), so the
/// raw-mode/ioctl branch is unreachable — `getfullchar` already
/// runs in the right mode via `zsetterm`. We invoke it explicitly
/// for a one-shot read, then forward to `selfinsert`.
pub fn quotedinsert() -> i32 {
    // c:899
    // c:899 — `getfullchar(0)`. Reads one full char, updates
    // LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) / lastchar_wide / lastchar_wide_valid.
    let _ = getfullchar(false);
    if LASTCHAR.load(SeqCst) < 0 {
        // c:919 LASTFULLCHAR == ZLEEOF
        return 1;
    }
    selfinsert(&[]) // c:922
}

/// Port of `parsedigit()` from `Src/Zle/zle_misc.c:919`.
/// C decl: `parsedigit(int inkey)`
/// WARNING: param names don't match C — Rust=(zle, inkey) vs C=(inkey)
pub fn parsedigit(inkey: i32) -> i32 {
    // c:919
    // c:1077 — `inkey &= 0x7f` (mask off Meta bit). Multibyte path
    // skips this; we mirror by always masking since Rust char vals
    // fit ASCII for digit chars.
    let inkey = inkey & 0x7f;
    let base = ZMOD.lock().unwrap().base;
    // c:1082-1090 — base > 10: accept lowercase a..(a+base-11) and
    // uppercase, plus digits 0-9.
    if base > 10 {
        if (b'a' as i32..b'a' as i32 + base - 10).contains(&inkey) {
            return inkey - b'a' as i32 + 10; // c:1083
        }
        if (b'A' as i32..b'A' as i32 + base - 10).contains(&inkey) {
            return inkey - b'A' as i32 + 10; // c:1085
        }
        if (b'0' as i32..=b'9' as i32).contains(&inkey) {
            // c:1087 idigit
            return inkey - b'0' as i32;
        }
        return -1; // c:1089
    }
    // c:1092-1093 — base <= 10: digit must be in '0'..'0'+base.
    if (b'0' as i32..b'0' as i32 + base).contains(&inkey) {
        return inkey - b'0' as i32;
    }
    -1 // c:1094
}

/// Port of `digitargument()` from `Src/Zle/zle_misc.c:950`.
/// C decl: `digitargument(UNUSED(char **args))`
pub fn digitargument() -> i32 {
    // c:950
    // c:1044 — `int sign = (zmult < 0) ? -1 : 1`.
    let sign: i32 = if ZMOD.lock().unwrap().mult < 0 { -1 } else { 1 };
    // c:1045 — `parsedigit(lastchar)`.
    let newdigit = parsedigit(LASTCHAR.load(SeqCst));
    if newdigit < 0 {
        // c:1047
        return 1; // c:1048
    }
    // c:1050-1051 — `if (!(zmod.flags & MOD_TMULT)) zmod.tmult = 0`.
    if !ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {
        ZMOD.lock().unwrap().tmult = 0;
    }
    // c:1052-1057 — MOD_NEG path: replace tmult with sign*newdigit.
    if ZMOD.lock().unwrap().flags & MOD_NEG != 0 {
        ZMOD.lock().unwrap().tmult = sign * newdigit;
        ZMOD.lock().unwrap().flags &= !MOD_NEG;
    } else {
        // c:1058 — `zmod.tmult = zmod.tmult * zmod.base + sign*newdigit`.
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.tmult = __g_zmod.tmult * __g_zmod.base + sign * newdigit;
    }
    ZMOD.lock().unwrap().flags |= MOD_TMULT; // c:1059
    PREFIXFLAG.store(1, SeqCst); // c:1060
    0 // c:1061
}

/// Port of `negargument()` from `Src/Zle/zle_misc.c:974`.
/// C decl: `negargument(UNUSED(char **args))`
/// ```c
/// int
/// negargument(UNUSED(char **args))
/// {
///     if (zmod.flags & MOD_TMULT)
///         return 1;
///     zmod.tmult = -1;
///     zmod.flags |= MOD_TMULT|MOD_NEG;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `negative-argument` widget — start a negative count prefix.
/// Refuses if a tmult is already in flight.
pub fn negargument() -> i32 {
    // c:974
    if ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {
        // c:976
        return 1; // c:977
    }
    ZMOD.lock().unwrap().tmult = -1; // c:978
    ZMOD.lock().unwrap().flags |= MOD_TMULT | MOD_NEG; // c:979
    PREFIXFLAG.store(1, SeqCst); // c:980
    0 // c:981 return 0
}

/// Port of `universalargument()` from `Src/Zle/zle_misc.c:986`.
/// C decl: `universalargument(char **args)`
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn universalargument(args: &[String]) -> i32 {
    // c:986
    // c:988-993 — `if (*args)` short-circuit when invoked with an
    //              explicit numeric arg.
    if let Some(a) = args.first() {
        if let Ok(n) = a.parse::<i32>() {
            ZMOD.lock().unwrap().mult = n;
            ZMOD.lock().unwrap().flags |= MOD_MULT;
            return 0;
        }
    }
    // c:1009-1023 — interactive byte-by-byte digit collection. Without
    //               a live keystream we mirror the no-input branch
    //               (no digits) which multiplies tmult by 4.
    let digcnt = 0;
    if digcnt == 0 {
        // c:1027 — `zmod.tmult *= 4`. Cannot lock ZMOD twice in one
        // expression (RHS guard outlives read, LHS lock deadlocks
        // the same thread on non-reentrant std::sync::Mutex).
        let mut g = ZMOD.lock().unwrap();
        g.tmult = g.tmult.saturating_mul(4);
    }
    ZMOD.lock().unwrap().flags |= MOD_TMULT; // c:1029
    PREFIXFLAG.store(1, SeqCst); // c:1030
    0
}

/// Port of `argumentbase()` from `Src/Zle/zle_misc.c:1038`.
/// C decl: `argumentbase(char **args)`
/// ```c
/// int
/// argumentbase(char **args)
/// {
///     int multbase;
///     if (*args)
///         multbase = (int)zstrtol(*args, NULL, 0);
///     else
///         multbase = zmod.mult;
///     if (multbase < 2 || multbase > ('9' - '0' + 1) + ('z' - 'a' + 1))
///         return 1;
///     zmod.base = multbase;
///     zmod.flags = 0;
///     zmod.mult = 1;
///     zmod.tmult = 1;
///     zmod.vibuf = 0;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `argument-base` widget — set the numeric base for digit-arg
/// parsing. Valid range 2..36 (10 digits + 26 letters). Returns 1
/// for out-of-range bases without changing state.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn argumentbase(args: &[String]) -> i32 {
    // c:1042-1045 — `if (*args) multbase = zstrtol(...) else zmod.mult`.
    let multbase = if let Some(arg) = args.first() {
        // c:1043 — `zstrtol(*args, NULL, 0)`. Base 0 means auto
        // (octal "0…", hex "0x…", else decimal).
        let s = arg.as_str();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i32::from_str_radix(hex, 16).unwrap_or(0)
        } else if s.starts_with('0') && s.len() > 1 {
            i32::from_str_radix(&s[1..], 8).unwrap_or(0)
        } else {
            s.parse::<i32>().unwrap_or(0)
        }
    } else {
        ZMOD.lock().unwrap().mult // c:1045
    };
    // c:1047-1048 — range check 2..(10+26)=36.
    if multbase < 2 || multbase > 36 {
        return 1;
    }
    ZMOD.lock().unwrap().base = multbase; // c:1050
                                          // c:1053-1056 — reset modifier apart from base.
    ZMOD.lock().unwrap().flags = 0;
    ZMOD.lock().unwrap().mult = 1;
    ZMOD.lock().unwrap().tmult = 1;
    ZMOD.lock().unwrap().vibuf = 0;
    // c:1059 — still operating on prefix arg.
    PREFIXFLAG.store(1, SeqCst);
    0 // c:1061 return 0
}

/// Port of `copyprevword()` from `Src/Zle/zle_misc.c:1066`.
/// C decl: `copyprevword(UNUSED(char **args))`
pub fn copyprevword() -> i32 {
    // c:1066
    // C body (c:1066-1110): walk back over zmult words, copy that
    // span, insert at cursor. Simplified: locate previous whitespace-
    // separated word, copy + insert.
    let n = ZMOD.lock().unwrap().mult;
    if n <= 0 {
        return 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut t0 = ZLECS.load(SeqCst);
    for _ in 0..n {
        // skip back over non-word-chars
        while t0 > 0 && !is_word(ZLELINE.lock().unwrap()[t0 - 1]) {
            t0 -= 1;
        }
        // skip back over word
        while t0 > 0 && is_word(ZLELINE.lock().unwrap()[t0 - 1]) {
            t0 -= 1;
        }
    }
    // span: t0..(start of search)
    let mut t1 = t0;
    while t1 < ZLECS.load(SeqCst) && is_word(ZLELINE.lock().unwrap()[t1]) {
        t1 += 1;
    }
    let len = t1 - t0;
    if len == 0 {
        return 1;
    }
    // c:1100-1103 — `spaceinline(len); ZS_memcpy(zleline + zlecs,
    //                  zleline + t0, len); zlecs += len;`.
    let copied: Vec<char> = ZLELINE.lock().unwrap()[t0..t1].to_vec();
    spaceinline(len as i32); // c:1100
    let cs = ZLECS.load(SeqCst);
    {
        let mut line = ZLELINE.lock().unwrap();
        for (i, &c) in copied.iter().enumerate() {
            if cs + i < line.len() {
                line[cs + i] = c; // c:1101
            }
        }
    }
    ZLECS.fetch_add(len, SeqCst); // c:1102
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `copyprevshellword()` from `Src/Zle/zle_misc.c:1108`.
/// C decl: `copyprevshellword(UNUSED(char **args))`
pub fn copyprevshellword() -> i32 {
    // c:1108
    // C body: similar to copyprevword but uses shell tokenizer to
    // identify the previous WORD (whitespace-bounded chunk). Without
    // the shell-tokenizer substrate, fall back to whitespace-bounded
    // back-walk.
    let mut t1 = ZLECS.load(SeqCst);
    while t1 > 0 && ZLELINE.lock().unwrap()[t1 - 1].is_whitespace() {
        t1 -= 1;
    }
    let mut t0 = t1;
    while t0 > 0 && !ZLELINE.lock().unwrap()[t0 - 1].is_whitespace() {
        t0 -= 1;
    }
    if t0 == t1 {
        return 1;
    }
    // c:1133 — `spaceinline(len); ZS_memcpy; zlecs += len;`.
    let copied: Vec<char> = ZLELINE.lock().unwrap()[t0..t1].to_vec();
    spaceinline(copied.len() as i32); // c:1133
    let cs = ZLECS.load(SeqCst);
    {
        let mut line = ZLELINE.lock().unwrap();
        for (i, &c) in copied.iter().enumerate() {
            if cs + i < line.len() {
                line[cs + i] = c; // c:1134
            }
        }
    }
    ZLECS.fetch_add(copied.len(), SeqCst); // c:1135
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `sendbreak()` from `Src/Zle/zle_misc.c:1144`.
/// C decl: `sendbreak(UNUSED(char **args))`
/// ```c
/// int
/// sendbreak(UNUSED(char **args))
/// {
///     errflag |= ERRFLAG_ERROR|ERRFLAG_INT;
///     return 1;
/// }
/// ```
/// `send-break` widget — abort the current editor session by
/// raising both `ERRFLAG_ERROR` and `ERRFLAG_INT` on the global
/// `errflag`, so `zleread` returns -1 to its caller.
/// WARNING: param names don't match C — Rust=() vs C=(args)
/// C body (2 lines):
///   `errflag |= ERRFLAG_ERROR|ERRFLAG_INT;
///    return 1;`
pub fn sendbreak() -> i32 {
    // c:1144
    errflag.fetch_or(ERRFLAG_ERROR | ERRFLAG_INT, Ordering::Relaxed); // c:1146
    1 // c:1147
}

/// Port of `quoteregion()` from `Src/Zle/zle_misc.c:1152`.
/// C decl: `quoteregion(UNUSED(char **args))`
pub fn quoteregion() -> i32 {
    // c:1152
    // c:1152 — `int extra = invicmdmode()`. Vi-cmd-mode bias.
    let mut extra = *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd";
    // c:1158-1159 — `if (mark > zlell) mark = zlell`.
    if MARK.load(SeqCst) > ZLELL.load(SeqCst) {
        MARK.store(ZLELL.load(SeqCst), SeqCst);
    }
    // c:1160-1170 — visual-line vs. char modes; normalize zlecs/mark.
    if REGION_ACTIVE.load(SeqCst) == 2 {
        let (a, b) = regionlines();
        ZLECS.store(a, SeqCst);
        MARK.store(b, SeqCst);
        extra = false;
    } else if MARK.load(SeqCst) < ZLECS.load(SeqCst) {
        std::mem::swap(&mut MARK.load(SeqCst), &mut ZLECS.load(SeqCst));
    }
    // c:1171-1172 — `if (extra) INCPOS(mark)`. Include cursor cell.
    if extra && MARK.load(SeqCst) < ZLELL.load(SeqCst) {
        MARK.fetch_add(1, SeqCst);
    }
    // c:1173-1175 — copy region into temp str; foredel; quote; insert.
    let region: Vec<char> = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)..MARK.load(SeqCst)].to_vec();
    let len = region.len();
    let quoted = makequote(&region);
    let qlen = quoted.len();
    // c:1176 — `foredel(len, CUT_RAW)` — delete region (no kill).
    ZLELINE
        .lock()
        .unwrap()
        .drain(ZLECS.load(SeqCst)..ZLECS.load(SeqCst) + len);
    ZLELL.fetch_sub(len, SeqCst);
    // c:1178-1179 — insert quoted text at cursor.
    for (i, &c) in quoted.iter().enumerate() {
        ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst) + i, c);
    }
    ZLELL.fetch_add(qlen, SeqCst);
    // c:1180-1181 — `mark = zlecs; zlecs += len`.
    MARK.store(ZLECS.load(SeqCst), SeqCst);
    ZLECS.fetch_add(qlen, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `quoteline()` from `Src/Zle/zle_misc.c:1187`.
/// C decl: `quoteline(UNUSED(char **args))`
pub fn quoteline() -> i32 {
    // c:1187
    // c:1187 — `len = zlell`. Quote whole buffer.
    let quoted = makequote(&ZLELINE.lock().unwrap()[..ZLELL.load(SeqCst)]);
    let len = quoted.len();
    // c:1193-1195 — `sizeline; ZS_memcpy; zlecs = zlell = len`.
    *ZLELINE.lock().unwrap() = quoted;
    ZLELL.store(len, SeqCst);
    ZLECS.store(len, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:1196
}

/// Port of `makequote()` from `Src/Zle/zle_misc.c:1201`.
/// C decl: `makequote(ZLE_STRING_T str, size_t *len)`
/// WARNING: param names don't match C — Rust=(s) vs C=(str, len)
pub fn makequote(s: &[char]) -> Vec<char> {
    // c:1201
    // c:1170-1173 — count qtct = number of `'` chars.
    let qtct = s.iter().filter(|&&c| c == '\'').count();
    // c:1174 — `*len += 2 + qtct*3`. Output capacity: 2 (outer
    // quotes) + len + qtct*3 (each ' becomes '\\'').
    let mut out = Vec::<char>::with_capacity(s.len() + 2 + qtct * 3);
    out.push('\''); // c:1176 *l++ = '\''
    for &c in s {
        // c:1177-1184
        if c == '\'' {
            // c:1179-1182 — ' → '\''
            out.push('\'');
            out.push('\\');
            out.push('\'');
            out.push('\'');
        } else {
            out.push(c);
        }
    }
    out.push('\''); // c:1185 *l++ = '\''
    out
}

/// Port of `static char *namedcmdstr` from `Src/Zle/zle_misc.c:1229`.
pub static namedcmdstr: std::sync::Mutex<String> = // c:1229
    std::sync::Mutex::new(String::new());

/// Port of `static LinkList namedcmdll` from `Src/Zle/zle_misc.c:1230`.
pub static namedcmdll: std::sync::Mutex<Vec<String>> = // c:1235
    std::sync::Mutex::new(Vec::new());

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1231`.
pub static namedcmdambig: std::sync::atomic::AtomicUsize = // c:1235
    std::sync::atomic::AtomicUsize::new(0);

/// Direct port of `static int scancompcmd(HashNode hn, UNUSED(int flags))`
/// from `Src/Zle/zle_misc.c:1235`.
/// Port of `scancompcmd()` from `Src/Zle/zle_misc.c:1235`. — C decl `scancompcmd(HashNode hn, UNUSED(int flags))`
pub fn scancompcmd(name: &str) -> i32 {
    // c:1235
    // c:1240 — `if (strpfx(namedcmdstr, t->nam))`.
    let prefix = namedcmdstr.lock().unwrap().clone();
    if !name.starts_with(&prefix) {
        return 0;
    }
    let mut ll = namedcmdll.lock().unwrap();
    let first = ll.first().cloned();
    ll.push(name.to_string()); // c:1241 addlinknode
    if let Some(f) = first {
        // c:1242 — `pfxlen(peekfirst(namedcmdll), t->nam)`.
        let l = f
            .bytes()
            .zip(name.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if l < namedcmdambig.load(Ordering::Relaxed) {
            namedcmdambig.store(l, Ordering::Relaxed); // c:1243
        }
    } else {
        namedcmdambig.store(name.len(), Ordering::Relaxed);
    }
    0
}

/// Port of `NAMLEN` from `Src/Zle/zle_misc.c:1249`. Maximum length
/// of a widget name buffer used by `executenamedcommand` for
/// `execute-named-command` / `where-is`. The C source declares this
/// as a macro just before the local-keymap fixture.
pub const NAMLEN: usize = 60; // c:1249

/// Port of `executenamedcommand()` from `Src/Zle/zle_misc.c:1261`.
/// C decl: `executenamedcommand(char *prmt)`
/// Prompts the user for a widget
/// name (with name-completion via thingytab), then resolves the
/// answer to a Thingy.
///
/// **Substrate trade-off:** the interactive prompt path requires a
/// live ZLE input loop (`getfullchar`/`displaywholeline` machinery)
/// that compcore-call-context ported can't easily reach. Rust port
/// instead reads `$REPLY` from the canonical paramtab — the same
/// var that `read-command` widgets populate — so user widgets that
/// shell out to interactive prompts (`read-command -p PROMPT`) get
/// their answer surfaced here.
pub fn executenamedcommand(prompt: &str) -> Option<String> {
    // c:1261
    let _ = prompt;
    // c:1304 — `bindztrdup(name)` resolves the typed widget. Rust
    // path reads $REPLY (set by widgets like `read-command`).
    crate::ported::params::getsparam("REPLY") // c:1304
        .filter(|s| !s.is_empty())
}

/// Port of `struct suffixset` from `Src/Zle/zle_misc.c:1530`. One node
/// in the auto-removable suffix list.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct suffixset {
    // c:1530
    /// Type bits (SUFTYP_POSSTR/POSRNG/etc.).
    pub tp: i32,
    /// Flag bits (SUFFLAGS_SPACE etc.).
    pub flags: i32,
    /// Characters to match (for *STR types).
    pub chars: Vec<char>,
    /// Length of `chars`.
    pub lenstr: i32,
    /// Suffix length to remove on insert.
    pub lensuf: i32,
}

// Suffix system                                                            // c:1500
/// Port of `addsuffix()` from `Src/Zle/zle_misc.c:1558`.
/// C decl: `addsuffix(int tp, int flags, ZLE_STRING_T chars, int lenstr, int lensuf)`
pub fn addsuffix(tp: i32, flags: i32, chars: Vec<char>, lenstr: i32, lensuf: i32) {
    // c:1558
    // c:1561 — newsuf = zalloc(sizeof(struct suffixset));
    // c:1562 — newsuf->next = suffixlist;
    // c:1563 — suffixlist = newsuf;            ← prepended to head
    // c:1565-1573 — copy tp/flags/chars/lenstr/lensuf into newsuf.
    // Rust mirrors prepend via insert(0, …) so the iteration order
    // in the c:1758 walk (`for ss=suffixlist; ss; ss=ss->next`) sees
    // most-recently-added first, matching C.
    let newsuf = suffixset {
        tp,                                                  // c:1565
        flags,                                               // c:1566
        chars: if lenstr != 0 { chars } else { Vec::new() }, // c:1567-1571
        lenstr,                                              // c:1572
        lensuf,                                              // c:1573
    };
    suffixlist().lock().unwrap().insert(0, newsuf);
}

/// Port of `addsuffixstring()` from `Src/Zle/zle_misc.c:1580`.
/// C decl: `addsuffixstring(int tp, int flags, char *chars, int lensuf)`
pub fn addsuffixstring(tp: i32, flags: i32, chars: &str, lensuf: i32) {
    // c:1580
    // C body: `chars = ztrdup(chars); suffixstr = stringaszleline(...);
    //          addsuffix(tp, flags, suffixstr, slen, lensuf)`.
    let chars_vec: Vec<char> = chars.chars().collect();
    let slen = chars_vec.len() as i32;
    addsuffix(tp, flags, chars_vec, slen, lensuf);
}

/// Port of `makesuffix()` from `Src/Zle/zle_misc.c:1598`.
/// C decl: `makesuffix(int n)`
/// Reads `$ZLE_REMOVE_SUFFIX_CHARS` from
/// paramtab and registers it as the active suffix-removal char set
/// via `addsuffixstring`. Defaults to ` \t\n;&|` when the param is
/// unset.
pub fn makesuffix(n: i32) {
    // c:1598
    // c:1602-1603 — `suffixchars = getsparam_u("ZLE_REMOVE_SUFFIX_CHARS")`.
    let suffix_chars = crate::ported::params::getsparam("ZLE_REMOVE_SUFFIX_CHARS")
        .unwrap_or_else(|| " \t\n;&|".to_string()); // default
    addsuffixstring(
        crate::ported::zle::zle_h::SUFTYP_POSSTR,
        0,
        &suffix_chars,
        n,
    ); // c:1605
       // c:1607-1609 — ZLE_SPACE_SUFFIX_CHARS added second so it takes precedence.
    if let Some(space_chars) = crate::ported::params::getsparam("ZLE_SPACE_SUFFIX_CHARS") {
        if !space_chars.is_empty() {
            addsuffixstring(
                crate::ported::zle::zle_h::SUFTYP_POSSTR,
                crate::ported::zle::zle_h::SUFFLAGS_SPACE,
                &space_chars,
                n,
            );
        }
    }
    // c:1611-1612 — record the active suffix length + no-insert-remove flag.
    // Without this the auto-added completion suffix (e.g. the space after a
    // unique match) was never a tracked removable suffix: `suffixlen` stayed
    // 0, so the suffix region highlight (bold) was never applied and the
    // suffix was not auto-stripped by the next delimiter keystroke.
    suffixlen.store(n, std::sync::atomic::Ordering::Relaxed);
    suffixnoinsrem.store(1, std::sync::atomic::Ordering::Relaxed);
}

/// Port of `makeparamsuffix()` from `Src/Zle/zle_misc.c:1623`.
/// C decl: `makeparamsuffix(int br, int n)`
pub fn makeparamsuffix(br: i32, n: i32) {
    // c:1623
    // C body (c:1692-1697): `charstr = ":[#%?-+="; lenstr = (br ||
    //                       unset(KSHARRAYS)) ? 2 : strlen(charstr);
    //                       addsuffix(SUFTYP_POSSTR, 0, charstr, lenstr, n)`.
    let charstr: Vec<char> = ":[#%?-+=".chars().collect();
    let kshcheck = !isset(KSHARRAYS);
    let lenstr = if br != 0 || kshcheck {
        2
    } else {
        charstr.len() as i32
    };
    let prefix: Vec<char> = charstr.iter().take(lenstr as usize).copied().collect();
    addsuffix(0, 0, prefix, lenstr, n);
}

/// Port of `static char *suffixfunc;` from `Src/Zle/zle_misc.c:1545`.
/// Name of the function to call after auto-suffix is consumed.
/// Set by `makesuffixstr(f, ...)`; read by `iremovesuffix` to fire
/// the user hook on auto-removal.
pub static suffixfunc: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Port of `int suffixnoinsrem;` from `Src/Zle/zle_misc.c:1549`.
/// "Whether to remove suffix on uninsertable characters" — set by
/// `makesuffixstr` from the `\-` / `^`-inverted class flag.
pub static suffixnoinsrem: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export int suffixlen;` from `Src/Zle/zle_misc.c:1554`.
/// "Length of the currently active, auto-removable suffix." Consumed
/// by `iremovesuffix` for the actual delete.
pub static suffixlen: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `makesuffixstr()` from `Src/Zle/zle_misc.c:1642`.
/// C decl: `makesuffixstr(char *f, char *s, int n)`
/// Three-way dispatch:
///   - `f` set → register `f` as the post-insert hook (suffixfunc)
///   - `s` set → parse char-class spec (`^/!` invert; `\-` flag;
///     `a-z` ranges) into addsuffix calls
///   - neither → fall back to `makesuffix(n)` default char set
pub fn makesuffixstr(f: Option<&str>, s: Option<&str>, n: i32) {
    // c:1642
    if let Some(f_str) = f {
        // c:1644
        // c:1645-1647 — `zsfree(suffixfunc); suffixfunc = ztrdup(f);
        //                suffixlen = n;`.
        if let Ok(mut g) = suffixfunc.lock() {
            *g = Some(f_str.to_string()); // c:1646
        }
        suffixlen.store(n, std::sync::atomic::Ordering::Relaxed); // c:1647
    } else if let Some(s_str) = s {
        // c:1648
        let mut inv: i32; // c:1649
        let _i: usize; // c:1649
        let mut z: i32 = 0; // c:1649
        let s_iter_start; // c:1650 ZLE_STRING_T s
        if s_str.starts_with('^') || s_str.starts_with('!') {
            // c:1652
            inv = 1; // c:1653
            s_iter_start = &s_str[1..]; // c:1654 `s++`
        } else {
            inv = 0; // c:1656
            s_iter_start = s_str;
        }
        // c:1657 — `s = getkeystring(s, &i, GETKEYS_SUFFIX, &z);`
        let (decoded, consumed) = crate::ported::utils::getkeystring(s_iter_start);
        // GETKEYS_SUFFIX scan sets `z` when `\-` appears; current
        // getkeystring port doesn't expose `z` separately. The `\-`
        // detection is observable inline by scanning the literal arg
        // for `\-` prior to getkeystring's escape collapse.
        if s_iter_start.contains("\\-") {
            // c:1657 z out-param
            z = 1;
        }
        let _ = consumed;
        // c:1658 — `s = metafy(s, i, META_USEHEAP);` (no-op for UTF-8 String)
        // c:1659 — `ws = stringaszleline(s, 0, &i, NULL, NULL);`
        let ws = crate::ported::zle::zle_utils::stringaszleline(&decoded, 0, None, None, None);
        let mut i: usize = ws.len();

        /*
         * Remove suffix on uninsertable characters if `\-` was given
         * and the character class wasn't negated -- or vice versa.
         */
        // c:1661-1662
        suffixnoinsrem.store(z ^ inv, std::sync::atomic::Ordering::Relaxed); // c:1663
        suffixlen.store(n, std::sync::atomic::Ordering::Relaxed); // c:1664

        // c:1666-1689 — walk `ws`, peeking for `a-b` range form.
        let mut lasts: usize = 0; // c:1666
        let mut wptr: usize = 0; // c:1666
        while i > 0 {
            // c:1667
            if i >= 3 && ws.get(wptr + 1) == Some(&'-') {
                // c:1668
                if wptr > lasts {
                    // c:1671
                    let span: Vec<char> = ws[lasts..wptr].to_vec();
                    let lenstr = (wptr - lasts) as i32;
                    addsuffix(
                        if inv != 0 {
                            SUFTYP_NEGSTR
                        } else {
                            SUFTYP_POSSTR
                        },
                        0,
                        span,
                        lenstr,
                        n,
                    ); // c:1672-1673
                }
                let mut s_arr: Vec<char> = Vec::with_capacity(2); // c:1669 ZLE_CHAR_T str[2]
                s_arr.push(ws[wptr]); // c:1674
                s_arr.push(ws[wptr + 2]); // c:1675
                addsuffix(
                    if inv != 0 {
                        SUFTYP_NEGRNG
                    } else {
                        SUFTYP_POSRNG
                    },
                    0,
                    s_arr,
                    2,
                    n,
                ); // c:1676-1677

                wptr += 3; // c:1679
                i -= 3; // c:1680
                lasts = wptr; // c:1681
            } else {
                wptr += 1; // c:1683
                i -= 1; // c:1684
                if i == 0 && wptr == ws.len() {
                    // Final char will be appended by the post-loop span
                    // below; nothing to do here.
                }
                // Catch the `for` loop's `i--` semantics — the C peeks two
                // ahead so when i<3 we can't enter the range branch.
                if wptr >= ws.len() {
                    break;
                }
            }
        }
        if wptr > lasts {
            // c:1687
            let span: Vec<char> = ws[lasts..wptr].to_vec();
            let lenstr = (wptr - lasts) as i32;
            addsuffix(
                if inv != 0 {
                    SUFTYP_NEGSTR
                } else {
                    SUFTYP_POSSTR
                },
                0,
                span,
                lenstr,
                n,
            ); // c:1688-1689
        }
        // c:1690 — `free(ws);` (Rust drop)
        let _ = (lasts, wptr);
        // Bind `inv` to avoid unused warning if all branches above happen
        // to skip the inverter (constant-folded short circuit).
        inv += 0;
        let _ = inv;
    } else {
        makesuffix(n); // c:1692
    }
}

// Remove suffix, if there is one, when inserting character c.             // c:1699
/// Port of `iremovesuffix()` from `Src/Zle/zle_misc.c:1699`.
/// C decl: `iremovesuffix(ZLE_INT_T c, int keep)`
/// Walks `suffixlist`; for each
/// matching entry, removes `lensuf` chars before `ZLECS` in
/// `ZLELINE` (unless `keep` is set), then either calls the
/// registered `suffixfunc` or just clears the list.
pub fn iremovesuffix(c: i32, keep: i32) -> i32 {
    // c:1699

    // c:1701 — `if (suffixfunc) { ... }` — run shfunc if registered.
    let sf = SUFFIXFUNC
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    if !sf.is_empty() {
        // c:1701
        // c:1703 — `getshfunc(suffixfunc)`.
        if let Some(mut shfunc) = crate::ported::utils::getshfunc(&sf) {
            // c:1720-1728 — build args [suffixfunc, suffixlen] and
            // fire the suffix-function hook under SFC_COMPLETE.
            let suffix_len = SUFFIXLEN.load(Ordering::Relaxed);
            let largs: Vec<String> = vec![sf.clone(), suffix_len.to_string()];
            // c:1728 — `doshfunc(shfunc, args, 1);`.
            let name_for_body = sf.clone();
            let body_args: Vec<String> = vec![suffix_len.to_string()];
            let body_runner = move || -> i32 {
                crate::ported::exec::run_function_body(&name_for_body, &body_args).unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(&mut shfunc, largs, true, body_runner);
        }
        // c:1729 — `zsfree(suffixfunc); suffixfunc = NULL`.
        if let Ok(mut g) = SUFFIXFUNC
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
        {
            g.clear();
        }
    }

    // c:1755-1813 — suffixlist walk, matching `ch` by suffix TYPE.
    use crate::ported::zle::zle_h::{
        NO_INSERT_CHAR, SUFFLAGS_SPACE, SUFTYP_NEGRNG, SUFTYP_NEGSTR, SUFTYP_POSRNG, SUFTYP_POSSTR,
    };
    let list = suffixlist().lock().map(|g| g.clone()).unwrap_or_default();
    let mut sl: i32 = 0;
    let mut sflags: i32 = 0;
    let ch = c; // ZLE_CHAR_T codepoint
    if c == NO_INSERT_CHAR {
        // c:1757 — nothing inserted: remove only if the `\-` (noinsrem) flag.
        sl = if suffixnoinsrem.load(Ordering::Relaxed) != 0 {
            SUFFIXLEN.load(Ordering::Relaxed)
        } else {
            0
        };
    } else {
        // c:1770-1813 — positive lists remove `lensuf`; negative lists block
        // removal on a match and otherwise carry their `lensuf` as the
        // fall-through (`negsuflen`) used if no positive match is found.
        let mut negsuflen = 0i32;
        let mut found = false;
        for entry in list.iter() {
            match entry.tp {
                t if t == SUFTYP_POSSTR => {
                    // c:1775 — ZS_memchr(chars, ch, lenstr)
                    if entry.chars.iter().any(|&x| x as i32 == ch) {
                        sl = entry.lensuf;
                        found = true;
                    }
                }
                t if t == SUFTYP_NEGSTR => {
                    // c:1782
                    if entry.chars.iter().any(|&x| x as i32 == ch) {
                        sl = 0;
                        found = true;
                    } else {
                        negsuflen = entry.lensuf;
                    }
                }
                t if t == SUFTYP_POSRNG => {
                    // c:1791 — chars[0] <= ch <= chars[1]
                    if entry.chars.len() >= 2
                        && (entry.chars[0] as i32) <= ch
                        && ch <= (entry.chars[1] as i32)
                    {
                        sl = entry.lensuf;
                        found = true;
                    }
                }
                t if t == SUFTYP_NEGRNG => {
                    // c:1799
                    if entry.chars.len() >= 2
                        && (entry.chars[0] as i32) <= ch
                        && ch <= (entry.chars[1] as i32)
                    {
                        sl = 0;
                        found = true;
                    } else {
                        negsuflen = entry.lensuf;
                    }
                }
                _ => {}
            }
            if found {
                // c:1806-1809
                sflags = entry.flags;
                break;
            }
        }
        if !found {
            // c:1812-1813
            sl = negsuflen;
        }
    }

    // c:1788-1795 — if sl > 0 && !keep, drop `sl` chars before the cursor
    // from the LIVE editor line. `doinsert` — the sole keep==0 caller — is
    // about to insert into the interactive ZLE buffer (zle_main::ZLELINE, a
    // Vec<char>), so the removable suffix must be stripped from that same
    // buffer, not the compcore metafied completion line. Dropping from the
    // wrong buffer left the completion suffix in place, so typing a
    // suffix-removal char (space, `;`, `&`, …) produced a doubled character.
    if sl > 0 && keep == 0 {
        let cs = ZLECS.load(SeqCst);
        let drop_n = (sl as usize).min(cs);
        let new_cs = cs - drop_n;
        if let Ok(mut g) = ZLELINE.lock() {
            if cs <= g.len() {
                g.drain(new_cs..cs);
            }
            ZLELL.store(g.len(), SeqCst);
        }
        ZLECS.store(new_cs, SeqCst);
        // c:1819-1826 — SUFFLAGS_SPACE: after removing the suffix, add a space
        // and advance over it (the `-r`/`-R` spec asked for a trailing space).
        if sflags & SUFFLAGS_SPACE != 0 {
            let cs2 = ZLECS.load(SeqCst);
            if let Ok(mut g) = ZLELINE.lock() {
                let pos = cs2.min(g.len());
                g.insert(pos, ' ');
                ZLELL.store(g.len(), SeqCst);
            }
            ZLECS.store(cs2 + 1, SeqCst);
        }
    }

    // c:1796 — clear suffix list.
    fixsuffix();
    0 // c:1797
}

// Fix the suffix in place, if there is one, making it non-removable.      // c:1824
/// Port of `fixsuffix()` from Src/Zle/zle_misc.c:1824.
pub fn fixsuffix() {
    // c:1824
    // C body (c:1826-1832): `while (suffixlist) { next = sl->next;
    //                       if (sl->lenstr) zfree(sl->chars, ...);
    //                       zfree(sl, ...); suffixlist = next; }
    //                       suffixlen = 0`.
    suffixlist().lock().unwrap().clear();
    SUFFIXLEN.store(0, SeqCst);
}
/// `DONE` static.
pub static DONE: AtomicI32 = AtomicI32::new(0); // c:79

/// Port of `mod_export int suffixlen` from `Src/Zle/zle_misc.c:1554`.
/// Length of the currently active, auto-removable suffix.
///
/// Re-export alias of the lowercase [`suffixlen`] static — C has ONE
/// `suffixlen`. Two separate atomics existed (`makesuffix` set lowercase and
/// the refresh suffix-highlight read it; `fixsuffix`/`iremovesuffix` and the
/// `$SUFFIXLEN` ZLE param used uppercase), so the completion suffix was never
/// cleared on the next keystroke — the bold highlight lingered onto the newly
/// typed character. Aliasing collapses them to one atomic so set and clear
/// hit the same state.
pub use self::suffixlen as SUFFIXLEN;

/// Port of `struct suffixset *suffixlist` from `Src/Zle/zle_misc.c:1540`.
/// Stack of registered auto-removable suffixes.
pub static SUFFIXLIST: std::sync::OnceLock<std::sync::Mutex<Vec<suffixset>>> =
    std::sync::OnceLock::new();

/// Port of `int suffixnoinsrem` from `Src/Zle/zle_misc.c:1549`.
/// Suppresses inserted-character suffix removal when set.
pub static SUFFIXNOINSREM: AtomicI32 = AtomicI32::new(0); // c:1549

/// Port of `static ZLE_INT_T vfindchar` from `Src/Zle/zle_move.c:734`.
/// The character argument to the most recent vi-find* command.
pub static VFINDCHAR: AtomicI32 = AtomicI32::new(0); // c:734

/// Port of `static int vfinddir, tailadd` from `Src/Zle/zle_move.c:735`.
/// vfinddir = +1 forward, -1 backward; tailadd = +1 land just after,
/// -1 land just before, 0 land on the char itself.
pub static VFINDDIR: AtomicI32 = AtomicI32::new(0); // c:735
/// `TAILADD` static.
pub static TAILADD: AtomicI32 = AtomicI32::new(0); // c:735

/// Port of `static int kct` from `Src/Zle/zle_misc.c:523`. Index into
/// the kill ring for the next yank-pop, or -1 for the original cutbuf
/// at the start of a yank sequence.
pub static KCT: AtomicI32 = AtomicI32::new(-1); // c:523

/// Port of `static int yankcs` from `Src/Zle/zle_misc.c:523`. Saved
/// cursor position at the start of the most-recent yank — `yank-pop`
/// rewinds to this and re-inserts the next ring entry.
pub static YANKCS: AtomicI32 = AtomicI32::new(0); // c:523

/// Port of `static Cutbuffer kctbuf` from `Src/Zle/zle_misc.c:529` —
/// "The original cutbuffer, either cutbuf or one of the vi buffers."
/// C stores a pointer; the Rust port encodes the referent as an index
/// (same sentinel trick as MARK's usize::MAX): -2 = NULL, -1 = the
/// unnamed CUTBUF, 0..=35 = vibuf\[n\].
pub static KCTBUF_SEL: AtomicI32 = AtomicI32::new(-2); // c:529

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1231`.
/// Length of the longest unambiguous prefix among all matched
/// `namedcmd` widget names — drives `execute-named-command` ambig
/// resolution. Mirrored on `NamedCmdState.namedcmdambig` already;
/// this is the searchable counterpart.
pub static NAMEDCMDAMBIG: AtomicI32 = AtomicI32::new(0); // c:1231

// ===== Pre/post-display strings (Src/Zle/zle_main.c) =====
//
// `ZLE_STRING_T predisplay` / `ZLE_STRING_T postdisplay` — text
// shown before/after the line buffer (used by `zle -K -P` and
// completion menu rendering).

/// Port of `ZLE_STRING_T predisplay` (zle_main.c). Storage for the
/// `$PREDISPLAY` parameter value.
pub static PREDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `ZLE_STRING_T postdisplay` (zle_main.c). Storage for the
/// `$POSTDISPLAY` parameter value.
pub static POSTDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `char *previous_search` from `Src/Zle/zle_hist.c`. Set
/// by `historyincrementalsearch*` on accept; read by `$LSEARCH`.
pub static PREVIOUS_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();

/// Port of `char *previous_aborted_search` from
/// `Src/Zle/zle_hist.c`. Set on isearch abort; read by `$LASEARCH`.
pub static PREVIOUS_ABORTED_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();

/// File-scope `char *suffixfunc` from `Src/Zle/zle_misc.c` — the
/// registered shfunc name run by `iremovesuffix` on suffix match.
pub static SUFFIXFUNC: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new(); // zle_misc.c

// `PasteBuffer` deleted — Rust-invented struct that wasn't referenced
// anywhere. The C source uses `Cutbuffer` (zle.h:342, ported as
// `cutbuffer` in zle_h.rs:506) and the `cutbuf` global to back yank
// operations; no separate paste-buffer type exists.

// insert a zle string, with repetition and suffix removal              // c:33

/// Self insert - insert the typed character
/// Stands in for selfinsert(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `selfinsert()` (`Src/Zle/zle_misc.c:113`), whose
/// faithful port is `selfinsert()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn self_insert(c: char) {
    // c:113
    ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst), c);
    ZLECS.fetch_add(1, SeqCst);
    ZLELL.fetch_add(1, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Self insert unmeta - insert character with meta bit stripped
/// Stands in for selfinsertunmeta(char **args) from zle_misc.c

/// Accept line - return the current line for execution
/// Stands in for acceptline(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `acceptline()` (`Src/Zle/zle_misc.c:401`), whose
/// faithful port is `acceptline()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn accept_line() -> String {
    // c:401
    ZLELINE.lock().unwrap().iter().collect()
}

/// Accept and hold - accept line but keep it in the buffer
/// Stands in for acceptandhold(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `acceptandhold()` (`Src/Zle/zle_misc.c:409`), whose
/// faithful port is `acceptandhold()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn accept_and_hold() -> String {
    ZLELINE.lock().unwrap().iter().collect()
}

/// Quoted insert - insert next char literally
/// Stands in for quotedinsert(char **args) from zle_misc.c

/// Bracketed paste - handle paste mode
/// Stands in for bracketedpaste(char **args) from zle_misc.c

/// Delete char under cursor
/// Stands in for deletechar(char **args) from zle_misc.c

/// Delete char before cursor
/// Stands in for backwarddeletechar(char **args) from zle_misc.c

/// Kill from cursor to end of line
/// Stands in for killline(char **args) from zle_misc.c

/// Kill from beginning of line to cursor
/// Stands in for backwardkillline(char **args) from zle_misc.c

/// Kill entire buffer
/// Stands in for killbuffer(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `killbuffer()` (`Src/Zle/zle_misc.c:215`), whose
/// faithful port is `killbuffer()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn kill_buffer() {
    if !ZLELINE.lock().unwrap().is_empty() {
        let text: Vec<char> = ZLELINE.lock().unwrap().drain(..).collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.store(0, SeqCst);
        ZLECS.store(0, SeqCst);
        MARK.store(0, SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Kill whole line (including newlines in multi-line mode)
/// Stands in for killwholeline(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `killwholeline()` (`Src/Zle/zle_misc.c:195`), whose
/// faithful port is `killwholeline()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn kill_whole_line() {
    kill_buffer();
}

/// Swap cursor and mark.
/// Stands in for `exchangepointandmark(UNUSED(char **args))` from Src/Zle/zle_move.c:496. The
/// C source has additional zmult-based behaviour (zmult==0 just
/// activates the region without swapping; zmult>0 also activates).
/// This bare method only swaps; the widget-level
/// `widget_exchange_point_and_mark` honours the count semantics.
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `exchangepointandmark()` (`Src/Zle/zle_move.c:496`), whose
/// faithful port is `exchangepointandmark()` in zle_move.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn exchange_point_and_mark() {
    std::mem::swap(&mut ZLECS.load(SeqCst), &mut MARK.load(SeqCst));
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Set mark at the current cursor position.
/// Stands in for `setmarkcommand(UNUSED(char **args))` from Src/Zle/zle_move.c:483 with the
/// activate-region branch elided. The widget-level
/// `widget_set_mark_command` covers the negative-count
/// deactivate path that the bare C source supports.
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `setmarkcommand()` (`Src/Zle/zle_move.c:483`), whose
/// faithful port is `setmarkcommand()` in zle_move.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn set_mark_here() {
    MARK.store(ZLECS.load(SeqCst), SeqCst);
}

/// Copy region as kill
/// Stands in for copyregionaskill(char **args) from zle_misc.c

/// Kill region (between point and mark)
/// Stands in for killregion(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `killregion()` (`Src/Zle/zle_misc.c:463`), whose
/// faithful port is `killregion()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn kill_region() {
    // c:463
    let (start, end) = if ZLECS.load(SeqCst) < MARK.load(SeqCst) {
        (ZLECS.load(SeqCst), MARK.load(SeqCst))
    } else {
        (MARK.load(SeqCst), ZLECS.load(SeqCst))
    };

    let text: Vec<char> = ZLELINE.lock().unwrap().drain(start..end).collect();
    KILLRING.lock().unwrap().push_front(text);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }

    ZLELL.fetch_sub(end - start, SeqCst);
    ZLECS.store(start, SeqCst);
    MARK.store(start, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Yank pop - cycle through kill ring
/// Stands in for yankpop(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `yankpop()` (`Src/Zle/zle_misc.c:728`), whose
/// faithful port is `yankpop()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn yank_pop() {
    // c:728
    if !YANKLAST.load(SeqCst) || KILLRING.lock().unwrap().is_empty() {
        return;
    }

    // Remove previously yanked text
    let prev_len = KILLRING
        .lock()
        .unwrap()
        .front()
        .map(|v| v.len())
        .unwrap_or(0);
    let start = MARK.load(SeqCst);
    for _ in 0..prev_len {
        if start < ZLELINE.lock().unwrap().len() {
            ZLELINE.lock().unwrap().remove(start);
        }
    }
    ZLECS.store(start, SeqCst);
    ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);

    // Rotate kill ring
    if let Some(front) = KILLRING.lock().unwrap().pop_front() {
        KILLRING.lock().unwrap().push_back(front);
    }

    // Insert new text
    if let Some(text) = KILLRING.lock().unwrap().front() {
        for &c in text {
            ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst), c);
            ZLECS.fetch_add(1, SeqCst);
        }
        ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
    }

    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Transpose chars
/// Stands in for transposechars(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `transposechars()` (`Src/Zle/zle_misc.c:313`), whose
/// faithful port is `transposechars()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn transpose_chars() {
    if ZLECS.load(SeqCst) == 0 || ZLELL.load(SeqCst) < 2 {
        return;
    }

    let pos = if ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
        ZLECS.load(SeqCst) - 1
    } else {
        ZLECS.load(SeqCst)
    };

    if pos > 0 {
        ZLELINE.lock().unwrap().swap(pos - 1, pos);
        ZLECS.store(pos + 1, SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Capitalize the next word: title-case the first letter, lowercase
/// the rest of the word.
/// Stands in for `capitalizeword(UNUSED(char **args))` from Src/Zle/zle_word.c (the C source
/// uses `casemodifyword()` with a CASMOD_CAPS flag). Mirrors emacs's
/// M-c convention. Cursor lands past the modified word.
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `capitalizeword()` (`Src/Zle/zle_word.c:577`), whose
/// faithful port is `capitalizeword()` in zle_word.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn capitalize_word() {
    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && !ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        ZLECS.fetch_add(1, SeqCst);
    }

    if ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphabetic()
    {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[ZLECS.load(SeqCst)] = __g[ZLECS.load(SeqCst)]
                .to_uppercase()
                .next()
                .unwrap_or(__g[ZLECS.load(SeqCst)]);
        }
        ZLECS.fetch_add(1, SeqCst);
    }

    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[ZLECS.load(SeqCst)] = __g[ZLECS.load(SeqCst)]
                .to_lowercase()
                .next()
                .unwrap_or(__g[ZLECS.load(SeqCst)]);
        }
        ZLECS.fetch_add(1, SeqCst);
    }

    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Lowercase the next word.
/// Stands in for `downcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
/// `casemodifyword()` with the CASMOD_LOWER flag.
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `downcaseword()` (`Src/Zle/zle_word.c:555`), whose
/// faithful port is `downcaseword()` in zle_word.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn downcase_word() {
    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && !ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        ZLECS.fetch_add(1, SeqCst);
    }

    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[ZLECS.load(SeqCst)] = __g[ZLECS.load(SeqCst)]
                .to_lowercase()
                .next()
                .unwrap_or(__g[ZLECS.load(SeqCst)]);
        }
        ZLECS.fetch_add(1, SeqCst);
    }

    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Uppercase the next word.
/// Stands in for `upcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
/// `casemodifyword()` with the CASMOD_UPPER flag.
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `upcaseword()` (`Src/Zle/zle_word.c:533`), whose
/// faithful port is `upcaseword()` in zle_word.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn upcase_word() {
    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && !ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        ZLECS.fetch_add(1, SeqCst);
    }

    while ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
        && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)].is_alphanumeric()
    {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[ZLECS.load(SeqCst)] = __g[ZLECS.load(SeqCst)]
                .to_uppercase()
                .next()
                .unwrap_or(__g[ZLECS.load(SeqCst)]);
        }
        ZLECS.fetch_add(1, SeqCst);
    }

    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Transpose words
/// Stands in for transpose words logic
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `transposewords()` (`Src/Zle/zle_word.c:652`), whose
/// faithful port is `transposewords()` in zle_word.rs.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn transpose_words() {
    if ZLELL.load(SeqCst) < 3 {
        return;
    }

    // Find boundaries of two words
    let mut end2 = ZLECS.load(SeqCst);
    while end2 < ZLELL.load(SeqCst) && ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
        end2 += 1;
    }
    while end2 < ZLELL.load(SeqCst) && !ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
        end2 += 1;
    }
    while end2 < ZLELL.load(SeqCst) && ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
        end2 += 1;
    }

    let mut start2 = end2;
    while start2 > 0 && ZLELINE.lock().unwrap()[start2 - 1].is_alphanumeric() {
        start2 -= 1;
    }

    let mut end1 = start2;
    while end1 > 0 && !ZLELINE.lock().unwrap()[end1 - 1].is_alphanumeric() {
        end1 -= 1;
    }

    let mut start1 = end1;
    while start1 > 0 && ZLELINE.lock().unwrap()[start1 - 1].is_alphanumeric() {
        start1 -= 1;
    }

    if start1 < end1 && start2 < end2 {
        let word1: Vec<char> = ZLELINE.lock().unwrap()[start1..end1].to_vec();
        let word2: Vec<char> = ZLELINE.lock().unwrap()[start2..end2].to_vec();

        // Replace word2 first (higher index)
        ZLELINE.lock().unwrap().drain(start2..end2);
        for (i, c) in word1.iter().enumerate() {
            ZLELINE.lock().unwrap().insert(start2 + i, *c);
        }

        // Replace word1
        let new_end1 = end1 - (end2 - start2) + word1.len();
        let _new_start1 = start1;
        ZLELINE.lock().unwrap().drain(start1..end1);
        for (i, c) in word2.iter().enumerate() {
            ZLELINE.lock().unwrap().insert(start1 + i, *c);
        }

        ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
        ZLECS.store(new_end1, SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Quote line
/// Stands in for quoteline(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `quoteline()` (`Src/Zle/zle_misc.c:1187`), whose
/// faithful port is `quoteline()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn quote_line() {
    ZLELINE.lock().unwrap().insert(0, '\'');
    ZLELL.fetch_add(1, SeqCst);
    ZLECS.fetch_add(1, SeqCst);
    ZLELINE.lock().unwrap().push('\'');
    ZLELL.fetch_add(1, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Quote region
/// Stands in for quoteregion(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `quoteregion()` (`Src/Zle/zle_misc.c:1152`), whose
/// faithful port is `quoteregion()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn quote_region() {
    let (start, end) = if ZLECS.load(SeqCst) < MARK.load(SeqCst) {
        (ZLECS.load(SeqCst), MARK.load(SeqCst))
    } else {
        (MARK.load(SeqCst), ZLECS.load(SeqCst))
    };

    ZLELINE.lock().unwrap().insert(end, '\'');
    ZLELINE.lock().unwrap().insert(start, '\'');
    ZLELL.fetch_add(2, SeqCst);
    ZLECS.store(end + 2, SeqCst);
    MARK.store(start, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// What cursor position - display cursor info
/// Stands in for whatcursorposition(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `whatcursorposition()` (`Src/Zle/zle_misc.c:851`), whose
/// faithful port is `whatcursorposition()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn what_cursor_position() -> String {
    if ZLECS.load(SeqCst) >= ZLELL.load(SeqCst) {
        return format!(
            "point={} of {} (EOL)",
            ZLECS.load(SeqCst),
            ZLELL.load(SeqCst)
        );
    }

    let c = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)];
    let code = c as u32;
    format!(
        "Char: {} (0{:o}, {:?}, 0x{:x})  point {} of {} ({}%)",
        c,
        code,
        code,
        code,
        ZLECS.load(SeqCst),
        ZLELL.load(SeqCst),
        (ZLECS.load(SeqCst) * 100)
            .checked_div(ZLELL.load(SeqCst))
            .unwrap_or(0)
    )
}

/// Universal argument - multiply next command
/// Stands in for universalargument(char **args) from zle_misc.c

/// Digit argument - accumulate numeric argument
/// Stands in for digitargument(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `digitargument()` (`Src/Zle/zle_misc.c:950`), whose
/// faithful port is `digitargument()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn digit_argument(digit: u8) {
    if MULT.load(SeqCst) == 1 && !NEG_ARG.load(SeqCst) {
        MULT.store(0, SeqCst);
    }
    MULT.store(
        MULT.load(SeqCst)
            .saturating_mul(10)
            .saturating_add(digit as i32),
        SeqCst,
    );
}

/// Negative argument
/// Stands in for negargument(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `negargument()` (`Src/Zle/zle_misc.c:974`), whose
/// faithful port is `negargument()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn neg_argument() {
    NEG_ARG.store(!NEG_ARG.load(SeqCst), SeqCst);
}

/// Undefined key - beep
/// Stands in for undefinedkey(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `undefinedkey()` (`Src/Zle/zle_misc.c:892`), whose
/// faithful port is `undefinedkey()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn undefined_key() {
    print!("\x07"); // Bell
}

/// Send break - abort current operation
/// Stands in for sendbreak(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `sendbreak()` (`Src/Zle/zle_misc.c:1144`), whose
/// faithful port is `sendbreak()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn send_break() {
    ZLELINE.lock().unwrap().clear();
    ZLELL.store(0, SeqCst);
    ZLECS.store(0, SeqCst);
    MARK.store(0, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Vi put after cursor
/// Stands in for viputafter(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `viputafter()` (`Src/Zle/zle_misc.c:644`), whose
/// faithful port is `viputafter()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn vi_put_after() {
    if ZLECS.load(SeqCst) < ZLELL.load(SeqCst) {
        ZLECS.fetch_add(1, SeqCst);
    }
    yank();
    if ZLECS.load(SeqCst) > 0 {
        ZLECS.fetch_sub(1, SeqCst);
    }
}

/// Vi put before cursor
/// Stands in for viputbefore(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `viputbefore()` (`Src/Zle/zle_misc.c:608`), whose
/// faithful port is `viputbefore()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn vi_put_before() {
    yank();
}

/// Overwrite mode toggle
/// Stands in for overwritemode(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `overwritemode()` (`Src/Zle/zle_misc.c:843`), whose
/// faithful port is `overwritemode()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn overwrite_mode() {
    INSMODE.fetch_xor(1, SeqCst);
}

/// Copy previous word
/// Stands in for copyprevword(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `copyprevword()` (`Src/Zle/zle_misc.c:1066`), whose
/// faithful port is `copyprevword()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn copy_prev_word() {
    if ZLECS.load(SeqCst) == 0 {
        return;
    }

    // Find start of previous word
    let mut end = ZLECS.load(SeqCst);
    while end > 0 && ZLELINE.lock().unwrap()[end - 1].is_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && !ZLELINE.lock().unwrap()[start - 1].is_whitespace() {
        start -= 1;
    }

    if start < end {
        let word: Vec<char> = ZLELINE.lock().unwrap()[start..end].to_vec();
        for c in word {
            ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst), c);
            ZLECS.fetch_add(1, SeqCst);
        }
        ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Copy previous shell word (respects quoting)
/// Stands in for copyprevshellword(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `copyprevshellword()` (`Src/Zle/zle_misc.c:1108`), whose
/// faithful port is `copyprevshellword()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn copy_prev_shell_word() {
    // Simplified - doesn't handle full shell quoting
    copy_prev_word();
}

/// Pound insert - comment toggle for vi mode
/// Stands in for poundinsert(UNUSED(char **args)) from zle_misc.c
/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart: this name does not exist anywhere in
/// `Src/`. It is a simplified Rust-side convenience wrapper standing
/// in for the C widget `poundinsert()` (`Src/Zle/zle_misc.c:369`), whose
/// faithful port is `poundinsert()` in this file.
/// Deliberately left uncited so the port audit keeps reporting it.
pub fn pound_insert() {
    if !ZLELINE.lock().unwrap().is_empty() && ZLELINE.lock().unwrap()[0] == '#' {
        ZLELINE.lock().unwrap().remove(0);
        ZLELL.fetch_sub(1, SeqCst);
        if ZLECS.load(SeqCst) > 0 {
            ZLECS.fetch_sub(1, SeqCst);
        }
    } else {
        ZLELINE.lock().unwrap().insert(0, '#');
        ZLELL.fetch_add(1, SeqCst);
        ZLECS.fetch_add(1, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

// `processcmd` is `Src/Zle/zle_tricky.c:2971`, so its port lives in
// `src/ported/zle/zle_tricky.rs`. The signature-adapting shim that
// used to sit here (`fn processcmd(&[String]) -> i32` forwarding to
// it) is gone; `zle_bindings.rs` wraps the call in a closure for the
// `which-command` / `run-help` widget rows instead, the same way it
// does for every other zero-arg widget body.

// `zgetline` lives at its canonical C location (zle_hist.c:898) →
// `crate::ported::zle::zle_hist::zgetline`. The duplicate that
// used to live here returned a bare 0 with no bufstack pop.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No upstream C counterpart. C reads the file-scope global
/// `static struct suffixset *suffixlist;` (`Src/Zle/zle_misc.c:1540`)
/// directly; Rust needs this accessor because `OnceLock::get_or_init`
/// is the only way to lazily construct the shared `Mutex`.
/// Deliberately left uncited so the port audit keeps reporting it.
fn suffixlist() -> &'static std::sync::Mutex<Vec<suffixset>> {
    SUFFIXLIST.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptline_sets_done() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:401-404 — `done = 1; return 0`.
        DONE.store(0, SeqCst);
        let r = acceptline();
        assert_eq!(r, 0);
        assert_eq!(DONE.load(SeqCst), 1);
    }

    #[test]
    fn undefinedkey_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:892-894 — single `return 1` body.
        assert_eq!(undefinedkey(), 1);
    }

    #[test]
    fn sendbreak_sets_errflag_and_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Reset errflag so the OR-set is observable.
        errflag.store(0, Ordering::Relaxed);
        let r = sendbreak();
        // c:1147 — return 1.
        assert_eq!(r, 1);
        // c:1146 — both ERRFLAG_ERROR | ERRFLAG_INT set.
        let f = errflag.load(Ordering::Relaxed);
        assert_ne!(f & ERRFLAG_ERROR, 0);
        assert_ne!(f & ERRFLAG_INT, 0);
        // Reset for other tests.
        errflag.store(0, Ordering::Relaxed);
    }

    #[test]
    fn sendbreak_preserves_existing_errflag_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1146 — `errflag |= ...` (OR-equal, not assign).
        errflag.store(0x1000, Ordering::Relaxed); // pretend bit 12 was set
        sendbreak();
        let f = errflag.load(Ordering::Relaxed);
        // Pre-existing bit preserved.
        assert_ne!(f & 0x1000, 0);
        // New bits also set.
        assert_ne!(f & ERRFLAG_ERROR, 0);
        assert_ne!(f & ERRFLAG_INT, 0);
        errflag.store(0, Ordering::Relaxed);
    }

    // ---------- negargument / overwritemode real-port tests ----------

    #[test]
    fn negargument_sets_tmult_neg_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:976-981 — sets tmult=-1 + TMULT|NEG flags + prefixflag.
        zle_reset();
        // Ensure clean modifier state.
        ZMOD.lock().unwrap().tmult = 1;
        ZMOD.lock().unwrap().flags = 0;
        PREFIXFLAG.store(0, SeqCst);
        let r = negargument();
        assert_eq!(r, 0);
        assert_eq!(ZMOD.lock().unwrap().tmult, -1);
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_TMULT, 0);
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_NEG, 0);
        assert_ne!(PREFIXFLAG.load(SeqCst), 0);
    }

    #[test]
    fn negargument_refuses_when_tmult_in_flight() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:976-977 — if MOD_TMULT already set → return 1.
        zle_reset();
        ZMOD.lock().unwrap().flags |= MOD_TMULT;
        ZMOD.lock().unwrap().tmult = 7; // some pre-existing value
        let r = negargument();
        assert_eq!(r, 1);
        // tmult NOT clobbered (early return).
        assert_eq!(ZMOD.lock().unwrap().tmult, 7);
    }

    #[test]
    fn overwritemode_toggles_insmode() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:845 — `insmode ^= 1`.
        zle_reset();
        INSMODE.store(1, SeqCst);
        overwritemode();
        assert_eq!(INSMODE.load(SeqCst), 0);
        overwritemode();
        assert_eq!(INSMODE.load(SeqCst), 1);
    }

    // ---------- argumentbase real-port tests ----------

    #[test]
    fn argumentbase_with_arg_sets_base() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1043 — parse arg, c:1050 set zmod.base.
        zle_reset();
        let r = argumentbase(&["8".to_string()]);
        assert_eq!(r, 0);
        assert_eq!(ZMOD.lock().unwrap().base, 8);
        assert_ne!(PREFIXFLAG.load(SeqCst), 0);
        // c:1053-1056 — modifier reset.
        assert_eq!(ZMOD.lock().unwrap().mult, 1);
        assert_eq!(ZMOD.lock().unwrap().tmult, 1);
        assert_eq!(ZMOD.lock().unwrap().vibuf, 0);
    }

    #[test]
    fn argumentbase_no_arg_uses_zmod_mult() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1045 — fallback to zmod.mult when no arg.
        zle_reset();
        ZMOD.lock().unwrap().mult = 16;
        argumentbase(&[]);
        assert_eq!(ZMOD.lock().unwrap().base, 16);
    }

    #[test]
    fn argumentbase_rejects_below_two() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1047-1048 — base < 2 → return 1, state unchanged.
        zle_reset();
        ZMOD.lock().unwrap().base = 10;
        let r = argumentbase(&["1".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(ZMOD.lock().unwrap().base, 10); // unchanged
    }

    #[test]
    fn argumentbase_rejects_above_36() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1047-1048 — base > 36 → return 1.
        zle_reset();
        ZMOD.lock().unwrap().base = 10;
        let r = argumentbase(&["100".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(ZMOD.lock().unwrap().base, 10);
    }

    #[test]
    fn argumentbase_hex_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1043 — `zstrtol(s, NULL, 0)`: '0x10' → 16.
        zle_reset();
        argumentbase(&["0x10".to_string()]);
        assert_eq!(ZMOD.lock().unwrap().base, 16);
    }

    #[test]
    fn argumentbase_octal_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1043 — '010' → octal 8.
        zle_reset();
        argumentbase(&["010".to_string()]);
        assert_eq!(ZMOD.lock().unwrap().base, 8);
    }

    // ---------- parsedigit real-port tests ----------

    #[test]
    fn parsedigit_decimal_base() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1092 — base=10, '0'..'9' → 0..9.
        zle_reset();
        ZMOD.lock().unwrap().base = 10;
        assert_eq!(parsedigit(b'0' as i32), 0);
        assert_eq!(parsedigit(b'5' as i32), 5);
        assert_eq!(parsedigit(b'9' as i32), 9);
        // Out of range for base 10
        assert_eq!(parsedigit(b'a' as i32), -1);
    }

    #[test]
    fn parsedigit_octal_base() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1092 — base=8, '0'..'7'.
        zle_reset();
        ZMOD.lock().unwrap().base = 8;
        assert_eq!(parsedigit(b'7' as i32), 7);
        // '8' rejected (out of range for octal).
        assert_eq!(parsedigit(b'8' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_lowercase() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1083 — base=16, 'a'..'f' → 10..15.
        zle_reset();
        ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'a' as i32), 10);
        assert_eq!(parsedigit(b'f' as i32), 15);
        // 'g' out of range (only a..f for base 16).
        assert_eq!(parsedigit(b'g' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_uppercase() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1085 — base=16, 'A'..'F' → 10..15.
        zle_reset();
        ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'A' as i32), 10);
        assert_eq!(parsedigit(b'F' as i32), 15);
    }

    #[test]
    fn parsedigit_hex_digits_still_work() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1087 — base > 10 still accepts '0'..'9' via idigit branch.
        zle_reset();
        ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'7' as i32), 7);
    }

    #[test]
    fn parsedigit_strips_meta_bit() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1077 — `inkey &= 0x7f`. 0xb5 = '5' | 0x80 → strips to '5'.
        zle_reset();
        ZMOD.lock().unwrap().base = 10;
        assert_eq!(parsedigit(0x80 | (b'5' as i32)), 5);
    }

    // ---------- digitargument real-port tests ----------

    #[test]
    fn digitargument_first_digit_no_tmult() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1050-1051 — `if (!TMULT) tmult = 0`. First digit: tmult=0
        // then tmult = 0*10 + 1*5 = 5.
        zle_reset();
        ZMOD.lock().unwrap().flags = 0;
        ZMOD.lock().unwrap().base = 10;
        ZMOD.lock().unwrap().mult = 1; // sign = 1
        LASTCHAR.store((b'5' as i32) as i32, SeqCst);
        let r = digitargument();
        assert_eq!(r, 0);
        assert_eq!(ZMOD.lock().unwrap().tmult, 5);
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_TMULT, 0);
        assert_ne!(PREFIXFLAG.load(SeqCst), 0);
    }

    #[test]
    fn digitargument_second_digit_accumulates() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1058 — second digit: tmult = 5*10 + 1*7 = 57.
        zle_reset();
        ZMOD.lock().unwrap().flags = MOD_TMULT;
        ZMOD.lock().unwrap().tmult = 5;
        ZMOD.lock().unwrap().base = 10;
        ZMOD.lock().unwrap().mult = 1; // sign = 1
        LASTCHAR.store((b'7' as i32) as i32, SeqCst);
        digitargument();
        assert_eq!(ZMOD.lock().unwrap().tmult, 57);
    }

    #[test]
    fn digitargument_invalid_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1047-1048 — parsedigit < 0 → return 1.
        zle_reset();
        ZMOD.lock().unwrap().base = 10;
        LASTCHAR.store((b'a' as i32) as i32, SeqCst); // not a decimal digit
        assert_eq!(digitargument(), 1);
    }

    #[test]
    fn digitargument_neg_flag_replaces_tmult() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1054-1056 — MOD_NEG: tmult = sign * newdigit, NEG cleared.
        // sign = -1 (zmult<0); first digit '3' → tmult = -1*3 = -3.
        zle_reset();
        ZMOD.lock().unwrap().flags = MOD_TMULT | MOD_NEG;
        ZMOD.lock().unwrap().tmult = -1; // set by negargument
        ZMOD.lock().unwrap().base = 10;
        ZMOD.lock().unwrap().mult = -1; // negative → sign = -1
        LASTCHAR.store((b'3' as i32) as i32, SeqCst);
        digitargument();
        assert_eq!(ZMOD.lock().unwrap().tmult, -3);
        // NEG cleared.
        assert_ne!(!ZMOD.lock().unwrap().flags & MOD_NEG, 0);
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_TMULT, 0);
    }

    // ---------- transpose_swap real-port tests ----------

    #[test]
    fn transpose_swap_equal_halves() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:254 — swap two equal-length adjacent slices.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        ZLELL.store(6, SeqCst);
        // Swap [0..2]="ab" with [2..4]="cd" → "cdabef".
        transpose_swap(0, 2, 4);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "cdabef");
    }

    #[test]
    fn transpose_swap_unequal_halves() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // First chunk len 1, second len 3.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        ZLELL.store(6, SeqCst);
        // Swap [0..1]="a" with [1..4]="bcd" → "bcdaef".
        transpose_swap(0, 1, 4);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "bcdaef");
    }

    #[test]
    fn transpose_swap_first_longer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // First chunk len 3, second len 1.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        ZLELL.store(6, SeqCst);
        // Swap [0..3]="abc" with [3..4]="d" → "dabcef".
        transpose_swap(0, 3, 4);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "dabcef");
    }

    #[test]
    fn transpose_swap_mid_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Swap not at the start.
        zle_reset();
        *ZLELINE.lock().unwrap() = "0123456789".chars().collect();
        ZLELL.store(10, SeqCst);
        // Swap [3..5]="34" with [5..7]="56" → "0125634789".
        transpose_swap(3, 5, 7);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "0125634789");
    }

    // ---------- Batch tests for fixunmeta/selfinsert/deletechar/etc ----------

    #[test]
    fn fixunmeta_strips_meta_and_normalizes_cr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        LASTCHAR.store((0x80 | b'a' as i32) as i32, SeqCst);
        fixunmeta();
        assert_eq!(LASTCHAR.load(SeqCst), b'a' as i32);
        LASTCHAR.store((b'\r' as i32) as i32, SeqCst);
        fixunmeta();
        assert_eq!(LASTCHAR.load(SeqCst), b'\n' as i32);
    }

    #[test]
    fn selfinsert_inserts_lastchar() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(1, SeqCst);
        LASTCHAR.store((b'X' as i32) as i32, SeqCst);
        LASTCHAR_WIDE_VALID.store(0, SeqCst);
        selfinsert(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aXbc");
    }

    #[test]
    fn selfinsertunmeta_chains_fixunmeta_and_selfinsert() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLELL.store(2, SeqCst);
        ZLECS.store(1, SeqCst);
        LASTCHAR.store((0x80 | b'X' as i32) as i32, SeqCst);
        LASTCHAR_WIDE_VALID.store(0, SeqCst);
        selfinsertunmeta(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aXb");
    }

    #[test]
    fn deletechar_removes_n_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, SeqCst);
        ZLECS.store(0, SeqCst);
        ZMOD.lock().unwrap().mult = 2;
        let r = deletechar();
        assert_eq!(r, 0);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "llo");
    }

    #[test]
    fn deletechar_returns_one_at_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLELL.store(2, SeqCst);
        ZLECS.store(2, SeqCst);
        ZMOD.lock().unwrap().mult = 1;
        assert_eq!(deletechar(), 1);
    }

    #[test]
    fn backwarddeletechar_clamps_to_zlecs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(2, SeqCst);
        ZMOD.lock().unwrap().mult = 99;
        backwarddeletechar();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "c");
        assert_eq!(ZLECS.load(SeqCst), 0);
    }

    #[test]
    fn killline_kills_to_eol_and_pushes_killring() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, SeqCst);
        ZLECS.store(6, SeqCst);
        ZMOD.lock().unwrap().mult = 1;
        killline();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "hello ");
        assert_eq!(ZLECS.load(SeqCst), 6);
        // c:437 backkill → cut → cuttext: the kill lands in CUTBUF.
        assert_eq!(
            crate::ported::zle::zle_main::CUTBUF.lock().unwrap().buf,
            "world"
        );
    }

    #[test]
    fn killbuffer_clears_and_pushes() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(2, SeqCst);
        killbuffer();
        assert!(ZLELINE.lock().unwrap().is_empty());
        assert_eq!(ZLELL.load(SeqCst), 0);
        assert_eq!(ZLECS.load(SeqCst), 0);
        // c:1084 forekill → cut → cuttext: the killed text lands in
        // CUTBUF (the kill ring only receives it on the next kill-
        // boundary rotation).
        assert_eq!(
            crate::ported::zle::zle_main::CUTBUF.lock().unwrap().buf,
            "abc"
        );
    }

    #[test]
    fn killwholeline_drops_one_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        ZLELL.store(11, SeqCst);
        ZLECS.store(5, SeqCst); // 'e' in 'def'
        ZMOD.lock().unwrap().mult = 1;
        killwholeline();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "abc\nghi");
    }

    #[test]
    fn copyregionaskill_copies_between_point_mark() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, SeqCst);
        ZLECS.store(0, SeqCst);
        MARK.store(3, SeqCst);
        copyregionaskill(&[]);
        // c:514 — the copy routes through cut → cuttext into CUTBUF;
        // the kill ring only receives it on a later rotation.
        assert_eq!(
            crate::ported::zle::zle_main::CUTBUF.lock().unwrap().buf,
            "hel"
        );
        // Buffer unchanged
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "hello");
    }

    #[test]
    fn regionlines_returns_bol_eol_around_region() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        ZLELL.store(11, SeqCst);
        ZLECS.store(1, SeqCst);
        MARK.store(5, SeqCst);
        let (start, end) = regionlines();
        // mark > zlecs branch: start=findbol()=0, end=findeol()=7
        assert_eq!(start, 0);
        assert_eq!(end, 7);
    }

    #[test]
    fn killregion_drains_between_mark_and_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        ZLELL.store(6, SeqCst);
        ZLECS.store(1, SeqCst);
        MARK.store(4, SeqCst);
        killregion();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aef");
        // c:483 forekill → cut → cuttext: the kill lands in CUTBUF.
        assert_eq!(
            crate::ported::zle::zle_main::CUTBUF.lock().unwrap().buf,
            "bcd"
        );
    }

    #[test]
    fn quoteline_wraps_in_single_quotes() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        quoteline();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "'abc'");
    }

    #[test]
    fn quoteline_escapes_internal_quote() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "it's".chars().collect();
        ZLELL.store(4, SeqCst);
        quoteline();
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "'it'\\''s'");
    }

    #[test]
    fn makequote_handles_no_quotes() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let s: Vec<char> = "abc".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'abc'");
    }

    #[test]
    fn makequote_escapes_quotes() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let s: Vec<char> = "a'b".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'a'\\''b'");
    }

    #[test]
    fn pastebuf_inserts_at_cursor_position_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "foo".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(1, SeqCst);
        let buf = crate::ported::zle::zle_h::cutbuffer {
            buf: "XX".to_string(),
            len: 2,
            flags: 0,
        };
        pastebuf(&buf, 1, 0);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "fXXoo");
    }

    #[test]
    fn pastebuf_inserts_after_cursor_position_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "foo".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(1, SeqCst);
        let buf = crate::ported::zle::zle_h::cutbuffer {
            buf: "XX".to_string(),
            len: 2,
            flags: 0,
        };
        pastebuf(&buf, 1, 1);
        // position=1 → INCCS first → insert at zlecs+1
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "foXXo");
    }

    #[test]
    fn yankpop_returns_one_when_lastcmd_not_yank() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Default lastcmd = empty (no YANK flag).
        assert_eq!(yankpop(), 1);
    }

    #[test]
    fn zle_usable_when_active_and_no_compfunc() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zleactive.store(1, SeqCst);
        INCOMPFUNC.store(0, SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 1);
        // With incompfunc set → 0
        INCOMPFUNC.store(1, SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
        // Reset
        INCOMPFUNC.store(0, SeqCst);
        zleactive.store(0, SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
    }

    /// `Src/Zle/zle_misc.c:919-946` — `parsedigit(int inkey)`. Default
    /// base 10 (set by `zle_reset()` to `modifier { base: 10, ... }`)
    /// hits the c:943 path `if (inkey >= '0' && inkey < '0' + zmod.base)`.
    #[test]
    fn parsedigit_decimal_recognises_zero_through_nine() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(ZMOD.lock().unwrap().base, 10);
        for d in 0..=9 {
            assert_eq!(parsedigit(b'0' as i32 + d), d, "'{}' → {}", d, d);
        }
    }

    /// c:934-942 — base > 10 path. Three sub-arms:
    /// `inkey >= 'a' && inkey < 'a' + zmod.base - 10` (c:935),
    /// `inkey >= 'A' && inkey < 'A' + zmod.base - 10` (c:937),
    /// `idigit(inkey)` (c:939). For base 16 the alpha bound is `< 'g'`
    /// so 'g'/'G' fall through to `return -1` at c:941.
    #[test]
    fn parsedigit_hex_recognises_full_alphabet() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'a' as i32), 10);
        assert_eq!(parsedigit(b'f' as i32), 15);
        assert_eq!(parsedigit(b'A' as i32), 10);
        assert_eq!(parsedigit(b'F' as i32), 15);
        assert_eq!(
            parsedigit(b'g' as i32),
            -1,
            "c:941 — past 'a'+base-10 bound"
        );
        assert_eq!(parsedigit(b'G' as i32), -1);
        assert_eq!(parsedigit(b'9' as i32), 9);
        assert_eq!(parsedigit(b'0' as i32), 0);
    }

    /// c:943-944 — `if (inkey >= '0' && inkey < '0' + zmod.base)`. For
    /// base 8 (`zmod.base = 8`) the bound `'0' + 8 == '8'` excludes
    /// '8' and '9', which fall through to `return -1` at c:945.
    #[test]
    fn parsedigit_octal_rejects_eight_and_nine() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMOD.lock().unwrap().base = 8;
        assert_eq!(parsedigit(b'0' as i32), 0);
        assert_eq!(parsedigit(b'7' as i32), 7);
        assert_eq!(parsedigit(b'8' as i32), -1, "c:945 — '8' fails '0'+8 bound");
        assert_eq!(parsedigit(b'9' as i32), -1);
    }

    /// c:945 — final `return -1` for non-digit inputs in the base ≤ 10
    /// path.
    #[test]
    fn parsedigit_rejects_non_digit_inputs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(parsedigit(b' ' as i32), -1);
        assert_eq!(parsedigit(b'.' as i32), -1);
        assert_eq!(parsedigit(b'-' as i32), -1);
        assert_eq!(
            parsedigit(b'a' as i32),
            -1,
            "c:945 — 'a' rejected in base 10"
        );
        assert_eq!(parsedigit(b'\n' as i32), -1);
    }

    /// c:928-930 — non-MULTIBYTE branch does `inkey &= 0x7f` to strip
    /// the Meta bit before digit classification. zshrs ports the
    /// non-MULTIBYTE form (always-mask) since Rust `char` is wide
    /// already. ESC-5 from the tty arrives as 0xb5 and must parse
    /// as 5.
    #[test]
    fn parsedigit_strips_meta_bit_before_classification() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        for d in 0..=9 {
            let meta = (b'0' as i32 + d) | 0x80;
            assert_eq!(
                parsedigit(meta),
                d,
                "Meta-{} (0x{:x}) must mask 0x80 and parse as {}",
                d,
                meta,
                d
            );
        }
    }

    /// `Src/Zle/zle_misc.c:1201-1224` — `makequote(str, *len)` wraps
    /// `str` in single quotes (c:1213 open, c:1222 close) and escapes
    /// every embedded `'` as 4 chars `'\''` (c:1215-1219). Plain input
    /// with no embedded quotes hits only c:1221 `*l++ = *str` for each
    /// char.
    #[test]
    fn makequote_wraps_plain_input_in_single_quotes() {
        let _g = crate::test_util::global_state_lock();
        let out: String = makequote(&"hello".chars().collect::<Vec<_>>())
            .iter()
            .collect();
        assert_eq!(out, "'hello'");
    }

    /// c:1215-1219 — `*str == ZWC('\'')` arm emits the 4-char escape
    /// `'\''`: close-quote, backslash, literal quote, re-open.
    #[test]
    fn makequote_escapes_embedded_single_quote() {
        let _g = crate::test_util::global_state_lock();
        let out: String = makequote(&"it's".chars().collect::<Vec<_>>())
            .iter()
            .collect();
        assert_eq!(out, r#"'it'\''s'"#);
    }

    /// c:1211 — `*len += 2 + qtct*3`. Two embedded quotes (qtct=2)
    /// drive output length to `4 + 2 + 6 = 12` chars.
    #[test]
    fn makequote_handles_consecutive_quotes() {
        let _g = crate::test_util::global_state_lock();
        let out: String = makequote(&"a''b".chars().collect::<Vec<_>>())
            .iter()
            .collect();
        assert_eq!(out, r#"'a'\'''\''b'"#);
        assert_eq!(out.len(), 12);
    }

    /// c:1213 + c:1222 — open and close quote are emitted unconditionally
    /// even for empty input (the `for` loop at c:1214 has zero
    /// iterations). Output is the 2-char string `''`.
    #[test]
    fn makequote_empty_input_returns_pair_of_quotes() {
        let _g = crate::test_util::global_state_lock();
        let out: String = makequote(&[]).iter().collect();
        assert_eq!(out, "''", "c:1213+c:1222 — quotes fire unconditionally");
    }

    /// `Src/Zle/zle_misc.c:255-269` — `transpose_swap(start, middle,
    /// end)` rotates `[start..middle)` with `[middle..end)`:
    /// c:263-264 saves first slice, c:266 `ZS_memmove` shifts the
    /// second into start, c:267 `ZS_memcpy` writes the saved first
    /// after it.
    #[test]
    fn transpose_swap_rotates_two_adjacent_slices() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "abcDEFG".chars().collect();
        }
        ZLELL.store(7, SeqCst);
        transpose_swap(0, 3, 7);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "DEFGabc");
    }

    /// c:255 — equal-length halves. Symmetric case is the easiest
    /// off-by-one trap: a regression confusing len1 (c:260) and len2
    /// (c:261) would silently no-op (same length, same result), so
    /// pair this with the asymmetric test above to catch it.
    #[test]
    fn transpose_swap_equal_length_halves() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "1234".chars().collect();
        }
        ZLELL.store(4, SeqCst);
        transpose_swap(0, 2, 4);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "3412");
    }

    // ─── zsh-corpus pins for transpose_swap edge cases ─────────────

    /// Single-char halves: "ab" with swap(0,1,2) → "ba".
    #[test]
    fn zle_misc_corpus_transpose_swap_single_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "ab".chars().collect();
        }
        ZLELL.store(2, SeqCst);
        transpose_swap(0, 1, 2);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "ba");
    }

    /// First half empty (start == middle): no-op.
    #[test]
    fn zle_misc_corpus_transpose_swap_empty_first_half() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "abcd".chars().collect();
        }
        ZLELL.store(4, SeqCst);
        transpose_swap(0, 0, 4);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "abcd", "empty first half = identity transform");
    }

    /// Second half empty (middle == end): no-op.
    #[test]
    fn zle_misc_corpus_transpose_swap_empty_second_half() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "abcd".chars().collect();
        }
        ZLELL.store(4, SeqCst);
        transpose_swap(0, 4, 4);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "abcd", "empty second half = identity transform");
    }

    /// Partial-buffer transpose: only middle bytes swapped, surrounds preserved.
    #[test]
    fn zle_misc_corpus_transpose_swap_partial_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        {
            let mut z = ZLELINE.lock().unwrap();
            *z = "XYabcdZW".chars().collect();
        }
        ZLELL.store(8, SeqCst);
        transpose_swap(2, 4, 6);
        let got: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(got, "XYcdabZW", "surrounding chars XY,ZW must be preserved");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Zle/zle_misc.c simple widgets.
    // ═══════════════════════════════════════════════════════════════════

    /// c:401 — `acceptline` sets DONE=1 and returns 0.
    #[test]
    fn acceptline_sets_done_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        DONE.store(0, SeqCst);
        let r = acceptline();
        assert_eq!(r, 0);
        assert_eq!(DONE.load(SeqCst), 1, "acceptline must set DONE=1");
    }

    /// c:409 — `acceptandhold` sets DONE=1, returns 0, pushes line
    /// to BUFSTACK, and saves cursor to STACKCS.
    #[test]
    fn acceptandhold_pushes_line_sets_done() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        DONE.store(0, SeqCst);
        BUFSTACK.lock().unwrap().clear();
        *ZLELINE.lock().unwrap() = "test line".chars().collect();
        ZLELL.store(9, SeqCst);
        ZLECS.store(4, SeqCst);
        let r = acceptandhold();
        assert_eq!(r, 0);
        assert_eq!(DONE.load(SeqCst), 1);
        assert_eq!(STACKCS.load(SeqCst), 4, "STACKCS must capture ZLECS");
        let bs = BUFSTACK.lock().unwrap();
        assert_eq!(bs[0], "test line", "BUFSTACK[0] must be the pushed line");
    }

    /// c:843 — `overwritemode` toggles INSMODE XOR 1 and returns 0.
    #[test]
    fn overwritemode_xor_toggles_insmode() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INSMODE.store(1, SeqCst);
        assert_eq!(overwritemode(), 0);
        assert_eq!(INSMODE.load(SeqCst), 0, "1 ^ 1 = 0");
        assert_eq!(overwritemode(), 0);
        assert_eq!(INSMODE.load(SeqCst), 1, "0 ^ 1 = 1");
    }

    /// c:892 — `undefinedkey` returns 1 (signals beep to dispatcher).
    #[test]
    fn undefinedkey_returns_one_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(undefinedkey(), 1, "undefined-key widget must return 1");
    }

    /// c:892 — `undefinedkey` is pure: repeated calls all return 1.
    #[test]
    fn undefinedkey_pure_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(undefinedkey(), 1);
        }
    }

    /// c:851 — `whatcursorposition` returns 0 (success, no error).
    #[test]
    fn whatcursorposition_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, SeqCst);
        ZLECS.store(2, SeqCst);
        assert_eq!(whatcursorposition(), 0);
    }

    /// c:851 — `whatcursorposition` at EOF position is safe (covers
    /// the `zlecs == zlell` → "EOF" branch c:858).
    #[test]
    fn whatcursorposition_at_eof_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(3, SeqCst); // at EOF
        assert_eq!(whatcursorposition(), 0);
    }

    /// c:355 — `backwardkillline` with cursor at 0 is a safe no-op.
    #[test]
    fn backwardkillline_at_bol_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "test".chars().collect();
        ZLELL.store(4, SeqCst);
        ZLECS.store(0, SeqCst);
        // No panic = pass; behavior depends on cuttext path.
        let _ = backwardkillline();
    }

    /// c:345 — `killbuffer` always returns 0 and clears buffer.
    #[test]
    fn killbuffer_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "to be killed".chars().collect();
        ZLELL.store(12, SeqCst);
        ZLECS.store(0, SeqCst);
        assert_eq!(killbuffer(), 0);
        // After killbuffer, ZLELL should be 0 (buffer cleared).
        assert_eq!(ZLELL.load(SeqCst), 0, "killbuffer clears entire buffer");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_misc.c
    // c:177 selfinsert / c:234 deletechar / c:271 backwarddeletechar /
    // c:297 killwholeline / c:355 backwardkillline / c:439 gosmacstransposechars /
    // c:478 transposechars / c:533 poundinsert / c:615 killline / c:662 regionlines /
    // c:693 killregion / c:826 viputbefore / c:857 viputafter
    // ═══════════════════════════════════════════════════════════════════

    /// c:177 — `selfinsert(empty)` return in u8 exit-code range.
    #[test]
    fn selfinsert_empty_args_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = selfinsert(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:234 — `deletechar` return in u8 exit-code range.
    #[test]
    fn deletechar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = deletechar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:271 — `backwarddeletechar` return in u8 exit-code range.
    #[test]
    fn backwarddeletechar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = backwarddeletechar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:297 — `killwholeline` return in u8 exit-code range.
    #[test]
    fn killwholeline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = killwholeline();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:439 — `gosmacstransposechars` return in u8 exit-code range.
    #[test]
    fn gosmacstransposechars_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = gosmacstransposechars();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:478 — `transposechars` return in u8 exit-code range.
    #[test]
    fn transposechars_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = transposechars();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:533 — `poundinsert` return in u8 exit-code range.
    #[test]
    fn poundinsert_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = poundinsert();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:615 — `killline` return in u8 exit-code range.
    #[test]
    fn killline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = killline();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:662 — `regionlines()` returns (usize, usize) with lo ≤ hi.
    #[test]
    fn regionlines_returns_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let (lo, hi) = regionlines();
        assert!(lo <= hi, "regionlines lo={} must ≤ hi={}", lo, hi);
    }

    /// c:693 — `killregion` return in u8 exit-code range.
    #[test]
    fn killregion_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = killregion();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:826 + c:857 — `viputbefore` + `viputafter` return in u8 range.
    #[test]
    fn viputbefore_viputafter_return_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for r in [viputbefore(), viputafter()] {
            assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
        }
    }

    /// c:604 — `acceptandhold` return in u8 exit-code range.
    #[test]
    fn acceptandhold_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = acceptandhold();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_misc.c
    // c:177 selfinsert / c:225 selfinsertunmeta / c:234 deletechar /
    // c:271 backwarddeletechar / c:355 backwardkillline /
    // c:478 transposechars / c:754 yank / c:781 pastebuf /
    // c:890 putreplaceselection / c:930 yankpop
    // ═══════════════════════════════════════════════════════════════════

    /// c:177 — `selfinsert(&[])` returns i32 (compile-time type pin).
    #[test]
    fn selfinsert_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selfinsert(&[]);
    }

    /// c:225 — `selfinsertunmeta(&[])` returns i32.
    #[test]
    fn selfinsertunmeta_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selfinsertunmeta(&[]);
    }

    /// c:234 — `deletechar` returns i32.
    #[test]
    fn deletechar_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = deletechar();
    }

    /// c:271 — `backwarddeletechar` returns i32.
    #[test]
    fn backwarddeletechar_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = backwarddeletechar();
    }

    /// c:533 — `yank` returns int (signature pin).
    #[test]
    fn yank_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = yank();
    }

    /// c:930 — `yankpop` returns i32.
    #[test]
    fn yankpop_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = yankpop();
    }

    /// c:890 — `putreplaceselection` returns i32.
    #[test]
    fn putreplaceselection_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = putreplaceselection();
    }

    /// c:355 — `backwardkillline` returns i32.
    #[test]
    fn backwardkillline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = backwardkillline();
    }

    /// c:478 — `transposechars` is safe on empty buffer (3-iter).
    #[test]
    fn transposechars_safe_empty_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..3 {
            let _ = transposechars();
        }
    }

    /// c:234 + c:271 — `deletechar`/`backwarddeletechar` safe on empty.
    #[test]
    fn delete_chars_safe_on_empty_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _ = deletechar();
        let _ = backwarddeletechar();
    }

    /// c:781 — `pastebuf(empty, _, _)` empty input safe.
    #[test]
    fn pastebuf_empty_buf_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let empty = crate::ported::zle::zle_h::cutbuffer::default();
        let _ = pastebuf(&empty, 1, 0);
        let _ = pastebuf(&empty, 1, 1);
    }

    /// c:781 — `pastebuf` returns i32 (compile-time type pin).
    #[test]
    fn pastebuf_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let empty = crate::ported::zle::zle_h::cutbuffer::default();
        let _: i32 = pastebuf(&empty, 1, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_misc.c
    // c:76 doinsert / c:177 selfinsert / c:225 selfinsertunmeta /
    // c:297 killwholeline / c:345 killbuffer / c:439 gosmacstransposechars /
    // c:592 acceptline / c:604 acceptandhold / c:615 killline /
    // c:662 regionlines / c:693 killregion / c:722 copyregionaskill /
    // c:533 poundinsert
    // ═══════════════════════════════════════════════════════════════════

    /// c:76 — `doinsert(empty)` is safe.
    #[test]
    fn doinsert_empty_chars_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        doinsert(&[]);
    }

    /// c:177 — `selfinsert` returns i32 (compile-time pin, alt).
    #[test]
    fn selfinsert_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selfinsert(&[]);
    }

    /// c:225 — `selfinsertunmeta` returns i32 (compile-time pin, alt).
    #[test]
    fn selfinsertunmeta_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = selfinsertunmeta(&[]);
    }

    /// c:297 — `killwholeline` returns i32.
    #[test]
    fn killwholeline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = killwholeline();
    }

    /// c:345 — `killbuffer` returns i32.
    #[test]
    fn killbuffer_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = killbuffer();
    }

    /// c:439 — `gosmacstransposechars` returns i32.
    #[test]
    fn gosmacstransposechars_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = gosmacstransposechars();
    }

    /// c:592 — `acceptline` returns i32.
    #[test]
    fn acceptline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = acceptline();
    }

    /// c:604 — `acceptandhold` returns i32.
    #[test]
    fn acceptandhold_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = acceptandhold();
    }

    /// c:615 — `killline` returns i32.
    #[test]
    fn killline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = killline();
    }

    /// c:662 — `regionlines` returns (usize, usize) tuple.
    #[test]
    fn regionlines_returns_usize_pair_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: (usize, usize) = regionlines();
    }

    /// c:693 — `killregion` returns i32.
    #[test]
    fn killregion_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = killregion();
    }

    /// c:722 — `copyregionaskill(empty)` returns i32.
    #[test]
    fn copyregionaskill_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = copyregionaskill(&[]);
    }

    /// c:533 — `poundinsert` returns i32 (alt name pin).
    #[test]
    fn poundinsert_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = poundinsert();
    }
}
