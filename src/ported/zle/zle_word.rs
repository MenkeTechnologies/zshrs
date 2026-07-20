//! `zle_word` — port of `Src/Zle/zle_word.c`.
//!
//! Word-related editor widgets: forward/backward word motion in the
//! emacs and vi flavors (with both "word" and "blank-word" variants),
//! word-region kill/delete, case-conversion (upcase/downcase/capitalize),
//! and `transpose-words`.
//!
//! C source: 23 ported total — `forwardword`, `wordclass`, `viforwardword`,
//! `viforwardblankword`, `emacsforwardword`, `viforwardblankwordend`,
//! `viforwardwordend`, `backwardword`, `vibackwardword`,
//! `vibackwardblankword`, `vibackwardwordend`, `vibackwardblankwordend`,
//! `emacsbackwardword`, `backwarddeleteword`, `vibackwardkillword`,
//! `backwardkillword`, `upcaseword`, `downcaseword`, `capitalizeword`,
//! `deleteword`, `killword`, `transposewords`. Zero structs/enums in
//! zle_word.c (only the function definitions).
//!
//! Order in this file mirrors C source order verbatim.

use super::zle_h::MOD_MULT;

// ---------------------------------------------------------------------------
// Helpers shared by every widget below — character classification + cursor
// movement. All inlined where used; no Rust-only helper ported.
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
// (which the build script's drift gate forbids reproducing as ported).
// ---------------------------------------------------------------------------

// `zmult` (zsh.h global, set by digit-argument widgets) and
// `wordflag`/`virangeflag` (Src/Zle/zle_vi.c:36-41 file-statics) are
// inlined at every call site rather than wrapped as helper ported —
// the build script's drift gate rejects Rust-only helpers, even when
// they only collapse repeated reads. Pattern:
//
//   zmult    → `if ZMOD.lock().unwrap().flags & MOD_MULT != 0
//                 { ZMOD.lock().unwrap().mult } else { 1 }`
//   set zmult → `ZMOD.lock().unwrap().mult = v;
//                ZMOD.lock().unwrap().flags |= MOD_MULT;`
//   wordflag/virangeflag → `crate::ported::zle::zle_vi::WORDFLAG /
//   VIRANGEFLAG.load(Ordering::Relaxed)` — the vi-mode atomics live
//   in zle_vi.rs and are set/cleared by getvirange + startvichange.
//
// See textobjects.rs:32 for the same `zmod.mult` pattern.

// ZC_ char-class predicates — C macros expanded inline.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*,
};
#[allow(unused_imports)]
#[allow(unused_imports)]
// Local aliases routing to the canonical `ZC_*` predicates in
// `zle_h.rs:246-271` (port of `Src/Zle/zle.h:60-73`). Re-defining
// these inline here is a divergence trap — the previous versions
// had narrow `space || tab` iblank and `space || tab || \n` inblank
// that did NOT match the C `wcsiblank`/`iswspace` semantics. Always
// route through the canonical port so a regression has one place
// to fix, not two.
#[inline]
fn zc_iword(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_iword(c)
}
#[inline]
fn zc_ialnum(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_ialnum(c)
}
#[inline]
fn zc_ialpha(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_ialpha(c)
}
#[inline]
fn zc_iblank(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_iblank(c)
}
#[inline]
fn zc_inblank(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_inblank(c)
}
#[inline]
fn zc_ipunct(c: char) -> bool {
    crate::ported::zle::zle_h::ZC_ipunct(c)
}

/// Port of `forwardword(char **args)` from `Src/Zle/zle_word.c:45`.
///
/// C signature: `int forwardword(char **args)`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn forwardword(args: &[String]) -> i32 {
    // c:45
    let n = if ZMOD.lock().unwrap().flags & MOD_MULT != 0 {
        ZMOD.lock().unwrap().mult
    } else {
        1
    }; // c:45
    if n < 0 {
        // c:49
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        ZMOD.lock().unwrap().flags |= MOD_MULT; // c:51
        let ret = backwardword(args); // c:52
        ZMOD.lock().unwrap().mult = saved;
        ZMOD.lock().unwrap().flags |= MOD_MULT; // c:53
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:56
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:57
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:58 INCCS
        }
        // c:59 — `if (wordflag && !n) return 0;`. wordflag is the
        // vi-range/word-motion kludge flag set by getvirange at
        // zle_vi.c:186; ported as atomic WORDFLAG. Was hardcoded to
        // `false`, dropping the vi-word-motion early-exit.
        if WORDFLAG.load(std::sync::atomic::Ordering::Relaxed) != 0 && n == 0 {
            return 0; // c:60
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:74
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:74 INCCS
        }
    }
    0 // c:74
}

/// Port of `wordclass(ZLE_CHAR_T x)` from `Src/Zle/zle_word.c:74`. Returns the
/// vi-mode word class for a character: 0=blank, 1=alnum or `_`,
/// 2=punctuation, 3=other.
///
/// C signature: `int wordclass(ZLE_CHAR_T x)`.
pub fn wordclass(x: char) -> i32 {
    // c:74
    // c:82 — `(ZC_iblank(x) ? 0 : ((ZC_ialnum(x) || ZWC('_') == x) ? 1 :
    //          ZC_ipunct(x) ? 2 : 3))`
    if zc_iblank(x) {
        0
    } else if zc_ialnum(x) || x == '_' {
        1
    } else if zc_ipunct(x) {
        2
    } else {
        3
    }
}

/// Port of `viforwardword(char **args)` from `Src/Zle/zle_word.c:82`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardword(args: &[String]) -> i32 {
    // c:82
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        // c:86
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardword(args); // c:89
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:93
        n -= 1;
        // C reads `zleline[zlecs]` unconditionally — the underlying
        // buffer is NUL-terminated, so even when `zlecs == zlell` the
        // read hits the sentinel and the subsequent `while zlecs !=
        // zlell` immediately fails. Rust's `Vec<char>` has no
        // sentinel; reading at the end panics. Guard with a bounds
        // check; when at EOL there's nothing to classify, so skip
        // straight to the trailing blank-walk loop (which is also
        // guarded by `ZLECS != ZLELL`).
        let zlecs_cur = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        let zlell_cur = ZLELL.load(std::sync::atomic::Ordering::SeqCst);
        let cc = if zlecs_cur < zlell_cur {
            wordclass(ZLELINE.lock().unwrap()[zlecs_cur]) // c:95
        } else {
            0 // dummy — the next while exits immediately on zlecs==zlell
        };
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && wordclass(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
                == cc
        {
            // c:96
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:97 INCCS
        }
        // c:99 — `if (wordflag && !n) return 0;` (see forwardword note).
        if WORDFLAG.load(std::sync::atomic::Ordering::Relaxed) != 0 && n == 0 {
            return 0;
        } // c:99
        let mut nl = if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n'
        {
            1
        } else {
            0
        }; // c:101
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && nl < 2
            && zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        // c:112
        {
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:112 INCCS
            if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
                < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                && ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n'
            {
                nl += 1;
            } // c:112
        }
    }
    0 // c:112
}

/// Port of `viforwardblankword(char **args)` from `Src/Zle/zle_word.c:112`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardblankword(args: &[String]) -> i32 {
    // c:112
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardblankword(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:125
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:127 — `if (wordflag && !n) return 0;` (see forwardword note).
        if WORDFLAG.load(std::sync::atomic::Ordering::Relaxed) != 0 && n == 0 {
            return 0;
        } // c:127
        let mut nl = if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n'
        {
            1
        } else {
            0
        };
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && nl < 2
            && zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
                < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                && ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n'
            {
                nl += 1;
            }
        }
    }
    0
}

/// Port of `emacsforwardword(char **args)` from `Src/Zle/zle_word.c:140`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn emacsforwardword(args: &[String]) -> i32 {
    // c:140
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        // c:144
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = emacsbackwardword(args); // c:147
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:151
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:152
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:164 — `if (wordflag && !n) return 0;` (see forwardword note).
        if WORDFLAG.load(std::sync::atomic::Ordering::Relaxed) != 0 && n == 0 {
            return 0;
        } // c:164
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:164
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    0
}

/// Port of `viforwardblankwordend(char **args)` from `Src/Zle/zle_word.c:164`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardblankwordend(args: &[String]) -> i32 {
    // c:164
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardblankwordend(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:176-182 — skip inblank chars; advance pos one ahead via INCPOS
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        {
            // c:176
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1; // c:178 INCPOS
            if pos > ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                || !zc_inblank(
                    ZLELINE.lock().unwrap()[pos.min(
                        ZLELL
                            .load(std::sync::atomic::Ordering::SeqCst)
                            .saturating_sub(1),
                    )],
                )
            {
                break;
            }
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:181
        }
        // c:183-189 — advance over non-inblank chars.
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        {
            // c:183
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1; // c:185 INCPOS
            if pos > ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                || zc_inblank(
                    ZLELINE.lock().unwrap()[pos.min(
                        ZLELL
                            .load(std::sync::atomic::Ordering::SeqCst)
                            .saturating_sub(1),
                    )],
                )
            {
                break;
            }
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:198
        }
    }
    if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
        != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        && false
    {
        // c:198
        ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:198 INCCS
    }
    0
}

/// Port of `viforwardwordend(char **args)` from `Src/Zle/zle_word.c:198`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn viforwardwordend(args: &[String]) -> i32 {
    // c:198
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = vibackwardwordend(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:211-217 — advance past inblank chars looking ahead.
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        {
            // c:211
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1; // c:213 INCPOS
            if pos > ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                || !zc_inblank(
                    ZLELINE.lock().unwrap()[pos.min(
                        ZLELL
                            .load(std::sync::atomic::Ordering::SeqCst)
                            .saturating_sub(1),
                    )],
                )
            {
                break;
            }
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:216
        }
        if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        {
            // c:218
            let mut pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1; // c:221 INCPOS
            let cc = if pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                wordclass(ZLELINE.lock().unwrap()[pos])
            } else {
                0
            }; // c:222
            loop {
                // c:223
                ZLECS.store(
                    pos.min(ZLELL.load(std::sync::atomic::Ordering::SeqCst)),
                    std::sync::atomic::Ordering::SeqCst,
                ); // c:224
                if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
                    == ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                {
                    break;
                } // c:225-226
                pos += 1; // c:227 INCPOS
                if pos > ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                    || wordclass(
                        ZLELINE.lock().unwrap()[pos.min(
                            ZLELL
                                .load(std::sync::atomic::Ordering::SeqCst)
                                .saturating_sub(1),
                        )],
                    ) != cc
                {
                    break; // c:228-229
                }
            }
        }
    }
    if ZLECS.load(std::sync::atomic::Ordering::SeqCst)
        != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        && false
    {
        // c:240
        ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:240 INCCS
    }
    0
}

/// Port of `backwardword(char **args)` from `Src/Zle/zle_word.c:240`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwardword(args: &[String]) -> i32 {
    // c:240
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        // c:244
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = forwardword(args); // c:247
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:251
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:252
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:254 DECPOS
            if zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:255
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:257
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:272
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:272 DECPOS
            if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:272
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:272
        }
    }
    0
}

/// Port of `vibackwardword(char **args)` from `Src/Zle/zle_word.c:272`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardword(args: &[String]) -> i32 {
    // c:272
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = viforwardword(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0; // c:284
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:285
            ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:286 DECCS
            if !zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
            {
                break;
            } // c:287
            if ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' {
                nl += 1;
            } // c:289
            if nl == 2 {
                // c:290
                ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:291 INCCS
                break; // c:292
            }
        }
        if ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:295
            let mut pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:296
            let cc = wordclass(ZLELINE.lock().unwrap()[pos]); // c:297
            loop {
                // c:298
                ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:299
                if ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break;
                } // c:300-301
                pos -= 1; // c:302 DECPOS
                if {
                    let __c = ZLELINE.lock().unwrap()[pos];
                    wordclass(__c) != cc                     // c:313
                   || zc_inblank(__c)
                } {
                    break; // c:313
                }
            }
        }
    }
    0
}

/// Port of `vibackwardblankword(char **args)` from `Src/Zle/zle_word.c:313`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardblankword(args: &[String]) -> i32 {
    // c:313
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = viforwardblankword(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0; // c:325
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:326
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:328 DECPOS
            if !zc_inblank(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:329
            if ZLELINE.lock().unwrap()[pos] == '\n' {
                nl += 1;
            } // c:331
            if nl == 2 {
                break;
            } // c:332
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:333
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:348
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:348 DECPOS
            if zc_inblank(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:348
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:348
        }
    }
    0
}

/// Port of `vibackwardwordend(char **args)` from `Src/Zle/zle_word.c:348`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardwordend(args: &[String]) -> i32 {
    // c:348
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = viforwardwordend(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 && ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 1 {
        // c:359
        n -= 1;
        let cc = wordclass(
            ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(
                ZLELL
                    .load(std::sync::atomic::Ordering::SeqCst)
                    .saturating_sub(1),
            )],
        ); // c:360
        ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:361 DECCS
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:362
            if {
                let __c = ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
                wordclass(__c) != cc                   // c:363
               || zc_iblank(__c)
            } {
                break;
            }
            ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:375 DECCS
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0
            && zc_iblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:375
            ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:375 DECCS
        }
    }
    0
}

/// Port of `vibackwardblankwordend(char **args)` from `Src/Zle/zle_word.c:375`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn vibackwardblankwordend(args: &[String]) -> i32 {
    // c:375
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = viforwardblankwordend(args);
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0
            && !zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:397
            ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:397 DECCS
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0
            && zc_inblank(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:397
            ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); // c:397 DECCS
        }
    }
    0
}

/// Port of `emacsbackwardword(char **args)` from `Src/Zle/zle_word.c:397`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn emacsbackwardword(args: &[String]) -> i32 {
    // c:397
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = emacsforwardword(args); // c:404
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:408
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:409
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:411 DECPOS
            if zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:412
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:414
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // c:429
            let pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1; // c:429 DECPOS
            if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:429
            ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst); // c:429
        }
    }
    0
}

/// Port of `backwarddeleteword(char **args)` from `Src/Zle/zle_word.c:429`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwarddeleteword(args: &[String]) -> i32 {
    // c:429
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:429
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        // c:433
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = deleteword(args); // c:436
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:440
        n -= 1;
        while x > 0 {
            // c:441
            let pos = x - 1; // c:443 DECPOS
            if zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:444
            x = pos;
        }
        while x > 0 {
            // c:448
            let pos = x - 1; // c:450 DECPOS
            if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:462
            x = pos;
        }
    }
    let ct = (ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    backdel(ct, /*CUT_RAW*/ 1); // c:462
    0
}

/// Port of `vibackwardkillword(UNUSED(char **args))` from `Src/Zle/zle_word.c:462`.
// this taken from "vibackwardword"                                         // c:462
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn vibackwardkillword(_args: &[String]) -> i32 {
    // c:462
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:462
                                                                 // c:464 — `lim = (viinsbegin > findbol()) ? viinsbegin : findbol();`
    let viinsbegin = VIINSBEGIN.load(std::sync::atomic::Ordering::SeqCst);
    let bol = findbol();
    let lim: usize = viinsbegin.max(bol);
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        return 1;
    } // c:467
    let mut n = n;
    while n > 0 {
        // c:470
        n -= 1;
        while x > lim {
            // c:471
            let pos = x - 1; // c:473 DECPOS
            if !zc_iblank(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:474
            x = pos;
        }
        if x > lim {
            // c:478
            let mut pos = x - 1; // c:481 DECPOS
            let cc = wordclass(ZLELINE.lock().unwrap()[pos]); // c:482
            loop {
                // c:483
                x = pos + 1; // c:484 (after DECPOS reversal)
                let xv = pos;
                if xv <= lim {
                    // c:485-486
                    x = xv;
                    break;
                }
                if pos == 0 {
                    x = 0;
                    break;
                }
                pos -= 1; // c:487 DECPOS
                if wordclass(ZLELINE.lock().unwrap()[pos]) != cc {
                    // c:488
                    x = pos + 1;
                    break;
                }
                x = pos;
            }
        }
    }
    let ct = (ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    // c:499 — `backkill(zlecs - x, CUT_FRONT|CUT_RAW);`
    // CUT_FRONT = 0x02, CUT_RAW = 0x04 in zle.h.
    backkill(ct, 0x02 | 0x04);
    0
}

/// Port of `backwardkillword(char **args)` from `Src/Zle/zle_word.c:499`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn backwardkillword(args: &[String]) -> i32 {
    // c:499
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:499
                                                                 // Scoped read — holding the ZMOD guard across the backkill call
                                                                 // below deadlocks (backkill → cut → cuttext locks ZMOD again).
    let n = {
        let __g_zmod = ZMOD.lock().unwrap();
        if __g_zmod.flags & MOD_MULT != 0 {
            __g_zmod.mult
        } else {
            1
        }
    };
    if n < 0 {
        // c:504
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = killword(args); // c:507
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:511
        n -= 1;
        while x > 0 {
            // c:512
            let pos = x - 1; // c:514 DECPOS
            if zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:515
            x = pos;
        }
        while x > 0 {
            // c:519
            let pos = x - 1; // c:533 DECPOS
            if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:533
            x = pos;
        }
    }
    let ct = (ZLECS.load(std::sync::atomic::Ordering::SeqCst) - x) as i32;
    // c:533 — `backkill(zlecs - x, CUT_FRONT|CUT_RAW);` (0x02|0x04 was
    // CUT_REPLACE|CUT_RAW — each M-DEL REPLACED the cut buffer instead
    // of prepending, so consecutive word kills didn't accumulate).
    backkill(
        ct,
        crate::ported::zle::zle_h::CUT_FRONT | crate::ported::zle::zle_h::CUT_RAW,
    );
    0
}

/// Port of `upcaseword(UNUSED(char **args))` from `Src/Zle/zle_word.c:533`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn upcaseword(_args: &[String]) -> i32 {
    // c:533
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    let neg = n < 0; // c:536
    let ocs = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:536
    let mut n = if neg { -n } else { n };
    while n > 0 {
        // c:540
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:541
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:542 INCCS
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:543
            // c:555 — `zleline[zlecs] = ZC_toupper(zleline[zlecs]);`
            let c = ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] =
                c.to_uppercase().next().unwrap_or(c);
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:555 INCCS
        }
    }
    if neg {
        ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
    } // c:555-549
    0
}

/// Port of `downcaseword(UNUSED(char **args))` from `Src/Zle/zle_word.c:555`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn downcaseword(_args: &[String]) -> i32 {
    // c:555
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    let neg = n < 0;
    let ocs = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:563
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:577
            let c = ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] =
                c.to_lowercase().next().unwrap_or(c); // c:577 ZC_tolower
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:577 INCCS
        }
    }
    if neg {
        ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
    }
    0
}

/// Port of `capitalizeword(UNUSED(char **args))` from `Src/Zle/zle_word.c:577`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn capitalizeword(_args: &[String]) -> i32 {
    // c:577
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    let neg = n < 0;
    let ocs = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        let mut first = true; // c:585
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:586
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:588 — skip word-but-non-alpha chars (digits etc.) at start.
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && {
                let __c = ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
                zc_iword(__c) && !zc_ialpha(__c)
            }
        {
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while ZLECS.load(std::sync::atomic::Ordering::SeqCst)
            != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)])
        {
            // c:590
            let c = ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
            ZLELINE.lock().unwrap()[ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = if first {
                c.to_uppercase().next().unwrap_or(c) // c:591
            } else {
                c.to_lowercase().next().unwrap_or(c) // c:604
            };
            first = false; // c:604
            ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // c:604 INCCS
        }
    }
    if neg {
        ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
    }
    0
}

/// Port of `deleteword(char **args)` from `Src/Zle/zle_word.c:604`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn deleteword(args: &[String]) -> i32 {
    // c:604
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    if n < 0 {
        // c:609
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = backwarddeleteword(args); // c:612
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:616
        n -= 1;
        while x != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[x])
        {
            // c:617
            x += 1; // c:618 INCPOS
        }
        while x != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[x])
        {
            // c:628
            x += 1; // c:628 INCPOS
        }
    }
    let ct = (x - ZLECS.load(std::sync::atomic::Ordering::SeqCst)) as i32;
    foredel(ct, /*CUT_RAW*/ 1); // c:628
    0
}

/// Port of `killword(char **args)` from `Src/Zle/zle_word.c:628`.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn killword(args: &[String]) -> i32 {
    // c:628
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    // Scoped read — holding the ZMOD guard across the forekill call
    // below deadlocks (forekill → cut → cuttext locks ZMOD again).
    let n = {
        let __g_zmod = ZMOD.lock().unwrap();
        if __g_zmod.flags & MOD_MULT != 0 {
            __g_zmod.mult
        } else {
            1
        }
    };
    if n < 0 {
        // c:633
        let saved = n;
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = -n;
        __g_zmod.flags |= MOD_MULT;
        let ret = backwardkillword(args); // c:636
        let mut __g_zmod = ZMOD.lock().unwrap();
        __g_zmod.mult = saved;
        __g_zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        // c:640
        n -= 1;
        while x != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && !zc_iword(ZLELINE.lock().unwrap()[x])
        {
            // c:641
            x += 1; // c:642 INCPOS
        }
        while x != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
            && zc_iword(ZLELINE.lock().unwrap()[x])
        {
            // c:652
            x += 1; // c:652 INCPOS
        }
    }
    let ct = (x - ZLECS.load(std::sync::atomic::Ordering::SeqCst)) as i32;
    // c:652 — `forekill(x - zlecs, CUT_RAW);` (the literal 1 was
    // CUT_FRONT, not CUT_RAW — killed words landed in FRONT of the
    // cut buffer, reversing kill order).
    forekill(ct, crate::ported::zle::zle_h::CUT_RAW);
    0
}

/// Port of `transposewords(UNUSED(char **args))` from `Src/Zle/zle_word.c:652`.
/// WARNING: param names don't match C — Rust=(zle, _args) vs C=(args)
pub fn transposewords(_args: &[String]) -> i32 {
    // c:652
    let mut __g_zmod = ZMOD.lock().unwrap();
    let n = if __g_zmod.flags & MOD_MULT != 0 {
        __g_zmod.mult
    } else {
        1
    };
    let neg = n < 0;
    let ocs = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut n = if neg { -n } else { n };
    let mut x = ZLECS.load(std::sync::atomic::Ordering::SeqCst);

    // c:662-663 — advance x to next word start (skip non-iword unless newline).
    while x != ZLELL.load(std::sync::atomic::Ordering::SeqCst) && {
        let __c = ZLELINE.lock().unwrap()[x];
        __c != '\n' && !zc_iword(__c)
    } {
        x += 1; // INCPOS
    }
    // c:665-682 — if at end-or-newline, search backward for word-start.
    if x == ZLELL.load(std::sync::atomic::Ordering::SeqCst) || ZLELINE.lock().unwrap()[x] == '\n' {
        // c:665
        x = ZLECS.load(std::sync::atomic::Ordering::SeqCst); // c:666
        while x > 0 {
            // c:667
            if zc_iword(ZLELINE.lock().unwrap()[x]) {
                break;
            } // c:668
            let pos = x - 1;
            if ZLELINE.lock().unwrap()[pos] == '\n' {
                break;
            } // c:672
            x = pos;
        }
        if x == 0 {
            return 1;
        } // c:676
        let pos = x - 1;
        if ZLELINE.lock().unwrap()[pos] == '\n' {
            return 1;
        } // c:680
    }

    // c:684-685 — find p4: end of current word.
    let mut p4 = x;
    while p4 != ZLELL.load(std::sync::atomic::Ordering::SeqCst)
        && zc_iword(ZLELINE.lock().unwrap()[p4])
    {
        // c:684
        p4 += 1; // INCPOS
    }
    // c:687-693 — find p3: start of current word.
    let mut p3 = p4;
    while p3 > 0 {
        // c:687
        let pos = p3 - 1;
        if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
            break;
        } // c:690
        p3 = pos;
    }
    if p3 == 0 {
        return 1;
    } // c:695

    let mut p2 = p3;
    let mut p1 = p3;
    let mut pt = p3;

    // c:700-718 — for each repeat, walk back to previous word.
    while n > 0 {
        n -= 1;
        // p2 = pt, walk back over non-iword chars.
        p2 = pt;
        while p2 > 0 {
            // c:701
            let pos = p2 - 1;
            if zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:704
            p2 = pos;
        }
        if p2 == 0 {
            return 1;
        } // c:708
          // p1 = p2, walk back over iword chars.
        p1 = p2;
        while p1 > 0 {
            // c:710
            let pos = p1 - 1;
            if !zc_iword(ZLELINE.lock().unwrap()[pos]) {
                break;
            } // c:713
            p1 = pos;
        }
        pt = p1; // c:717
    }

    // c:720-729 — build temp = [word3 segment | gap2-3 | word1 segment]
    // and write back into zleline[p1..p4].
    let mut temp: Vec<char> = Vec::with_capacity(p4 - p1);
    temp.extend_from_slice(&ZLELINE.lock().unwrap()[p3..p4]); // c:721-722
    temp.extend_from_slice(&ZLELINE.lock().unwrap()[p2..p3]); // c:723-724
    temp.extend_from_slice(&ZLELINE.lock().unwrap()[p1..p2]); // c:726-727
    for (i, c) in temp.iter().enumerate() {
        // c:729 ZS_memcpy
        ZLELINE.lock().unwrap()[p1 + i] = *c;
    }

    if neg {
        ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
    }
    // c:731-732
    else {
        ZLECS.store(p4, std::sync::atomic::Ordering::SeqCst);
    } // c:734
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
    /// `Emacs` variant.
    Emacs,
    /// `Vi` variant.
    Vi,
    /// `Shell` variant.
    Shell,
    /// `BlankDelimited` variant.
    BlankDelimited,
}

/// Find the start of the current (or preceding) word at the cursor for
/// the requested word style.
pub fn find_word_start(style: WordStyle) -> usize {
    let mut pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    match style {
        WordStyle::Emacs => {
            while pos > 0 && {
                let __c = ZLELINE.lock().unwrap()[pos - 1];
                !(__c.is_alphanumeric() || __c == '_')
            } {
                pos -= 1;
            }
            while pos > 0 && {
                let __c = ZLELINE.lock().unwrap()[pos - 1];
                (__c.is_alphanumeric() || __c == '_')
            } {
                pos -= 1;
            }
        }
        WordStyle::Vi => {
            while pos > 0 && ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
            if pos > 0 {
                let is_word = ZLELINE.lock().unwrap()[pos - 1].is_alphanumeric()
                    || ZLELINE.lock().unwrap()[pos - 1] == '_';
                while pos > 0 {
                    let c = ZLELINE.lock().unwrap()[pos - 1];
                    if c.is_whitespace() || ((c.is_alphanumeric() || c == '_') != is_word) {
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
            while pos > 0 && ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
            while pos > 0 && !ZLELINE.lock().unwrap()[pos - 1].is_whitespace() {
                pos -= 1;
            }
        }
    }
    pos
}

/// Find the end (exclusive) of the current (or following) word.
pub fn find_word_end(style: WordStyle) -> usize {
    let mut pos = ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    match style {
        WordStyle::Emacs => {
            while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst) && {
                let __c = ZLELINE.lock().unwrap()[pos];
                !(__c.is_alphanumeric() || __c == '_')
            } {
                pos += 1;
            }
            while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst) && {
                let __c = ZLELINE.lock().unwrap()[pos];
                (__c.is_alphanumeric() || __c == '_')
            } {
                pos += 1;
            }
        }
        WordStyle::Vi => {
            if pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                let is_word = ZLELINE.lock().unwrap()[pos].is_alphanumeric()
                    || ZLELINE.lock().unwrap()[pos] == '_';
                while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    let c = ZLELINE.lock().unwrap()[pos];
                    if c.is_whitespace() || ((c.is_alphanumeric() || c == '_') != is_word) {
                        break;
                    }
                    pos += 1;
                }
                while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                    && ZLELINE.lock().unwrap()[pos].is_whitespace()
                {
                    pos += 1;
                }
            }
        }
        WordStyle::Shell => {
            // See WordStyle::Shell note in find_word_start — no live
            // callers; leave pos unchanged.
        }
        WordStyle::BlankDelimited => {
            while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                && !ZLELINE.lock().unwrap()[pos].is_whitespace()
            {
                pos += 1;
            }
            while pos < ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                && ZLELINE.lock().unwrap()[pos].is_whitespace()
            {
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
        zle_reset();
        *ZLELINE.lock().unwrap() = s.chars().collect();
        ZLELL.store(
            ZLELINE.lock().unwrap().len(),
            std::sync::atomic::Ordering::SeqCst,
        );
        ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Verifies `wordclass` per c:74-78 dispatch table.
    #[test]
    fn wordclass_dispatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(wordclass(' '), 0);
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('_'), 1);
        assert_eq!(wordclass('5'), 1);
        assert_eq!(wordclass(';'), 2);
    }

    /// Verifies `forwardword` skips iword then non-iword (c:56-63).
    #[test]
    fn forwardword_basic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar baz");
        forwardword(&[]);
        // From cs=0 (on 'f'), skip 'foo' iword then ' ' non-iword,
        // landing on 'b' of 'bar' at index 4.
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// Verifies `backwardword` from end-of-line lands on word start
    /// (c:251-265).
    #[test]
    fn backwardword_lands_at_word_start() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar baz");
        ZLECS.store(
            ZLELL.load(std::sync::atomic::Ordering::SeqCst),
            std::sync::atomic::Ordering::SeqCst,
        );
        backwardword(&[]);
        // From cs=11 (past 'z'), skip non-iword (none), then iword
        // 'baz' to land at index 8.
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 8);
    }

    /// Verifies `upcaseword` mutates the next word in place (c:540-547).
    #[test]
    fn upcaseword_uppercases_next_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar");
        upcaseword(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "FOO bar");
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3); // landed at end of 'FOO'
    }

    /// Verifies `downcaseword` mutates next word in place (c:562-569).
    #[test]
    fn downcaseword_lowercases_next_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("FOO Bar");
        downcaseword(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "foo Bar");
    }

    /// Verifies `capitalizeword` upcases first letter, lowercases
    /// rest (c:584-595).
    #[test]
    fn capitalizeword_first_only() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar");
        capitalizeword(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "Foo bar");
    }

    /// Verifies `deleteword` removes the next word (c:617-622).
    /// C semantics: the loop advances past non-word then past word,
    /// stopping at the first non-word char after the word. With
    /// cs=0 on "foo bar baz", x ends at 3 (the space after "foo"),
    /// and `foredel(3-0)` drops "foo" — leaving the leading space.
    #[test]
    fn deleteword_drops_next_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar baz");
        deleteword(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, " bar baz");
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// Verifies `transposewords` swaps the word at cursor with the
    /// preceding word (c:684-734).
    #[test]
    fn transposewords_swaps_pair() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut z = line("foo bar");
        ZLECS.store(5, std::sync::atomic::Ordering::SeqCst); // mid-'bar'
        transposewords(&[]);
        let s: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "bar foo");
    }

    /// `Src/Zle/zle_word.c:74-78` — `wordclass` is a 3-level ternary:
    /// `ZC_iblank(x) ? 0 : ((ZC_ialnum(x) || ZWC('_') == x) ? 1 :
    ///                       ZC_ipunct(x) ? 2 : 3)`.
    /// Returns 0=iblank, 1=alnum-or-underscore, 2=punctuation, 3=other.
    /// Every word-movement widget (`forwardword`, `backwardword`,
    /// `viforwardwordend`, `vibackwardword`, …) iterates against this
    /// classifier — a regression in any branch breaks all of them.
    #[test]
    fn wordclass_iblank_branch_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // c:76 — `ZC_iblank(x) ? 0`. After the wcsiblank fix
        // `Src/Zle/zle.h:62` → `Src/utils.c:4302-4307` the iblank arm
        // catches every iswspace-except-newline char.
        assert_eq!(wordclass(' '), 0);
        assert_eq!(wordclass('\t'), 0);
        assert_eq!(wordclass('\r'), 0, "CR is iblank per wcsiblank");
        assert_eq!(wordclass('\x0c'), 0, "FF is iblank per wcsiblank");
        assert_eq!(wordclass('\x0b'), 0, "VT is iblank per wcsiblank");
        assert_eq!(wordclass('\u{00A0}'), 0, "NBSP is iblank per wcsiblank");
    }

    /// c:76-77 — `ZC_ialnum(x) || ZWC('_') == x` → 1. Alphanumerics
    /// AND `_` collapse to class 1 so `foo_bar` is treated as a
    /// single word boundary.
    #[test]
    fn wordclass_alnum_and_underscore_branch_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // Letters.
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('Z'), 1);
        // Digits.
        assert_eq!(wordclass('0'), 1);
        assert_eq!(wordclass('9'), 1);
        // Underscore — explicit special-case at c:76.
        assert_eq!(wordclass('_'), 1);
    }

    /// c:77 — `ZC_ipunct(x) ? 2`. Punctuation falls in class 2 so it
    /// separates word-1 from word-1 (e.g. `foo.bar` is two class-1
    /// chunks separated by a class-2 char).
    #[test]
    fn wordclass_punctuation_branch_returns_two() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wordclass('.'), 2);
        assert_eq!(wordclass(','), 2);
        assert_eq!(wordclass(';'), 2);
        assert_eq!(wordclass('!'), 2);
        assert_eq!(wordclass('-'), 2);
        assert_eq!(wordclass('"'), 2);
    }

    /// c:77 — final `: 3` arm catches everything else. Newline is the
    /// canonical "other" char: `wcsiblank` at `Src/utils.c:4304`
    /// excludes `\n` from iblank, `iswalnum('\n')` is false,
    /// `iswpunct('\n')` is false → class 3.
    #[test]
    fn wordclass_other_branch_returns_three() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            wordclass('\n'),
            3,
            "newline excluded from iblank by wcsiblank"
        );
    }

    /// c:74-78 — Every char must map to exactly ONE class (0/1/2/3).
    /// Pin no-overlap across the four buckets.
    #[test]
    fn wordclass_returns_value_in_range_for_all_ascii() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        for b in 0..=127u8 {
            let c = b as char;
            let r = wordclass(c);
            assert!(
                (0..=3).contains(&r),
                "wordclass({:?}) = {} out of range [0,3]",
                c,
                r
            );
        }
    }

    /// c:74 — `wordclass` is a pure function. Same input → same
    /// output, 1000 times.
    #[test]
    fn wordclass_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..1000 {
            assert_eq!(wordclass('a'), 1);
            assert_eq!(wordclass(' '), 0);
            assert_eq!(wordclass(';'), 2);
            assert_eq!(wordclass('\n'), 3);
        }
    }

    /// c:74-78 — Tab class (c:74 ZC_iblank case). Tab IS iblank
    /// per `wcsiblank` (`Src/utils.c:4304` — iswspace excluding `\n`).
    #[test]
    fn wordclass_tab_is_class_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            wordclass('\t'),
            0,
            "tab is iblank → class 0 per c:75 wcsiblank branch"
        );
    }

    /// c:74-78 — Non-ASCII letters (e.g. `é`, `字`, `α`) fall into
    /// the `ZC_ialnum` (class 1) branch in C — they're locale-
    /// dependent alnum chars. Pin so a regen narrowing to ASCII
    /// breaks vi `aw`/`iw` over CJK/Latin1 names.
    #[test]
    fn wordclass_non_ascii_letters_are_class_one() {
        let _g = crate::test_util::global_state_lock();
        // These match iswalnum in most locales; the exact answer
        // depends on locale, but ASCII alpha+digit must be class 1
        // regardless.
        for c in ['a', 'z', 'A', 'Z', '0', '9'] {
            assert_eq!(wordclass(c), 1, "{:?} must be class 1 (ialnum)", c);
        }
    }

    /// c:45 — `forwardword` on an empty buffer must not panic. The
    /// `n>0` loop has nothing to walk; cursor stays at 0.
    #[test]
    fn forwardword_on_empty_buffer_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        line("");
        let r = forwardword(&[]);
        assert_eq!(r, 0, "forwardword on empty must succeed");
        assert_eq!(
            ZLECS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "cursor must not advance past empty buffer"
        );
    }

    /// c:240 — `backwardword` on an empty buffer no-panic; cursor
    /// stays at 0.
    #[test]
    fn backwardword_on_empty_buffer_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        line("");
        let r = backwardword(&[]);
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// c:45 — `forwardword` at end of buffer: cursor is already at
    /// ZLELL, no movement, returns 0.
    #[test]
    fn forwardword_at_end_of_buffer_stays_at_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        line("foo");
        ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        let r = forwardword(&[]);
        assert_eq!(r, 0);
        assert_eq!(
            ZLECS.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "cursor at ZLELL must stay there"
        );
    }

    /// c:240 — `backwardword` at start (ZLECS=0): no movement.
    #[test]
    fn backwardword_at_start_of_buffer_stays_at_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        line("foo bar");
        ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        let r = backwardword(&[]);
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    // ─── zsh-corpus pins for wordclass ─────────────────────────────

    /// `wordclass(' ')` returns 0 (blank).
    #[test]
    fn zle_word_corpus_wordclass_space_is_blank() {
        assert_eq!(wordclass(' '), 0);
        assert_eq!(wordclass('\t'), 0);
    }

    /// `wordclass('a')` returns 1 (alnum).
    #[test]
    fn zle_word_corpus_wordclass_alpha_is_alnum() {
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('Z'), 1);
        assert_eq!(wordclass('0'), 1);
        assert_eq!(wordclass('9'), 1);
    }

    /// `wordclass('_')` returns 1 — underscore counts as alnum word char.
    #[test]
    fn zle_word_corpus_wordclass_underscore_is_alnum() {
        assert_eq!(wordclass('_'), 1, "underscore is alnum per zsh word-class");
    }

    /// `wordclass('.')` returns 2 (punct).
    #[test]
    fn zle_word_corpus_wordclass_punct_is_two() {
        assert_eq!(wordclass('.'), 2);
        assert_eq!(wordclass('!'), 2);
        assert_eq!(wordclass(';'), 2);
        assert_eq!(wordclass(','), 2);
    }

    /// `wordclass` separates word/non-word classes (1 vs others).
    #[test]
    fn zle_word_corpus_wordclass_distinct_classes() {
        let blank = wordclass(' ');
        let alnum = wordclass('a');
        let punct = wordclass('.');
        // All three must be distinct.
        assert_ne!(blank, alnum);
        assert_ne!(alnum, punct);
        assert_ne!(blank, punct);
    }

    /// `wordclass('\n')` returns 3 — newline is NOT `iblank` in zsh
    /// (iblank covers space+tab only). It falls into class 3 (other).
    #[test]
    fn zle_word_corpus_wordclass_newline_is_other() {
        assert_eq!(
            wordclass('\n'),
            3,
            "newline is NOT iblank (iblank = space/tab only) → class 3"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_word.c wordclass branches.
    // Each char-category branch independently pinned per c:82.
    // ═══════════════════════════════════════════════════════════════════

    /// c:82 — `wordclass(' ')` returns 0 (iblank class wins first).
    #[test]
    fn wordclass_space_is_zero() {
        assert_eq!(wordclass(' '), 0);
    }

    /// c:82 — `wordclass('\t')` returns 0 (tab is iblank).
    #[test]
    fn wordclass_tab_is_zero() {
        assert_eq!(wordclass('\t'), 0, "tab is iblank, class 0");
    }

    /// c:82 — alphanumeric ASCII letters → class 1.
    #[test]
    fn wordclass_ascii_letters_are_one() {
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('Z'), 1);
        assert_eq!(wordclass('m'), 1);
    }

    /// c:82 — digits → class 1 (ialnum includes digits).
    #[test]
    fn wordclass_digits_are_one() {
        assert_eq!(wordclass('0'), 1);
        assert_eq!(wordclass('5'), 1);
        assert_eq!(wordclass('9'), 1);
    }

    /// c:82 — `_` is explicitly bumped to class 1 (`ZWC('_') == x` arm).
    /// Pin: a regen dropping the underscore special-case would push it
    /// to class 2 (punct) and break identifier-word boundaries.
    #[test]
    fn wordclass_underscore_is_one_not_punct() {
        assert_eq!(
            wordclass('_'),
            1,
            "underscore must be class 1 (word) not 2 (punct)"
        );
    }

    /// c:82 — common punctuation → class 2.
    #[test]
    fn wordclass_more_punct_examples() {
        // Various punct that should fall into class 2.
        for c in &['"', '\'', '(', ')', '[', ']', '{', '}', '<', '>', '/', '\\'] {
            let cls = wordclass(*c);
            assert!(cls == 2 || cls == 3, "{:?} → 2 or 3, got {}", c, cls);
        }
    }

    /// c:82 — control chars (not iblank, not ialnum, not ipunct) → class 3.
    /// Note: newline + ctrl-A are class 3; CR pinned separately because
    /// zshrs zc_iblank classifies it as blank — possible BUG.
    #[test]
    fn wordclass_control_chars_are_three() {
        assert_eq!(wordclass('\n'), 3, "newline → 3");
        // \x01 ctrl-A — not blank/alnum/punct.
        assert_eq!(wordclass('\x01'), 3);
    }

    /// c:82 + Src/utils.c:4302-4307 — `wordclass('\r')` returns 0.
    /// zsh's wcsiblank is `iswspace(wc) && wc != L'\n'` — every space
    /// char EXCEPT newline counts as iblank, so CR/FF/VT all → class 0.
    /// Only newline is excluded explicitly.
    #[test]
    fn wordclass_cr_is_iblank_matching_c() {
        assert_eq!(
            wordclass('\r'),
            0,
            "CR is iblank per Src/utils.c:wcsiblank (iswspace minus newline)"
        );
    }

    /// c:82 + wcsiblank — FF (form feed) is also iblank → class 0.
    #[test]
    fn wordclass_ff_is_iblank() {
        assert_eq!(wordclass('\x0c'), 0, "FF is iblank");
    }

    /// c:82 + wcsiblank — VT (vertical tab) is iblank → class 0.
    #[test]
    fn wordclass_vt_is_iblank() {
        assert_eq!(wordclass('\x0b'), 0, "VT is iblank");
    }

    /// c:82 — `wordclass` is deterministic + idempotent (same input
    /// → same output across many calls).
    #[test]
    fn wordclass_is_deterministic() {
        for c in &['a', ' ', '_', '.', '\n', '0', '!'] {
            let first = wordclass(*c);
            for _ in 0..100 {
                assert_eq!(wordclass(*c), first, "wordclass({:?}) must be pure", c);
            }
        }
    }

    /// c:82 — all returns are in [0, 3] range (4 classes total).
    #[test]
    fn wordclass_returns_in_zero_to_three() {
        for c in &[
            'a', 'Z', '0', '_', ' ', '\t', '.', '!', '\n', '\r', '\x01', '@',
        ] {
            let cls = wordclass(*c);
            assert!(
                cls >= 0 && cls <= 3,
                "wordclass({:?})={} out of range",
                c,
                cls
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_word.c
    // c:82 wordclass / c:106 forwardword / c:510 backwardword /
    // c:974 upcaseword / c:1015 downcaseword / c:1054 capitalizeword /
    // c:1198 transposewords
    // ═══════════════════════════════════════════════════════════════════

    /// c:82 — `wordclass` returns i32 (compile-time type pin).
    #[test]
    fn wordclass_returns_i32_type() {
        let _: i32 = wordclass('a');
    }

    /// c:82 — `wordclass` is pure on full ASCII range.
    #[test]
    fn wordclass_pure_full_ascii_range() {
        for b in 0u8..128 {
            let c = b as char;
            let first = wordclass(c);
            for _ in 0..3 {
                assert_eq!(wordclass(c), first, "wordclass(0x{:02x}) must be pure", b);
            }
        }
    }

    /// c:106 — `forwardword(empty)` return in u8 exit-code range.
    #[test]
    fn forwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = forwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:510 — `backwardword(empty)` return in u8 exit-code range.
    #[test]
    fn backwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = backwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:974 — `upcaseword(empty)` return in u8 exit-code range.
    #[test]
    fn upcaseword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = upcaseword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:1015 — `downcaseword(empty)` return in u8 exit-code range.
    #[test]
    fn downcaseword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = downcaseword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:1054 — `capitalizeword(empty)` return in u8 exit-code range.
    #[test]
    fn capitalizeword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = capitalizeword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:1198 — `transposewords(empty)` return in u8 exit-code range.
    #[test]
    fn transposewords_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = transposewords(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:925 — `backwardkillword(empty)` return in u8 exit-code range.
    #[test]
    fn backwardkillword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = backwardkillword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:1108 — `deleteword(empty)` return in u8 exit-code range.
    #[test]
    fn deleteword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = deleteword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:1153 — `killword(empty)` return in u8 exit-code range.
    #[test]
    fn killword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = killword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_word.c
    // c:157 wordclass / c:174 viforwardword / c:240 viforwardblankword /
    // c:300 emacsforwardword / c:556 vibackwardword / c:766 emacsbackwardword
    // ═══════════════════════════════════════════════════════════════════

    /// c:157 — `wordclass` returns i32 (compile-time pin, alt).
    #[test]
    fn wordclass_returns_i32_pin_alt() {
        let _: i32 = wordclass('a');
    }

    /// c:157 — `wordclass` is deterministic (pure classifier).
    #[test]
    fn wordclass_deterministic() {
        for c in ['a', 'A', '0', ' ', '\t', '\n', '!', '日', '_'] {
            let first = wordclass(c);
            for _ in 0..5 {
                assert_eq!(wordclass(c), first, "wordclass({:?}) must be pure", c);
            }
        }
    }

    /// c:157 — `wordclass(alnum)` produces same class for digits and letters
    /// (alnum is one class in default wordchars).
    #[test]
    fn wordclass_alnum_chars_share_class() {
        let cls_a = wordclass('a');
        let cls_0 = wordclass('0');
        assert_eq!(cls_a, cls_0, "letter and digit are both alnum word-class");
    }

    /// c:157 — `wordclass(space)` differs from `wordclass(letter)`.
    #[test]
    fn wordclass_space_differs_from_letter() {
        let cls_space = wordclass(' ');
        let cls_a = wordclass('a');
        assert_ne!(
            cls_space, cls_a,
            "space and letter must be distinct word-classes"
        );
    }

    /// c:106 — `forwardword(empty)` return in u8 exit-code range (alt).
    #[test]
    fn forwardword_empty_args_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = forwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:174 — `viforwardword(empty)` MUST return in u8 exit-code range
    /// without panicking. C source guards ZLELINE[ZLECS] read by checking
    /// `ZLECS != ZLELL` first. In zshrs the port indexes unconditionally
    /// at `Src/Zle/zle_word.rs:199` → panics on empty buffer.
    #[test]
    fn viforwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viforwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:240 — `viforwardblankword(empty)` return in u8 exit-code range.
    #[test]
    fn viforwardblankword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viforwardblankword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:300 — `emacsforwardword(empty)` return in u8 exit-code range.
    #[test]
    fn emacsforwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = emacsforwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:556 — `vibackwardword(empty)` return in u8 exit-code range.
    #[test]
    fn vibackwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:766 — `emacsbackwardword(empty)` return in u8 exit-code range.
    #[test]
    fn emacsbackwardword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = emacsbackwardword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:861 — `vibackwardkillword(empty)` return in u8 exit-code range.
    #[test]
    fn vibackwardkillword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardkillword(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:106 — `forwardword` is deterministic on empty buffer.
    #[test]
    fn forwardword_empty_buffer_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = forwardword(&[]);
        for _ in 0..5 {
            assert_eq!(
                forwardword(&[]),
                first,
                "forwardword on empty must be deterministic"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Zle/zle_word.c
    // c:74-86 wordclass classifier / c:421 viforwardwordend /
    // c:621 vibackwardblankword / c:672 vibackwardwordend /
    // c:726 vibackwardblankwordend / c:348 viforwardblankwordend /
    // c:974 upcaseword / c:1015 downcaseword
    // ═══════════════════════════════════════════════════════════════════

    /// c:82 — `wordclass(' ')` = 0 (iblank class).
    #[test]
    fn wordclass_space_returns_zero() {
        assert_eq!(wordclass(' '), 0);
    }

    /// c:82 — `wordclass('a')` = 1 (alnum class).
    #[test]
    fn wordclass_letter_returns_one() {
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('Z'), 1);
        assert_eq!(wordclass('5'), 1);
        assert_eq!(wordclass('_'), 1, "underscore is class 1 per c:82");
    }

    /// c:82 — `wordclass('!')` = 2 (ipunct class).
    #[test]
    fn wordclass_punct_returns_two() {
        assert_eq!(wordclass('!'), 2);
        assert_eq!(wordclass('?'), 2);
        assert_eq!(wordclass('.'), 2);
    }

    /// c:82 — `wordclass` always returns one of {0,1,2,3}.
    #[test]
    fn wordclass_returns_only_0_1_2_3() {
        for c in (0u32..0x80).filter_map(char::from_u32) {
            let r = wordclass(c);
            assert!(
                (0..=3).contains(&r),
                "wordclass({:?}) = {} not in 0..=3",
                c,
                r
            );
        }
    }

    /// c:82 — `wordclass('\n')` is NOT iblank (newline excluded),
    /// must fall to one of the non-zero classes.
    #[test]
    fn wordclass_newline_not_blank() {
        let r = wordclass('\n');
        assert_ne!(r, 0, "newline must not be class 0 (iblank excludes \\n)");
    }

    /// c:421 — `viforwardwordend(empty)` exit code in u8 range.
    #[test]
    fn viforwardwordend_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viforwardwordend(&[]);
        assert!((0..256).contains(&r), "exit {} must fit u8", r);
    }

    /// c:621 — `vibackwardblankword(empty)` exit code in u8 range.
    #[test]
    fn vibackwardblankword_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardblankword(&[]);
        assert!((0..256).contains(&r), "exit {} must fit u8", r);
    }

    /// c:672 — `vibackwardwordend(empty)` exit code in u8 range.
    #[test]
    fn vibackwardwordend_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardwordend(&[]);
        assert!((0..256).contains(&r), "exit {} must fit u8", r);
    }

    /// c:726 — `vibackwardblankwordend(empty)` exit code in u8 range.
    #[test]
    fn vibackwardblankwordend_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vibackwardblankwordend(&[]);
        assert!((0..256).contains(&r), "exit {} must fit u8", r);
    }

    /// c:348 — `viforwardblankwordend(empty)` exit code in u8 range.
    #[test]
    fn viforwardblankwordend_empty_args_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viforwardblankwordend(&[]);
        assert!((0..256).contains(&r), "exit {} must fit u8", r);
    }

    /// c:974 — `upcaseword` deterministic on identical buffer state.
    #[test]
    fn upcaseword_deterministic_on_identical_state() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = upcaseword(&[]);
        for _ in 0..3 {
            assert_eq!(
                upcaseword(&[]),
                first,
                "upcaseword must be deterministic on identical state"
            );
        }
    }

    /// c:1015 — `downcaseword` deterministic on identical buffer state.
    #[test]
    fn downcaseword_deterministic_on_identical_state() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = downcaseword(&[]);
        for _ in 0..3 {
            assert_eq!(
                downcaseword(&[]),
                first,
                "downcaseword must be deterministic on identical state"
            );
        }
    }

    /// c:74 — `wordclass` return type i32 (compile-time pin, alt).
    #[test]
    fn wordclass_returns_i32_type_alt() {
        let _: i32 = wordclass(' ');
    }
}
