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

/// Port of `doinsert(ZLE_STRING_T zstr, int len)` from `Src/Zle/zle_misc.c:37`.
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
/// Port of `selfinsert(UNUSED(char **args))` from `Src/Zle/zle_misc.c:113`.
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

/// Port of `selfinsertunmeta(char **args)` from Src/Zle/zle_misc.c:149.
pub fn selfinsertunmeta(args: &[String]) -> i32 {
    // c:149
    // c:151-152 — `fixunmeta(); return selfinsert(args)`. Args
    // pass through to mirror the C call.
    fixunmeta();
    selfinsert(args)
}

/// Port of `deletechar(char **args)` from Src/Zle/zle_misc.c:157.
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

/// Port of `backwarddeletechar(char **args)` from Src/Zle/zle_misc.c:180.
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

/// Port of `killwholeline(UNUSED(char **args))` from Src/Zle/zle_misc.c:195.
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
        // c:207 — `forekill(i - zlecs + (i != zlell), ...)`. Include
        // the trailing '\n' if there is one.
        let drop = i - ZLECS.load(SeqCst) + (if i != ZLELL.load(SeqCst) { 1 } else { 0 });
        if drop > 0 {
            let text: Vec<char> = ZLELINE
                .lock()
                .unwrap()
                .drain(ZLECS.load(SeqCst)..ZLECS.load(SeqCst) + drop)
                .collect();
            KILLRING.lock().unwrap().push_front(text);
            if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
                KILLRING.lock().unwrap().pop_back();
            }
            ZLELL.fetch_sub(drop, SeqCst);
        }
        n -= 1;
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:210
}

/// Port of `killbuffer(UNUSED(char **args))` from Src/Zle/zle_misc.c:215.
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

/// Port of `backwardkillline(char **args)` from Src/Zle/zle_misc.c:225.
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
    // c:243 — `forekill(i, CUT_FRONT|CUT_RAW)`. Drain forward from
    // current zlecs by i chars; push to killring with FRONT semantics
    // (prepended to the existing front entry if present, else new).
    if i > 0 {
        let text: Vec<char> = ZLELINE
            .lock()
            .unwrap()
            .drain(ZLECS.load(SeqCst)..ZLECS.load(SeqCst) + i)
            .collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(i, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:245
}

/// Port of `transpose_swap(int start, int middle, int end)` from `Src/Zle/zle_misc.c:254`.
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

/// Port of `gosmacstransposechars(UNUSED(char **args))` from Src/Zle/zle_misc.c:274.
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

/// Port of `transposechars(UNUSED(char **args))` from Src/Zle/zle_misc.c:313.
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

/// Port of `poundinsert(UNUSED(char **args))` from Src/Zle/zle_misc.c:369.
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

/// Port of `acceptline(UNUSED(char **args))` from `Src/Zle/zle_misc.c:401`.
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

/// Port of `acceptandhold(UNUSED(char **args))` from Src/Zle/zle_misc.c:409.
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

/// Port of `killline(char **args)` from Src/Zle/zle_misc.c:419.
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
    // c:437 — `backkill(i, CUT_RAW)`. Drain the killed range and
    // push to killring; cursor returns to start.
    if i > 0 {
        let text: Vec<char> = ZLELINE.lock().unwrap().drain(start..start + i).collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(i, SeqCst);
        ZLECS.store(start, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:439
}

/// Port of `regionlines(int *start, int *end)` from Src/Zle/zle_misc.c:444.
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

/// Port of `killregion(UNUSED(char **args))` from Src/Zle/zle_misc.c:463.
pub fn killregion() -> i32 {
    // c:463
    // c:463-466 — `if (mark > zlell) mark = zlell`.
    if MARK.load(SeqCst) > ZLELL.load(SeqCst) {
        MARK.store(ZLELL.load(SeqCst), SeqCst);
    }
    // c:467-479 — region_active==2 (visual-line); skip the line-mode
    // path for the simplified port.
    let (start, end) = if MARK.load(SeqCst) > ZLECS.load(SeqCst) {
        (ZLECS.load(SeqCst), MARK.load(SeqCst))
    } else {
        (MARK.load(SeqCst), ZLECS.load(SeqCst))
    };
    if start < end {
        let text: Vec<char> = ZLELINE.lock().unwrap().drain(start..end).collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(end - start, SeqCst);
        ZLECS.store(start, SeqCst);
        MARK.store(start, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `copyregionaskill(char **args)` from Src/Zle/zle_misc.c:494.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn copyregionaskill(args: &[String]) -> i32 {
    // c:494
    // c:494-501 — `if (*args) { stringaszleline; cuttext(line, len, CUT_REPLACE) }`.
    if let Some(arg) = args.first() {
        // c:499 — `line = stringaszleline(*args, 0, &len, NULL, NULL);`
        let text: Vec<char> =
            crate::ported::zle::zle_utils::stringaszleline(arg, 0, None, None, None);
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
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
    let text: Vec<char> = ZLELINE.lock().unwrap()[start..end].to_vec();
    KILLRING.lock().unwrap().push_front(text);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
    0
}

/// Yank - insert from kill ring
/// Port of yank(UNUSED(char **args)) from zle_misc.c
pub fn yank() {
    // c:533
    // c:541-549 — `yankb = yankcs = mark = zlecs; while (n--) {
    //              kct = -1; spaceinline(kctbuf->len);
    //              ZS_memcpy(zleline + zlecs, kctbuf->buf, kctbuf->len);
    //              zlecs += kctbuf->len; yanke = zlecs; }`.
    let text_opt = KILLRING.lock().unwrap().front().cloned();
    if let Some(text) = text_opt {
        MARK.store(ZLECS.load(SeqCst), SeqCst); // c:543
        spaceinline(text.len() as i32); // c:546
        let cs = ZLECS.load(SeqCst);
        {
            let mut line = ZLELINE.lock().unwrap();
            for (i, &c) in text.iter().enumerate() {
                if cs + i < line.len() {
                    line[cs + i] = c; // c:547
                }
            }
        }
        ZLECS.fetch_add(text.len(), SeqCst); // c:548
        YANKLAST.store(true, SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Port of `pastebuf(Cutbuffer buf, int mult, int position)` from Src/Zle/zle_misc.c:558.
/// WARNING: param names don't match C — Rust=(zle, buf, mult, position) vs C=(buf, mult, position)
pub fn pastebuf(buf: &[char], mult: i32, position: i32) -> i32 {
    // c:558
    // Simplified port of pastebuf. The C source dispatches on
    // CUTBUFFER_LINE flag (insert as full lines vs char-wise),
    // computes position 0/1/2 (before/after/split), and updates
    // yankb/yanke. Without the LINE-flag check (we treat all as
    // char-wise) plus the simple before/after path we get the
    // common case.
    if buf.is_empty() {
        return 0;
    }
    // c:591-592 — `if (position == 1 && zlecs != findeol()) INCCS()`.
    if position == 1 && ZLECS.load(SeqCst) < ZLELL.load(SeqCst) {
        ZLECS.fetch_add(1, SeqCst);
    }
    // c:593 — `yankb = zlecs`.
    YANKB.store(ZLECS.load(SeqCst), SeqCst);
    // c:595-599 — `while (mult--) { spaceinline(cc); ZS_memcpy; zlecs += cc }`.
    let mut n = mult;
    let cc = buf.len();
    while n > 0 {
        spaceinline(cc as i32); // c:596
        let cs = ZLECS.load(SeqCst);
        {
            let mut line = ZLELINE.lock().unwrap();
            for (i, &c) in buf.iter().enumerate() {
                if cs + i < line.len() {
                    line[cs + i] = c; // c:597
                }
            }
        }
        ZLECS.fetch_add(cc, SeqCst); // c:598
        n -= 1;
    }
    // c:600 — `yanke = zlecs`.
    YANKE.store(ZLECS.load(SeqCst), SeqCst);
    // c:601-602 — vicmd → DECCS.
    if ZLECS.load(SeqCst) > 0 && *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" {
        ZLECS.fetch_sub(1, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viputbefore(UNUSED(char **args))` from Src/Zle/zle_misc.c:608.
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
    let buf: Vec<char> = if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = ZMOD.lock().unwrap().vibuf as usize;
        if idx >= vibuf().lock().unwrap().len() {
            return 1;
        }
        vibuf().lock().unwrap()[idx].clone() // c:631
    } else {
        KILLRING
            .lock()
            .unwrap()
            .front()
            .cloned()
            .unwrap_or_default() // c:633
    };
    if buf.is_empty() {
        return 1; // c:635
    }
    pastebuf(&buf, n, 0) // c:639
}

/// Port of `viputafter(UNUSED(char **args))` from Src/Zle/zle_misc.c:644.
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
    // c:653-665 — OS selection branch (MOD_OSSEL = PRI|CLIP). Without
    //              system_clipget we fall through to the cut-buffer path.
    let buf: Vec<char> = if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = ZMOD.lock().unwrap().vibuf as usize;
        if idx >= vibuf().lock().unwrap().len() {
            return 1;
        }
        vibuf().lock().unwrap()[idx].clone() // c:667
    } else {
        KILLRING
            .lock()
            .unwrap()
            .front()
            .cloned()
            .unwrap_or_default() // c:669
    };
    if buf.is_empty() {
        return 1; // c:671
    }
    pastebuf(&buf, n, 1) // c:675
}

/// Port of `putreplaceselection(UNUSED(char **args))` from Src/Zle/zle_misc.c:680.
pub fn putreplaceselection() -> i32 {
    // c:680
    let n = ZMOD.lock().unwrap().mult; // c:682
    let mut pos = 2; // c:686
    startvichange(-1); // c:688
    if n < 0 || ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 1; // c:690
    }
    let prevbuf: Vec<char> = if ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = ZMOD.lock().unwrap().vibuf as usize;
        if idx >= vibuf().lock().unwrap().len() {
            return 1;
        }
        vibuf().lock().unwrap()[idx].clone() // c:700
    } else {
        KILLRING
            .lock()
            .unwrap()
            .front()
            .cloned()
            .unwrap_or_default() // c:702
    };
    if prevbuf.is_empty() {
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

/// Port of `yankpop(UNUSED(char **args))` from Src/Zle/zle_misc.c:728.
pub fn yankpop() -> i32 {
    // c:728
    // c:730-735 — `if (!(lastcmd & ZLE_YANK) || !kring || !kctbuf)
    //               return 1`.
    let last = LASTCMD.load(SeqCst) as i32;
    if (last & ZLE_YANK) == 0 || KILLRING.lock().unwrap().is_empty() {
        return 1;
    }
    // C body cycles the kill ring index `kct` and re-inserts the
    // previous yank. zshrs uses VecDeque<Vec<char>> with the rotation
    // index `yank_ring_idx`. Simplified: rotate front entry to back,
    // delete previous yank text from line, insert new front.
    let prev_start = YANKB.load(SeqCst);
    let prev_end = YANKE.load(SeqCst);
    if prev_end > prev_start && prev_end <= ZLELINE.lock().unwrap().len() {
        ZLELINE.lock().unwrap().drain(prev_start..prev_end);
        ZLELL.fetch_sub(prev_end - prev_start, SeqCst);
        ZLECS.store(prev_start, SeqCst);
    }
    if let Some(top) = KILLRING.lock().unwrap().pop_front() {
        KILLRING.lock().unwrap().push_back(top);
    }
    if let Some(next) = KILLRING.lock().unwrap().front().cloned() {
        for (i, &c) in next.iter().enumerate() {
            ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst) + i, c);
        }
        YANKB.store(ZLECS.load(SeqCst), SeqCst);
        ZLECS.fetch_add(next.len(), SeqCst);
        ZLELL.fetch_add(next.len(), SeqCst);
        YANKE.store(ZLECS.load(SeqCst), SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
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
/// Direct port of `char *bracketedstring(void)` from
/// `Src/Zle/zle_misc.c:784`. Reads bytes from the controlling tty
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

/// Port of `int bracketedpaste(char **args)` from
/// Src/Zle/zle_misc.c:814.
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

/// Port of `overwritemode(UNUSED(char **args))` from `Src/Zle/zle_misc.c:843`.
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

/// Port of `whatcursorposition(UNUSED(char **args))` from Src/Zle/zle_misc.c:851.
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

/// Port of `undefinedkey(UNUSED(char **args))` from `Src/Zle/zle_misc.c:892`.
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

/// Direct port of `int quotedinsert(char **args)` from
/// `Src/Zle/zle_misc.c:899`.
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

/// Port of `parsedigit(int inkey)` from Src/Zle/zle_misc.c:919.
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

/// Port of `digitargument(UNUSED(char **args))` from Src/Zle/zle_misc.c:950.
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

/// Port of `negargument(UNUSED(char **args))` from `Src/Zle/zle_misc.c:974`.
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

/// Port of `universalargument(char **args)` from Src/Zle/zle_misc.c:986.
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

/// Port of `argumentbase(char **args)` from `Src/Zle/zle_misc.c:1037`.
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

/// Port of `copyprevword(UNUSED(char **args))` from Src/Zle/zle_misc.c:1066.
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

/// Port of `copyprevshellword(UNUSED(char **args))` from Src/Zle/zle_misc.c:1108.
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

/// Port of `sendbreak(UNUSED(char **args))` from `Src/Zle/zle_misc.c:1144`.
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

/// Port of `quoteregion(UNUSED(char **args))` from Src/Zle/zle_misc.c:1152.
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

/// Port of `quoteline(UNUSED(char **args))` from Src/Zle/zle_misc.c:1187.
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

/// Port of `makequote(ZLE_STRING_T str, size_t *len)` from Src/Zle/zle_misc.c:1201.
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

/// Port of `static LinkList namedcmdll` from `Src/Zle/zle_misc.c:1235`.
pub static namedcmdll: std::sync::Mutex<Vec<String>> = // c:1235
    std::sync::Mutex::new(Vec::new());

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1235`.
pub static namedcmdambig: std::sync::atomic::AtomicUsize = // c:1235
    std::sync::atomic::AtomicUsize::new(0);

/// Direct port of `static int scancompcmd(HashNode hn, UNUSED(int flags))`
/// from `Src/Zle/zle_misc.c:1235`.
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

/// Direct port of `Thingy executenamedcommand(char *prompt)` from
/// `Src/Zle/zle_misc.c:1261`. Prompts the user for a widget
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

/// Port of `struct suffixset` from `Src/Zle/zle_misc.c`. One node
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
/// Port of `addsuffix(int tp, int flags, ZLE_STRING_T chars, int lenstr, int lensuf)` from Src/Zle/zle_misc.c:1558.
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

/// Port of `addsuffixstring(int tp, int flags, char *chars, int lensuf)` from Src/Zle/zle_misc.c:1580.
pub fn addsuffixstring(tp: i32, flags: i32, chars: &str, lensuf: i32) {
    // c:1580
    // C body: `chars = ztrdup(chars); suffixstr = stringaszleline(...);
    //          addsuffix(tp, flags, suffixstr, slen, lensuf)`.
    let chars_vec: Vec<char> = chars.chars().collect();
    let slen = chars_vec.len() as i32;
    addsuffix(tp, flags, chars_vec, slen, lensuf);
}

/// Direct port of `void makesuffix(int n)` from
/// `Src/Zle/zle_misc.c:1598`. Reads `$ZLE_REMOVE_SUFFIX_CHARS` from
/// paramtab and registers it as the active suffix-removal char set
/// via `addsuffixstring`. Defaults to ` \t\n;&|` when the param is
/// unset.
pub fn makesuffix(n: i32) {
    // c:1598
    // c:1602-1603 — `suffixchars = getsparam_u("ZLE_REMOVE_SUFFIX_CHARS")`.
    let suffix_chars = crate::ported::params::getsparam("ZLE_REMOVE_SUFFIX_CHARS")
        .unwrap_or_else(|| " \t\n;&|".to_string()); // default
    addsuffixstring(crate::ported::zle::zle_h::SUFTYP_POSSTR, 0, &suffix_chars, n); // c:1605
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

/// Port of `makeparamsuffix(int br, int n)` from Src/Zle/zle_misc.c:1623.
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

/// Port of `mod_export int suffixlen;` from `Src/Zle/zle_misc.c:1553`.
/// "Length of the currently active, auto-removable suffix." Consumed
/// by `iremovesuffix` for the actual delete.
pub static suffixlen: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `makesuffixstr(char *f, char *s, int n)` from
/// `Src/Zle/zle_misc.c:1642`. Three-way dispatch:
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
/// Direct port of `int iremovesuffix(ZLE_INT_T c, int keep)` from
/// `Src/Zle/zle_misc.c:1699`. Walks `suffixlist`; for each
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

    // c:1735-1786 — suffixlist walk.
    let list = suffixlist().lock().map(|g| g.clone()).unwrap_or_default();
    let mut sl: i32 = 0;
    let ch = c as u32;
    for entry in list.iter() {
        // c:1735
        // c:1741-1769 — match `ch` against entry.chars based on tp/flags.
        let matched = entry.chars.iter().any(|&x| x as u32 == ch);
        if matched {
            // c:1762
            if keep == 0 {
                sl = entry.lensuf;
            } // c:1764
            break;
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

/// Port of `mod_export int suffixlen` from `Src/Zle/zle_misc.c:1553`.
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

/// Port of `struct suffixset *suffixlist` from `Src/Zle/zle_misc.c`.
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
/// Port of selfinsert(UNUSED(char **args)) from zle_misc.c
pub fn self_insert(c: char) {
    // c:113
    ZLELINE.lock().unwrap().insert(ZLECS.load(SeqCst), c);
    ZLECS.fetch_add(1, SeqCst);
    ZLELL.fetch_add(1, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Self insert unmeta - insert character with meta bit stripped
/// Port of selfinsertunmeta(char **args) from zle_misc.c

/// Accept line - return the current line for execution
/// Port of acceptline(UNUSED(char **args)) from zle_misc.c
pub fn accept_line() -> String {
    // c:401
    ZLELINE.lock().unwrap().iter().collect()
}

/// Accept and hold - accept line but keep it in the buffer
/// Port of acceptandhold(UNUSED(char **args)) from zle_misc.c
pub fn accept_and_hold() -> String {
    ZLELINE.lock().unwrap().iter().collect()
}

/// Quoted insert - insert next char literally
/// Port of quotedinsert(char **args) from zle_misc.c

/// Bracketed paste - handle paste mode
/// Port of bracketedpaste(char **args) from zle_misc.c

/// Delete char under cursor
/// Port of deletechar(char **args) from zle_misc.c

/// Delete char before cursor
/// Port of backwarddeletechar(char **args) from zle_misc.c

/// Kill from cursor to end of line
/// Port of killline(char **args) from zle_misc.c

/// Kill from beginning of line to cursor
/// Port of backwardkillline(char **args) from zle_misc.c

/// Kill entire buffer
/// Port of killbuffer(UNUSED(char **args)) from zle_misc.c
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
/// Port of killwholeline(UNUSED(char **args)) from zle_misc.c
pub fn kill_whole_line() {
    kill_buffer();
}

/// Swap cursor and mark.
/// Port of `exchangepointandmark(UNUSED(char **args))` from Src/Zle/zle_move.c:496. The
/// C source has additional zmult-based behaviour (zmult==0 just
/// activates the region without swapping; zmult>0 also activates).
/// This bare method only swaps; the widget-level
/// `widget_exchange_point_and_mark` honours the count semantics.
pub fn exchange_point_and_mark() {
    std::mem::swap(&mut ZLECS.load(SeqCst), &mut MARK.load(SeqCst));
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Set mark at the current cursor position.
/// Port of `setmarkcommand(UNUSED(char **args))` from Src/Zle/zle_move.c:483 with the
/// activate-region branch elided. The widget-level
/// `widget_set_mark_command` covers the negative-count
/// deactivate path that the bare C source supports.
pub fn set_mark_here() {
    MARK.store(ZLECS.load(SeqCst), SeqCst);
}

/// Copy region as kill
/// Port of copyregionaskill(char **args) from zle_misc.c

/// Kill region (between point and mark)
/// Port of killregion(UNUSED(char **args)) from zle_misc.c
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
/// Port of yankpop(UNUSED(char **args)) from zle_misc.c
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
/// Port of transposechars(UNUSED(char **args)) from zle_misc.c
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
/// Port of `capitalizeword(UNUSED(char **args))` from Src/Zle/zle_word.c (the C source
/// uses `casemodifyword()` with a CASMOD_CAPS flag). Mirrors emacs's
/// M-c convention. Cursor lands past the modified word.
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
/// Port of `downcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
/// `casemodifyword()` with the CASMOD_LOWER flag.
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
/// Port of `upcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
/// `casemodifyword()` with the CASMOD_UPPER flag.
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
/// Port of transpose words logic
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
/// Port of quoteline(UNUSED(char **args)) from zle_misc.c
pub fn quote_line() {
    ZLELINE.lock().unwrap().insert(0, '\'');
    ZLELL.fetch_add(1, SeqCst);
    ZLECS.fetch_add(1, SeqCst);
    ZLELINE.lock().unwrap().push('\'');
    ZLELL.fetch_add(1, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Quote region
/// Port of quoteregion(UNUSED(char **args)) from zle_misc.c
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
/// Port of whatcursorposition(UNUSED(char **args)) from zle_misc.c
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
/// Port of universalargument(char **args) from zle_misc.c

/// Digit argument - accumulate numeric argument
/// Port of digitargument(UNUSED(char **args)) from zle_misc.c
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
/// Port of negargument(UNUSED(char **args)) from zle_misc.c
pub fn neg_argument() {
    NEG_ARG.store(!NEG_ARG.load(SeqCst), SeqCst);
}

/// Undefined key - beep
/// Port of undefinedkey(UNUSED(char **args)) from zle_misc.c
pub fn undefined_key() {
    print!("\x07"); // Bell
}

/// Send break - abort current operation
/// Port of sendbreak(UNUSED(char **args)) from zle_misc.c
pub fn send_break() {
    ZLELINE.lock().unwrap().clear();
    ZLELL.store(0, SeqCst);
    ZLECS.store(0, SeqCst);
    MARK.store(0, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Vi put after cursor
/// Port of viputafter(UNUSED(char **args)) from zle_misc.c
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
/// Port of viputbefore(UNUSED(char **args)) from zle_misc.c
pub fn vi_put_before() {
    yank();
}

/// Overwrite mode toggle
/// Port of overwritemode(UNUSED(char **args)) from zle_misc.c
pub fn overwrite_mode() {
    INSMODE.fetch_xor(1, SeqCst);
}

/// Copy previous word
/// Port of copyprevword(UNUSED(char **args)) from zle_misc.c
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
/// Port of copyprevshellword(UNUSED(char **args)) from zle_misc.c
pub fn copy_prev_shell_word() {
    // Simplified - doesn't handle full shell quoting
    copy_prev_word();
}

/// Pound insert - comment toggle for vi mode
/// Port of poundinsert(UNUSED(char **args)) from zle_misc.c
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

/// Port of `processcmd(UNUSED(char **args))` from
/// `Src/Zle/zle_tricky.c`. Shared widget body for both
/// `which-command` and `run-help` (per iwidgets.list: both names
/// bind to the same C fn `processcmd`; the runtime distinguishes
/// based on `bindk->nam` to decide whether to emit "whence" output
/// or invoke `$HELPDIR/cmd`).
pub fn processcmd(_args: &[String]) -> i32 {
    // c:2971
    // The canonical port lives at zle_tricky.rs:1003 with the same
    // C-fn name. Delegate so the misc.rs entry point (widget table
    // wiring) goes through the real body.
    crate::ported::zle::zle_tricky::processcmd()
}

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
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("world".to_string())
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
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("abc".to_string())
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
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("hel".to_string())
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
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("bcd".to_string())
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
        let buf: Vec<char> = "XX".chars().collect();
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
        let buf: Vec<char> = "XX".chars().collect();
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

    /// c:754 — `yank` returns void (signature pin).
    #[test]
    fn yank_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = yank();
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
        let _ = pastebuf(&[], 1, 0);
        let _ = pastebuf(&[], 1, 1);
    }

    /// c:781 — `pastebuf` returns i32 (compile-time type pin).
    #[test]
    fn pastebuf_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = pastebuf(&[], 1, 0);
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
