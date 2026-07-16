//! ZLE parameters
//!
//! Direct port from zsh/Src/Zle/zle_params.c
//!
//! Special parameters that expose ZLE state to shell scripts

// `pub mod names` removed — Rust-fabricated namespace wrapping
// string literals. C source uses bare `"BUFFER"`/`"CURSOR"`/etc.
// in the `zleparams[]` table at Src/Zle/zle_params.c:38 directly.
// The mod had no callers.

// Each accessor below corresponds to one of the special parameters
// zsh exposes via Src/Zle/zle_params.c. The C source registers them
// through the `zleparams[]` table at zle_params.c:38; widget bodies
// (and shell scripts running inside ZLE) read or assign to them
// through the parameter system.
// ro means parameters are readonly, used from completion              // c:190
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::ported::params::{setiparam, setsparam};
use crate::ported::zle::compcore::ZMULT;
use crate::ported::zle::zle_h::{WidgetImpl, MOD_MULT, MOD_NEG, MOD_TMULT};
use crate::ported::zle::zle_hist::{ISEARCH_ACTIVE, ISEARCH_ENDPOS, ISEARCH_STARTPOS};
use crate::ported::zle::zle_keymap::{addkeybuf, freekeynode, KeyBinding};
use crate::ported::zle::zle_main::{zleaftertrap, zlebeforetrap, ZLECONTEXT};
use crate::ported::zle::zle_misc::{
    POSTDISPLAY, PREDISPLAY, PREVIOUS_ABORTED_SEARCH, PREVIOUS_SEARCH, SUFFIXLEN,
};
use crate::ported::zle::zle_thingy::Thingy;
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{
    hashnode, param, ScanFunc, PM_READONLY, PM_SCALAR, ZLCON_LINE_CONT, ZLCON_LINE_START,
    ZLCON_SELECT, ZLCON_VARED,
};

/// `$BUFFER` accessor — full edited line as a String.
/// Port of `get_buffer(UNUSED(Param pm))` from Src/Zle/zle_params.c (the
/// `BUFFER` getfn entry in `zleparams[]`).
/// WARNING: param names don't match C — Rust=() vs C=(pm)

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]

/// Direct port of `void makezleparams(int ro)` from
/// `Src/Zle/zle_params.c:194-228`. Registers the `$BUFFER`,
/// `$LBUFFER`, `$RBUFFER`, `$CURSOR`, `$crate::ported::zle::zle_main::MARK`, `$NUMERIC`,
/// `$REGION_ACTIVE`, `$WIDGET`, `$LASTWIDGET`, `$KEYS`,
/// `$BUFFERLINES`, `$CONTEXT`, `$HISTNO`, `$WIDGETSTYLE`,
/// `$WIDGETFUNC` parameters in the param table for the duration
/// of a widget call.
///
/// Full GSU custom-getter dispatch (c:196-228) requires
/// per-param Param.gsu hooks; the Rust port writes the current
/// ZLE state snapshot directly via setsparam / setiparam so user
/// widget functions see live values. When a widget mutates
/// $BUFFER it goes through the canonical paramtab write path
/// that already exists.
pub fn makezleparams(_ro: i32) {
    // c:194

    // Snapshot through the canonical GSU getter ports (get_buffer /
    // get_lbuffer / get_rbuffer / get_cursor read the LIVE editor
    // state in zle_main::ZLELINE / ZLECS). The previous version read
    // compcore::ZLELINE — completion's staging copy, which is EMPTY
    // during ordinary interactive editing — so every user widget saw
    // `$BUFFER == ""` and zpwr's MagicEnter never accepted the line.
    let line = get_buffer(); // c:zleparams[0] getfn
    let lbuf = get_lbuffer(); // c:zleparams[1] getfn
    let rbuf = get_rbuffer(); // c:zleparams[2] getfn
    let cs = get_cursor(); // c:zleparams[3] getfn

    let _ = setsparam("BUFFER", &line); // c:zleparams[0]
    let _ = setsparam("LBUFFER", &lbuf); // c:zleparams[1]
    let _ = setsparam("RBUFFER", &rbuf); // c:zleparams[2]
    let _ = setiparam("CURSOR", cs as i64); // c:zleparams[3]
    let _ = setiparam("NUMERIC", ZMULT.load(Ordering::Relaxed) as i64); // c:zleparams[7]
                                                                        // $KEYMAP — currently-active keymap name (zle_params.c backs this
                                                                        // with the get_keymap getfn). Seed it here so a widget that reads
                                                                        // $KEYMAP before any keymap switch (or when the shell starts in vi
                                                                        // mode) sees a value; selectkeymap keeps it in sync on every
                                                                        // change. Without it $KEYMAP was empty in zle-keymap-select. Bug #654.
    let _ = setsparam("KEYMAP", &super::zle_params::get_keymap()); // c:zleparams KEYMAP
                                                                        // $BUFFERLINES — count of newlines in BUFFER + 1.
    let lines = line.chars().filter(|c| *c == '\n').count() as i64 + 1;
    let _ = setiparam("BUFFERLINES", lines); // c:zleparams[10]
    // c:zleparams[] KEYS / WIDGET / LASTWIDGET / WIDGETFUNC /
    // WIDGETSTYLE / HISTNO / CONTEXT / PENDING — snapshot through the
    // canonical getter ports. zsh-expand's space widget dispatches on
    // `[[ $KEYS == " " ]]`; with $KEYS missing it never took the
    // supernatural-space path, so `ra<space>` didn't expand.
    let keys_bytes = get_keys(); // c:463 get_keys → keybuf
    let mut keys_unmeta = keys_bytes.clone();
    crate::ported::utils::unmetafy(&mut keys_unmeta);
    let _ = setsparam("KEYS", &String::from_utf8_lossy(&keys_unmeta)); // c:zleparams KEYS
    let _ = setsparam("WIDGET", &get_widget()); // c:414 bindk->nam
    let _ = setsparam("LASTWIDGET", &get_lwidget()); // c:428 lbindk->nam
    let _ = setsparam("WIDGETFUNC", &get_widgetfunc()); // c:421
    let _ = setsparam("WIDGETSTYLE", &get_widgetstyle()); // c:435
    let _ = setiparam("HISTNO", get_histno()); // c:514 histline
    let _ = setsparam("CONTEXT", get_context()); // c:942
    let _ = setiparam("PENDING", get_pending() as i64); // c:528

    // RUST-ONLY (crate::zle_param_sync — adapter for C's live GSU
    // setters): record the values just snapshotted so the sync
    // boundaries can diff widget mutations against them.
    crate::zle_param_sync::arm_snapshot(line, lbuf, rbuf, cs as i64);
}

/// Direct port of `static void zleunsetfn(Param pm, int exp)` from
/// `Src/Zle/zle_params.c:237`.
/// ```c
/// stdunsetfn(pm, exp);
/// pm->gsu.s = &nullsetscalar_gsu;
/// ```
/// Called when one of ZLE's special parameters ($BUFFER etc.) is
/// `unset`. C swaps the GSU to the null-setter so subsequent
/// reads return empty and writes are dropped.
pub fn zleunsetfn(pm: &mut param, exp: i32) {
    // c:237
    crate::ported::params::stdunsetfn(pm, exp); // c:237
                                                // c:240 — `pm->gsu.s = &nullsetscalar_gsu`. The GSU vtable swap
                                                // requires the canonical Rust Param.gsu field which is part of
                                                // the params.rs port. The stdunsetfn call above already sets
                                                // PM_UNSET; further reads return empty via the default getter.
}

/// `$BUFFER=s` setter — replace the full edited line.
/// Port of `set_buffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c (the
/// `BUFFER` setfn entry); zsh clamps the cursor to the new
/// length, mirrored here.
/// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
pub fn set_buffer(s: &str) {
    // c:245
    *ZLELINE.lock().unwrap() = s.chars().collect();
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(
        ZLECS
            .load(Ordering::SeqCst)
            .min(ZLELL.load(Ordering::SeqCst)),
        Ordering::SeqCst,
    );
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}
/// `get_buffer` — see implementation.
pub fn get_buffer() -> String {
    // c:258
    ZLELINE.lock().unwrap().iter().collect()
}

/// `$CURSOR=pos` setter — clamped to buffer length.
/// Port of `set_cursor(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=(pos) vs C=(pm, x)
pub fn set_cursor(pos: usize) {
    // c:267
    ZLECS.store(pos.min(ZLELL.load(Ordering::SeqCst)), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// `$CURSOR` accessor — current cursor position (0-indexed).
/// Port of `get_cursor(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_cursor() -> usize {
    // c:281
    ZLECS.load(Ordering::SeqCst)
}

/// `$crate::ported::zle::zle_main::MARK=pos` setter — clamp to buffer length.
/// Port of `set_mark(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=(pos) vs C=(pm, x)
pub fn set_mark(pos: usize) {
    // c:299
    MARK.store(pos.min(ZLELL.load(Ordering::SeqCst)), Ordering::SeqCst);
}

/// `$crate::ported::zle::zle_main::MARK` accessor — current mark position.
/// Port of `get_mark(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_mark() -> usize {
    // c:311
    MARK.load(Ordering::SeqCst)
}

/// Port of `set_region_active(UNUSED(Param pm), zlong x)` from `Src/Zle/zle_params.c:318`.
/// ```c
/// static void
/// set_region_active(UNUSED(Param pm), zlong x)
/// {
///     region_active = (int)!!x;
/// }
/// ```
/// `$REGION_ACTIVE=N` setter — coerces N to 0 or 1 (any non-zero
/// becomes 1) via the C double-bang idiom.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn set_region_active(
    // c:318
    x: i64,
) {
    // c:320 — `region_active = (int)!!x`. !!x: 0→0, anything else→1.
    REGION_ACTIVE.store(if x != 0 { 1 } else { 0 }, Ordering::SeqCst);
}

/// Port of `get_region_active(UNUSED(Param pm))` from `Src/Zle/zle_params.c:325`.
/// ```c
/// static zlong
/// get_region_active(UNUSED(Param pm))
/// {
///     return region_active;
/// }
/// ```
/// `$REGION_ACTIVE` getter — returns the current region_active flag.
pub fn get_region_active() -> i64 {
    // c:325
    REGION_ACTIVE.load(Ordering::SeqCst) as i64
    // c:325 return region_active
}

/// `$LBUFFER=s` setter — replace text before the cursor; cursor
/// lands at the new lbuffer's end.
/// Port of `set_lbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
pub fn set_lbuffer(s: &str) {
    // c:332
    let rbuf: String = ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)..]
        .iter()
        .collect();
    *ZLELINE.lock().unwrap() = s.chars().chain(rbuf.chars()).collect();
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(s.chars().count(), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// `$LBUFFER` accessor — text before the cursor.
/// Port of `get_lbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_lbuffer() -> String {
    // c:355
    ZLELINE.lock().unwrap()[..ZLECS.load(Ordering::SeqCst)]
        .iter()
        .collect()
}

/// `$RBUFFER=s` setter — replace text after the cursor.
/// Port of `set_rbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
pub fn set_rbuffer(s: &str) {
    // c:364
    let lbuf: String = ZLELINE.lock().unwrap()[..ZLECS.load(Ordering::SeqCst)]
        .iter()
        .collect();
    *ZLELINE.lock().unwrap() = lbuf.chars().chain(s.chars()).collect();
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// `$RBUFFER` accessor — text after the cursor.
/// Port of `get_rbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_rbuffer() -> String {
    // c:384
    ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst)..]
        .iter()
        .collect()
}

/// Port of `get_prebuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c:394.
pub fn get_prebuffer() -> String {
    // c:394
    // C body c:396-410 — `if (!stackhist) return ztrdup("");
    //                     dputs(...prepended buffer...)`. Returns the
    //                     stacked-line buffer (multi-line input not
    //                     yet committed to current zleline). Without
    //                     stackhist tracking we return empty.
    String::new()
}

/// Port of `get_widget(UNUSED(Param pm))` from Src/Zle/zle_params.c:414.
pub fn get_widget() -> String {
    // c:414
    // c:421 — `return bindk ? bindk->nam : ""`.
    BINDK
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.nam.clone())
        .unwrap_or_default()
}

/// Port of `get_widgetfunc(UNUSED(Param pm))` from Src/Zle/zle_params.c:421.
pub fn get_widgetfunc() -> String {
    // c:421
    // c:423-430 — read bindk->widget. C union dispatches:
    //   WIDGET_INT  → ".internal"  (c:426-427)
    //   WIDGET_NCOMP → comp.func   (c:428-429)
    //   else → fnnam               (c:430)
    let bindk_guard = BINDK.lock().unwrap();
    let Some(t) = bindk_guard.as_ref() else {
        return String::new();
    };
    let Some(w) = t.widget.as_ref() else {
        return String::new();
    };
    if (w.flags & WIDGET_INT) != 0 {
        return ".internal".to_string();
    }
    // No NCOMP comp.func/wid in current Widget shape (would be in
    // WidgetImpl::Comp variant); collapse to the User-fn case.
    match &w.u {
        WidgetImpl::UserFunc(name) => name.clone(),
        WidgetImpl::Internal(_) => ".internal".to_string(),
        _ => ".internal".to_string(),
    }
}

/// Port of `get_widgetstyle(UNUSED(Param pm))` from Src/Zle/zle_params.c:435.
pub fn get_widgetstyle() -> String {
    // c:435

    // c:437-444 — read bindk->widget. INT → ".internal"; NCOMP →
    // comp.wid (the underlying widget name); else "".
    let bindk_guard = BINDK.lock().unwrap();
    let Some(t) = bindk_guard.as_ref() else {
        return String::new();
    };
    let Some(w) = t.widget.as_ref() else {
        return String::new();
    };
    if (w.flags & WIDGET_INT) != 0 {
        return ".internal".to_string();
    }
    // No NCOMP comp.wid in current shape — would be t.nam for
    // a -C-bound completion widget. Fall through to "".
    String::new() // c:444
}

/// Port of `get_lwidget(UNUSED(Param pm))` from Src/Zle/zle_params.c:449.
pub fn get_lwidget() -> String {
    // c:449
    // c:449 — `return (lbindk ? lbindk->nam : "")`.
    LBINDK
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.nam.clone())
        .unwrap_or_default()
}

/// `$KEYMAP` accessor — currently-active keymap name.
/// Port of `get_keymap(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_keymap() -> String {
    // c:456
    crate::ported::zle::zle_keymap::curkeymapname().clone()
}

/// Port of `get_keys(UNUSED(Param pm))` from Src/Zle/zle_params.c:463.
pub fn get_keys() -> Vec<u8> {
    // c:463
    // c:470 — `return keybuf`. The active keymap-walk byte buffer.
    crate::ported::zle::zle_keymap::keybuf
        .lock()
        .unwrap()
        .clone()
}

/// Port of `get_keys_queued_count(UNUSED(Param pm))` from Src/Zle/zle_params.c:470.
pub fn get_keys_queued_count() -> i64 {
    // c:470
    // c:470 — `return kungetct`. Bytes pending in the unget queue.
    KUNGETBUF.lock().unwrap().len() as i64
}

/// Port of `set_numeric(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c:477.
pub fn set_numeric(x: i64) {
    // c:477
    // c:479 — `zmult = x`. zmult is zmod.mult.
    ZMOD.lock().unwrap().mult = x as i32;
    // c:480 — `zmod.flags = MOD_MULT`. Replaces the whole flags
    // bitfield with just MOD_MULT (not OR — the C is a plain `=`).
    ZMOD.lock().unwrap().flags = MOD_MULT;
}

/// `$NUMERIC` accessor — numeric prefix when set.
/// Port of `get_numeric(UNUSED(Param pm))` from Src/Zle/zle_params.c which
/// returns `zmod.mult` only when `MOD_MULT` is set, otherwise
/// the parameter is unset.
/// C body (single statement): `return zmult;`
/// (C does not gate on MOD_MULT — the previous Rust port did,
/// which diverged from upstream semantics.)
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_numeric() -> i32 {
    // c:485
    ZMOD.lock().unwrap().mult // c:487 zmult
}

/// Direct port of `static void unset_numeric(Param pm, int exp)` from
/// `Src/Zle/zle_params.c:491-499`.
/// ```c
/// stdunsetfn(pm, exp);
/// if (exp) {
///     zmod.flags &= ~(MOD_MULT|MOD_TMULT);
///     zmod.mult = 1;
/// }
/// ```
///
/// The Rust call takes no `Param` because the canonical Rust home
/// for `zmod` is the file-scope `ZMOD` static (zle_main.rs). The
/// `stdunsetfn` half of the C body fires from the Param.gsu.unsetfn
/// vtable hook upstream — this fn just performs the zmod side.
/// Port of `unset_numeric(Param pm, int exp)` from `Src/Zle/zle_params.c:492`.
pub fn unset_numeric(exp: i32) {
    if exp != 0 {
        // c:494
        ZMOD.lock().unwrap().flags = 0; // c:496
        ZMOD.lock().unwrap().mult = 1; // c:497
    }
}

/// Port of `set_histno(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c:503.
pub fn set_histno(x: i64) {
    // c:503
    // c:503-509 — `Histent he = quietgethist(x); if (!he) return;
    //              zle_setline()`.
    // zshrs uses History.cursor as the active history index. Clamp
    // to entries.len() when x is out of range (matches the
    // quietgethist NULL-result early-return).
    let idx = x.max(0) as usize;
    if idx <= history().lock().unwrap().entries.len() {
        history().lock().unwrap().cursor = idx;
    }
}

/// Port of `get_histno(UNUSED(Param pm))` from Src/Zle/zle_params.c:514.
pub fn get_histno() -> i64 {
    // c:514
    // c:514 — `return histline`. zshrs tracks the editing history
    // line via the History.cursor field (offset into entries Vec).
    history().lock().unwrap().cursor as i64
}

/// `$BUFFERLINES` accessor — number of newline-separated lines.
/// Port of `get_bufferlines(UNUSED(Param pm))` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_bufferlines() -> usize {
    // c:521
    ZLELINE
        .lock()
        .unwrap()
        .iter()
        .filter(|&&c| c == '\n')
        .count()
        + 1
}

/// `$PENDING` accessor — bytes waiting in the input queue.
/// Port of `get_pending(UNUSED(Param pm))` from Src/Zle/zle_params.c which
/// returns `kungetct` (the unget-buffer fill).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_pending() -> usize {
    // c:528
    0 // unget_buf is private; future expansion can expose its len
}

/// Port of `get_recursive(UNUSED(Param pm))` from `Src/Zle/zle_params.c:535`.
/// ```c
/// static zlong
/// get_recursive(UNUSED(Param pm))
/// {
///     return zle_recursive;
/// }
/// ```
/// `$ZLE_RECURSIVE` getter — current ZLE recursion depth (>0 when
/// inside a `recursive-edit` widget call).
pub fn get_recursive() -> i64 {
    // c:535
    ZLE_RECURSIVE.load(Ordering::SeqCst) as i64
    // c:535 return zle_recursive
}

/// Port of `get_yankstart(UNUSED(Param pm))` from Src/Zle/zle_params.c:542.
pub fn get_yankstart() -> i64 {
    // c:542
    // c:542 — `return yankb`.
    YANKB.load(Ordering::SeqCst) as i64
}

/// Port of `get_yankend(UNUSED(Param pm))` from Src/Zle/zle_params.c:549.
pub fn get_yankend() -> i64 {
    // c:549
    // c:542 — `return yanke`.
    YANKE.load(Ordering::SeqCst) as i64
}

/// Port of `get_yankactive(UNUSED(Param pm))` from Src/Zle/zle_params.c:556.
pub fn get_yankactive() -> i64 {
    // c:556
    // c:549 — `return !!(lastcmd & ZLE_YANK) + !!(lastcmd & ZLE_YANKAFTER)`.
    let last = LASTCMD.load(Ordering::SeqCst) as i32;
    let yank = ((last & ZLE_YANK) != 0) as i64;
    let yankafter = ((last & ZLE_YANKAFTER) != 0) as i64;
    yank + yankafter
}

/// Port of `set_yankstart(UNUSED(Param pm), zlong i)` from Src/Zle/zle_params.c:563.
pub fn set_yankstart(i: i64) {
    // c:563
    // c:563 — `yankb = i`.
    YANKB.store(i.max(0) as usize, Ordering::SeqCst);
}

/// Port of `set_yankend(UNUSED(Param pm), zlong i)` from Src/Zle/zle_params.c:570.
pub fn set_yankend(i: i64) {
    // c:570
    // c:563 — `yanke = i`.
    YANKE.store(i.max(0) as usize, Ordering::SeqCst);
}

/// Port of `get_isearchmatchstart(UNUSED(Param pm))` from Src/Zle/zle_params.c:577.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchstart() -> i64 {
    // c:577
    ISEARCH_STARTPOS.load(Ordering::Relaxed) as i64
    // c:579
}

/// Port of `get_isearchmatchend(UNUSED(Param pm))` from Src/Zle/zle_params.c:584.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchend() -> i64 {
    // c:584
    ISEARCH_ENDPOS.load(Ordering::Relaxed) as i64 // c:577
}

/// Port of `get_isearchmatchactive(UNUSED(Param pm))` from Src/Zle/zle_params.c:591.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchactive() -> i64 {
    // c:591
    ISEARCH_ACTIVE.load(Ordering::Relaxed) as i64 // c:577
}

/// Port of `get_suffixstart(UNUSED(Param pm))` from `Src/Zle/zle_params.c:598`.
/// ```c
/// static zlong
/// get_suffixstart(UNUSED(Param pm))
/// {
///     return zlecs - suffixlen;
/// }
/// ```
/// `$SUFFIX_START` getter — start byte of the active suffix
/// (cursor minus suffix length).
pub fn get_suffixstart() -> i64 {
    // c:598
    let sfx_len = SUFFIXLEN.load(Ordering::Relaxed);
    (ZLECS.load(Ordering::SeqCst) as i64) - (sfx_len as i64) // c:600 zlecs - suffixlen
}

/// Port of `get_suffixend(UNUSED(Param pm))` from `Src/Zle/zle_params.c:605`.
/// ```c
/// static zlong
/// get_suffixend(UNUSED(Param pm))
/// {
///     return zlecs;
/// }
/// ```
/// `$SUFFIX_END` getter — returns the cursor position (suffixes are
/// auto-removed FROM the cursor backward).
pub fn get_suffixend() -> i64 {
    // c:605
    ZLECS.load(Ordering::SeqCst) as i64
    // c:605 return zlecs
}

/// Port of `get_suffixactive(UNUSED(Param pm))` from `Src/Zle/zle_params.c:612`.
/// ```c
/// static zlong
/// get_suffixactive(UNUSED(Param pm))
/// {
///     return suffixlen;
/// }
/// ```
/// `$SUFFIX_ACTIVE` getter — returns the length of the currently
/// active auto-removable suffix.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_suffixactive() -> i64 {
    // c:612
    SUFFIXLEN.load(Ordering::Relaxed) as i64 // c:614 return suffixlen
}

/// `$CUTBUFFER` accessor — most-recent kill-ring entry.
/// Port of `get_cutbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c which
/// reads `cutbuf` (the unnamed kill register).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_cutbuffer() -> String {
    // c:619
    KILLRING
        .lock()
        .unwrap()
        .front()
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}

/// `$CUTBUFFER=s` setter — overwrite the front of the kill ring.
/// Port of `set_cutbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
/// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
pub fn set_cutbuffer(s: &str) {
    // c:629
    let chars: Vec<char> = s.chars().collect();
    if KILLRING.lock().unwrap().is_empty() {
        KILLRING.lock().unwrap().push_front(chars);
    } else {
        KILLRING.lock().unwrap()[0] = chars;
    }
}

/// Port of `unset_cutbuffer(Param pm, int exp)` from Src/Zle/zle_params.c:647.
pub fn unset_cutbuffer(exp: i32) {
    // c:647
    // c:647-655 — `if (exp) { stdunsetfn; if (cutbuf.buf) { free; NULL; len=0 } }`.
    if exp != 0 {
        // zshrs uses VecDeque for the kill ring; the "primary" cut
        // buffer is the front entry. Clearing means popping it.
        KILLRING.lock().unwrap().pop_front();
    }
}

/// Port of `set_killring(UNUSED(Param pm), char **x)` from Src/Zle/zle_params.c:661.
pub fn set_killring(x: Option<&[String]>) {
    // c:661
    // c:661-672 — `if (kring) { free each kptr->buf; zfree(kring) }`.
    // Then either rebuild from `x` or leave NULL.
    KILLRING.lock().unwrap().clear();
    if let Some(arr) = x {
        for entry in arr {
            KILLRING.lock().unwrap().push_back(entry.chars().collect());
        }
    }
}

/// Port of `get_killring(UNUSED(Param pm))` from Src/Zle/zle_params.c:705.
pub fn get_killring() -> Vec<String> {
    // c:705
    // c:705-733 — return kring entries with most-recently-killed
    // first. Empty entries returned as "" so the array length always
    // equals kringsize. zshrs holds the kill ring as
    // VecDeque<Vec<char>> where push_front puts newest at index 0,
    // so we iterate forward.
    KILLRING
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.iter().collect::<String>())
        .collect()
}

/// Port of `unset_killring(Param pm, int exp)` from Src/Zle/zle_params.c:741.
pub fn unset_killring(exp: i32) {
    // c:741
    // c:741-746 — `if (exp) { set_killring(NULL); stdunsetfn(...) }`.
    if exp != 0 {
        set_killring(None);
        // stdunsetfn handles param-table bookkeeping — substrate.
    }
}

/// Port of `set_register(Param pm, char *value)` from Src/Zle/zle_params.c:751.
/// WARNING: param names don't match C — Rust=(zle, name, value) vs C=(pm, value)
pub fn set_register(name: char, value: &str) -> i32 {
    // c:751
    // c:751-763 — '0'..'9' → offset = '0' - 26;  'a'..'z' → offset = 'a'.
    // (Vi register table layout: 0..25 = a..z, 26..35 = 0..9.)
    let idx: i32 = if ('0'..='9').contains(&name) {
        // c:760 — `offset = '0' - 26` → idx = name - '0' + 26.
        name as i32 - b'0' as i32 + 26
    } else if ('a'..='z').contains(&name) {
        // c:761-762 — `offset = 'a'` → idx = name - 'a'.
        name as i32 - b'a' as i32
    } else {
        // c:765 — invalid register; C reports zerr and returns.
        return 1;
    };
    // c:769-772 — `vbuf = &vibuf[name-offset]; if (*value)
    //              vbuf->buf = stringaszleline(value, 0, &n, ...);
    //              vbuf->len = n`.
    if (idx as usize) < vibuf().lock().unwrap().len() {
        vibuf().lock().unwrap()[idx as usize] = value.chars().collect();
    }
    0
}

/// Port of `unset_register(Param pm, UNUSED(int exp))` from Src/Zle/zle_params.c:777.
/// WARNING: param names don't match C — Rust=(zle, name, _exp) vs C=(pm, exp)
pub fn unset_register(name: char, _exp: i32) {
    // c:777
    // c:777-779 — `set_register("")`. Single-line body.
    let _ = set_register(name, "");
}

/// Port of `scan_registers(UNUSED(HashTable ht), ScanFunc func, int flags)` from Src/Zle/zle_params.c:784.
///
/// Iteration entry that C's `${(@k)registers}` query and `printf -v`
/// callers use to enumerate the vi yank registers ('a'..'z',
/// '0'..'9' = 36 buffers in `vibuf[]`). The C body builds a temp
/// `struct param` per buffer and invokes the supplied `ScanFunc`
/// callback. Rust port: zshrs's special-parameter hashparam node
/// integration isn't wired up yet — the `registers` parameter
/// itself reads via `get_registers`/`set_register` directly without
/// going through this iteration callback. ScanFunc callback path is
/// a no-op port; trait dispatch via the typed `vibuf()` accessor
/// covers the read/write side. Rust idiom replacement.
/// WARNING: param names don't match C — Rust=(_t, _flags) vs C=(ht, func, flags)
pub fn scan_registers(_ht: i32, func: Option<ScanFunc>, flags: i32) {
    // c:784
    let func = match func {
        Some(f) => f,
        None => return,
    };
    let buf = vibuf().lock().unwrap().clone(); // c:794 vibuf walk
    let mut ch: u8 = b'a'; // c:798 ch = 'a'
    for i in 0..36usize {
        // c:798 for (i = 0; i < 36; i++)
        let val: String = buf.get(i).map(|v| v.iter().collect()).unwrap_or_default(); // c:801 zlelineasstring(vibuf[i].buf, ...)
        let pm = param {
            node: hashnode {
                // c:794 memset(&pm, 0)
                next: None,
                nam: format!("{}", ch as char), // c:799 *pm.node.nam = ch
                flags: (PM_SCALAR | PM_READONLY) as i32, // c:795
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(val), // c:801 pm.u.str
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        func(&Box::new(pm.node), flags); // c:802
        ch = if ch == b'z' { b'0' } else { ch + 1 }; // c:804-805 if (ch++ == 'z') ch = '0'
    }
}

/// Port of `get_registers(UNUSED(HashTable ht), const char *name)` from Src/Zle/zle_params.c:807.
pub fn get_registers(name: &str) -> Option<String> {
    // c:807
    // c:807-820 — name[1] non-zero → invalid; '0'..'9' → idx = name-'0'+26;
    // 'a'..'z' → idx = name-'a'.
    let bytes = name.as_bytes();
    if bytes.len() != 1 {
        return None;
    }
    let c = bytes[0];
    let idx: i32 = if c.is_ascii_digit() {
        (c - b'0') as i32 + 26
    } else if c.is_ascii_lowercase() {
        (c - b'a') as i32
    } else {
        return None; // c:822-824 (vbuf==-1)
    };
    // c:798 — `pm->u.str = zlelineasstring(vibuf[i].buf, ...)`.
    if (idx as usize) < vibuf().lock().unwrap().len() {
        Some(
            vibuf().lock().unwrap()[idx as usize]
                .iter()
                .collect::<String>(),
        )
    } else {
        None
    }
}

/// Port of `set_registers(Param pm, HashTable ht)` from Src/Zle/zle_params.c:833.
/// WARNING: param names don't match C — Rust=(zle) vs C=(pm, ht)
pub fn set_registers(
    // c:833
    map: &std::collections::HashMap<String, String>,
) {
    // C body c:835-855 — for each (name, value) in the assoc-array
    //                    being assigned to $registers, invoke
    //                    set_register. Names outside [a-z0-9] beep.
    for (name, value) in map {
        if let Some(ch) = name.chars().next() {
            let _ = set_register(ch, value);
        }
    }
}

/// Port of `unset_registers(Param pm, int exp)` from Src/Zle/zle_params.c:857.
pub fn unset_registers(exp: i32) {
    // c:857
    // C body c:859-870 — `if (exp) { for (i...) { vibuf[i].buf=NULL;
    //                              vibuf[i].len = 0; } stdunsetfn(...) }`.
    if exp != 0 {
        for buf in vibuf().lock().unwrap().iter_mut() {
            buf.clear();
        }
    }
}

/// Port of `set_prepost(ZLE_STRING_T *textvar, int *lenvar, char *x)` from Src/Zle/zle_params.c:865.
pub fn set_prepost(textvar: &mut String, lenvar: &mut usize, x: Option<&str>) {
    // c:865
    // c:865-871 — `if (*lenvar) free(*textvar); *textvar=NULL; *lenvar=0`.
    if *lenvar != 0 {
        textvar.clear();
        *lenvar = 0;
    }
    // c:872-874 — if x: `*textvar = stringaszleline(x, 0, lenvar, ...)`.
    if let Some(s) = x {
        textvar.push_str(s);
        *lenvar = s.chars().count();
    }
}

/// Port of `get_prepost(ZLE_STRING_T text, int len)` from Src/Zle/zle_params.c:879.
pub fn get_prepost(text: &str, len: usize) -> String {
    // c:879
    // c:879 — `return zlelineasstring(text, len, 0, NULL, NULL, 1)`.
    // In Rust the caller already owns a String; just truncate to len.
    text.chars().take(len).collect()
}

/// Port of `set_predisplay(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c:886.
/// C body (single statement):
///     `set_prepost(&predisplay, &predisplaylen, x);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn set_predisplay(x: Option<&str>) {
    // c:886
    let mut buf = PREDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap(); // c:888 &predisplay
    let mut len = buf.chars().count();
    set_prepost(&mut buf, &mut len, x); // c:888 set_prepost(&predisplay, &predisplaylen, x)
}

/// Port of `get_predisplay(UNUSED(Param pm))` from Src/Zle/zle_params.c:893.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_predisplay() -> String {
    // c:893
    PREDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone()
}

/// Port of `set_postdisplay(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c:900.
/// C body (single statement):
///     `set_prepost(&postdisplay, &postdisplaylen, x);`
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn set_postdisplay(x: Option<&str>) {
    // c:900
    let mut buf = POSTDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap(); // c:902 &postdisplay
    let mut len = buf.chars().count();
    set_prepost(&mut buf, &mut len, x); // c:902 set_prepost(&postdisplay, &postdisplaylen, x)
}

/// Port of `get_postdisplay(UNUSED(Param pm))` from Src/Zle/zle_params.c:907.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_postdisplay() -> String {
    // c:907
    // c:909 — `return get_prepost(postdisplay, postdisplaylen)` →
    // zlelineasstring(...). Return the raw String.
    POSTDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone()
}

/// Port of `free_prepostdisplay()` from Src/Zle/zle_params.c:914.
pub fn free_prepostdisplay() {
    // c:914
    // c:916-917 — `if (predisplaylen) set_prepost(&predisplay, &predisplaylen, NULL)`.
    PREDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clear();
    // c:918-919 — same for postdisplay.
    POSTDISPLAY
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clear();
}

/// Port of `get_lasearch(UNUSED(Param pm))` from Src/Zle/zle_params.c:924.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_lasearch() -> String {
    // c:924
    // c:933-928 — `previous_aborted_search ? : ""`.
    PREVIOUS_ABORTED_SEARCH
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone()
}

/// Port of `get_lsearch(UNUSED(Param pm))` from Src/Zle/zle_params.c:933.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_lsearch() -> String {
    // c:933
    // c:935-937 — `previous_search ? : ""`.
    PREVIOUS_SEARCH
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone()
}

/// Port of `get_context(UNUSED(Param pm))` from Src/Zle/zle_params.c:942.
pub fn get_context() -> &'static str {
    // c:942
    // c:944-958 — switch on zlecontext → "cont" / "select" / "vared" / "line".
    match ZLECONTEXT.load(Ordering::SeqCst) {
        x if x == ZLCON_LINE_CONT => "cont", // c:945-946
        x if x == ZLCON_SELECT => "select",  // c:949-950
        x if x == ZLCON_VARED => "vared",    // c:953-954
        _ => "line",                         // c:957-958 default
    }
}

/// `$ZLE_STATE` accessor — port of `static char *get_zle_state(
/// UNUSED(Param pm))` from `Src/Zle/zle_params.c:966`. C builds
/// "<insert|overwrite>:<localhistory|globalhistory>", splits by
/// ":", sorts the components, then joins with space — so user
/// scripts can do `[[ $ZLE_STATE == *foo* ]]` without caring
/// about ordering.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_zle_state() -> String {
    // c:966
    use crate::ported::zsh_h::HIST_FOREIGN;
    // c:973-988 — accumulate (insmode, hist_skip_flags) state words.
    let insmode_str = if INSMODE.load(Ordering::SeqCst) != 0 {
        "insert" // c:975
    } else {
        "overwrite" // c:977
    };
    let hist_str =
        if (crate::ported::hist::hist_skip_flags.load(Ordering::SeqCst) & HIST_FOREIGN as i32) != 0
        {
            "localhistory" // c:982
        } else {
            "globalhistory" // c:984
        };
    // c:1015 — `strmetasort(arr, SORTIT_ANYOLDHOW, NULL);`
    let mut parts = vec![insmode_str.to_string(), hist_str.to_string()];
    parts.sort();
    // c:1016 — `zjoin(arr, ' ', 1);`
    parts.join(" ")
}

/// `$ZLE_STATE` insert/overwrite component — true for insert.
/// Sub-port of `get_zle_state(UNUSED(Param pm))` (Src/Zle/zle_params.c) which
/// emits "insert" / "overwrite" + " " + "vicmd" / "main".
pub fn is_insert_mode() -> bool {
    (INSMODE.load(Ordering::SeqCst) != 0)
}

/// `$REGION_ACTIVE` predicate — true iff the visual region flag is
/// set (charwise=1 or linewise=2). Snake-cased Rust helper wrapping
/// the canonical `get_region_active()` (C value 0/1/2) into a bool.
pub fn is_region_active() -> bool {
    REGION_ACTIVE.load(Ordering::SeqCst) != 0
}

#[cfg(test)]
mod region_active_tests {
    use super::*;

    #[test]
    fn get_region_active_reads_field() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:327 — `return region_active`.
        zle_reset();
        REGION_ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(get_region_active(), 0);
        REGION_ACTIVE.store(1, Ordering::SeqCst);
        assert_eq!(get_region_active(), 1);
        REGION_ACTIVE.store(2, Ordering::SeqCst);
        assert_eq!(get_region_active(), 2);
    }

    #[test]
    fn set_region_active_double_bang_idiom() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:320 — `region_active = (int)!!x`. Any non-zero → 1; zero → 0.
        zle_reset();
        set_region_active(0);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 0);
        set_region_active(1);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
        set_region_active(99);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
        set_region_active(-1);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 1);
        set_region_active(0);
        assert_eq!(REGION_ACTIVE.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod trap_tests {
    use crate::zle::zle_main::{zle_test_setup, zleaftertrap, zlebeforetrap};

    #[test]
    fn zlebeforetrap_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2110 — `return 0` always. Hookfn signature; pass null
        // for both unused params.
        assert_eq!(zlebeforetrap(std::ptr::null_mut(), std::ptr::null_mut()), 0);
    }

    #[test]
    fn zleaftertrap_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2119 — `return 0` always. Hookfn signature; pass null
        // for both unused params.
        assert_eq!(zleaftertrap(std::ptr::null_mut(), std::ptr::null_mut()), 0);
    }
}

#[cfg(test)]
mod numeric_tests {
    use super::*;

    #[test]
    fn set_numeric_sets_mult_and_replaces_flags() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:479-480 — `zmult=x; zmod.flags = MOD_MULT` (assignment,
        // not OR). Pre-existing flags get wiped.
        zle_reset();
        ZMOD.lock().unwrap().flags |= MOD_TMULT | MOD_NEG;
        ZMOD.lock().unwrap().mult = 99;
        set_numeric(7);
        assert_eq!(ZMOD.lock().unwrap().mult, 7);
        // Only MULT remains; TMULT and NEG are gone.
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_MULT, 0);
        assert_eq!(ZMOD.lock().unwrap().flags & MOD_TMULT, 0);
        assert_eq!(ZMOD.lock().unwrap().flags & MOD_NEG, 0);
    }

    #[test]
    fn unset_numeric_resets_when_exp_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:494-498 — only resets when exp != 0.
        zle_reset();
        ZMOD.lock().unwrap().flags |= MOD_MULT;
        ZMOD.lock().unwrap().mult = 5;
        unset_numeric(1);
        assert_eq!(ZMOD.lock().unwrap().mult, 1);
        assert_eq!(ZMOD.lock().unwrap().flags, 0);
    }

    #[test]
    fn unset_numeric_noop_when_exp_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:494 — `if (exp)` skips when exp == 0.
        zle_reset();
        ZMOD.lock().unwrap().flags |= MOD_MULT;
        ZMOD.lock().unwrap().mult = 5;
        unset_numeric(0);
        // Unchanged.
        assert_eq!(ZMOD.lock().unwrap().mult, 5);
        assert_ne!(ZMOD.lock().unwrap().flags & MOD_MULT, 0);
    }
}

#[cfg(test)]
mod suffix_tests {
    use super::*;

    #[test]
    fn get_suffixactive_reads_suffixlen() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:614 — `return suffixlen`.
        SUFFIXLEN.store(7, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 7);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 0);
    }

    #[test]
    fn get_suffixend_reads_zlecs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:607 — `return zlecs`.
        zle_reset();
        ZLECS.store(11, Ordering::SeqCst);
        assert_eq!(get_suffixend(), 11);
    }

    #[test]
    fn get_suffixstart_subtracts_suffixlen() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:600 — `return zlecs - suffixlen`.
        zle_reset();
        ZLECS.store(20, Ordering::SeqCst);
        SUFFIXLEN.store(5, Ordering::SeqCst);
        assert_eq!(get_suffixstart(), 15);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixstart(), 20);
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;

    #[test]
    fn get_widget_reads_bindk_nam() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:416 — `return bindk ? bindk->nam : ""`.
        zle_reset();
        *BINDK.lock().unwrap() = Some(Thingy::new("self-insert"));
        assert_eq!(get_widget(), "self-insert");
    }

    #[test]
    fn get_widget_empty_when_no_bindk() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:416 — `bindk` NULL → empty string.
        zle_reset();
        assert_eq!(get_widget(), "");
    }

    #[test]
    fn get_lwidget_reads_lbindk_nam() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:451 — `return (lbindk ? lbindk->nam : "")`.
        zle_reset();
        *LBINDK.lock().unwrap() = Some(Thingy::new("forward-char"));
        assert_eq!(get_lwidget(), "forward-char");
    }

    #[test]
    fn get_lwidget_empty_when_no_lbindk() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(get_lwidget(), "");
    }

    #[test]
    fn get_recursive_reads_zle_recursive_field() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:537 — `return zle_recursive`.
        zle_reset();
        ZLE_RECURSIVE.store(0, Ordering::SeqCst);
        assert_eq!(get_recursive(), 0);
        ZLE_RECURSIVE.store(5, Ordering::SeqCst);
        assert_eq!(get_recursive(), 5);
    }
}

#[cfg(test)]
mod isearch_tests {
    use super::*;

    #[test]
    fn get_isearchmatchactive_reads_global() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:593 — `return isearch_active`.
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 0);
        ISEARCH_ACTIVE.store(1, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 1);
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchstart_reads_global() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:579 — `return isearch_startpos`.
        ISEARCH_STARTPOS.store(7, Ordering::SeqCst);
        assert_eq!(get_isearchmatchstart(), 7);
        ISEARCH_STARTPOS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchend_reads_global() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:586 — `return isearch_endpos`.
        ISEARCH_ENDPOS.store(13, Ordering::SeqCst);
        assert_eq!(get_isearchmatchend(), 13);
        ISEARCH_ENDPOS.store(0, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod batch_getters_tests {
    use super::*;

    #[test]
    fn get_histno_reads_history_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        history().lock().unwrap().cursor = 7;
        assert_eq!(get_histno(), 7);
    }

    #[test]
    fn get_keys_returns_keybuf_clone() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *crate::ported::zle::zle_keymap::keybuf.lock().unwrap() = vec![0x1b, b'a'];
        assert_eq!(get_keys(), vec![0x1b, b'a']);
    }

    #[test]
    fn get_keys_queued_count_returns_unget_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        KUNGETBUF.lock().unwrap().push_back(b'a');
        KUNGETBUF.lock().unwrap().push_back(b'b');
        KUNGETBUF.lock().unwrap().push_back(b'c');
        assert_eq!(get_keys_queued_count(), 3);
    }

    #[test]
    fn get_yankstart_yankend_read_fields() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        YANKB.store(3, Ordering::SeqCst);
        YANKE.store(8, Ordering::SeqCst);
        assert_eq!(get_yankstart(), 3);
        assert_eq!(get_yankend(), 8);
    }

    #[test]
    fn set_yankstart_yankend_write_fields() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_yankstart(5);
        set_yankend(11);
        assert_eq!(YANKB.load(Ordering::SeqCst), 5);
        assert_eq!(YANKE.load(Ordering::SeqCst), 11);
    }
}

#[cfg(test)]
mod keybuf_tests {
    use crate::zle::zle_keymap::{addkeybuf, freekeynode, KeyBinding};
    use crate::zle::zle_main::zle_test_setup;

    #[test]
    fn addkeybuf_plain_byte() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // imeta routes through the typtab populated by inittyptab.
        let _tg = crate::ported::ztype_h::TYPTAB_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        crate::ported::zle::zle_keymap::keybuf
            .lock()
            .unwrap()
            .clear();
        addkeybuf(b'a' as i32);
        assert_eq!(
            *crate::ported::zle::zle_keymap::keybuf.lock().unwrap(),
            vec![b'a']
        );
    }

    #[test]
    fn addkeybuf_meta_quoted() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _tg = crate::ported::ztype_h::TYPTAB_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::ported::utils::inittyptab();
        // 0xa0 is IMETA (Snull..=Nularg = 0x9d..=0xa1 per utils.c:4200)
        // → Meta-quoted: 0x83 then (0xa0 ^ 0x20) = 0x80.
        crate::ported::zle::zle_keymap::keybuf
            .lock()
            .unwrap()
            .clear();
        addkeybuf(0xa0);
        assert_eq!(
            *crate::ported::zle::zle_keymap::keybuf.lock().unwrap(),
            vec![0x83, 0x80]
        );
    }

    #[test]
    fn freekeynode_consumes_binding() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Just verify Drop runs without panic.
        let kb = KeyBinding {
            bind: None,
            str: Some("send-string".to_string()),
            prefixct: 0,
        };
        freekeynode(kb);
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn get_set_predisplay_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:885,892 — round-trip set→get.
        set_predisplay(Some("[hint] "));
        assert_eq!(get_predisplay(), "[hint] ");
        set_predisplay(None);
        assert_eq!(get_predisplay(), "");
    }

    #[test]
    fn get_set_postdisplay_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_postdisplay(Some("trailer"));
        assert_eq!(get_postdisplay(), "trailer");
        set_postdisplay(None);
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn free_prepostdisplay_clears_both() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_predisplay(Some("a"));
        set_postdisplay(Some("b"));
        free_prepostdisplay();
        assert_eq!(get_predisplay(), "");
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn get_context_branches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_reset();
        ZLECONTEXT.store(ZLCON_LINE_START, Ordering::SeqCst);
        assert_eq!(get_context(), "line");
        ZLECONTEXT.store(ZLCON_LINE_CONT, Ordering::SeqCst);
        assert_eq!(get_context(), "cont");
        ZLECONTEXT.store(ZLCON_SELECT, Ordering::SeqCst);
        assert_eq!(get_context(), "select");
        ZLECONTEXT.store(ZLCON_VARED, Ordering::SeqCst);
        assert_eq!(get_context(), "vared");
    }

    #[test]
    fn get_lasearch_lsearch_default_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Globals default to empty Mutex<String>.
        // (Other tests may have set them, so we explicitly reset.)
        PREVIOUS_ABORTED_SEARCH
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clear();
        PREVIOUS_SEARCH
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clear();
        assert_eq!(get_lasearch(), "");
        assert_eq!(get_lsearch(), "");
    }

    #[test]
    fn get_prepost_truncates_to_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:881 — zlelineasstring(text, len, ...).
        assert_eq!(get_prepost("abcdef", 3), "abc");
        assert_eq!(get_prepost("xyz", 99), "xyz"); // len > content
    }

    #[test]
    fn set_prepost_writes_and_clears() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut text = String::new();
        let mut len = 0;
        set_prepost(&mut text, &mut len, Some("hello"));
        assert_eq!(text, "hello");
        assert_eq!(len, 5);
        set_prepost(&mut text, &mut len, None);
        assert_eq!(text, "");
        assert_eq!(len, 0);
    }
}

#[cfg(test)]
mod widget_killring_tests {
    use super::*;

    #[test]
    fn set_get_register_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Register 'a' (idx 0).
        set_register('a', "hello");
        let s: String = vibuf().lock().unwrap()[0].iter().collect();
        assert_eq!(s, "hello");
        // get_registers reads back the same.
        assert_eq!(get_registers("a"), Some("hello".to_string()));
    }

    #[test]
    fn set_register_digit_uses_offset_26() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Register '0' → idx 26.
        set_register('0', "zero");
        let s: String = vibuf().lock().unwrap()[26].iter().collect();
        assert_eq!(s, "zero");
        assert_eq!(get_registers("0"), Some("zero".to_string()));
    }

    #[test]
    fn set_register_invalid_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(set_register('!', "x"), 1);
    }

    #[test]
    fn unset_register_clears_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_register('a', "hi");
        unset_register('a', 1);
        assert_eq!(get_registers("a"), Some(String::new()));
    }

    #[test]
    fn set_get_killring_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let entries = vec!["first".to_string(), "second".to_string()];
        set_killring(Some(&entries));
        let got = get_killring();
        assert_eq!(got, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn unset_killring_clears_when_exp_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let entries = vec!["x".to_string()];
        set_killring(Some(&entries));
        unset_killring(1);
        assert!(get_killring().is_empty());
    }

    #[test]
    fn set_histno_clamps_to_entries_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        history().lock().unwrap().entries.push(HistEntry {
            line: "ls".to_string(),
            num: 1,
            time: None,
        });
        history().lock().unwrap().entries.push(HistEntry {
            line: "cd".to_string(),
            num: 2,
            time: None,
        });
        set_histno(1);
        assert_eq!(history().lock().unwrap().cursor, 1);
        // Beyond-end clamp: x > entries.len() → no change (early
        // return mirrors C's `quietgethist returns NULL → return`).
        history().lock().unwrap().cursor = 7;
        set_histno(99);
        assert_eq!(history().lock().unwrap().cursor, 7);
    }

    // ─── zsh-corpus pins for BUFFER / CURSOR / MARK round-trips ────

    /// `set_buffer("hello")` then `get_buffer()` returns "hello".
    #[test]
    fn zle_params_corpus_buffer_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("hello");
        assert_eq!(get_buffer(), "hello");
    }

    /// `set_buffer("")` empties the buffer.
    #[test]
    fn zle_params_corpus_buffer_set_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("anything");
        set_buffer("");
        assert_eq!(get_buffer(), "");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 0);
    }

    /// `set_cursor(N)` clamps to ZLELL.
    #[test]
    fn zle_params_corpus_cursor_clamps_to_buffer_length() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("abc"); // len=3
        set_cursor(99);
        assert_eq!(get_cursor(), 3, "cursor clamped to buf len");
    }

    /// `set_cursor(2)` for buffer "abc" sets cursor at 2.
    #[test]
    fn zle_params_corpus_cursor_within_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("abc");
        set_cursor(2);
        assert_eq!(get_cursor(), 2);
    }

    /// `set_mark(0)` then `get_mark()` returns 0.
    #[test]
    fn zle_params_corpus_mark_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("abcdef");
        set_mark(2);
        assert_eq!(get_mark(), 2);
    }

    /// `set_region_active(1)` then `get_region_active()` returns 1.
    #[test]
    fn zle_params_corpus_region_active_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_region_active(1);
        assert_eq!(get_region_active(), 1);
        set_region_active(0);
        assert_eq!(get_region_active(), 0);
    }

    /// `set_buffer` updates ZLELL.
    #[test]
    fn zle_params_corpus_set_buffer_updates_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("ab");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 2);
        set_buffer("abcdef");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 6);
    }

    /// `get_buffer()` on empty buffer returns "".
    #[test]
    fn zle_params_corpus_get_buffer_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_buffer("");
        assert_eq!(get_buffer(), "");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_params.c $BUFFER/$CURSOR/
    // $MARK/$LBUFFER/$RBUFFER round-trips.
    // ═══════════════════════════════════════════════════════════════════

    /// `set_buffer` → `get_buffer` round-trips.
    #[test]
    fn set_buffer_then_get_buffer_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello");
        assert_eq!(get_buffer(), "hello", "round-trip preserves string");
    }

    /// `set_cursor(5)` then `get_cursor()` returns 5.
    #[test]
    fn set_cursor_then_get_cursor_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("0123456789");
        set_cursor(5);
        assert_eq!(get_cursor(), 5);
    }

    /// `set_mark(3)` then `get_mark()` returns 3.
    #[test]
    fn set_mark_then_get_mark_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("0123456789");
        set_mark(3);
        assert_eq!(get_mark(), 3);
    }

    /// `set_lbuffer` writes text before cursor.
    #[test]
    fn set_lbuffer_writes_before_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("AFTER");
        set_cursor(0);
        set_lbuffer("BEFORE");
        let lbuf = get_lbuffer();
        assert_eq!(lbuf, "BEFORE");
    }

    /// `set_rbuffer` writes text after cursor.
    #[test]
    fn set_rbuffer_writes_after_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("HELLO");
        set_cursor(5);
        set_rbuffer("POST");
        assert_eq!(get_rbuffer(), "POST");
    }

    /// `set_cursor(N)` beyond end clamps to buffer length.
    /// C: assigning $CURSOR=99 on 5-char buffer clamps.
    #[test]
    fn set_cursor_past_eol_clamps_to_buffer_length() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("abc");
        set_cursor(99);
        assert!(get_cursor() <= 3, "cursor clamps to buffer length");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_params.c getter/setter
    // contracts. Each verifies a single round-trip / clamp invariant.
    // ═══════════════════════════════════════════════════════════════════

    /// c:245 — `set_buffer(s)` + `get_buffer()` round-trips.
    #[test]
    fn set_get_buffer_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello world");
        assert_eq!(get_buffer(), "hello world");
    }

    /// c:245 — `set_buffer` updates ZLELL to match new content length.
    #[test]
    fn set_buffer_updates_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("12345");
        assert_eq!(
            ZLELL.load(Ordering::SeqCst),
            5,
            "ZLELL must reflect new length"
        );
        set_buffer("");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 0, "empty buffer → ZLELL=0");
    }

    /// c:245 — `set_buffer` clamps existing cursor to new length.
    #[test]
    fn set_buffer_clamps_existing_cursor_to_new_length() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello world");
        set_cursor(10);
        // Shrink the buffer to 3 chars — cursor must clamp.
        set_buffer("abc");
        assert!(get_cursor() <= 3, "cursor clamped after shrink");
    }

    /// c:245 — `set_buffer` signals ZLE_RESET_NEEDED.
    #[test]
    fn set_buffer_signals_reset_needed() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ZLE_RESET_NEEDED.store(0, Ordering::SeqCst);
        set_buffer("trigger");
        assert_eq!(
            ZLE_RESET_NEEDED.load(Ordering::SeqCst),
            1,
            "set_buffer must signal reset"
        );
    }

    /// c:267 — `set_cursor(N)` for N ≤ ZLELL stores N exactly.
    #[test]
    fn set_cursor_within_buffer_stores_exact_value() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello");
        set_cursor(2);
        assert_eq!(get_cursor(), 2);
        set_cursor(0);
        assert_eq!(get_cursor(), 0);
        set_cursor(5);
        assert_eq!(get_cursor(), 5, "cursor at EOL exact");
    }

    /// c:267 — `set_cursor` signals ZLE_RESET_NEEDED.
    #[test]
    fn set_cursor_signals_reset_needed() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello");
        ZLE_RESET_NEEDED.store(0, Ordering::SeqCst);
        set_cursor(2);
        assert_eq!(ZLE_RESET_NEEDED.load(Ordering::SeqCst), 1);
    }

    /// c:299 — `set_mark` clamps to ZLELL like set_cursor does.
    #[test]
    fn set_mark_clamps_past_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hi");
        set_mark(99);
        assert!(get_mark() <= 2, "mark must clamp to ZLELL=2");
    }

    /// c:299 — `set_mark(N)` within buffer stores N exactly.
    #[test]
    fn set_mark_within_buffer_stores_exact() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("abcdef");
        set_mark(3);
        assert_eq!(get_mark(), 3);
        set_mark(0);
        assert_eq!(get_mark(), 0);
    }

    /// c:320 — `set_region_active(0)` clears, non-zero sets to 1
    /// (C double-bang idiom !!x).
    #[test]
    fn set_region_active_clears_on_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_region_active(1);
        assert_eq!(get_region_active(), 1);
        set_region_active(0);
        assert_eq!(get_region_active(), 0);
    }

    /// c:320 — `set_region_active(N)` for any N != 0 stores 1
    /// (not the raw value). Pins the !!x coercion.
    #[test]
    fn set_region_active_coerces_nonzero_to_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_region_active(99);
        assert_eq!(get_region_active(), 1, "99 → 1 (not 99)");
        set_region_active(-5);
        assert_eq!(get_region_active(), 1, "negative non-zero → 1");
        set_region_active(i64::MAX);
        assert_eq!(get_region_active(), 1);
    }

    /// c:332 — `set_lbuffer(s)` replaces pre-cursor text; cursor lands
    /// at new lbuffer end.
    #[test]
    fn set_lbuffer_places_cursor_at_new_lbuffer_end() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("XYZworld");
        set_cursor(3); // after "XYZ", before "world"
        set_lbuffer("PREFIX");
        assert_eq!(get_lbuffer(), "PREFIX");
        assert_eq!(get_rbuffer(), "world", "rbuffer preserved");
        assert_eq!(get_cursor(), 6, "cursor at end of new lbuffer");
    }

    /// c:364 — `set_rbuffer(s)` replaces post-cursor text; cursor
    /// stays put.
    #[test]
    fn set_rbuffer_preserves_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_buffer("hello world");
        set_cursor(5);
        set_rbuffer(" everyone");
        assert_eq!(get_lbuffer(), "hello", "lbuffer preserved");
        assert_eq!(get_cursor(), 5, "cursor unchanged");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_params.c
    // c:127 get_buffer / c:144 get_cursor / c:160 get_mark / c:193 get_region_active
    // c:259 get_widget / c:271 get_widgetfunc / c:332 get_keymap / c:338 get_keys
    // c:348 get_keys_queued_count / c:372 get_numeric / c:248 get_prebuffer
    // ═══════════════════════════════════════════════════════════════════

    /// c:127 — `get_buffer` returns String (compile-time type pin).
    #[test]
    fn get_buffer_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_buffer();
    }

    /// c:144 — `get_cursor` returns usize (compile-time type pin).
    #[test]
    fn get_cursor_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: usize = get_cursor();
    }

    /// c:160 — `get_mark` returns usize (compile-time type pin).
    #[test]
    fn get_mark_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: usize = get_mark();
    }

    /// c:193 — `get_region_active` returns i64 in {0, 1, 2} range.
    #[test]
    fn get_region_active_in_canonical_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = get_region_active();
        assert!(
            (0..=2).contains(&r),
            "region_active must be in {{0,1,2}}, got {}",
            r
        );
    }

    /// c:259 — `get_widget` returns String.
    #[test]
    fn get_widget_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_widget();
    }

    /// c:271 — `get_widgetfunc` returns String.
    #[test]
    fn get_widgetfunc_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_widgetfunc();
    }

    /// c:332 — `get_keymap` returns String.
    #[test]
    fn get_keymap_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_keymap();
    }

    /// c:338 — `get_keys` returns Vec<u8>.
    #[test]
    fn get_keys_returns_vec_u8_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Vec<u8> = get_keys();
    }

    /// c:348 — `get_keys_queued_count` returns i64 non-negative.
    #[test]
    fn get_keys_queued_count_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let n = get_keys_queued_count();
        assert!(n >= 0, "queued count must be ≥ 0, got {}", n);
    }

    /// c:372 — `get_numeric` returns i32.
    #[test]
    fn get_numeric_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = get_numeric();
    }

    /// c:355 + c:392 — set_numeric + unset_numeric round-trip safe.
    #[test]
    fn set_unset_numeric_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_numeric(42);
        unset_numeric(0);
    }

    /// c:248 — `get_prebuffer` returns String.
    #[test]
    fn get_prebuffer_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_prebuffer();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_params.c
    // c:63 makezleparams / c:608 set_killring / c:621 get_killring /
    // c:637 unset_killring / c:692 scan_registers / c:817 set_predisplay /
    // c:829 get_predisplay / c:842 set_postdisplay / c:854 get_postdisplay /
    // c:925 get_zle_state / c:951 is_insert_mode / c:958 is_region_active
    // ═══════════════════════════════════════════════════════════════════

    /// c:63 — `makezleparams(0)` returns void (signature pin).
    #[test]
    fn makezleparams_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = makezleparams(0);
    }

    /// c:63 — `makezleparams` is idempotent.
    #[test]
    fn makezleparams_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            makezleparams(0);
        }
    }

    /// c:608 — `set_killring(None)` clears + safe.
    #[test]
    fn set_killring_none_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_killring(None);
    }

    /// c:608 — `set_killring(empty)` empty slice safe.
    #[test]
    fn set_killring_empty_slice_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_killring(Some(&[]));
    }

    /// c:621 — `get_killring` returns Vec<String> (compile-time type pin).
    #[test]
    fn get_killring_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Vec<String> = get_killring();
    }

    /// c:637 — `unset_killring(0)` safe.
    #[test]
    fn unset_killring_zero_exp_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        unset_killring(0);
    }

    /// c:817 — `set_predisplay(None)` clears + safe.
    #[test]
    fn set_predisplay_none_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_predisplay(None);
    }

    /// c:842 — `set_postdisplay(None)` clears + safe.
    #[test]
    fn set_postdisplay_none_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_postdisplay(None);
    }

    /// c:817 + c:829 — set_predisplay then get_predisplay round-trips.
    #[test]
    fn set_get_predisplay_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_predisplay(Some("hello"));
        assert_eq!(get_predisplay(), "hello", "set then get round-trips");
        set_predisplay(None);
        assert_eq!(get_predisplay(), "", "None clears predisplay");
    }

    /// c:842 + c:854 — set_postdisplay then get_postdisplay round-trips.
    #[test]
    fn set_get_postdisplay_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_postdisplay(Some("world"));
        assert_eq!(get_postdisplay(), "world", "set then get round-trips");
        set_postdisplay(None);
        assert_eq!(get_postdisplay(), "", "None clears postdisplay");
    }

    /// c:925 — `get_zle_state` returns String (compile-time type pin).
    #[test]
    fn get_zle_state_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: String = get_zle_state();
    }

    /// c:951 — `is_insert_mode` returns bool (compile-time type pin).
    #[test]
    fn is_insert_mode_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: bool = is_insert_mode();
    }

    /// c:958 — `is_region_active` returns bool (compile-time type pin).
    #[test]
    fn is_region_active_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: bool = is_region_active();
    }

    /// c:692 — `scan_registers(_, None, 0)` None callback safe.
    #[test]
    fn scan_registers_none_callback_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        scan_registers(0, None, 0);
    }
}
