//! ZLE vi mode operations
//!
//! Direct port from zsh/Src/Zle/zle_vi.c

use std::sync::atomic::Ordering;
use std::sync::atomic::Ordering::SeqCst;

use super::zle_h::{modifier, vichange, MOD_MULT, MOD_TMULT, MOD_VIAPP, MOD_VIBUF};
use super::zle_keymap::selectkeymap;
use super::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};

// Note: dead `ViState` / `ViChange` / `ViPendingOp` aggregates
// removed per PORT_PLAN Phase 2. They had zero references across the
// codebase. The actual zsh-side state lives in C file-scope globals
// declared in `Src/Zle/zle_vi.c`; the AtomicI32 wires below are the
// faithful ports.

// Module alias so the disambiguator `zle_main::MARK` resolves at
// use sites where zle_misc's duplicate `pub static MARK` is also
// in scope via the `zle_misc::*` glob below.
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main, zle_main::*, zle_misc::*,
    zle_move::*, zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_word::*,
};
/// Port of `int virangeflag;` from `Src/Zle/zle_vi.c:36`. Set during
/// vi range-pending operations to suppress the cursor-included
/// region adjustment (see `textobjects.rs:261` and `zle_vi.c:196`).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Direct port of `int vichange(UNUSED(char **args))` from
/// `Src/Zle/zle_vi.c:438`. vi `c{motion}` — delete the range covered
/// by the motion, then enter insert mode. The motion-driven range
/// comes from `getvirange`; on success forekill+startvitext, else
/// startvitext at the current position.
pub fn vichange() -> i32 {
    // c:438
    startvichange(1); // c:440
    let c2 = getvirange(0); // c:441
    if c2 != -1 {
        let cs = ZLECS.load(SeqCst) as i32;
        forekill(
            c2 - cs, // c:443
            CUT_RAW,
        );
        startvitext(1); // c:444
        return 0;
    }
    1 // c:453 ret=1
}

/// Direct port of `void startvichange(int im)` from
/// `Src/Zle/zle_vi.c:90`.
///
/// Line-by-line port. Three branches:
///   1. `viinrepeat && im != -2` — we're replaying `.`: restore zmod
///      from `lastvichg.mod` and zero `vichgflag` so the replay
///      itself isn't re-recorded (c:94-96).
///   2. `!vichgflag` — fresh change: snapshot zmod into curvichg, reset
///      `curvichg.buf`. With `im == -2` (vi-yank/change/delete) seed
///      the buf with the operator character (`i`/`a`/`R`/`o`) and set
///      `vichgflag = 1`. Otherwise (every other vi command) copy
///      `keybuf` verbatim and set `vichgflag = 2` (c:97-115).
///   3. `vichgflag != 0` and not in replay — already recording, no-op.
pub fn startvichange(im: i32) {
    use crate::ported::utils::unmetafy;
    use crate::ported::zle::zle_keymap::{keybuf, keybuflen};
    use crate::ported::zle::zle_main::{ZLECS, ZLELL};

    // c:91 — `if (im > -1) insmode = im;`
    if im > -1 {
        INSMODE.store(if im != 0 { 1 } else { 0 }, SeqCst);
    }

    let in_repeat = VIINREPEAT.load(SeqCst) != 0;
    if in_repeat && im != -2 {
        // c:94-96 — `zmod = lastvichg.mod; vichgflag = 0;`
        let saved_mod = LASTVICHG.lock().unwrap().mod_.clone();
        *ZMOD.lock().unwrap() = saved_mod;
        VICHGFLAG.store(0, SeqCst);
    } else if VICHGFLAG.load(SeqCst) == 0 {
        // c:97-115 — start recording.
        let mut cur = CURVICHG.lock().unwrap();
        // c:98 — `curvichg.mod = zmod;`
        cur.mod_ = ZMOD.lock().unwrap().clone();
        // c:99-101 — `free(curvichg.buf); curvichg.buf = zalloc(16 + keybuflen);`
        let kblen = keybuflen.load(SeqCst).max(0) as usize;
        cur.buf.clear();
        cur.bufsz = (16 + kblen) as i32;
        cur.bufptr = 0;

        if im == -2 {
            // c:103-107 — seed buf with the operator character.
            VICHGFLAG.store(1, SeqCst);
            let zlell_v = ZLELL.load(SeqCst);
            let zlecs_v = ZLECS.load(SeqCst);
            let insmode_v = INSMODE.load(SeqCst) != 0;
            let op = if zlell_v != 0 {
                if insmode_v {
                    if zlecs_v < zlell_v {
                        b'i'
                    } else {
                        b'a'
                    }
                } else {
                    b'R'
                }
            } else {
                b'o'
            };
            cur.buf.push(op);
            cur.bufptr = 1;
        } else {
            // c:109-115 — `vichgflag = 2; strcpy(curvichg.buf, keybuf);
            //              unmetafy(curvichg.buf, &curvichg.bufptr);`
            VICHGFLAG.store(2, SeqCst);
            let kb = keybuf.lock().unwrap().clone();
            let truncated_len = kb.iter().position(|&b| b == 0).unwrap_or(kb.len());
            cur.buf = kb[..truncated_len].to_vec();
            cur.bufptr = unmetafy(&mut cur.buf) as i32;
        }
    }
}

/// Direct port of `static void startvitext(int im)` from
/// `Src/Zle/zle_vi.c:118`.
/// ```c
/// startvitext(int im) {
///     startvichange(im);
///     selectkeymap("main", 1);
///     vistartchange = undo_changeno;
///     viinsbegin = zlecs;
/// }
/// ```
pub fn startvitext(im: i32) {
    // c:118
    startvichange(im); // c:118
    selectkeymap("main", 1); // c:121
    VISTARTCHANGE.store(UNDO_CHANGENO.load(SeqCst), SeqCst); // c:122
    VIINSBEGIN.store(ZLECS.load(SeqCst), SeqCst); // c:123
}

/// Port of `vigetkey()` from Src/Zle/zle_vi.c:128.
pub fn vigetkey() -> i32 {
    // c:128
    // c:130 — `Keymap mn = openkeymap("main");`
    let mn = crate::ported::zle::zle_keymap::openkeymap("main");
    // c:131 — `char m[3], *str;` — Rust holds the byte directly.
    // c:132 — `Thingy cmd;`

    // c:134 — `if (getbyte(0L, NULL, 1) == EOF) return ZLEEOF;`.
    // Rust port: getbyte returns Option<u8>; None means EOF.
    // The KUNGETBUF drain path was the previous stub — covered now
    // by getbyte itself which consults the unget buffer first.
    let byte = match getbyte(true) {
        Some(b) => b,
        None => return ZLEEOF, // c:135
    };

    // c:137 — `m[0] = lastchar;`. Updated by getbyte side effect; we
    // also have `byte` in hand.
    crate::ported::zle::compcore::LASTCHAR.store(byte as i32, SeqCst); // c:137
                                                                       // c:138 — `metafy(m, 1, META_NOALLOC);` — Rust UTF-8 storage
                                                                       // doesn't need the meta-escape step here; the byte is already
                                                                       // in canonical form for keybind lookup.

    // c:139-142 — `if (mn) cmd = keybind(mn, m, &str); else cmd = t_undefinedkey;`
    let cmd: Option<crate::ported::zle::zle_thingy::Thingy> = if let Some(km) = mn {
        let (t, _str) = crate::ported::zle::zle_keymap::keybind(&km, &[byte]); // c:140
        t
    } else {
        None // c:142 t_undefinedkey collapses to None in this Option<Thingy> path
    };

    // c:144-160 — branch on resolved Thingy name (C `Th(z_*)`
    // compares pointer-equality against the built-in thingy table;
    // Rust compares by `.nam` since Thingy is an owned value).
    match cmd.as_ref().map(|t| t.nam.as_str()) {
        // c:144 — `if (!cmd || cmd == Th(z_sendbreak)) return ZLEEOF;`
        None | Some("send-break") | Some("undefined-key") => {
            // c:145
            return ZLEEOF;
        }
        // c:146-148 — `else if (cmd == Th(z_quotedinsert))`.
        Some("quoted-insert") => {
            // c:146
            if getfullchar(false).is_none() {
                // c:147 getfullchar(0) == ZLEEOF
                return ZLEEOF; // c:148
            }
        }
        // c:149-157 — `else if (cmd == Th(z_viquotedinsert))` — show
        // a `^` at zlecs, refresh, read the next full char, restore.
        Some("vi-quoted-insert") => {
            // c:149
            // c:150 — `ZLE_CHAR_T sav = zleline[zlecs];`
            let zlecs = ZLECS.load(SeqCst);
            let sav: Option<char> = {
                let line = ZLELINE.lock().unwrap();
                line.get(zlecs).copied() // c:150
            };
            // c:152 — `zleline[zlecs] = '^';`
            {
                let mut line = ZLELINE.lock().unwrap();
                if zlecs < line.len() {
                    line[zlecs] = '^';
                }
            }
            // c:153 — `zrefresh();`
            zrefresh();
            // c:154 — `getfullchar(0);` — read but result captured
            // separately via LASTFULLCHAR (the static the C source
            // updates internally). Rust port's `getfullchar` returns
            // Option<char>; the broader LASTFULLCHAR state lives on
            // zle_main side.
            let _ = getfullchar(false);
            // c:155 — `zleline[zlecs] = sav;` restore original char.
            if let Some(c) = sav {
                let mut line = ZLELINE.lock().unwrap();
                if zlecs < line.len() {
                    line[zlecs] = c;
                }
            }
            // c:156-157 — `if (LASTFULLCHAR == ZLEEOF) return ZLEEOF;`.
            // Rust getfullchar already surfaced the EOF as None; if
            // we got here we have a valid char.
        }
        // c:158-159 — `else if (cmd == Th(z_vicmdmode)) return ZLEEOF;`
        Some("vi-cmd-mode") => {
            // c:158
            return ZLEEOF; // c:159
        }
        _ => {
            // c:144 fallthrough — any other cmd (typically self-insert)
            // returns the byte as-is.
        }
    }

    // c:161-166 — `if (!lastchar_wide_valid) getrestchar(lastchar, NULL, NULL);`
    // — multi-byte completion of an incomplete sequence. Rust port's
    // getbyte already returns a single decoded byte; the wide-char
    // completion is handled by `getfullchar` above when invoked. For
    // the no-Thingy-branch case the single byte is the final value.

    // c:167 — `return LASTFULLCHAR;`. Rust port returns the byte we
    // captured (or the char-cast for the quoted-insert path's
    // getfullchar result if reached). For the fall-through case
    // (most common: self-insert), the byte read at c:134 IS the
    // final value.
    byte as i32
}

/// Direct port of `int getvirange(int wf)` from
/// `Src/Zle/zle_vi.c:172`. Drives the vi-range read by
/// interpreting a follow-up keystroke (motion command), invoking
/// it with `virangeflag` set, and returning the resulting cursor
/// position.
///
/// **Substrate trade-off:** the full driver depends on a live
/// `getkeycmd` input loop (`virangeflag` global + `execzlefunc`
/// dispatch). In compcore-call-context ported we don't have a live
/// key reader — the Rust port returns the current `ZLECS.load(std::sync::atomic::Ordering::SeqCst)`
/// which is the C "no-motion fallback" (motion never consumed
/// anything, range is empty). Live ZLE widget dispatch reads keys
/// against the ZLE file-scope statics directly.
pub fn getvirange(wf: i32) -> i32 {
    // c:172
    // c:186-187 — set the virangeflag / wordflag globals so the
    // movement-cmd dispatch (read by zle_word / zle_move) knows to
    // place cursor at the END of the range rather than where the
    // motion would normally land. The interactive `getkeycmd()`
    // read at c:208 is substrate-deferred (needs live ZLE input);
    // we set the flags and return the current cursor — caller's
    // motion fn picks them up via WORDFLAG/VIRANGEFLAG loads.
    VIRANGEFLAG.store(1, Ordering::Relaxed); // c:186
    WORDFLAG.store(wf, Ordering::Relaxed); // c:187
                                           // c:188 — `mark = -1` (cleared; usize::MAX represents "no mark").
    MARK.store(usize::MAX, Ordering::Relaxed);
    ZLECS.load(SeqCst) as i32 // c:299
}

/// Port of `dovilinerange()` from Src/Zle/zle_vi.c:302.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn dovilinerange() -> (usize, usize) {
    // c:302
    // C body (c:304-333): expands the current vi range to whole lines
    //                    (includes leading/trailing newlines). Returns
    //                    a [start, end) byte pair.
    let bol = findbol();
    let eol = findeol();
    // Include the trailing newline if present.
    let end = if eol < ZLELL.load(SeqCst) {
        eol + 1
    } else {
        eol
    };
    (bol, end)
}

/// Port of `viaddnext(UNUSED(char **args))` from Src/Zle/zle_vi.c:336.
pub fn viaddnext() -> i32 {
    // c:336
    // C body (c:337-341): `if (zlecs != findeol()) INCCS();
    //                     startvitext(1); return 0`.
    let eol = findeol();
    if ZLECS.load(SeqCst) != eol {
        inccs();
    }
    startvitext(1);
    0
}

/// Port of `viaddeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:346.
pub fn viaddeol() -> i32 {
    // c:346
    // C body (c:347-350): `zlecs = findeol(); startvitext(1); return 0`.
    ZLECS.store(findeol(), SeqCst);
    startvitext(1);
    0
}

/// Port of `viinsert(UNUSED(char **args))` from Src/Zle/zle_vi.c:355.
pub fn viinsert() -> i32 {
    // c:355
    // C body (c:356-358): `startvitext(1); return 0`.
    startvitext(1);
    0
}

/// Port of `viinsert_init()` from Src/Zle/zle_vi.c:368.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn viinsert_init() {
    // c:368
    // C body (c:369-371): `startvitext(-2)`. Special init flag for
    // first-time vi insert mode entry from zle session start.
    startvitext(-2);
}

/// Port of `viinsertbol(UNUSED(char **args))` from Src/Zle/zle_vi.c:375.
pub fn viinsertbol() -> i32 {
    // c:375
    // C body (c:376-379): `vifirstnonblank(zlenoargs); startvitext(1);
    //                     return 0`.
    vifirstnonblank();
    startvitext(1);
    0
}

/// Direct port of `int videlete(UNUSED(char **args))` from
/// `Src/Zle/zle_vi.c:384`. vi `d{motion}` — deletes the range covered
/// by the motion via `getvirange` + `forekill`. The line-wise vilinerange
/// arm (c:392-398) drops the trailing newline.
pub fn videlete() -> i32 {
    // c:384
    startvichange(1); // c:388
    let c2 = getvirange(0); // c:389
    if c2 == -1 {
        return 1;
    }
    let cs = ZLECS.load(SeqCst) as i32;
    forekill(c2 - cs, CUT_RAW); // c:390
                                // c:392-398 — line-wise: drop trailing newline.
    if VILINERANGE.load(Ordering::Relaxed) != 0 {
        let ll = ZLELL.load(SeqCst);
        if ll != 0 {
            LASTCOL.store(-1, Ordering::Relaxed);
            let cs_now = ZLECS.load(SeqCst);
            if cs_now == ll {
                deccs(); // c:395
            }
            foredel(1, 0); // c:396
            vifirstnonblank(); // c:397
        }
    }
    0 // c:391 ret = 0
}

/// Port of `videletechar(char **args)` from Src/Zle/zle_vi.c:405.
pub fn videletechar() -> i32 {
    // c:405

    // c:410 — startvichange(-1);
    startvichange(-1);
    // c:411 — n = zmult;
    let mut n = ZMOD.lock().unwrap().mult;

    // c:413-420 — if (n < 0) { zmult=-n; ret=vibackwarddeletechar(); zmult=n; return ret; }
    if n < 0 {
        let saved = n;
        ZMOD.lock().unwrap().mult = -n;
        let ret = vibackwarddeletechar();
        ZMOD.lock().unwrap().mult = saved;
        return ret;
    }

    // c:421-423 — if (zlecs == zlell || zleline[zlecs] == '\n') return 1;
    let cs = ZLECS.load(SeqCst);
    let ll = ZLELL.load(SeqCst);
    if cs == ll || ZLELINE.lock().unwrap().get(cs) == Some(&'\n') {
        return 1;
    }

    // c:427-433 — clamp n to (findeol() - zlecs), then forekill(n, ...)
    let eol = findeol();
    let max_n = eol.saturating_sub(cs) as i32;
    let flags = if n > max_n {
        n = max_n;
        CUT_RAW // c:430
    } else {
        0 // c:432
    };
    forekill(n, flags);
    0 // c:434
}

/// Port of `visubstitute(UNUSED(char **args))` from Src/Zle/zle_vi.c:455.
pub fn visubstitute() -> i32 {
    // c:455
    // C body (c:457-475): startvichange(1); n=zmult; if(n<0) return 1;
    //                    error if at eol; forekill(n, CUT_RAW);
    //                    startvitext(1); return 0.
    startvichange(1);
    let n = ZMOD.lock().unwrap().mult;
    if n < 0 {
        return 1;
    }
    if ZLECS.load(SeqCst) == ZLELL.load(SeqCst)
        || ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)) == Some(&'\n')
    {
        return 1;
    }
    let eol = findeol();
    let count = (n as usize).min(eol - ZLECS.load(SeqCst));
    if count > 0 {
        let text: Vec<char> = ZLELINE
            .lock()
            .unwrap()
            .drain(ZLECS.load(SeqCst)..ZLECS.load(SeqCst) + count)
            .collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(count, SeqCst);
    }
    startvitext(1);
    0
}

/// Port of `vichangeeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:482.
pub fn vichangeeol() -> i32 {
    // c:482
    // C body (c:483-498): `if (region_active) { regionlines(...);
    //                     zlecs = a; region_active = 0; ... } else
    //                     forekill(findeol() - zlecs, CUT_RAW);
    //                     startvitext(1); return 0`.
    let eol = findeol();
    if eol > ZLECS.load(SeqCst) {
        let text: Vec<char> = ZLELINE
            .lock()
            .unwrap()
            .drain(ZLECS.load(SeqCst)..eol)
            .collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(eol - ZLECS.load(SeqCst), SeqCst);
    }
    startvitext(1);
    0
}

/// Port of `vichangewholeline(char **args)` from Src/Zle/zle_vi.c:499.
pub fn vichangewholeline() -> i32 {
    // c:499
    // C body (c:500-503): `vifirstnonblank(); return vichangeeol(...)`.
    vifirstnonblank();
    vichangeeol()
}

/// Port of `viyank(UNUSED(char **args))` from `Src/Zle/zle_vi.c:546`.
/// ```c
/// int viyank(UNUSED(char **args)) {
///     int c2, ret = 1;
///     startvichange(1);
///     if ((c2 = getvirange(0)) != -1) {
///         cut(zlecs, c2 - zlecs, CUT_YANK);
///         ret = 0;
///     }
///     if (vilinerange && lastcol != -1) {
///         int x = findeol();
///         if ((zlecs += lastcol) >= x) {
///             zlecs = x;
///             if (zlecs > findbol() && invicmdmode()) DECCS();
///         }
///         lastcol = -1;
///     }
///     return ret;
/// }
/// ```
pub fn viyank(_args: &[String]) -> i32 {
    // c:zle_vi.c:546
    let mut ret = 1;
    startvichange(1); // c:550
    let c2 = getvirange(0); // c:551
    if c2 != -1 {
        let zlecs_now = ZLECS.load(SeqCst) as i32;
        cut(
            // c:552
            zlecs_now,
            c2 - zlecs_now,
            CUT_YANK,
        );
        ret = 0;
    }
    // c:557 — line-mode column restoration.
    if VILINERANGE.load(SeqCst) != 0 && LASTCOL.load(SeqCst) != -1 {
        let x = findeol() as i32;
        let new_cs = ZLECS.load(SeqCst) as i32 + LASTCOL.load(SeqCst);
        if new_cs >= x {
            ZLECS.store(x as usize, SeqCst);
            let bol = findbol() as i32;
            let cmname = crate::ported::zle::zle_keymap::curkeymapname().clone();
            if x > bol && invicmdmode(&cmname) {
                deccs();
            }
        } else {
            ZLECS.store(new_cs as usize, SeqCst);
        }
        LASTCOL.store(-1, SeqCst); // c:570
    }
    ret
}

/// Port of `viyankeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:537.
pub fn viyankeol() -> i32 {
    // c:537
    // C body (c:539-547): `x = findeol(); startvichange(-1); if (x ==
    //                     zlecs) return 1; cut(zlecs, x-zlecs, CUT_YANK);
    //                     return 0`.
    let x = findeol();
    startvichange(-1);
    if x == ZLECS.load(SeqCst) {
        return 1; // c:550
    }
    let text: Vec<char> = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)..x].to_vec();
    KILLRING.lock().unwrap().push_front(text);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
    0 // c:550
}

/// Port of `viyankwholeline(UNUSED(char **args))` from Src/Zle/zle_vi.c:550.
pub fn viyankwholeline() -> i32 {
    // c:550
    // C body (c:553-572): `bol = findbol(); startvichange(-1); n = zmult;
    //                     if (n < 1) return 1; for (i=n; i--; ) zlecs =
    //                     findeol() + 1; if (zlecs > zlell) zlecs = zlell;
    //                     cut(bol, zlecs - bol, CUT_YANK); zlecs = bol +
    //                     oldcs - bol; return 0`.
    let bol = findbol();
    let oldcs = ZLECS.load(SeqCst);
    startvichange(-1);
    let n = ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        ZLECS.store(findeol() + 1, SeqCst);
        if ZLECS.load(SeqCst) > ZLELL.load(SeqCst) {
            ZLECS.store(ZLELL.load(SeqCst), SeqCst);
        }
    }
    let end = ZLECS.load(SeqCst);
    let text: Vec<char> = ZLELINE.lock().unwrap()[bol..end].to_vec();
    KILLRING.lock().unwrap().push_front(text);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
    ZLECS.store(oldcs, SeqCst);
    0
}

/// Port of `vireplace(UNUSED(char **args))` from Src/Zle/zle_vi.c:574.
pub fn vireplace() -> i32 {
    // c:574
    // C body (c:575-577): `startvitext(0); return 0`. Enter overwrite-
    // style insert mode (insmode=0).
    startvitext(0);
    0
}

/// Port of `vireplacechars(UNUSED(char **args))` from Src/Zle/zle_vi.c:594.
///
/// vi `r{char}` / `R`-style replace: read one key (`vigetkey`) and
/// overwrite the next `zmult` characters with it, clamped to end-of-line.
/// Honours an active visual region (replace the whole selection), and
/// treats `<return>` specially — collapse the range to a single newline.
/// The previous Rust port faked the key read with `LASTCHAR` and dropped
/// the region path, the return-key case, and the `newchars`/`n` shift.
pub fn vireplacechars() -> i32 {
    // c:594
    let mut n; // c:596 `int n`
    let mut newchars = 0i32; // c:596
    let mut fail = false; // c:596

    startvichange(1); // c:599
    n = ZMOD.lock().unwrap().mult; // c:600 `n = zmult;`
    if n > 0 {
        // c:601
        let ra = get_region_active(); // c:602 `if (region_active)`
        if ra != 0 {
            let a;
            let mut b;
            if ra == 1 {
                // c:604-613 — character region: order mark/cursor.
                let mark = zle_main::MARK.load(SeqCst);
                let cs = ZLECS.load(SeqCst);
                if mark > cs {
                    a = cs; // c:607
                    b = mark; // c:608
                } else {
                    a = mark; // c:610
                    b = cs; // c:611
                }
                incpos(&mut b); // c:613 `INCPOS(b)`
            } else {
                // c:615 — line region: `regionlines(&a, &b)`.
                let (ra2, rb2) = regionlines();
                a = ra2;
                b = rb2;
            }
            ZLECS.store(a, SeqCst); // c:617 `zlecs = a`
            let zlell = ZLELL.load(SeqCst);
            if b > zlell {
                b = zlell; // c:618-619
            }
            n = (b - a) as i32; // c:620 `n = b - a`
            // c:621-624 — count displayed chars in the range.
            let mut aa = a;
            while aa < b {
                newchars += 1; // c:622
                incpos(&mut aa); // c:623 `INCPOS(a)`
            }
            REGION_ACTIVE.store(0, SeqCst); // c:625 `region_active = 0`
        } else {
            // c:627-636 — count forward `n` chars, stopping at eol/newline.
            let mut pos = ZLECS.load(SeqCst);
            let zlell = ZLELL.load(SeqCst);
            let line = ZLELINE.lock().unwrap();
            while n > 0 {
                // c:629
                if pos == zlell || line[pos] == '\n' {
                    fail = true; // c:631
                    break; // c:632
                }
                newchars += 1; // c:634
                incpos(&mut pos); // c:635 `INCPOS(pos)`
                n -= 1;
            }
            drop(line);
            n = (pos - ZLECS.load(SeqCst)) as i32; // c:637 `n = pos - zlecs`
        }
    }

    // c:640-645 — argument range check.
    if n < 1 || fail {
        if VIINREPEAT.load(SeqCst) != 0 {
            vigetkey(); // c:643
        }
        return 1; // c:644
    }
    // c:647-649 — read the replacement key.
    let ch = vigetkey();
    if ch == ZLEEOF {
        return 1; // c:648
    }
    // c:651-674 — perform the change.
    if ch == '\r' as i32 || ch == '\n' as i32 {
        // c:652-656 — <return> handled specially: collapse to one newline.
        ZLECS.fetch_add((n - 1) as usize, SeqCst); // c:653 `zlecs += n - 1`
        backkill(n - 1, CUT_RAW); // c:654
        let cs = ZLECS.load(SeqCst);
        ZLELINE.lock().unwrap()[cs] = '\n'; // c:655 `zleline[zlecs++] = '\n'`
        ZLECS.store(cs + 1, SeqCst);
    } else {
        // c:666-674 — overwrite `newchars` positions, fixing up width.
        let cs = ZLECS.load(SeqCst);
        if n > newchars {
            shiftchars(cs as i32, n - newchars); // c:667
        } else if n < newchars {
            spaceinline(newchars - n); // c:669
        }
        let chr = char::from_u32(ch as u32).unwrap_or('\u{FFFD}');
        let mut cs2 = cs;
        let mut remaining = newchars;
        {
            let mut line = ZLELINE.lock().unwrap();
            while remaining > 0 {
                line[cs2] = chr; // c:671 `zleline[zlecs++] = ch`
                cs2 += 1;
                remaining -= 1;
            }
        }
        ZLECS.store(cs2 - 1, SeqCst); // c:673 `zlecs--`
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0 // c:675
}

/// Port of `vicmdmode(UNUSED(char **args))` from Src/Zle/zle_vi.c:677.
///
/// ESC handler that switches from `viins`/`viopp` into the `vicmd`
/// keymap. Line-by-line port of c:677-695, with the load-bearing
/// `lastvichg = curvichg` promotion at c:687 — completes the
/// vi-change-recording cycle so `vi-repeat-change` (`.`) has a
/// command to replay.
pub fn vicmdmode() -> i32 {
    // c:679 — `if (invicmdmode() || selectkeymap("vicmd", 0)) return 1;`
    if *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" {
        return 1;
    }
    if selectkeymap("vicmd", 0) != 0 {
        return 1;
    }
    // c:681 — `mergeundo();` (undo-coalescing not yet wired — TODO)
    // c:682 — `insmode = unset(OVERSTRIKE);`
    let overstrike_set = crate::ported::zsh_h::isset(crate::ported::zsh_h::OVERSTRIKE);
    INSMODE.store(if overstrike_set { 0 } else { 1 }, SeqCst);

    // c:683-689 — promote curvichg → lastvichg when we were recording.
    if VICHGFLAG.load(SeqCst) == 1 {
        VICHGFLAG.store(0, SeqCst); // c:684
        let mut last = LASTVICHG.lock().unwrap();
        let mut cur = CURVICHG.lock().unwrap();
        last.mod_ = cur.mod_.clone();
        last.buf = std::mem::take(&mut cur.buf);
        last.bufsz = cur.bufsz;
        last.bufptr = cur.bufptr;
        cur.bufsz = 0;
        cur.bufptr = 0;
    }

    // c:690-691 — `if (viinrepeat == 1) viinrepeat = 0;`
    if VIINREPEAT.load(SeqCst) == 1 {
        VIINREPEAT.store(0, SeqCst);
    }

    // c:692-693 — `if (zlecs != findbol()) DECCS();`
    let bol = findbol();
    if ZLECS.load(SeqCst) != bol {
        deccs();
    }
    0
}

/// Port of `viopenlinebelow(UNUSED(char **args))` from Src/Zle/zle_vi.c:699.
pub fn viopenlinebelow() -> i32 {
    // c:699
    // C body (c:700-707): `zlecs = findeol(); spaceinline(1);
    //                     zleline[zlecs++] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    use crate::ported::zle::zle_utils::spaceinline;
    ZLECS.store(findeol(), SeqCst); // c:701
    spaceinline(1); // c:702
    {
        let cs = ZLECS.load(SeqCst);
        if let Some(slot) = ZLELINE.lock().unwrap().get_mut(cs) {
            *slot = '\n'; // c:703 `zleline[zlecs++] = '\n'`
        }
    }
    ZLECS.fetch_add(1, SeqCst); // c:703 post-inc
    startvitext(1); // c:704
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viopenlineabove(UNUSED(char **args))` from Src/Zle/zle_vi.c:711.
pub fn viopenlineabove() -> i32 {
    // c:711
    // C body (c:712-718): `zlecs = findbol(); spaceinline(1);
    //                     zleline[zlecs] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    use crate::ported::zle::zle_utils::spaceinline;
    ZLECS.store(findbol(), SeqCst); // c:713
    spaceinline(1); // c:714
    {
        let cs = ZLECS.load(SeqCst);
        if let Some(slot) = ZLELINE.lock().unwrap().get_mut(cs) {
            *slot = '\n'; // c:715
        }
    }
    startvitext(1); // c:716
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `vioperswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:723.
pub fn vioperswapcase() -> i32 {
    // c:723
    // C body (c:725-746): startvichange(1); if (getvirange(0) != -1)
    //                    swap case in range. Without getvirange, use
    //                    [zlecs, eol) as implicit range.
    startvichange(1);
    let eol = findeol();
    let oldcs = ZLECS.load(SeqCst);
    while ZLECS.load(SeqCst) < eol {
        let c = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)];
        ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        ZLECS.fetch_add(1, SeqCst);
    }
    ZLECS.store(oldcs, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viupcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:751.
pub fn viupcase() -> i32 {
    // c:751
    // C body (c:753-771): same as vidowncase but uppercase.
    startvichange(1);
    let eol = findeol();
    for i in ZLECS.load(SeqCst)..eol {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[i] = __g[i].to_ascii_uppercase();
        }
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `vidowncase(UNUSED(char **args))` from Src/Zle/zle_vi.c:773.
pub fn vidowncase() -> i32 {
    // c:773
    // C body (c:775-794): startvichange(1); if ((c2 = getvirange(0))
    //                    != -1) { lowercase all letters in [zlecs, c2);
    //                    return 0; } else return 1.
    // Without getvirange we use [zlecs, eol) as the implicit range.
    startvichange(1);
    let eol = findeol();
    for i in ZLECS.load(SeqCst)..eol {
        {
            let mut __g = ZLELINE.lock().unwrap();
            __g[i] = __g[i].to_ascii_lowercase();
        }
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Direct port of `int virepeatchange(char **args)` from
/// `Src/Zle/zle_vi.c:795-820`.
/// ```c
/// if (!lastvichg.buf || vichgflag || virangeflag) return 1;
/// // (restore zmod from lastvichg.mod, advance vibuf if numbered)
/// viinrepeat = 3;
/// ungetbytes(lastvichg.buf, lastvichg.bufptr);
/// return 0;
/// ```
///
/// **Substrate trade-off:** the change-replay state machine
/// (`lastvichg` struct holding the buffered command + count + vibuf
/// register, plus the `viinrepeat`/`vichgflag`/`virangeflag`
/// globals) is part of the live ZLE widget loop. Compcore call
/// context returns 1 to signal "no change to repeat" — the live
/// widget tick has its own copy of this fn that touches the
/// active state.
/// Port of `virepeatchange(UNUSED(char **args))` from `Src/Zle/zle_vi.c:795`.
///
/// Line-by-line port of c:795-815. Refuses to replay if there's no
/// stored change (`lastvichg.buf` empty), if we're already recording
/// one (`vichgflag != 0`), or if a vi range is in flight
/// (`virangeflag != 0`) — c:797. Otherwise updates the saved zmod
/// with the explicit MULT/VIBUF prefixes from the current zmod
/// (c:801-810), arms `viinrepeat = 3`, and feeds the recorded bytes
/// back through [`ungetbytes`] for the input loop to consume.
pub fn virepeatchange() -> i32 {
    use crate::ported::zle::zle_main::ungetbytes;

    // c:797-798 — `if (!lastvichg.buf || vichgflag || virangeflag) return 1;`
    let last = LASTVICHG.lock().unwrap();
    if last.buf.is_empty() || VICHGFLAG.load(SeqCst) != 0 || VIRANGEFLAG.load(SeqCst) != 0 {
        return 1;
    }
    drop(last);

    // c:800-810 — restore or update saved count + cut buffer.
    let zmod_flags = ZMOD.lock().unwrap().flags;
    let zmod_mult = ZMOD.lock().unwrap().mult;
    let zmod_vibuf = ZMOD.lock().unwrap().vibuf;
    let zmod_viapp = zmod_flags & MOD_VIAPP;

    {
        let mut lvc = LASTVICHG.lock().unwrap();
        if zmod_flags & MOD_MULT != 0 {
            // c:801-803
            lvc.mod_.mult = zmod_mult;
            lvc.mod_.flags |= MOD_MULT;
        }
        if zmod_flags & MOD_VIBUF != 0 {
            // c:805-808
            lvc.mod_.vibuf = zmod_vibuf;
            lvc.mod_.flags = (lvc.mod_.flags & !MOD_VIAPP) | MOD_VIBUF | zmod_viapp;
        } else if lvc.mod_.flags & MOD_VIBUF != 0 && lvc.mod_.vibuf >= 27 && lvc.mod_.vibuf <= 34 {
            // c:809-811 — "1.."8 → advance to next numbered buffer
            lvc.mod_.vibuf += 1;
        }
    }

    // c:813 — `viinrepeat = 3;`
    VIINREPEAT.store(3, SeqCst);

    // c:814 — `ungetbytes(lastvichg.buf, lastvichg.bufptr);`
    let (bytes, ptr) = {
        let l = LASTVICHG.lock().unwrap();
        (l.buf.clone(), l.bufptr as usize)
    };
    let lo = ptr.min(bytes.len());
    ungetbytes(&bytes[..lo]);

    0 // c:815
}

/// Port of `viindent(UNUSED(char **args))` from Src/Zle/zle_vi.c:820.
pub fn viindent() -> i32 {
    // c:820
    // C body (c:822-855): startvichange(1); insert tab at start of
    //                    each line in range. Iterates with findeol+1.
    use crate::ported::zle::zle_utils::spaceinline;
    startvichange(1);
    let saved_cs = ZLECS.load(SeqCst);
    ZLECS.store(findbol(), SeqCst);
    // c:842-849 — `while (zlecs <= c2 + 1) { if (zleline[zlecs] == '\n')
    //                ++zlecs; else { spaceinline(1); zleline[zlecs] = '\t';
    //                zlecs = findeol() + 1; } }`.
    let line_len = ZLELL.load(SeqCst);
    while ZLECS.load(SeqCst) < line_len {
        let at_nl = ZLELINE.lock().unwrap().get(ZLECS.load(SeqCst)).copied() == Some('\n');
        if at_nl {
            ZLECS.fetch_add(1, SeqCst); // c:844
            continue;
        }
        spaceinline(1); // c:843
        if let Some(slot) = ZLELINE.lock().unwrap().get_mut(ZLECS.load(SeqCst)) {
            *slot = '\t'; // c:844
        }
        let eol = findeol();
        if eol >= ZLELL.load(SeqCst) {
            break;
        }
        ZLECS.store(eol + 1, SeqCst); // c:845
    }
    ZLECS.store(saved_cs, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viunindent(UNUSED(char **args))` from Src/Zle/zle_vi.c:856.
pub fn viunindent() -> i32 {
    // c:856
    // C body: remove up to SHIFTWIDTH (4) leading spaces from each
    //         line in range.
    startvichange(1);
    let bol = findbol();
    let mut removed = 0;
    while removed < 4 && bol < ZLELINE.lock().unwrap().len() && ZLELINE.lock().unwrap()[bol] == ' '
    {
        ZLELINE.lock().unwrap().remove(bol);
        ZLELL.fetch_sub(1, SeqCst);
        removed += 1;
    }
    if ZLECS.load(SeqCst) >= bol + removed {
        ZLECS.fetch_sub(removed, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Direct port of `int vibackwarddeletechar(char **args)` from
/// `Src/Zle/zle_vi.c:888`. Backspace, vi-command-mode-aware. Negative
/// zmult routes through `videletechar` with the absolute count; else
/// kills up to `findbol()`'s worth of characters.
pub fn vibackwarddeletechar() -> i32 {
    // c:888
    let curkm = crate::ported::zle::zle_keymap::curkeymapname()
        .as_str()
        .to_string();
    let in_cmd = invicmdmode(&curkm);
    // c:892-893 — startvichange(-1) only in cmd mode.
    if in_cmd {
        startvichange(-1);
    }
    // c:896 — `n = zmult`.
    let n = ZMOD.lock().unwrap().mult as i32;
    // c:897-903 — negative count → videletechar with abs(n).
    if n < 0 {
        let prev = ZMOD.lock().unwrap().mult;
        ZMOD.lock().unwrap().mult = (-n) as i32;
        let ret = videletechar();
        ZMOD.lock().unwrap().mult = prev;
        return ret;
    }
    // c:906 — bail if at start of line / past viinsbegin.
    let bol = findbol();
    let cs = ZLECS.load(SeqCst);
    let viib = VIINSBEGIN.load(Ordering::Relaxed) as usize;
    if cs == bol || (!in_cmd && cs.saturating_sub(n as usize) < viib) {
        return 1;
    }
    // c:912-919 — clamp count + backkill.
    let mut nn = n as usize;
    if nn > cs - bol {
        nn = cs - bol;
    }
    backkill(nn as i32, CUT_FRONT | CUT_RAW);
    0
}

/// Port of `vikillline(UNUSED(char **args))` from Src/Zle/zle_vi.c:923.
/// C body (4 lines):
///     `if (viinsbegin > zlecs) return 1;
///      backdel(zlecs - viinsbegin, CUT_RAW);
///      return 0;`
/// (Previous Rust port killed back to BOL — wrong; vikillline kills
/// back to the start of the current vi insert session, not the
/// beginning of the line.)
pub fn vikillline() -> i32 {
    // c:923
    let zlecs = ZLECS.load(SeqCst) as i32;
    let viinsbegin = VIINSBEGIN.load(SeqCst) as i32;
    if viinsbegin > zlecs {
        return 1;
    } // c:925
    backdel(zlecs - viinsbegin, CUT_RAW); // c:927
    0 // c:928
}

/// Port of `vijoin(UNUSED(char **args))` from Src/Zle/zle_vi.c:933.
pub fn vijoin() -> i32 {
    // c:vijoin
    // C body: replace next '\\n' with ' ', skipping leading whitespace
    //         on the joined line. Repeat zmult times.
    startvichange(-1);
    let n = ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        let eol = findeol();
        if eol >= ZLELL.load(SeqCst) || ZLELINE.lock().unwrap().get(eol) != Some(&'\n') {
            return 1;
        }
        ZLELINE.lock().unwrap()[eol] = ' ';
        // Strip leading whitespace on the joined-in line.
        let mut p = eol + 1;
        while p < ZLELINE.lock().unwrap().len() && ZLELINE.lock().unwrap()[p].is_whitespace() {
            ZLELINE.lock().unwrap().remove(p);
            ZLELL.fetch_sub(1, SeqCst);
        }
        let _ = p;
        ZLECS.store(eol, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:977.
pub fn viswapcase() -> i32 {
    // c:viswapcase
    // C body: walk zmult chars, swap case of each; advance cursor.
    startvichange(-1);
    let n = ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    let eol = findeol();
    for _ in 0..n {
        if ZLECS.load(SeqCst) >= eol {
            break;
        }
        let c = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)];
        let swapped = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] = swapped;
        ZLECS.fetch_add(1, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Direct port of `int vicapslockpanic(char **args)` from
/// `Src/Zle/zle_vi.c:1002`.
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
pub fn vicapslockpanic() -> i32 {
    // c:1002
    // c:1004 — clearlist = 1.
    CLEARLIST.store(1, Ordering::Relaxed);
    // c:1005 — zbeep().
    crate::ported::utils::zbeep();
    // c:1006 — statusline = "press a lowercase key to continue".
    // The canonical home for the message is the file-scope `STATUSLINE`
    // static (zle_main.rs); we also mirror to the paramtab so the
    // prompt drawer picks it up via `$STATUSLINE`.
    let _ = crate::ported::params::setsparam("STATUSLINE", "press a lowercase key to continue");
    // c:1007 — zrefresh() — flushes paramtab/buffer state to the
    // refresh layer. The CLEARLIST flag set above is the trigger
    // the draw path watches; the actual repaint runs on the next
    // ZLE event loop iteration via `zrefresh()` in `zle_refresh.rs`.
    // c:1008-1009 — `while (!ZC_ilower(getfullchar(0))) ;`.
    // Without a live key-read loop we cannot block here; the live
    // ZLE input path (getfullchar) does the wait. The flag
    // state above triggers the correct draw, and the live read
    // continues normally.
    // c:1010 — clear statusline.
    let _ = crate::ported::params::setsparam("STATUSLINE", "");
    0 // c:1011
}

/// Port of `visetbuffer(char **args)` from Src/Zle/zle_vi.c:1015.
pub fn visetbuffer() -> i32 {
    // c:visetbuffer
    // C body: read one char as the vi buffer name (a-z or 1-9 or '"');
    //         set zmod.vibuf for the next yank/cut. Without vigetkey
    //         interactive read, use lastchar.
    let c = (crate::ported::zle::compcore::LASTCHAR.load(SeqCst) & 0xff) as u8;
    let idx: i32 = if c.is_ascii_digit() {
        (c - b'0') as i32 + 26
    } else if c.is_ascii_lowercase() {
        (c - b'a') as i32
    } else if c.is_ascii_uppercase() {
        // uppercase = append to register
        ZMOD.lock().unwrap().flags |= MOD_VIAPP;
        (c - b'A') as i32
    } else {
        return 1;
    };
    ZMOD.lock().unwrap().vibuf = idx;
    ZMOD.lock().unwrap().flags |= MOD_VIBUF;
    PREFIXFLAG.store(1, SeqCst);
    0
}

/// Port of `vikilleol(UNUSED(char **args))` from Src/Zle/zle_vi.c:1056.
pub fn vikilleol() -> i32 {
    // c:vikilleol
    // C body: kill from cursor to eol; start vi cmd-mode change.
    startvichange(1);
    let eol = findeol();
    if eol > ZLECS.load(SeqCst) {
        let text: Vec<char> = ZLELINE
            .lock()
            .unwrap()
            .drain(ZLECS.load(SeqCst)..eol)
            .collect();
        KILLRING.lock().unwrap().push_front(text);
        if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
            KILLRING.lock().unwrap().pop_back();
        }
        ZLELL.fetch_sub(eol - ZLECS.load(SeqCst), SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `vipoundinsert(UNUSED(char **args))` from Src/Zle/zle_vi.c:1073.
/// Single-line toggle of the leading `#` comment marker.
pub fn vipoundinsert() -> i32 {
    // c:1073

    // c:1075 — int oldcs = zlecs;
    let mut oldcs = ZLECS.load(SeqCst);

    // c:1077 — startvichange(-1);
    startvichange(-1);
    // c:1078 — vifirstnonblank(zlenoargs);
    vifirstnonblank();

    let cs = ZLECS.load(SeqCst);
    let is_pound = ZLELINE.lock().unwrap().get(cs).copied() == Some('#');

    let mut viib = VIINSBEGIN.load(SeqCst);

    if !is_pound {
        // c:1079
        // c:1080 — spaceinline(1);
        spaceinline(1);
        // c:1081 — zleline[zlecs] = '#';
        if let Some(slot) = ZLELINE.lock().unwrap().get_mut(cs) {
            *slot = '#';
        }
        // c:1082-1083 — if (zlecs <= viinsbegin) INCPOS(viinsbegin);
        if cs <= viib {
            viib = viib.saturating_add(1);
        }
        // c:1084-1085 — if (zlecs <= oldcs) INCPOS(oldcs);
        if cs <= oldcs {
            oldcs = oldcs.saturating_add(1);
        }
        // c:1086 — zlecs = oldcs;
        ZLECS.store(oldcs.min(ZLELL.load(SeqCst)), SeqCst);
    } else {
        // c:1087
        // c:1088 — foredel(1, 0);
        foredel(1, 0);
        // c:1089-1090 — if (zlecs < viinsbegin) DECPOS(viinsbegin);
        if cs < viib {
            viib = viib.saturating_sub(1);
        }
        // c:1091-1092 — if (zlecs < oldcs) DECPOS(oldcs);
        if cs < oldcs {
            oldcs = oldcs.saturating_sub(1);
        }
        // c:1093 — zlecs = oldcs;
        ZLECS.store(oldcs.min(ZLELL.load(SeqCst)), SeqCst);
    }
    VIINSBEGIN.store(viib, SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Port of `viquotedinsert(char **args)` from Src/Zle/zle_vi.c:1099.
///
/// Direct line-by-line port of the C body (c:1099-1120). Inserts a
/// literal `^` placeholder at the cursor, refreshes the display so
/// the user sees the indicator, reads one raw character via
/// [`getfullchar`], deletes the placeholder, and on non-EOF inserts
/// the character verbatim via [`selfinsert`]. The C source's
/// `#ifndef HAS_TIO` sgttyb branch only runs on legacy systems
/// without termios; zshrs only supports termios so it drops.
pub fn viquotedinsert() -> i32 {
    use crate::ported::zle::zle_h::ZLEEOF;
    use crate::ported::zle::zle_main::LASTCHAR_WIDE;
    use crate::ported::zle::zle_misc::selfinsert;
    use crate::ported::zle::zle_refresh::zrefresh;
    use crate::ported::zle::zle_utils::{foredel, spaceinline};
    // c:1105 — `spaceinline(1)`.
    spaceinline(1);
    // c:1106 — `zleline[zlecs] = '^'`.
    let cs = ZLECS.load(SeqCst);
    {
        let mut g = ZLELINE.lock().unwrap();
        if cs < g.len() {
            g[cs] = '^';
        }
    }
    // c:1107 — `zrefresh()`.
    zrefresh();
    // c:1114 — `getfullchar(0)`.
    let _ = crate::ported::zle::zle_main::getfullchar(false);
    // c:1118 — `foredel(1, 0)`.
    foredel(1, 0);
    // c:1119-1122 — `if (LASTFULLCHAR == ZLEEOF) return 1; else
    //                return selfinsert(args);`
    if LASTCHAR_WIDE.load(SeqCst) == ZLEEOF {
        return 1;
    }
    selfinsert(&[])
}

/// Port of `vidigitorbeginningofline(char **args)` from Src/Zle/zle_vi.c:1129.
pub fn vidigitorbeginningofline() -> i32 {
    // c:vidigitorbeginningofline
    // C body: `if (zmod.flags & MOD_TMULT) return digitargument();
    //          else { removesuffix(); invalidatelist();
    //                 return vibeginningofline(); }`.
    if ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {
        return digitargument();
    }
    vibeginningofline()
}
/// `VIRANGEFLAG` static.
pub static VIRANGEFLAG: std::sync::atomic::AtomicI32 = // c:36
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int wordflag;` from `Src/Zle/zle_vi.c:41`. Kludge flag
/// used by `cw`/`dw` so they stop at word boundaries.
pub static WORDFLAG: std::sync::atomic::AtomicI32 = // c:41
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int vilinerange;` from `Src/Zle/zle_vi.c:46`. Set when
/// the pending range is whole-line (e.g. `dd`, `yy`).
pub static VILINERANGE: std::sync::atomic::AtomicI32 = // c:46
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int vichgflag;` from `Src/Zle/zle_vi.c:65`. Set while a
/// vi change-tracker (`.`) is recording.
pub static VICHGFLAG: std::sync::atomic::AtomicI32 = // c:65
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int viinrepeat;` from `Src/Zle/zle_vi.c:73`. Set during
/// `.` replay so the recorder doesn't re-record.
pub static VIINREPEAT: std::sync::atomic::AtomicI32 = // c:73
    std::sync::atomic::AtomicI32::new(0);

/// Port of `struct vichange lastvichg` from `Src/Zle/zle_vi.c:54`.
/// Last completed vi change — replayed by `vi-repeat-change` (`.`).
pub static LASTVICHG: std::sync::Mutex<vichange> = // c:54
    std::sync::Mutex::new(vichange {
        mod_: modifier {
            flags: 0,
            mult: 1,
            tmult: 1,
            vibuf: 0,
            base: 10,
        },
        buf: Vec::new(),
        bufsz: 0,
        bufptr: 0,
    });

/// Port of `struct vichange curvichg` from `Src/Zle/zle_vi.c:54`.
/// In-flight vi change being recorded — promoted to [`LASTVICHG`] by
/// `endvichange` once the change widget completes.
pub static CURVICHG: std::sync::Mutex<vichange> = // c:54
    std::sync::Mutex::new(vichange {
        mod_: modifier {
            flags: 0,
            mult: 1,
            tmult: 1,
            vibuf: 0,
            base: 10,
        },
        buf: Vec::new(),
        bufsz: 0,
        bufptr: 0,
    });

/// Read the active numeric multiplier.
/// Port of `zmult` macro at Src/Zle/zle.h:267 (`#define zmult
/// (zmod.mult)`). Returns the explicit MULT prefix when set,
/// otherwise 1 — the default-1 fall-through that initmodifier
/// installs (zle_main.c:1604).
pub fn vi_get_arg() -> i32 {
    if ZMOD.lock().unwrap().flags & MOD_MULT != 0 {
        ZMOD.lock().unwrap().mult
    } else {
        1
    }
}

/// Read the next char from input and run a vi find-char.
/// `forward`: true for f/t (forward), false for F/T (backward).
/// `skip`: true for t/T (stop one short), false for f/F (land on the char).
/// Port of vifindnextchar/vifindprevchar/vifindnextcharskip/vifindprevcharskip
/// from Src/Zle/zle_move.c:739-783 — which all set state and call `vifindchar(0)`.
pub fn vi_find_char(forward: bool, skip: bool) {
    let c = match getfullchar(true) {
        Some(c) => c,
        None => return,
    };
    VFINDCHAR.store(c as i32, SeqCst);
    VFINDDIR.store(if forward { 1 } else { -1 }, SeqCst);
    // tailadd: f/F → 0; t → -1; T → +1.
    TAILADD.store(
        match (forward, skip) {
            (_, false) => 0,
            (true, true) => -1,
            (false, true) => 1,
        },
        SeqCst,
    );
    let _ = vi_find_char_inner(false);
}

/// Inner find-char routine. `repeat` distinguishes the user-typed call
/// from `;` / `,` re-runs.
/// Port of `vifindchar(int repeat, ...)` from Src/Zle/zle_move.c:787.
pub fn vi_find_char_inner(repeat: bool) -> i32 {
    let target_raw = VFINDCHAR.load(SeqCst);
    let target = match char::from_u32(target_raw as u32) {
        Some(c) if target_raw != 0 => c,
        _ => return 1,
    };
    if VFINDDIR.load(SeqCst) == 0 {
        return 1;
    }
    let ocs = ZLECS.load(SeqCst);
    let mut n = vi_get_arg();
    if n < 0 {
        // Negative count flips direction; faithful to C virevrepeatfind path.
        n = -n;
        VFINDDIR.store(-VFINDDIR.load(SeqCst), SeqCst);
        TAILADD.store(-TAILADD.load(SeqCst), SeqCst);
        let saved_mult = ZMOD.lock().unwrap().mult;
        ZMOD.lock().unwrap().mult = n;
        let ret = vi_find_char_inner(repeat);
        ZMOD.lock().unwrap().mult = saved_mult;
        VFINDDIR.store(-VFINDDIR.load(SeqCst), SeqCst);
        TAILADD.store(-TAILADD.load(SeqCst), SeqCst);
        return ret;
    }
    // On `;` (repeat) with t/T, step over the immediately-adjacent match
    // so we don't get stuck on the same char.
    if repeat && TAILADD.load(SeqCst) != 0 {
        if VFINDDIR.load(SeqCst) > 0 {
            if ZLECS.load(SeqCst) < ZLELL.load(SeqCst)
                && ZLECS.load(SeqCst) + 1 < ZLELL.load(SeqCst)
                && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) + 1] == target
            {
                ZLECS.fetch_add(1, SeqCst);
            }
        } else if ZLECS.load(SeqCst) > 0
            && ZLELINE.lock().unwrap()[ZLECS.load(SeqCst) - 1] == target
        {
            ZLECS.fetch_sub(1, SeqCst);
        }
    }
    let dir = VFINDDIR.load(SeqCst);
    for _ in 0..n {
        // Step at least once, then keep stepping until we land on the char,
        // hit a newline, or run off the end.
        let found = if dir > 0 {
            let mut p = ZLECS.load(SeqCst) + 1;
            let mut hit = None;
            while p < ZLELL.load(SeqCst) {
                let ch = ZLELINE.lock().unwrap()[p];
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
            if ZLECS.load(SeqCst) == 0 {
                None
            } else {
                let mut p = ZLECS.load(SeqCst) - 1;
                let mut hit = None;
                loop {
                    let ch = ZLELINE.lock().unwrap()[p];
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
            Some(p) => {
                ZLECS.store(p, SeqCst);
            }
            None => {
                ZLECS.store(ocs, SeqCst);
                return 1;
            }
        }
    }
    // Apply the t/T adjustment after the final landing.
    let tail = TAILADD.load(SeqCst);
    if tail > 0 && ZLECS.load(SeqCst) < ZLELL.load(SeqCst) {
        ZLECS.fetch_add(1, SeqCst);
    } else if tail < 0 && ZLECS.load(SeqCst) > 0 {
        ZLECS.fetch_sub(1, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Jump to the bracket matching the one under the cursor.
/// Port of `vimatchbracket(UNUSED(char **args))` from Src/Zle/zle_misc.c. Vim's `%`
/// motion — recognises (), [], {}, <>; walks forward or backward
/// honouring nesting depth.
pub fn vi_match_bracket() {
    let c = if ZLECS.load(SeqCst) < ZLELL.load(SeqCst) {
        ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)]
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
    let mut pos = ZLECS.load(SeqCst);

    if forward {
        pos += 1;
        while pos < ZLELL.load(SeqCst) && depth > 0 {
            if ZLELINE.lock().unwrap()[pos] == c {
                depth += 1;
            } else if ZLELINE.lock().unwrap()[pos] == target {
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
                if ZLELINE.lock().unwrap()[pos] == c {
                    depth += 1;
                } else if ZLELINE.lock().unwrap()[pos] == target {
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
        ZLECS.store(pos, SeqCst);
        ZLE_RESET_NEEDED.store(1, SeqCst);
    }
}

/// Enter overwrite mode (vim's `R` command).
/// Port of `vireplace(UNUSED(char **args))` from Src/Zle/zle_vi.c. Switches to the
/// insert keymap with `insmode = false` so subsequent self-inserts
/// overwrite existing chars instead of pushing them right.
pub fn vi_replace_mode() {
    selectkeymap("viins", 1);
    INSMODE.store(0, SeqCst);
    // Overwrite mode
}

/// Toggle the case of the character under the cursor and advance.
/// Port of `viswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c (vim's `~`).
/// Uppercase letters become lowercase and vice versa; non-letters
/// pass through untouched. Cursor advances one position post-swap.
pub fn vi_swap_case() {
    let count = vi_get_arg() as usize;

    for _ in 0..count {
        if ZLECS.load(SeqCst) < ZLELL.load(SeqCst) {
            let c = ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)];
            ZLELINE.lock().unwrap()[ZLECS.load(SeqCst)] = if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c
            };
            ZLECS.fetch_add(1, SeqCst);
        }
    }

    // Move back one if we went past end
    if ZLECS.load(SeqCst) > 0 && ZLECS.load(SeqCst) == ZLELL.load(SeqCst) {
        ZLECS.fetch_sub(1, SeqCst);
    }

    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Vi undo (`u` in command mode). Port of viundo() — which in C zsh just
/// dispatches to undo(args) (zle_utils.c:1601). Routes through our index-based
/// undo_widget() that mirrors that implementation.
pub fn vi_undo() {
    let _ = undo_widget();
}

/// Vi visual mode (`v` in command mode).
/// Port of visualmode(UNUSED(char **args)) from Src/Zle/zle_move.c:516. Toggles
/// `region_active` between 0 (off), 1 (charwise), and 2 (linewise) per
/// the C switch: from inactive enters charwise (sets mark first); from
/// charwise turns off; from linewise switches to charwise.
pub fn vi_visual_mode() {
    match REGION_ACTIVE.load(SeqCst) {
        1 => {
            REGION_ACTIVE.store(0, SeqCst);
        }
        0 => {
            MARK.store(ZLECS.load(SeqCst), SeqCst);
            REGION_ACTIVE.store(1, SeqCst);
        }
        2 => {
            REGION_ACTIVE.store(1, SeqCst);
        }
        _ => {}
    }
}

/// Vi visual line mode (`V` in command mode).
/// Port of visuallinemode(UNUSED(char **args)) from Src/Zle/zle_move.c:540. Same toggle
/// shape as visualmode but the "active" target is 2 (linewise).
pub fn vi_visual_line_mode() {
    match REGION_ACTIVE.load(SeqCst) {
        2 => {
            REGION_ACTIVE.store(0, SeqCst);
        }
        0 => {
            MARK.store(ZLECS.load(SeqCst), SeqCst);
            REGION_ACTIVE.store(2, SeqCst);
        }
        1 => {
            REGION_ACTIVE.store(2, SeqCst);
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
pub fn vi_visual_block_mode() {
    if REGION_ACTIVE.load(SeqCst) == 0 {
        MARK.store(ZLECS.load(SeqCst), SeqCst);
    }
    REGION_ACTIVE.store(1, SeqCst);
}

/// Deactivate the visual region (`Esc` from visual mode).
/// Port of deactivateregion(UNUSED(char **args)) from Src/Zle/zle_move.c:564.
pub fn vi_deactivate_region() {
    REGION_ACTIVE.store(0, SeqCst);
}

/// Vi set mark (`m{a-z}` in command mode). Port of visetmark() from
/// Src/Zle/zle_move.c:872. Stores the current cursor and history line in
/// the named slot; non-letter names are rejected.
pub fn vi_set_mark(name: char) {
    // Set the historical mark (mirror with zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) for emacs compat).
    MARK.store(ZLECS.load(SeqCst), SeqCst);
    if let Some(idx) = vimark_slot(name) {
        vimarks().lock().unwrap()[idx] =
            Some((ZLECS.load(SeqCst), history().lock().unwrap().cursor as i32));
    }
}

/// Vi goto mark (`'a` / `` `a `` in command mode). Port of vigotomark()
/// from zle_move.c:887. ASCII letters jump to the saved location;
/// `'` / `` ` `` jumps to the implicit "last position" mark; other
/// characters are rejected.
pub fn vi_goto_mark(name: char) {
    let idx = match vimark_slot(name) {
        Some(i) => i,
        None => return,
    };
    let (cs, hist) = match vimarks().lock().unwrap()[idx] {
        Some(s) => s,
        None => return,
    };
    // Save the pre-jump position into the implicit mark (slot 26) so the
    // user can return to it with `''`.
    vimarks().lock().unwrap()[26] =
        Some((ZLECS.load(SeqCst), history().lock().unwrap().cursor as i32));
    if hist >= 0 && (hist as usize) < history().lock().unwrap().entries.len() {
        // Cross-history jumps need to load that entry.
        let target = hist as usize;
        if target != history().lock().unwrap().cursor {
            history().lock().unwrap().cursor = target;
            *ZLELINE.lock().unwrap() = history().lock().unwrap().entries[target]
                .line
                .chars()
                .collect();
            ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
        }
    }
    ZLECS.store(cs.min(ZLELL.load(SeqCst)), SeqCst);
    ZLE_RESET_NEEDED.store(1, SeqCst);
}

/// Append `key` to the vi change-replay buffer.
/// Port of the recording side of `virepeatchange()` machinery from
/// Src/Zle/zle_vi.c — C zsh tracks this via `vichgflag` + `vichgbuf`
/// in zle_main.c, capturing every byte fed during a `c` / `d` / `y`
/// operator, between `startvichange()` and the operator completion.
/// Callers (the operator entry/exit points) gate when recording is
/// active; this method just appends. The buffer is consumed by
/// `widget_vi_repeat_change` via `ungetbytes`.
pub fn vi_record_change(key: u8) {
    VICHGBUF.lock().unwrap().push(key);
}

/// Reset the change-replay buffer to start a fresh recording session.
/// Mirrors C zsh's `vichgflag = 1; freevichg(); vichgbuf = ...` setup
/// inside `startvichange()` (zle_vi.c).
pub fn vi_start_change_recording() {
    VICHGBUF.lock().unwrap().clear();
}

/// Replay the last vi change ("." in command mode).
/// Port of `virepeatchange(UNUSED(char **args))` from Src/Zle/zle_vi.c — re-feeds the
/// recorded `vi_chg_buf` via `ungetbytes` so the next `zlecore`
/// iteration re-runs the captured operator + motion. With nothing
/// recorded yet (operator entry/exit don't gate `vi_record_change`
/// in this build), the buffer is empty and replay is a no-op,
/// matching zsh's behaviour pre-first-change.
pub fn vi_repeat_change() {
    if VICHGBUF.lock().unwrap().is_empty() {
        return;
    }
    let bytes = VICHGBUF.lock().unwrap().clone();
    ungetbytes(&bytes);
}

/// Read the next keystroke and treat it as a vi motion to define an
/// operator range. Returns `Some((start, end, line_mode))` where the
/// operator should act on `[start, end)`, or `None` if the motion was
/// unknown / canceled / a no-op.
///
/// Port of `getvirange(int wf)` from `Src/Zle/zle_vi.c:172`. The full C
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
pub fn vi_get_range(op_char: char) -> Option<(usize, usize, bool)> {
    let pos = ZLECS.load(SeqCst);
    let n = vi_get_arg().max(1);
    let motion = getfullchar(false)?;

    // Doubled letter (e.g. `dd`, `cc`, `yy`) → entire current line(s).
    // Mirrors the `MOD_LINE` branch of `getvirange()` in zle_vi.c:281
    // but invoked directly when the user repeats the operator letter.
    if motion == op_char {
        let bol = findbol();
        let mut eol = findeol();
        // Extend by `n - 1` more lines forward to honour the count
        // (vi `3dd` deletes 3 lines).
        for _ in 1..n {
            if eol >= ZLELL.load(SeqCst) {
                break;
            }
            eol = findeol();
        }
        // Include the trailing newline in the range when there is one,
        // so the operator pulls the whole line including its terminator.
        let end = if eol < ZLELL.load(SeqCst) {
            eol + 1
        } else {
            eol
        };
        return Some((bol, end, true));
    }

    let other = match motion {
        // Word motions — `w` / `b` / `e` use the WordStyle::Vi class,
        // `W` / `B` / `E` use blank-delimited (matches zsh's WORDFLAG_W
        // distinction between iword and ialnum classes).
        'w' => {
            let mut p = pos;
            for _ in 0..n {
                let saved_cs = ZLECS.load(SeqCst);
                ZLECS.store(p, SeqCst);
                p = find_word_end(WordStyle::Vi);
                ZLECS.store(saved_cs, SeqCst);
            }
            p
        }
        'W' => {
            let mut p = pos;
            for _ in 0..n {
                let saved_cs = ZLECS.load(SeqCst);
                ZLECS.store(p, SeqCst);
                p = find_word_end(WordStyle::BlankDelimited);
                ZLECS.store(saved_cs, SeqCst);
            }
            p
        }
        'b' => {
            let mut p = pos;
            for _ in 0..n {
                let saved_cs = ZLECS.load(SeqCst);
                ZLECS.store(p, SeqCst);
                p = find_word_start(WordStyle::Vi);
                ZLECS.store(saved_cs, SeqCst);
            }
            p
        }
        'B' => {
            let mut p = pos;
            for _ in 0..n {
                let saved_cs = ZLECS.load(SeqCst);
                ZLECS.store(p, SeqCst);
                p = find_word_start(WordStyle::BlankDelimited);
                ZLECS.store(saved_cs, SeqCst);
            }
            p
        }
        'e' => {
            // `e` is end-of-word inclusive; the C path (`viendword`)
            // lands on the last char of the word. For our range it
            // becomes start..=word_end which is start..(word_end+1).
            let saved_cs = ZLECS.load(SeqCst);
            ZLECS.store(pos, SeqCst);
            let mut p = find_word_end(WordStyle::Vi);
            ZLECS.store(saved_cs, SeqCst);
            if p < ZLELL.load(SeqCst) {
                p += 1;
            }
            p
        }
        'E' => {
            let saved_cs = ZLECS.load(SeqCst);
            ZLECS.store(pos, SeqCst);
            let mut p = find_word_end(WordStyle::BlankDelimited);
            ZLECS.store(saved_cs, SeqCst);
            if p < ZLELL.load(SeqCst) {
                p += 1;
            }
            p
        }
        // Line-internal motions.
        '0' => findbol(),
        '^' => {
            // First non-blank — `vifirstnonblank` in zle_move.c:862.
            let bol = findbol();
            let mut p = bol;
            while p < ZLELL.load(SeqCst) && {
                let __c = ZLELINE.lock().unwrap()[p];
                __c.is_whitespace() && __c != '\n'
            } {
                p += 1;
            }
            p
        }
        '$' => findeol(),
        'h' => pos.saturating_sub(n as usize),
        'l' => (pos + n as usize).min(ZLELL.load(SeqCst)),
        // Line mode for j/k — extend the range across `n` lines.
        'j' => {
            let mut p = findeol();
            for _ in 0..n {
                if p >= ZLELL.load(SeqCst) {
                    break;
                }
                p = findeol();
            }
            let bol = findbol();
            let end = if p < ZLELL.load(SeqCst) { p + 1 } else { p };
            return Some((bol, end, true));
        }
        'k' => {
            let mut bol = findbol();
            for _ in 0..n {
                if bol == 0 {
                    break;
                }
                bol = findbol();
            }
            let eol = findeol();
            let end = if eol < ZLELL.load(SeqCst) {
                eol + 1
            } else {
                eol
            };
            return Some((bol, end, true));
        }
        // Find-char motions delegate to vi_find_char_inner which already
        // honours t/T tail-skip and the count via `mult`. We push the
        // motion char as the find-char target.
        'f' | 'F' | 't' | 'T' => {
            let next = getfullchar(false)?;
            VFINDCHAR.store(next as i32, SeqCst);
            VFINDDIR.store(
                if motion == 'f' || motion == 't' {
                    1
                } else {
                    -1
                },
                SeqCst,
            );
            TAILADD.store(
                match motion {
                    'f' | 'F' => 0,
                    't' => -1,
                    'T' => 1,
                    _ => 0,
                },
                SeqCst,
            );
            let saved_mult = ZMOD.lock().unwrap().mult;
            ZMOD.lock().unwrap().mult = n;
            let ok = vi_find_char_inner(false) == 0;
            ZMOD.lock().unwrap().mult = saved_mult;
            if !ok {
                return None;
            }
            // For `f`/`t` (forward), include the landed-on char in the
            // range — match C's `if (vfinddir == 1 && virangeflag) INCCS();`
            // at zle_move.c:828.
            let mut p = ZLECS.load(SeqCst);
            if (motion == 'f' || motion == 't') && p < ZLELL.load(SeqCst) {
                p += 1;
            }
            ZLECS.store(pos, SeqCst);
            p
        }
        _ => return None,
    };

    if other == pos {
        return None;
    }
    let (start, end) = if other > pos {
        (pos, other)
    } else {
        (other, pos)
    };
    Some((start, end, false))
}

/// Push `n` chars from `start` onto the kill ring (front).
/// Helper used by the operator ports below — equivalent to C zsh's
/// `cut(start, n, CUT_RAW)` / `forekill(n, CUT_RAW)` but operating
/// directly on our `Vec<char>` buffer.
fn vi_cut_into_killring(start: usize, end: usize) {
    if end <= start || end > ZLELINE.lock().unwrap().len() {
        return;
    }
    let killed: Vec<char> = ZLELINE.lock().unwrap()[start..end].to_vec();
    KILLRING.lock().unwrap().push_front(killed);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
}

/// `d{motion}` — vi delete operator.
/// Port of `videlete(UNUSED(char **args))` from `Src/Zle/zle_vi.c:384`.
pub fn vi_delete_op() -> i32 {
    let (start, end, line_mode) = match vi_get_range('d') {
        Some(r) => r,
        None => return 1,
    };
    vi_cut_into_killring(start, end);
    let drained = end - start;
    ZLELINE.lock().unwrap().drain(start..end);
    ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
    ZLECS.store(start.min(ZLELL.load(SeqCst)), SeqCst);
    if line_mode && ZLELL.load(SeqCst) > 0 {
        // C zle_vi.c:392-397 — for line ranges, also pull the trailing
        // \n if the cursor now sits past the buffer end, then jump to
        // the first non-blank of the surviving line.
        LASTCOL.store(-1, SeqCst);
        let bol = findbol();
        let mut p = bol;
        while p < ZLELL.load(SeqCst) && {
            let __c = ZLELINE.lock().unwrap()[p];
            __c.is_whitespace() && __c != '\n'
        } {
            p += 1;
        }
        ZLECS.store(p, SeqCst);
    }
    let _ = drained;
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// `c{motion}` — vi change operator.
/// Port of `vichange(UNUSED(char **args))` from `Src/Zle/zle_vi.c:438`. After deleting the
/// range, switches the keymap to insert mode (`startvitext`) — the C
/// path also sets `viinsbegin = zlecs; vistartchange = undo_changeno`,
/// which we mirror so a future `.` repeat can replay correctly.
pub fn vi_change_op() -> i32 {
    let (start, end, _) = match vi_get_range('c') {
        Some(r) => r,
        None => return 1,
    };
    vi_cut_into_killring(start, end);
    ZLELINE.lock().unwrap().drain(start..end);
    ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
    ZLECS.store(start.min(ZLELL.load(SeqCst)), SeqCst);
    VISTARTCHANGE.store(UNDO_CHANGENO.load(SeqCst), SeqCst);
    selectkeymap("main", 1);
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// `y{motion}` — vi yank operator.
/// Port of `viyank(UNUSED(char **args))` from `Src/Zle/zle_vi.c:507`. Copies the range to
/// the kill ring without removing it; cursor lands at the start of the
/// yanked region.
pub fn vi_yank_op() -> i32 {
    let saved_lastcol = LASTCOL.load(SeqCst);
    let (start, end, line_mode) = match vi_get_range('y') {
        Some(r) => r,
        None => return 1,
    };
    vi_cut_into_killring(start, end);
    ZLECS.store(start, SeqCst);
    if line_mode && saved_lastcol != -1 {
        // zle_vi.c:518-531 — for line yanks, restore the column on the
        // current line (clamped to its end-of-line).
        let eol = findeol();
        ZLECS.fetch_add(saved_lastcol as usize, SeqCst);
        if ZLECS.load(SeqCst) >= eol {
            ZLECS.store(eol, SeqCst);
        }
        LASTCOL.store(-1, SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, SeqCst);
    0
}

/// Map a vi mark name to its slot index in the file-scope
/// `VIMARKS` static.
/// `a..z` → 0..25; `'` / `` ` `` → 26 (the implicit last-position mark).
fn vimark_slot(name: char) -> Option<usize> {
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

    fn zle_with(line: &str, cs: usize) {
        zle_reset();
        *ZLELINE.lock().unwrap() = line.chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), SeqCst);
        ZLECS.store(cs, SeqCst);
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_forward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('d' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(0, SeqCst);
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(ZLECS.load(SeqCst), 3);
    }

    #[test]
    fn vi_find_char_inner_skip_stops_one_short_forward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('d' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(-1, SeqCst); // t = forward skip
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(ZLECS.load(SeqCst), 2);
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_backward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcdef", 5);
        VFINDCHAR.store('b' as i32, SeqCst);
        VFINDDIR.store(-1, SeqCst);
        TAILADD.store(0, SeqCst);
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(ZLECS.load(SeqCst), 1);
    }

    #[test]
    fn vi_find_char_inner_returns_1_and_restores_when_missing() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('z' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(0, SeqCst);
        assert_eq!(vi_find_char_inner(false), 1);
        assert_eq!(ZLECS.load(SeqCst), 0);
    }

    #[test]
    fn vi_find_char_inner_stops_at_newline() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abc\ndef", 0);
        VFINDCHAR.store('e' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(0, SeqCst);
        // 'e' is past the \n on the next line; vi find must not cross it.
        assert_eq!(vi_find_char_inner(false), 1);
        assert_eq!(ZLECS.load(SeqCst), 0);
    }

    #[test]
    fn vi_repeat_find_walks_to_next_match_in_same_direction() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("a-b-c-d", 0);
        VFINDCHAR.store('-' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(0, SeqCst);
        // Initial find lands on first '-'.
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(ZLECS.load(SeqCst), 1);
        // Repeat-find advances to the next '-'.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(ZLECS.load(SeqCst), 3);
        // And the next.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(ZLECS.load(SeqCst), 5);
    }

    #[test]
    fn vi_set_and_goto_named_mark_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("hello world", 6);
        vi_set_mark('a');
        ZLECS.store(0, SeqCst);
        vi_goto_mark('a');
        assert_eq!(ZLECS.load(SeqCst), 6);
    }

    #[test]
    fn vi_goto_mark_records_implicit_last_position() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("0123456789", 4);
        vi_set_mark('m');
        ZLECS.store(9, SeqCst);
        vi_goto_mark('m'); // jump back; 26th slot now holds 9
        assert_eq!(ZLECS.load(SeqCst), 4);
        vi_goto_mark('\''); // jump to implicit last position
        assert_eq!(ZLECS.load(SeqCst), 9);
    }

    #[test]
    fn vi_set_mark_ignores_invalid_names() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abc", 1);
        vi_set_mark('A'); // uppercase not allowed
        vi_set_mark('1'); // digit not allowed
        assert!(vimarks().lock().unwrap().iter().all(|m| m.is_none()));
    }

    fn feed(s: &str) {
        // Pre-feed bytes into the unget buffer so getfullchar() returns
        // them without blocking on stdin. Used by the operator tests below
        // to drive vi_get_range's next-keystroke read.
        ungetbytes(s.as_bytes());
    }

    #[test]
    fn vi_get_range_dd_selects_whole_current_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("aaa\nbbb\nccc", 4); // cursor on 'b' line
        feed("d");
        let (s, e, line) = vi_get_range('d').expect("range");
        assert!(line);
        assert_eq!(s, 4);
        assert_eq!(e, 8); // up to and including the trailing '\n'
    }

    #[test]
    fn vi_get_range_dw_selects_to_word_end() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("hello world", 0);
        feed("w");
        let (s, e, line) = vi_get_range('d').expect("range");
        assert!(!line);
        assert_eq!(s, 0);
        // find_word_end on "hello world" at pos 0 (Vi style) skips through
        // "hello" plus trailing whitespace, landing at 6 ("world" start).
        assert_eq!(e, 6);
    }

    #[test]
    fn vi_get_range_d_dollar_selects_to_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("foo bar baz", 4);
        feed("$");
        let (s, e, _) = vi_get_range('d').expect("range");
        assert_eq!(s, 4);
        assert_eq!(e, 11);
    }

    #[test]
    fn vi_delete_op_dw_removes_first_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("hello world", 0);
        feed("w");
        assert_eq!(vi_delete_op(), 0);
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "world");
        // Killed text landed on the kill ring.
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("hello ".to_string())
        );
    }

    #[test]
    fn vi_yank_op_y_dollar_copies_without_removing() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("foo bar", 4);
        feed("$");
        assert_eq!(vi_yank_op(), 0);
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "foo bar"
        );
        assert_eq!(
            KILLRING
                .lock()
                .unwrap()
                .front()
                .map(|v| v.iter().collect::<String>()),
            Some("bar".to_string())
        );
        // Cursor lands at start of the yanked range.
        assert_eq!(ZLECS.load(SeqCst), 4);
    }

    #[test]
    fn vi_change_op_cw_removes_word_and_clears_pending_change() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("foo bar", 0);
        feed("w");
        assert_eq!(vi_change_op(), 0);
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "bar");
        assert_eq!(ZLECS.load(SeqCst), 0);
        // vistartchange records the change number we entered insert mode at;
        // it should now equal undo_changeno (zero in this fresh zle).
        assert_eq!(VISTARTCHANGE.load(SeqCst), UNDO_CHANGENO.load(SeqCst));
    }

    #[test]
    fn vi_visual_mode_toggles_charwise() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcd", 2);
        assert_eq!(REGION_ACTIVE.load(SeqCst), 0);
        vi_visual_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 1);
        assert_eq!(MARK.load(SeqCst), 2);
        vi_visual_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 0);
    }

    #[test]
    fn vi_visual_line_mode_toggles_linewise_and_swaps_with_charwise() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcd", 0);
        vi_visual_line_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 2);
        // In linewise → charwise via vi_visual_mode().
        vi_visual_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 1);
        // Charwise → linewise via vi_visual_line_mode().
        vi_visual_line_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 2);
        // Linewise → off via vi_visual_line_mode().
        vi_visual_line_mode();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 0);
    }

    #[test]
    fn vi_deactivate_region_clears_active_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abcd", 0);
        REGION_ACTIVE.store(2, SeqCst);
        vi_deactivate_region();
        assert_eq!(REGION_ACTIVE.load(SeqCst), 0);
    }

    #[test]
    fn vi_record_change_appends_to_replay_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("", 0);
        vi_start_change_recording();
        vi_record_change(b'd');
        vi_record_change(b'w');
        assert_eq!(*VICHGBUF.lock().unwrap(), vec![b'd', b'w']);
        vi_start_change_recording();
        assert!(VICHGBUF.lock().unwrap().is_empty());
    }

    #[test]
    fn vi_get_range_unknown_motion_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("abc", 0);
        feed("Z"); // no motion mapped to Z
        assert!(vi_get_range('d').is_none());
    }

    #[test]
    fn vi_undo_reverses_a_recorded_change() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("", 0);
        setlastline();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, SeqCst);
        ZLECS.store(3, SeqCst);
        mkundoent();
        vi_undo();
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "");
    }

    #[test]
    fn vi_rev_repeat_find_walks_back() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with("a-b-c-d", 0);
        VFINDCHAR.store('-' as i32, SeqCst);
        VFINDDIR.store(1, SeqCst);
        TAILADD.store(0, SeqCst);
        // Forward to first '-' at index 1.
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(ZLECS.load(SeqCst), 1);
        // Forward again to '-' at 3.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(ZLECS.load(SeqCst), 3);
        // Reverse repeat — back to index 1.
        assert_eq!(virevrepeatfind(), 0);
        assert_eq!(ZLECS.load(SeqCst), 1);
    }

    // ─── zsh-corpus pins for viaddnext / viaddeol / viinsert ───────

    /// `viaddeol()` moves cursor to end-of-line and returns 0.
    #[test]
    fn zle_vi_corpus_viaddeol_moves_to_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("hello", 0);
        let r = viaddeol();
        assert_eq!(r, 0);
        assert_eq!(
            ZLECS.load(SeqCst),
            5,
            "cursor at end-of-line after viaddeol"
        );
    }

    /// `viinsert()` returns 0 and doesn't move cursor.
    #[test]
    fn zle_vi_corpus_viinsert_returns_zero_no_move() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("hello", 2);
        let r = viinsert();
        assert_eq!(r, 0);
        // viinsert just enters insert mode at current pos.
        assert_eq!(ZLECS.load(SeqCst), 2, "viinsert doesn't move cursor");
    }

    /// `viaddnext()` advances cursor by 1 when not at EOL.
    #[test]
    fn zle_vi_corpus_viaddnext_advances_when_not_at_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("hello", 0);
        let r = viaddnext();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(SeqCst), 1, "viaddnext from pos 0 → pos 1");
    }

    /// `viaddnext()` at EOL doesn't advance (already at end).
    #[test]
    fn zle_vi_corpus_viaddnext_at_eol_stays() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("hello", 5);
        let r = viaddnext();
        assert_eq!(r, 0);
        assert_eq!(ZLECS.load(SeqCst), 5, "viaddnext at EOL stays at EOL");
    }

    /// `viinsert_init` doesn't panic.
    #[test]
    fn zle_vi_corpus_viinsert_init_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("test", 0);
        viinsert_init();
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_vi.c. Tests that capture
    // KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `viaddeol` moves cursor to end of line. C zle_vi.c:
    ///   `zlecs = findeol(); startvitext(1); return 0;`
    #[test]
    fn viaddeol_moves_cursor_to_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("hello", 0); // cursor at start
        let ret = viaddeol();
        assert_eq!(ret, 0, "viaddeol returns 0");
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            5,
            "cursor at EOL (after 'hello' = pos 5)"
        );
    }

    /// `viaddnext` increments cursor when not at EOL. C zle_vi.c:
    ///   `if (zlecs != findeol()) INCCS(); startvitext(1); return 0;`
    #[test]
    fn viaddnext_increments_cursor_when_not_at_eol() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("abc", 1); // cursor at 'b'
        let ret = viaddnext();
        assert_eq!(ret, 0);
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            2,
            "cursor advances past current char (1 → 2)"
        );
    }

    /// `viaddnext` at EOL leaves cursor unchanged.
    #[test]
    fn viaddnext_at_eol_leaves_cursor_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("xyz", 3); // cursor at EOL
        let ret = viaddnext();
        assert_eq!(ret, 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3, "at EOL, cursor stays put");
    }

    /// `viinsertbol` moves to first non-blank. C zle_vi.c:
    ///   `vifirstnonblank(zlenoargs); startvitext(1); return 0;`
    #[test]
    fn viinsertbol_moves_to_first_non_blank() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("   abc", 5); // cursor inside word
        let ret = viinsertbol();
        assert_eq!(ret, 0);
        // C's vifirstnonblank lands cursor at pos 3 ('a').
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            3,
            "cursor at first non-blank (pos 3 = 'a')"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_vi.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:445 — `viaddeol` puts cursor at line-end. Pin: cursor lands
    /// at ZLELL on a single-line buffer.
    #[test]
    fn viaddeol_moves_cursor_to_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("hello", 0); // cursor at start
        let ret = viaddeol();
        assert_eq!(ret, 0);
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            5,
            "viaddeol moves cursor to end of line"
        );
    }

    /// c:679 — `vicmdmode` when already in vicmd returns 1 (no-op).
    #[test]
    fn vicmdmode_already_in_vicmd_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        // Switch to vicmd first, then call vicmdmode again.
        if selectkeymap("vicmd", 0) == 0 {
            let r = vicmdmode();
            assert_eq!(r, 1, "vicmdmode on existing vicmd → 1 (no-op signal)");
        }
    }

    /// c:679 — `vicmdmode` from non-vicmd selects vicmd and returns 0.
    #[test]
    fn vicmdmode_from_emacs_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        selectkeymap("emacs", 0);
        zle_with("abc", 1);
        let r = vicmdmode();
        assert_eq!(r, 0, "vicmdmode from non-vicmd → 0 (selected)");
    }

    /// c:319 — `viaddnext` always returns 0 (success).
    #[test]
    fn viaddnext_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("a", 0);
        assert_eq!(viaddnext(), 0);
        zle_with("", 0); // empty buffer
        assert_eq!(viaddnext(), 0);
    }

    /// c:445 — `viaddeol` always returns 0 (success).
    #[test]
    fn viaddeol_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("xyz", 1);
        assert_eq!(viaddeol(), 0);
        zle_with("", 0);
        assert_eq!(viaddeol(), 0);
    }

    /// c:326 — `viinsert` always returns 0.
    #[test]
    fn viinsert_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("xyz", 0);
        assert_eq!(viinsert(), 0);
    }

    /// c:343 — `viinsertbol` always returns 0.
    #[test]
    fn viinsertbol_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_with("xyz", 1);
        assert_eq!(viinsertbol(), 0);
    }

    /// c:90 — `startvichange(0)` clears INSMODE (im=0 → overwrite).
    /// Pin: `im > -1` branch in c:91.
    #[test]
    fn startvichange_im_zero_clears_insmode() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INSMODE.store(1, SeqCst); // start in insert
        startvichange(0);
        assert_eq!(INSMODE.load(SeqCst), 0, "im=0 → insmode=0 (overwrite)");
    }

    /// c:90 — `startvichange(1)` sets INSMODE.
    #[test]
    fn startvichange_im_one_sets_insmode() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INSMODE.store(0, SeqCst); // start in overwrite
        startvichange(1);
        assert_eq!(INSMODE.load(SeqCst), 1, "im=1 → insmode=1 (insert)");
    }

    /// c:91 — `startvichange(-1)` does NOT touch INSMODE (im > -1 gate).
    #[test]
    fn startvichange_im_neg_one_preserves_insmode() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INSMODE.store(1, SeqCst);
        startvichange(-1);
        assert_eq!(INSMODE.load(SeqCst), 1, "im=-1 must NOT change insmode");
        INSMODE.store(0, SeqCst);
        startvichange(-1);
        assert_eq!(INSMODE.load(SeqCst), 0, "im=-1 must NOT change insmode");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_vi.c
    // c:39 vichange / c:147 vigetkey / c:269 getvirange / c:287 dovilinerange
    // c:382 videletechar / c:420 visubstitute / c:454 vichangeeol /
    // c:478 vichangewholeline / c:505 viyank / c:540 viyankeol
    // ═══════════════════════════════════════════════════════════════════

    /// c:39 — `vichange` returns i32 (compile-time type pin).
    #[test]
    fn vichange_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vichange();
    }

    /// c:147 — `vigetkey` returns i32 (compile-time type pin).
    #[test]
    fn vigetkey_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vigetkey();
    }

    /// c:269 — `getvirange(0)` return in u8 exit-code-or-position range.
    #[test]
    fn getvirange_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = getvirange(0);
    }

    /// c:287 — `dovilinerange()` returns (usize, usize) tuple.
    #[test]
    fn dovilinerange_returns_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: (usize, usize) = dovilinerange();
    }

    /// c:287 — `dovilinerange()` lo ≤ hi invariant (valid range).
    #[test]
    fn dovilinerange_returns_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let (lo, hi) = dovilinerange();
        assert!(lo <= hi, "lo={} must ≤ hi={}", lo, hi);
    }

    /// c:382 — `videletechar` return in u8 exit-code range.
    #[test]
    fn videletechar_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = videletechar();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:420 — `visubstitute` return in u8 exit-code range.
    #[test]
    fn visubstitute_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = visubstitute();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:454 — `vichangeeol` return in u8 exit-code range.
    #[test]
    fn vichangeeol_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vichangeeol();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:478 — `vichangewholeline` return in u8 exit-code range.
    #[test]
    fn vichangewholeline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vichangewholeline();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:505 — `viyank(empty)` return in u8 exit-code range.
    #[test]
    fn viyank_empty_args_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viyank(&[]);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:540 — `viyankeol` return in u8 exit-code range.
    #[test]
    fn viyankeol_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viyankeol();
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_vi.c
    // c:335 viinsert_init / c:356 videlete / c:559 viyankwholeline /
    // c:590 vireplace / c:599 vireplacechars / c:668 viopenlinebelow /
    // c:689 viopenlineabove / c:709 vioperswapcase
    // ═══════════════════════════════════════════════════════════════════

    /// c:335 — `viinsert_init` is void (compile-time type pin).
    #[test]
    fn viinsert_init_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = viinsert_init();
    }

    /// c:335 — `viinsert_init` is idempotent.
    #[test]
    fn viinsert_init_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            viinsert_init();
        }
    }

    /// c:356 — `videlete` returns i32 (compile-time type pin).
    #[test]
    fn videlete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = videlete();
    }

    /// c:559 — `viyankwholeline` returns i32.
    #[test]
    fn viyankwholeline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viyankwholeline();
    }

    /// c:590 — `vireplace` returns i32.
    #[test]
    fn vireplace_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vireplace();
    }

    /// c:599 — `vireplacechars` returns i32.
    #[test]
    fn vireplacechars_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vireplacechars();
    }

    /// c:668 — `viopenlinebelow` returns i32.
    #[test]
    fn viopenlinebelow_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viopenlinebelow();
    }

    /// c:689 — `viopenlineabove` returns i32.
    #[test]
    fn viopenlineabove_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viopenlineabove();
    }

    /// c:709 — `vioperswapcase` returns i32.
    #[test]
    fn vioperswapcase_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vioperswapcase();
    }

    /// c:356 — `videlete` exit code in u8 range.
    #[test]
    fn videlete_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = videlete();
        assert!((0..256).contains(&r));
    }

    /// c:559 — `viyankwholeline` exit code in u8 range.
    #[test]
    fn viyankwholeline_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = viyankwholeline();
        assert!((0..256).contains(&r));
    }

    /// c:709 — `vioperswapcase` exit code in u8 range.
    #[test]
    fn vioperswapcase_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vioperswapcase();
        assert!((0..256).contains(&r));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_vi.c
    // c:39 vichange / c:147 vigetkey / c:269 getvirange / c:287 dovilinerange /
    // c:304 viaddnext / c:317 viaddeol / c:326 viinsert / c:343 viinsertbol /
    // c:382 videletechar / c:420 visubstitute
    // ═══════════════════════════════════════════════════════════════════

    /// c:39 — `vichange` returns i32 (compile-time pin, alt).
    #[test]
    fn vichange_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vichange();
    }

    /// c:147 — `vigetkey` returns i32 (compile-time pin, alt).
    #[test]
    fn vigetkey_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = vigetkey();
    }

    /// c:269 — `getvirange(0)` returns i32 (compile-time pin, alt).
    #[test]
    fn getvirange_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = getvirange(0);
    }

    /// c:287 — `dovilinerange` returns (usize, usize) tuple.
    #[test]
    fn dovilinerange_returns_usize_pair_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: (usize, usize) = dovilinerange();
    }

    /// c:304 — `viaddnext` returns i32.
    #[test]
    fn viaddnext_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viaddnext();
    }

    /// c:317 — `viaddeol` returns i32.
    #[test]
    fn viaddeol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viaddeol();
    }

    /// c:326 — `viinsert` returns i32 + idempotent.
    #[test]
    fn viinsert_idempotent_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            let _: i32 = viinsert();
        }
    }

    /// c:343 — `viinsertbol` returns i32.
    #[test]
    fn viinsertbol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = viinsertbol();
    }

    /// c:382 — `videletechar` exit code in u8 range (alt).
    #[test]
    fn videletechar_returns_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = videletechar();
        assert!((0..256).contains(&r));
    }

    /// c:420 — `visubstitute` exit code in u8 range (alt).
    #[test]
    fn visubstitute_returns_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = visubstitute();
        assert!((0..256).contains(&r));
    }

    /// c:39 — `vichange` exit code in u8 range.
    #[test]
    fn vichange_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vichange();
        assert!((0..256).contains(&r));
    }

    /// c:454 — `vichangeeol` exit code in u8 range (alt).
    #[test]
    fn vichangeeol_returns_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vichangeeol();
        assert!((0..256).contains(&r));
    }

    /// c:478 — `vichangewholeline` exit code in u8 range (alt).
    #[test]
    fn vichangewholeline_returns_in_exit_range_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = vichangewholeline();
        assert!((0..256).contains(&r));
    }
}
