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

use super::zle_main::Zle; use super::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};

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
//   zmult    → `if zle.zmod.flags & MOD_MULT != 0
//                 { zle.zmod.mult } else { 1 }`
//   set zmult → `zle.zmod.mult = v;
//                zle.zmod.flags |= MOD_MULT;`
//   wordflag/virangeflag → `false` (vi-mode plumbing not yet wired).
//
// See textobjects.rs:32 for the same `zmod.mult` pattern.

// ZC_ char-class predicates — C macros expanded inline.
#[inline] fn zc_iword(c: char) -> bool { c.is_alphanumeric() || c == '_' }
#[inline] fn zc_ialnum(c: char) -> bool { c.is_alphanumeric() }
#[inline] fn zc_ialpha(c: char) -> bool { c.is_alphabetic() }
#[inline] fn zc_iblank(c: char) -> bool { c == ' ' || c == '\t' }
#[inline] fn zc_inblank(c: char) -> bool { c == ' ' || c == '\t' || c == '\n' }
#[inline] fn zc_ipunct(c: char) -> bool { c.is_ascii_punctuation() }

/// Port of `forwardword(args)` from `Src/Zle/zle_word.c:45`.
///
/// C signature: `int forwardword(char **args)`.
pub fn forwardword(zle: &mut Zle, args: &[String]) -> i32 {              // c:45
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };                                                  // c:47
    if n < 0 {                                                           // c:49
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;                                              // c:51
        let ret = backwardword(zle, args);                               // c:52
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;                                           // c:53
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:56
        n -= 1;
        while zle.zlecs != zle.zlell && zc_iword(zle.zleline[zle.zlecs]) {  // c:57
            zle.zlecs += 1;                                              // c:58 INCCS
        }
        if false && n == 0 {                                        // c:59
            return 0;                                                    // c:60
        }
        while zle.zlecs != zle.zlell && !zc_iword(zle.zleline[zle.zlecs]) {  // c:61
            zle.zlecs += 1;                                              // c:62 INCCS
        }
    }
    0                                                                    // c:64
}

/// Port of `wordclass(x)` from `Src/Zle/zle_word.c:74`. Returns the
/// vi-mode word class for a character: 0=blank, 1=alnum or `_`,
/// 2=punctuation, 3=other.
///
/// C signature: `int wordclass(ZLE_CHAR_T x)`.
pub fn wordclass(x: char) -> i32 {                                       // c:74
    // c:76 — `(ZC_iblank(x) ? 0 : ((ZC_ialnum(x) || ZWC('_') == x) ? 1 :
    //          ZC_ipunct(x) ? 2 : 3))`
    if zc_iblank(x) { 0 }
    else if zc_ialnum(x) || x == '_' { 1 }
    else if zc_ipunct(x) { 2 }
    else { 3 }
}

/// Port of `viforwardword(args)` from `Src/Zle/zle_word.c:82`.
pub fn viforwardword(zle: &mut Zle, args: &[String]) -> i32 {            // c:82
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:86
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = vibackwardword(zle, args);                             // c:89
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:93
        n -= 1;
        let cc = wordclass(zle.zleline[zle.zlecs]);                      // c:95
        while zle.zlecs != zle.zlell && wordclass(zle.zleline[zle.zlecs]) == cc {  // c:96
            zle.zlecs += 1;                                              // c:97 INCCS
        }
        if false && n == 0 { return 0; }                            // c:99
        let mut nl = if zle.zlecs < zle.zlell && zle.zleline[zle.zlecs] == '\n' { 1 } else { 0 };  // c:101
        while zle.zlecs != zle.zlell && nl < 2
              && zc_inblank(zle.zleline[zle.zlecs])                      // c:102
        {
            zle.zlecs += 1;                                              // c:103 INCCS
            if zle.zlecs < zle.zlell && zle.zleline[zle.zlecs] == '\n' { nl += 1; }  // c:104
        }
    }
    0                                                                    // c:107
}

/// Port of `viforwardblankword(args)` from `Src/Zle/zle_word.c:112`.
pub fn viforwardblankword(zle: &mut Zle, args: &[String]) -> i32 {       // c:112
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = vibackwardblankword(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while zle.zlecs != zle.zlell && !zc_inblank(zle.zleline[zle.zlecs]) {  // c:125
            zle.zlecs += 1;
        }
        if false && n == 0 { return 0; }                            // c:127
        let mut nl = if zle.zlecs < zle.zlell && zle.zleline[zle.zlecs] == '\n' { 1 } else { 0 };
        while zle.zlecs != zle.zlell && nl < 2
              && zc_inblank(zle.zleline[zle.zlecs])
        {
            zle.zlecs += 1;
            if zle.zlecs < zle.zlell && zle.zleline[zle.zlecs] == '\n' { nl += 1; }
        }
    }
    0
}

/// Port of `emacsforwardword(args)` from `Src/Zle/zle_word.c:140`.
pub fn emacsforwardword(zle: &mut Zle, args: &[String]) -> i32 {         // c:140
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:144
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = emacsbackwardword(zle, args);                          // c:147
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:151
        n -= 1;
        while zle.zlecs != zle.zlell && !zc_iword(zle.zleline[zle.zlecs]) {  // c:152
            zle.zlecs += 1;
        }
        if false && n == 0 { return 0; }                            // c:154
        while zle.zlecs != zle.zlell && zc_iword(zle.zleline[zle.zlecs]) {  // c:156
            zle.zlecs += 1;
        }
    }
    0
}

/// Port of `viforwardblankwordend(args)` from `Src/Zle/zle_word.c:164`.
pub fn viforwardblankwordend(zle: &mut Zle, args: &[String]) -> i32 {    // c:164
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = vibackwardblankwordend(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:176-182 — skip inblank chars; advance pos one ahead via INCPOS
        while zle.zlecs != zle.zlell {                                   // c:176
            let pos = zle.zlecs + 1;                                     // c:178 INCPOS
            if pos > zle.zlell || !zc_inblank(zle.zleline[pos.min(zle.zlell.saturating_sub(1))]) {
                break;
            }
            zle.zlecs = pos;                                             // c:181
        }
        // c:183-189 — advance over non-inblank chars.
        while zle.zlecs != zle.zlell {                                   // c:183
            let pos = zle.zlecs + 1;                                     // c:185 INCPOS
            if pos > zle.zlell || zc_inblank(zle.zleline[pos.min(zle.zlell.saturating_sub(1))]) {
                break;
            }
            zle.zlecs = pos;                                             // c:188
        }
    }
    if zle.zlecs != zle.zlell && false {                         // c:191
        zle.zlecs += 1;                                                  // c:192 INCCS
    }
    0
}

/// Port of `viforwardwordend(args)` from `Src/Zle/zle_word.c:198`.
pub fn viforwardwordend(zle: &mut Zle, args: &[String]) -> i32 {         // c:198
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = vibackwardwordend(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        // c:211-217 — advance past inblank chars looking ahead.
        while zle.zlecs != zle.zlell {                                   // c:211
            let pos = zle.zlecs + 1;                                     // c:213 INCPOS
            if pos > zle.zlell || !zc_inblank(zle.zleline[pos.min(zle.zlell.saturating_sub(1))]) {
                break;
            }
            zle.zlecs = pos;                                             // c:216
        }
        if zle.zlecs != zle.zlell {                                      // c:218
            let mut pos = zle.zlecs + 1;                                 // c:221 INCPOS
            let cc = if pos < zle.zlell { wordclass(zle.zleline[pos]) }
                     else { 0 };                                         // c:222
            loop {                                                       // c:223
                zle.zlecs = pos.min(zle.zlell);                          // c:224
                if zle.zlecs == zle.zlell { break; }                     // c:225-226
                pos += 1;                                                // c:227 INCPOS
                if pos > zle.zlell || wordclass(zle.zleline[pos.min(zle.zlell.saturating_sub(1))]) != cc {
                    break;                                               // c:228-229
                }
            }
        }
    }
    if zle.zlecs != zle.zlell && false {                         // c:233
        zle.zlecs += 1;                                                  // c:234 INCCS
    }
    0
}

/// Port of `backwardword(args)` from `Src/Zle/zle_word.c:240`.
pub fn backwardword(zle: &mut Zle, args: &[String]) -> i32 {             // c:240
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:244
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = forwardword(zle, args);                                // c:247
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:251
        n -= 1;
        while zle.zlecs > 0 {                                            // c:252
            let pos = zle.zlecs - 1;                                     // c:254 DECPOS
            if zc_iword(zle.zleline[pos]) { break; }                     // c:255
            zle.zlecs = pos;                                             // c:257
        }
        while zle.zlecs > 0 {                                            // c:259
            let pos = zle.zlecs - 1;                                     // c:261 DECPOS
            if !zc_iword(zle.zleline[pos]) { break; }                    // c:262
            zle.zlecs = pos;                                             // c:264
        }
    }
    0
}

/// Port of `vibackwardword(args)` from `Src/Zle/zle_word.c:272`.
pub fn vibackwardword(zle: &mut Zle, args: &[String]) -> i32 {           // c:272
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = viforwardword(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0;                                             // c:284
        while zle.zlecs > 0 {                                            // c:285
            zle.zlecs -= 1;                                              // c:286 DECCS
            if !zc_inblank(zle.zleline[zle.zlecs]) { break; }            // c:287
            if zle.zleline[zle.zlecs] == '\n' { nl += 1; }               // c:289
            if nl == 2 {                                                 // c:290
                zle.zlecs += 1;                                          // c:291 INCCS
                break;                                                   // c:292
            }
        }
        if zle.zlecs > 0 {                                               // c:295
            let mut pos = zle.zlecs;                                     // c:296
            let cc = wordclass(zle.zleline[pos]);                        // c:297
            loop {                                                       // c:298
                zle.zlecs = pos;                                         // c:299
                if zle.zlecs == 0 { break; }                             // c:300-301
                pos -= 1;                                                // c:302 DECPOS
                if wordclass(zle.zleline[pos]) != cc                     // c:303
                   || zc_inblank(zle.zleline[pos]) {
                    break;                                               // c:304
                }
            }
        }
    }
    0
}

/// Port of `vibackwardblankword(args)` from `Src/Zle/zle_word.c:313`.
pub fn vibackwardblankword(zle: &mut Zle, args: &[String]) -> i32 {      // c:313
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = viforwardblankword(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        let mut nl: i32 = 0;                                             // c:325
        while zle.zlecs > 0 {                                            // c:326
            let pos = zle.zlecs - 1;                                     // c:328 DECPOS
            if !zc_inblank(zle.zleline[pos]) { break; }                  // c:329
            if zle.zleline[pos] == '\n' { nl += 1; }                     // c:331
            if nl == 2 { break; }                                        // c:332
            zle.zlecs = pos;                                             // c:333
        }
        while zle.zlecs > 0 {                                            // c:335
            let pos = zle.zlecs - 1;                                     // c:337 DECPOS
            if zc_inblank(zle.zleline[pos]) { break; }                   // c:338
            zle.zlecs = pos;                                             // c:340
        }
    }
    0
}

/// Port of `vibackwardwordend(args)` from `Src/Zle/zle_word.c:348`.
pub fn vibackwardwordend(zle: &mut Zle, args: &[String]) -> i32 {        // c:348
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = viforwardwordend(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 && zle.zlecs > 1 {                                       // c:359
        n -= 1;
        let cc = wordclass(zle.zleline[zle.zlecs.min(zle.zlell.saturating_sub(1))]);  // c:360
        zle.zlecs -= 1;                                                  // c:361 DECCS
        while zle.zlecs > 0 {                                            // c:362
            if wordclass(zle.zleline[zle.zlecs]) != cc                   // c:363
               || zc_iblank(zle.zleline[zle.zlecs]) {
                break;
            }
            zle.zlecs -= 1;                                              // c:365 DECCS
        }
        while zle.zlecs > 0 && zc_iblank(zle.zleline[zle.zlecs]) {       // c:367
            zle.zlecs -= 1;                                              // c:368 DECCS
        }
    }
    0
}

/// Port of `vibackwardblankwordend(args)` from `Src/Zle/zle_word.c:375`.
pub fn vibackwardblankwordend(zle: &mut Zle, args: &[String]) -> i32 {   // c:375
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = viforwardblankwordend(zle, args);
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {
        n -= 1;
        while zle.zlecs > 0 && !zc_inblank(zle.zleline[zle.zlecs]) {     // c:387
            zle.zlecs -= 1;                                              // c:388 DECCS
        }
        while zle.zlecs > 0 && zc_inblank(zle.zleline[zle.zlecs]) {      // c:389
            zle.zlecs -= 1;                                              // c:390 DECCS
        }
    }
    0
}

/// Port of `emacsbackwardword(args)` from `Src/Zle/zle_word.c:397`.
pub fn emacsbackwardword(zle: &mut Zle, args: &[String]) -> i32 {        // c:397
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = emacsforwardword(zle, args);                           // c:404
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:408
        n -= 1;
        while zle.zlecs > 0 {                                            // c:409
            let pos = zle.zlecs - 1;                                     // c:411 DECPOS
            if zc_iword(zle.zleline[pos]) { break; }                     // c:412
            zle.zlecs = pos;                                             // c:414
        }
        while zle.zlecs > 0 {                                            // c:416
            let pos = zle.zlecs - 1;                                     // c:418 DECPOS
            if !zc_iword(zle.zleline[pos]) { break; }                    // c:419
            zle.zlecs = pos;                                             // c:421
        }
    }
    0
}

/// Port of `backwarddeleteword(args)` from `Src/Zle/zle_word.c:429`.
pub fn backwarddeleteword(zle: &mut Zle, args: &[String]) -> i32 {       // c:429
    let mut x = zle.zlecs;                                               // c:431
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:433
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = deleteword(zle, args);                                 // c:436
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:440
        n -= 1;
        while x > 0 {                                                    // c:441
            let pos = x - 1;                                             // c:443 DECPOS
            if zc_iword(zle.zleline[pos]) { break; }                     // c:444
            x = pos;
        }
        while x > 0 {                                                    // c:448
            let pos = x - 1;                                             // c:450 DECPOS
            if !zc_iword(zle.zleline[pos]) { break; }                    // c:451
            x = pos;
        }
    }
    let ct = (zle.zlecs - x) as i32;
    crate::ported::zle::zle_utils::backdel(zle, ct, /*CUT_RAW*/ 1);      // c:456
    0
}

/// Port of `vibackwardkillword(args)` from `Src/Zle/zle_word.c:462`.
// this taken from "vibackwardword"                                         // c:469
pub fn vibackwardkillword(zle: &mut Zle, _args: &[String]) -> i32 {      // c:462
    let mut x = zle.zlecs;                                               // c:464
    // c:464 — `lim = (viinsbegin > findbol()) ? viinsbegin : findbol();`
    // viinsbegin and findbol() not yet wired in zshrs; treat lim as 0
    // (the safe lower bound — equivalent to `findbol()` returning 0
    // when at/near start of single-line buffer). See TODO.md.
    let lim: usize = 0;
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 { return 1; }                                               // c:467
    let mut n = n;
    while n > 0 {                                                        // c:470
        n -= 1;
        while x > lim {                                                  // c:471
            let pos = x - 1;                                             // c:473 DECPOS
            if !zc_iblank(zle.zleline[pos]) { break; }                   // c:474
            x = pos;
        }
        if x > lim {                                                     // c:478
            let mut pos = x - 1;                                         // c:481 DECPOS
            let cc = wordclass(zle.zleline[pos]);                        // c:482
            loop {                                                       // c:483
                x = pos + 1;                                             // c:484 (after DECPOS reversal)
                let xv = pos;
                if xv <= lim {                                           // c:485-486
                    x = xv;
                    break;
                }
                if pos == 0 { x = 0; break; }
                pos -= 1;                                                // c:487 DECPOS
                if wordclass(zle.zleline[pos]) != cc {                   // c:488
                    x = pos + 1;
                    break;
                }
                x = pos;
            }
        }
    }
    let ct = (zle.zlecs - x) as i32;
    // c:493 — `backkill(zlecs - x, CUT_FRONT|CUT_RAW);`
    // CUT_FRONT = 0x02, CUT_RAW = 0x04 in zle.h.
    crate::ported::zle::zle_utils::backkill(zle, ct, 0x02 | 0x04);
    0
}

/// Port of `backwardkillword(args)` from `Src/Zle/zle_word.c:499`.
pub fn backwardkillword(zle: &mut Zle, args: &[String]) -> i32 {         // c:499
    let mut x = zle.zlecs;                                               // c:501
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:504
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = killword(zle, args);                                   // c:507
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:511
        n -= 1;
        while x > 0 {                                                    // c:512
            let pos = x - 1;                                             // c:514 DECPOS
            if zc_iword(zle.zleline[pos]) { break; }                     // c:515
            x = pos;
        }
        while x > 0 {                                                    // c:519
            let pos = x - 1;                                             // c:521 DECPOS
            if !zc_iword(zle.zleline[pos]) { break; }                    // c:522
            x = pos;
        }
    }
    let ct = (zle.zlecs - x) as i32;
    crate::ported::zle::zle_utils::backkill(zle, ct, 0x02 | 0x04);       // c:527
    0
}

/// Port of `upcaseword(args)` from `Src/Zle/zle_word.c:533`.
pub fn upcaseword(zle: &mut Zle, _args: &[String]) -> i32 {              // c:533
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    let neg = n < 0;                                                     // c:536
    let ocs = zle.zlecs;                                                 // c:536
    let mut n = if neg { -n } else { n };
    while n > 0 {                                                        // c:540
        n -= 1;
        while zle.zlecs != zle.zlell && !zc_iword(zle.zleline[zle.zlecs]) {  // c:541
            zle.zlecs += 1;                                              // c:542 INCCS
        }
        while zle.zlecs != zle.zlell && zc_iword(zle.zleline[zle.zlecs]) {  // c:543
            // c:544 — `zleline[zlecs] = ZC_toupper(zleline[zlecs]);`
            let c = zle.zleline[zle.zlecs];
            zle.zleline[zle.zlecs] = c.to_uppercase().next().unwrap_or(c);
            zle.zlecs += 1;                                              // c:545 INCCS
        }
    }
    if neg { zle.zlecs = ocs; }                                          // c:548-549
    0
}

/// Port of `downcaseword(args)` from `Src/Zle/zle_word.c:555`.
pub fn downcaseword(zle: &mut Zle, _args: &[String]) -> i32 {            // c:555
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = zle.zlecs;
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        while zle.zlecs != zle.zlell && !zc_iword(zle.zleline[zle.zlecs]) {  // c:563
            zle.zlecs += 1;
        }
        while zle.zlecs != zle.zlell && zc_iword(zle.zleline[zle.zlecs]) {   // c:565
            let c = zle.zleline[zle.zlecs];
            zle.zleline[zle.zlecs] = c.to_lowercase().next().unwrap_or(c);   // c:566 ZC_tolower
            zle.zlecs += 1;                                              // c:567 INCCS
        }
    }
    if neg { zle.zlecs = ocs; }
    0
}

/// Port of `capitalizeword(args)` from `Src/Zle/zle_word.c:577`.
pub fn capitalizeword(zle: &mut Zle, _args: &[String]) -> i32 {          // c:577
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = zle.zlecs;
    let mut n = if neg { -n } else { n };
    while n > 0 {
        n -= 1;
        let mut first = true;                                            // c:585
        while zle.zlecs != zle.zlell && !zc_iword(zle.zleline[zle.zlecs]) {  // c:586
            zle.zlecs += 1;
        }
        // c:588 — skip word-but-non-alpha chars (digits etc.) at start.
        while zle.zlecs != zle.zlell
              && zc_iword(zle.zleline[zle.zlecs])
              && !zc_ialpha(zle.zleline[zle.zlecs]) {
            zle.zlecs += 1;
        }
        while zle.zlecs != zle.zlell && zc_iword(zle.zleline[zle.zlecs]) {   // c:590
            let c = zle.zleline[zle.zlecs];
            zle.zleline[zle.zlecs] = if first {
                c.to_uppercase().next().unwrap_or(c)                     // c:591
            } else {
                c.to_lowercase().next().unwrap_or(c)                     // c:592
            };
            first = false;                                               // c:593
            zle.zlecs += 1;                                              // c:594 INCCS
        }
    }
    if neg { zle.zlecs = ocs; }
    0
}

/// Port of `deleteword(args)` from `Src/Zle/zle_word.c:604`.
pub fn deleteword(zle: &mut Zle, args: &[String]) -> i32 {               // c:604
    let mut x = zle.zlecs;
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:609
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = backwarddeleteword(zle, args);                         // c:612
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:616
        n -= 1;
        while x != zle.zlell && !zc_iword(zle.zleline[x]) {              // c:617
            x += 1;                                                      // c:618 INCPOS
        }
        while x != zle.zlell && zc_iword(zle.zleline[x]) {               // c:619
            x += 1;                                                      // c:620 INCPOS
        }
    }
    let ct = (x - zle.zlecs) as i32;
    crate::ported::zle::zle_utils::foredel(zle, ct, /*CUT_RAW*/ 1);      // c:622
    0
}

/// Port of `killword(args)` from `Src/Zle/zle_word.c:628`.
pub fn killword(zle: &mut Zle, args: &[String]) -> i32 {                 // c:628
    let mut x = zle.zlecs;
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    if n < 0 {                                                           // c:633
        let saved = n;
        zle.zmod.mult = -n; zle.zmod.flags |= MOD_MULT;
        let ret = backwardkillword(zle, args);                           // c:636
        zle.zmod.mult = saved; zle.zmod.flags |= MOD_MULT;
        return ret;
    }
    let mut n = n;
    while n > 0 {                                                        // c:640
        n -= 1;
        while x != zle.zlell && !zc_iword(zle.zleline[x]) {              // c:641
            x += 1;                                                      // c:642 INCPOS
        }
        while x != zle.zlell && zc_iword(zle.zleline[x]) {               // c:643
            x += 1;                                                      // c:644 INCPOS
        }
    }
    let ct = (x - zle.zlecs) as i32;
    crate::ported::zle::zle_utils::forekill(zle, ct, /*CUT_RAW*/ 1);     // c:646
    0
}

/// Port of `transposewords(args)` from `Src/Zle/zle_word.c:652`.
pub fn transposewords(zle: &mut Zle, _args: &[String]) -> i32 {          // c:652
    let n = if zle.zmod.flags & MOD_MULT != 0 { zle.zmod.mult } else { 1 };
    let neg = n < 0;
    let ocs = zle.zlecs;
    let mut n = if neg { -n } else { n };
    let mut x = zle.zlecs;

    // c:662-663 — advance x to next word start (skip non-iword unless newline).
    while x != zle.zlell && zle.zleline[x] != '\n' && !zc_iword(zle.zleline[x]) {
        x += 1;                                                          // INCPOS
    }
    // c:665-682 — if at end-or-newline, search backward for word-start.
    if x == zle.zlell || zle.zleline[x] == '\n' {                        // c:665
        x = zle.zlecs;                                                   // c:666
        while x > 0 {                                                    // c:667
            if zc_iword(zle.zleline[x]) { break; }                       // c:668
            let pos = x - 1;
            if zle.zleline[pos] == '\n' { break; }                       // c:672
            x = pos;
        }
        if x == 0 { return 1; }                                          // c:676
        let pos = x - 1;
        if zle.zleline[pos] == '\n' { return 1; }                        // c:680
    }

    // c:684-685 — find p4: end of current word.
    let mut p4 = x;
    while p4 != zle.zlell && zc_iword(zle.zleline[p4]) {                 // c:684
        p4 += 1;                                                         // INCPOS
    }
    // c:687-693 — find p3: start of current word.
    let mut p3 = p4;
    while p3 > 0 {                                                       // c:687
        let pos = p3 - 1;
        if !zc_iword(zle.zleline[pos]) { break; }                        // c:690
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
            if zc_iword(zle.zleline[pos]) { break; }                     // c:704
            p2 = pos;
        }
        if p2 == 0 { return 1; }                                         // c:708
        // p1 = p2, walk back over iword chars.
        p1 = p2;
        while p1 > 0 {                                                   // c:710
            let pos = p1 - 1;
            if !zc_iword(zle.zleline[pos]) { break; }                    // c:713
            p1 = pos;
        }
        pt = p1;                                                         // c:717
    }

    // c:720-729 — build temp = [word3 segment | gap2-3 | word1 segment]
    // and write back into zleline[p1..p4].
    let mut temp: Vec<char> = Vec::with_capacity(p4 - p1);
    temp.extend_from_slice(&zle.zleline[p3..p4]);                        // c:721-722
    temp.extend_from_slice(&zle.zleline[p2..p3]);                        // c:723-724
    temp.extend_from_slice(&zle.zleline[p1..p2]);                        // c:726-727
    for (i, c) in temp.iter().enumerate() {                              // c:729 ZS_memcpy
        zle.zleline[p1 + i] = *c;
    }

    if neg { zle.zlecs = ocs; }                                          // c:731-732
    else { zle.zlecs = p4; }                                             // c:734
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::zle_main::Zle;

    fn line(s: &str) -> Zle {
        let mut z = Zle::new();
        z.zleline = s.chars().collect();
        z.zlell = z.zleline.len();
        z.zlecs = 0;
        z
    }

    /// Verifies `wordclass` per c:74-78 dispatch table.
    #[test]
    fn wordclass_dispatch() {
        assert_eq!(wordclass(' '), 0);
        assert_eq!(wordclass('a'), 1);
        assert_eq!(wordclass('_'), 1);
        assert_eq!(wordclass('5'), 1);
        assert_eq!(wordclass(';'), 2);
    }

    /// Verifies `forwardword` skips iword then non-iword (c:56-63).
    #[test]
    fn forwardword_basic() {
        let mut z = line("foo bar baz");
        forwardword(&mut z, &[]);
        // From cs=0 (on 'f'), skip 'foo' iword then ' ' non-iword,
        // landing on 'b' of 'bar' at index 4.
        assert_eq!(z.zlecs, 4);
    }

    /// Verifies `backwardword` from end-of-line lands on word start
    /// (c:251-265).
    #[test]
    fn backwardword_lands_at_word_start() {
        let mut z = line("foo bar baz");
        z.zlecs = z.zlell;
        backwardword(&mut z, &[]);
        // From cs=11 (past 'z'), skip non-iword (none), then iword
        // 'baz' to land at index 8.
        assert_eq!(z.zlecs, 8);
    }

    /// Verifies `upcaseword` mutates the next word in place (c:540-547).
    #[test]
    fn upcaseword_uppercases_next_word() {
        let mut z = line("foo bar");
        upcaseword(&mut z, &[]);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "FOO bar");
        assert_eq!(z.zlecs, 3); // landed at end of 'FOO'
    }

    /// Verifies `downcaseword` mutates next word in place (c:562-569).
    #[test]
    fn downcaseword_lowercases_next_word() {
        let mut z = line("FOO BAR");
        downcaseword(&mut z, &[]);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "foo BAR");
    }

    /// Verifies `capitalizeword` upcases first letter, lowercases
    /// rest (c:584-595).
    #[test]
    fn capitalizeword_first_only() {
        let mut z = line("foo bar");
        capitalizeword(&mut z, &[]);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "Foo bar");
    }

    /// Verifies `deleteword` removes the next word (c:617-622).
    /// C semantics: the loop advances past non-word then past word,
    /// stopping at the first non-word char after the word. With
    /// cs=0 on "foo bar baz", x ends at 3 (the space after "foo"),
    /// and `foredel(3-0)` drops "foo" — leaving the leading space.
    #[test]
    fn deleteword_drops_next_word() {
        let mut z = line("foo bar baz");
        deleteword(&mut z, &[]);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, " bar baz");
        assert_eq!(z.zlecs, 0);
    }

    /// Verifies `transposewords` swaps the word at cursor with the
    /// preceding word (c:684-734).
    #[test]
    fn transposewords_swaps_pair() {
        let mut z = line("foo bar");
        z.zlecs = 5;  // mid-'bar'
        transposewords(&mut z, &[]);
        let s: String = z.zleline.iter().collect();
        assert_eq!(s, "bar foo");
    }
}
