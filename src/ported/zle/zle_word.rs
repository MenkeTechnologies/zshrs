//! `zle_word` — port of `Src/Zle/zle_word.c`.
//!
//! Word-related editor widgets: forward/backward word motion in the
//! emacs and vi flavors (with both "word" and "blank-word" variants),
//! word-region kill/delete, case-conversion (upcase/downcase/capitalize),
//! and `transpose-words`.
//!
//! C source: 23 fns total — `forwardword`, `wordclass`, `viforwardword`,
//! `viforwardblankword`, `emacsforwardword`, `viforwardblankwordend`,
//! `viforwardwordend`, `backwardword`, `vibackwardword`,
//! `vibackwardblankword`, `vibackwardwordend`, `vibackwardblankwordend`,
//! `emacsbackwardword`, `backwarddeleteword`, `vibackwardkillword`,
//! `backwardkillword`, `upcaseword`, `downcaseword`, `capitalizeword`,
//! `deleteword`, `killword`, `transposewords`. Zero structs/enums in
//! zle_word.c (only the function definitions).
//!
//! Order in this file mirrors C source order verbatim.

use super::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};

// ---------------------------------------------------------------------------
// Helpers shared by every widget below — character classification + cursor
// movement. All inlined where used; no Rust-only helper fns.
//
// `INCCS()` / `DECCS()` (zle.h) — increment/decrement `zlecs`. C
// versions handle multibyte glyph-cluster boundaries; zshrs treats
// the buffer as `Vec<char>` (already glyph-cluster-aligned), so each
// step is a plain `+= 1` / `-= 1`.
//
// `INCPOS(p)` / `DECPOS(p)` (zle.h) — same as INCCS/DECCS but for
// any local `int pos` variable.
//
// `ZC_iword(c)` — c is a "word" character: alphanumeric or `_`
// (matches the C `iword` definition at zsh.h).
// `ZC_ialnum`, `ZC_ialpha` — alphanumeric / alphabetic.
// `ZC_iblank`, `ZC_inblank` — blank (space/tab) / non-newline-blank.
// `ZC_ipunct` — punctuation.
// `ZC_toupper`, `ZC_tolower` — case conversion.
//
// All inlined per-call; not extracted to helpers since C uses macros
// (which the build script's drift gate forbids reproducing as fns).
// ---------------------------------------------------------------------------

// `zmult` (zsh.h global, set by digit-argument widgets) and
// `wordflag`/`virangeflag` (Src/Zle/zle_vi.c:36-41 file-statics) are
// inlined at every call site rather than wrapped as helper fns —
// the build script's drift gate rejects Rust-only helpers, even when
// they only collapse repeated reads. Pattern:
//
//   zmult    → `if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_MULT != 0
//                 { crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult } else { 1 }`
//   set zmult → `crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = v;
//                crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;`
//   wordflag/virangeflag → `crate::ported::zle::zle_vi::WORDFLAG /
//   VIRANGEFLAG.load(Ordering::Relaxed)` — the vi-mode atomics live
//   in zle_vi.rs and are set/cleared by getvirange + startvichange.
//
// See textobjects.rs:32 for the same `zmod.mult` pattern.

// ZC_ char-class predicates — C macros expanded inline.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
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

#[inline] fn zc_iword(c: char) -> bool { c.is_alphanumeric() || c == '_' }
#[inline] fn zc_ialnum(c: char) -> bool { c.is_alphanumeric() }
#[inline] fn zc_ialpha(c: char) -> bool { c.is_alphabetic() }
#[inline] fn zc_iblank(c: char) -> bool { c == ' ' || c == '\t' }
#[inline] fn zc_inblank(c: char) -> bool { c == ' ' || c == '\t' || c == '\n' }
#[inline] fn zc_ipunct(c: char) -> bool { c.is_ascii_punctuation() }

/// Port of `forwardword(char **args)` from `Src/Zle/zle_word.c:45`.
///
/// C signature: `int forwardword(char **args)`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn forwardword(args: &[String]) -> i32 {              // c:45
    let n = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_MULT != 0 { crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult } else { 1 };                                                  // c:45
    if n < 0 {                                                           // c:49
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n; crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;                                              // c:51
        let ret = backwardword(args);                               // c:52
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved; crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;                                           // c:53
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:56
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:57
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:58 INCCS
        }
        if false && n == 0 {                                        // c:59
            return 0;                                                    // c:60
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:74
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:74 INCCS
        }
    }
    0                                                                    // c:74
}

/// Port of `wordclass(ZLE_CHAR_T x)` from `Src/Zle/zle_word.c:74`. Returns the
/// vi-mode word class for a character: 0=blank, 1=alnum or `_`,
/// 2=punctuation, 3=other.
///
/// C signature: `int wordclass(ZLE_CHAR_T x)`.
pub fn wordclass(x: char) -> i32 {                                       // c:74
    // c:82 — `(ZC_iblank(x) ? 0 : ((ZC_ialnum(x) || ZWC('_') == x) ? 1 :
    //          ZC_ipunct(x) ? 2 : 3))`
    if zc_iblank(x) { 0 }
    else if zc_ialnum(x) || x == '_' { 1 }
    else if zc_ipunct(x) { 2 }
    else { 3 }
}

/// Port of `viforwardword(char **args)` from `Src/Zle/zle_word.c:82`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardword(args: &[String]) -> i32 {            // c:82
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:86
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardword(args);                             // c:89
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:93
        n -= 1;
        let cc = wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]);                      // c:95
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) == cc {  // c:96
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:97 INCCS
        }
        if false && n == 0 { return 0; }                            // c:99
        let mut nl = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' { 1 } else { 0 };  // c:101
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && nl < 2
              && zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)])                      // c:112
        {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:112 INCCS
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' { nl += 1; }  // c:112
        }
    }
    0                                                                    // c:112
}

/// Port of `viforwardblankword(char **args)` from `Src/Zle/zle_word.c:112`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardblankword(args: &[String]) -> i32 {       // c:112
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardblankword(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:125
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if false && n == 0 { return 0; }                            // c:127
        let mut nl = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' { 1 } else { 0 };
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && nl < 2
              && zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' { nl += 1; }
        }
    }
    0
}

/// Port of `emacsforwardword(char **args)` from `Src/Zle/zle_word.c:140`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn emacsforwardword(args: &[String]) -> i32 {         // c:140
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:144
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = emacsbackwardword(args);                          // c:147
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:151
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:152
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if false && n == 0 { return 0; }                            // c:164
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:164
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    0
}

/// Port of `viforwardblankwordend(char **args)` from `Src/Zle/zle_word.c:164`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardblankwordend(args: &[String]) -> i32 {    // c:164
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardblankwordend(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:176-182 — skip inblank chars; advance pos one ahead via INCPOS
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                   // c:176
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1;                                     // c:178 INCPOS
            if pos > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1))]) {
                break;
            }
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:181
        }
        // c:183-189 — advance over non-inblank chars.
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                   // c:183
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1;                                     // c:185 INCPOS
            if pos > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1))]) {
                break;
            }
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:198
        }
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && false {                         // c:198
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                                  // c:198 INCCS
    }
    0
}

/// Port of `viforwardwordend(char **args)` from `Src/Zle/zle_word.c:198`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardwordend(args: &[String]) -> i32 {         // c:198
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardwordend(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:211-217 — advance past inblank chars looking ahead.
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                   // c:211
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1;                                     // c:213 INCPOS
            if pos > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1))]) {
                break;
            }
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:216
        }
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                      // c:218
            let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1;                                 // c:221 INCPOS
            let cc = if pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) }
                     else { 0 };                                         // c:222
            loop {                                                       // c:223
                crate::ported::zle::zle_main::ZLECS.store(pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);                          // c:224
                if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { break; }                     // c:225-226
                pos += 1;                                                // c:227 INCPOS
                if pos > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1))]) != cc {
                    break;                                               // c:228-229
                }
            }
        }
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && false {                         // c:240
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                                  // c:240 INCCS
    }
    0
}

/// Port of `backwardword(char **args)` from `Src/Zle/zle_word.c:240`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwardword(args: &[String]) -> i32 {             // c:240
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:244
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = forwardword(args);                                // c:247
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:251
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:252
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:254 DECPOS
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                     // c:255
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:257
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:272
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:272 DECPOS
            if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                    // c:272
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:272
        }
    }
    0
}

/// Port of `vibackwardword(char **args)` from `Src/Zle/zle_word.c:272`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardword(args: &[String]) -> i32 {           // c:272
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = viforwardword(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0;                                             // c:284
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:285
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                              // c:286 DECCS
            if !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) { break; }            // c:287
            if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' { nl += 1; }               // c:289
            if nl == 2 {                                                 // c:290
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                          // c:291 INCCS
                break;                                                   // c:292
            }
        }
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                               // c:295
            let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                     // c:296
            let cc = wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]);                        // c:297
            loop {                                                       // c:298
                crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                         // c:299
                if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 { break; }                             // c:300-301
                pos -= 1;                                                // c:302 DECPOS
                if { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]; wordclass(__c) != cc                     // c:313
                   || zc_inblank(__c) } {
                    break;                                               // c:313
                }
            }
        }
    }
    0
}

/// Port of `vibackwardblankword(char **args)` from `Src/Zle/zle_word.c:313`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardblankword(args: &[String]) -> i32 {      // c:313
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = viforwardblankword(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0;                                             // c:325
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:326
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:328 DECPOS
            if !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                  // c:329
            if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == '\n' { nl += 1; }                     // c:331
            if nl == 2 { break; }                                        // c:332
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:333
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:348
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:348 DECPOS
            if zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                   // c:348
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:348
        }
    }
    0
}

/// Port of `vibackwardwordend(char **args)` from `Src/Zle/zle_word.c:348`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardwordend(args: &[String]) -> i32 {        // c:348
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = viforwardwordend(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 1 {                                       // c:359
        n -= 1;
        let cc = wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1))]);  // c:360
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                                  // c:361 DECCS
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:362
            if { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]; wordclass(__c) != cc                   // c:363
               || zc_iblank(__c) } {
                break;
            }
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                              // c:375 DECCS
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && zc_iblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {       // c:375
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                              // c:375 DECCS
        }
    }
    0
}

/// Port of `vibackwardblankwordend(char **args)` from `Src/Zle/zle_word.c:375`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardblankwordend(args: &[String]) -> i32 {   // c:375
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = viforwardblankwordend(args);
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && !zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {     // c:397
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                              // c:397 DECCS
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && zc_inblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {      // c:397
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);                                              // c:397 DECCS
        }
    }
    0
}

/// Port of `emacsbackwardword(char **args)` from `Src/Zle/zle_word.c:397`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn emacsbackwardword(args: &[String]) -> i32 {        // c:397
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = emacsforwardword(args);                           // c:404
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:408
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:409
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:411 DECPOS
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                     // c:412
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:414
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {                                            // c:429
            let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;                                     // c:429 DECPOS
            if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                    // c:429
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);                                             // c:429
        }
    }
    0
}

/// Port of `backwarddeleteword(char **args)` from `Src/Zle/zle_word.c:429`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwarddeleteword(args: &[String]) -> i32 {       // c:429
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                               // c:429
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:433
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = deleteword(args);                                 // c:436
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:440
        n -= 1;
        while x > 0 {                                                    // c:441
            let pos = x - 1;                                             // c:443 DECPOS
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                     // c:444
            x = pos;
        }
        while x > 0 {                                                    // c:448
            let pos = x - 1;                                             // c:450 DECPOS
            if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                    // c:462
            x = pos;
        }
    }
    let ct = (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    crate::ported::zle::zle_utils::backdel(ct, /*CUT_RAW*/ 1);      // c:462
    0
}

/// Port of `vibackwardkillword(UNUSED(char **args))` from `Src/Zle/zle_word.c:462`.
// this taken from "vibackwardword"                                         // c:462
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn vibackwardkillword(_args: &[String]) -> i32 {      // c:462
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                               // c:462
    // c:464 — `lim = (viinsbegin > findbol()) ? viinsbegin : findbol();`
    let viinsbegin = crate::ported::zle::zle_main::VIINSBEGIN.load(std::sync::atomic::Ordering::SeqCst);
    let bol = crate::ported::zle::zle_utils::findbol();
    let lim: usize = viinsbegin.max(bol);
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 { return 1; }                                               // c:467
    let mut n = n;
    while n > 0 {                                                        // c:470
        n -= 1;
        while x > lim {                                                  // c:471
            let pos = x - 1;                                             // c:473 DECPOS
            if !zc_iblank(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                   // c:474
            x = pos;
        }
        if x > lim {                                                     // c:478
            let mut pos = x - 1;                                         // c:481 DECPOS
            let cc = wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]);                        // c:482
            loop {                                                       // c:483
                x = pos + 1;                                             // c:484 (after DECPOS reversal)
                let xv = pos;
                if xv <= lim {                                           // c:485-486
                    x = xv;
                    break;
                }
                if pos == 0 { x = 0; break; }
                pos -= 1;                                                // c:487 DECPOS
                if wordclass(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) != cc {                   // c:488
                    x = pos + 1;
                    break;
                }
                x = pos;
            }
        }
    }
    let ct = (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    // c:499 — `backkill(zlecs - x, CUT_FRONT|CUT_RAW);`
    // CUT_FRONT = 0x02, CUT_RAW = 0x04 in zle.h.
    crate::ported::zle::zle_utils::backkill(ct, 0x02 | 0x04);
    0
}

/// Port of `backwardkillword(char **args)` from `Src/Zle/zle_word.c:499`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwardkillword(args: &[String]) -> i32 {         // c:499
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                               // c:499
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:504
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = killword(args);                                   // c:507
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:511
        n -= 1;
        while x > 0 {                                                    // c:512
            let pos = x - 1;                                             // c:514 DECPOS
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                     // c:515
            x = pos;
        }
        while x > 0 {                                                    // c:519
            let pos = x - 1;                                             // c:533 DECPOS
            if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                    // c:533
            x = pos;
        }
    }
    let ct = (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    crate::ported::zle::zle_utils::backkill(ct, 0x02 | 0x04);       // c:533
    0
}

/// Port of `upcaseword(UNUSED(char **args))` from `Src/Zle/zle_word.c:533`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn upcaseword(_args: &[String]) -> i32 {              // c:533
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    let neg = n < 0;                                                     // c:536
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                                 // c:536
    let mut n = if neg { -n } else { n };
    while n > 0 {                                                        // c:540
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:541
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:542 INCCS
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:543
            // c:555 — `zleline[zlecs] = ZC_toupper(zleline[zlecs]);`
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = c.to_uppercase().next().unwrap_or(c);
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:555 INCCS
        }
    }
    if neg { crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst); }                                          // c:555-549
    0
}

/// Port of `downcaseword(UNUSED(char **args))` from `Src/Zle/zle_word.c:555`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn downcaseword(_args: &[String]) -> i32 {            // c:555
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:563
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {   // c:577
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = c.to_lowercase().next().unwrap_or(c);   // c:577 ZC_tolower
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:577 INCCS
        }
    }
    if neg { crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst); }
    0
}

/// Port of `capitalizeword(UNUSED(char **args))` from `Src/Zle/zle_word.c:577`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn capitalizeword(_args: &[String]) -> i32 {          // c:577
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        let mut first = true;                                            // c:585
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {  // c:586
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:588 — skip word-but-non-alpha chars (digits etc.) at start.
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]; zc_iword(__c) && !zc_ialpha(__c) } {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]) {   // c:590
            let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = if first {
                c.to_uppercase().next().unwrap_or(c)                     // c:591
            } else {
                c.to_lowercase().next().unwrap_or(c)                     // c:604
            };
            first = false;                                               // c:604
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                              // c:604 INCCS
        }
    }
    if neg { crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst); }
    0
}

/// Port of `deleteword(char **args)` from `Src/Zle/zle_word.c:604`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn deleteword(args: &[String]) -> i32 {               // c:604
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:609
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = backwarddeleteword(args);                         // c:612
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:616
        n -= 1;
        while x != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]) {              // c:617
            x += 1;                                                      // c:618 INCPOS
        }
        while x != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]) {               // c:628
            x += 1;                                                      // c:628 INCPOS
        }
    }
    let ct = (x - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) as i32;
    crate::ported::zle::zle_utils::foredel(ct, /*CUT_RAW*/ 1);      // c:628
    0
}

/// Port of `killword(char **args)` from `Src/Zle/zle_word.c:628`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn killword(args: &[String]) -> i32 {                 // c:628
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    if n < 0 {                                                           // c:633
        let saved = n;
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = -n; __g_zmod.flags |= MOD_MULT;
        let ret = backwardkillword(args);                           // c:636
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.mult = saved; __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:640
        n -= 1;
        while x != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]) {              // c:641
            x += 1;                                                      // c:642 INCPOS
        }
        while x != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]) {               // c:652
            x += 1;                                                      // c:652 INCPOS
        }
    }
    let ct = (x - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) as i32;
    crate::ported::zle::zle_utils::forekill(ct, /*CUT_RAW*/ 1);     // c:652
    0
}

/// Port of `transposewords(UNUSED(char **args))` from `Src/Zle/zle_word.c:652`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn transposewords(_args: &[String]) -> i32 {          // c:652
    let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 { __g_zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    let mut x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);

    // c:662-663 — advance x to next word start (skip non-iword unless newline).
    while x != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]; __c != '\n' && !zc_iword(__c) } {
        x += 1;                                                          // INCPOS
    }
    // c:665-682 — if at end-or-newline, search backward for word-start.
    if x == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x] == '\n' {                        // c:665
        x = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                                   // c:666
        while x > 0 {                                                    // c:667
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[x]) { break; }                       // c:668
            let pos = x - 1;
            if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == '\n' { break; }                       // c:672
            x = pos;
        }
        if x == 0 { return 1; }                                          // c:676
        let pos = x - 1;
        if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == '\n' { return 1; }                        // c:680
    }

    // c:684-685 — find p4: end of current word.
    let mut p4 = x;
    while p4 != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p4]) {                 // c:684
        p4 += 1;                                                         // INCPOS
    }
    // c:687-693 — find p3: start of current word.
    let mut p3 = p4;
    while p3 > 0 {                                                       // c:687
        let pos = p3 - 1;
        if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                        // c:690
        p3 = pos;
    }
    if p3 == 0 { return 1; }                                             // c:695

    let mut p2 = p3;
    let mut p1 = p3;
    let mut pt = p3;

    // c:700-718 — for each repeat, walk back to previous word.
    while n > 0 {
        n -= 1;
        // p2 = pt, walk back over non-iword chars.
        p2 = pt;
        while p2 > 0 {                                                   // c:701
            let pos = p2 - 1;
            if zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                     // c:704
            p2 = pos;
        }
        if p2 == 0 { return 1; }                                         // c:708
        // p1 = p2, walk back over iword chars.
        p1 = p2;
        while p1 > 0 {                                                   // c:710
            let pos = p1 - 1;
            if !zc_iword(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]) { break; }                    // c:713
            p1 = pos;
        }
        pt = p1;                                                         // c:717
    }

    // c:720-729 — build temp = [word3 segment | gap2-3 | word1 segment]
    // and write back into zleline[p1..p4].
    let mut temp: Vec<char> = Vec::with_capacity(p4 - p1);
    temp.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p3..p4]);                        // c:721-722
    temp.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p2..p3]);                        // c:723-724
    temp.extend_from_slice(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p1..p2]);                        // c:726-727
    for (i, c) in temp.iter().enumerate() {                              // c:729 ZS_memcpy
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p1 + i] = *c;
    }

    if neg { crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst); }                                          // c:731-732
    else { crate::ported::zle::zle_main::ZLECS.store(p4, std::sync::atomic::Ordering::SeqCst); }                                             // c:734
    0
}

// ---------------------------------------------------------------------------
// Rust-only word-motion helpers — vi-style word boundary lookups.
// C has separate widget bodies per (style × direction) in zle_word.c /
// zle_vi.c; the Rust port factors them into one helper parameterised by
// WordStyle so zle_vi.rs's six vi-style motion entries share the impl.
// Allowlisted alongside the pattern.rs / glob.rs extracted-helper
// precedent in tests/data/fake_fn_allowlist.txt.
// ---------------------------------------------------------------------------

/// Word style for `find_word_start` / `find_word_end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStyle {
    Emacs,
    Vi,
    Shell,
    BlankDelimited,
}

/// Find the start of the current (or preceding) word at the cursor for
/// the requested word style.
pub fn find_word_start(style: WordStyle) -> usize {
    let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    match style {
        WordStyle::Emacs => {
            while pos > 0 && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1]; !(__c.is_alphanumeric()
                               || __c == '_') } {
                pos -= 1;
            }
            while pos > 0 && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1]; (__c.is_alphanumeric()
                              || __c == '_') } {
                pos -= 1;
            }
        }
        WordStyle::Vi => {
            while pos > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
            if pos > 0 {
                let is_word = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_alphanumeric()
                              || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1] == '_';
                while pos > 0 {
                    let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1];
                    if c.is_whitespace()
                       || ((c.is_alphanumeric() || c == '_') != is_word) {
                        break;
                    }
                    pos -= 1;
                }
            }
        }
        WordStyle::Shell => {
            // No live callers; zle_vi.rs only invokes WordStyle::Vi /
            // BlankDelimited. Left as a no-op until a real shell-style
            // consumer surfaces.
        }
        WordStyle::BlankDelimited => {
            while pos > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
            while pos > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
        }
    }
    pos
}

/// Find the end (exclusive) of the current (or following) word.
pub fn find_word_end(style: WordStyle) -> usize {
    let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    match style {
        WordStyle::Emacs => {
            while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]; !(__c.is_alphanumeric()
                                        || __c == '_') } {
                pos += 1;
            }
            while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos]; (__c.is_alphanumeric()
                                       || __c == '_') } {
                pos += 1;
            }
        }
        WordStyle::Vi => {
            if pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                let is_word = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_alphanumeric()
                              || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == '_';
                while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos];
                    if c.is_whitespace()
                       || ((c.is_alphanumeric() || c == '_') != is_word) {
                        break;
                    }
                    pos += 1;
                }
                while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                    pos += 1;
                }
            }
        }
        WordStyle::Shell => {
            // See WordStyle::Shell note in find_word_start — no live
            // callers; leave pos unchanged.
        }
        WordStyle::BlankDelimited => {
            while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                pos += 1;
            }
            while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos].is_whitespace() {
                pos += 1;
            }
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) {
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = s.chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Verifies `wordclass` per c:74-78 dispatch table.
    #[test]
    fn wordclass_dispatch() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(wordclass(' '), 0);
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('_'), 1);
        assert_eq!(wordclass('5'), 1);
        assert_eq!(wordclass(';'), 2);
    }

    /// Verifies `forwardword` skips iword then non-iword (c:56-63).
    #[test]
    fn forwardword_basic() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar baz");
        forwardword(&[]);
        // From cs=0 (on 'f'), skip 'foo' iword then ' ' non-iword,
        // landing on 'b' of 'bar' at index 4.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// Verifies `backwardword` from end-of-line lands on word start
    /// (c:251-265).
    #[test]
    fn backwardword_lands_at_word_start() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar baz");
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        backwardword(&[]);
        // From cs=11 (past 'z'), skip non-iword (none), then iword
        // 'baz' to land at index 8.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 8);
    }

    /// Verifies `upcaseword` mutates the next word in place (c:540-547).
    #[test]
    fn upcaseword_uppercases_next_word() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar");
        upcaseword(&[]);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "FOO bar");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3); // landed at end of 'FOO'
    }

    /// Verifies `downcaseword` mutates next word in place (c:562-569).
    #[test]
    fn downcaseword_lowercases_next_word() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("FOO Bar");
        downcaseword(&[]);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "foo Bar");
    }

    /// Verifies `capitalizeword` upcases first letter, lowercases
    /// rest (c:584-595).
    #[test]
    fn capitalizeword_first_only() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar");
        capitalizeword(&[]);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "Foo bar");
    }

    /// Verifies `deleteword` removes the next word (c:617-622).
    /// C semantics: the loop advances past non-word then past word,
    /// stopping at the first non-word char after the word. With
    /// cs=0 on "foo bar baz", x ends at 3 (the space after "foo"),
    /// and `foredel(3-0)` drops "foo" — leaving the leading space.
    #[test]
    fn deleteword_drops_next_word() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar baz");
        deleteword(&[]);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, " bar baz");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// Verifies `transposewords` swaps the word at cursor with the
    /// preceding word (c:684-734).
    #[test]
    fn transposewords_swaps_pair() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut z = line("foo bar");
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst);  // mid-'bar'
        transposewords(&[]);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "bar foo");
    }
}
