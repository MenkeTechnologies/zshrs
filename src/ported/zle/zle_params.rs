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
    /// `$BUFFER` accessor — full edited line as a String.
    /// Port of `get_buffer(UNUSED(Param pm))` from Src/Zle/zle_params.c (the
    /// `BUFFER` getfn entry in `zleparams[]`).
    /// WARNING: param names don't match C — Rust=() vs C=(pm)

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::zle_h::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
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

/// Direct port of `void makezleparams(int ro)` from
/// `Src/Zle/zle_params.c:194-228`. Registers the `$BUFFER`,
/// `$LBUFFER`, `$RBUFFER`, `$CURSOR`, `$MARK`, `$NUMERIC`,
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
pub fn makezleparams(_ro: i32) {                                             // c:194
    use crate::ported::zle::compcore::{ZLECS, ZLELINE, ZMULT};

    let line = ZLELINE.get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    let cs = ZLECS.load(std::sync::atomic::Ordering::Relaxed) as usize;
    let (lbuf, rbuf) = if cs <= line.len() {
        (line[..cs].to_string(), line[cs..].to_string())
    } else {
        (line.clone(), String::new())
    };

    let _ = crate::ported::params::setsparam("BUFFER", &line);              // c:zleparams[0]
    let _ = crate::ported::params::setsparam("LBUFFER", &lbuf);             // c:zleparams[1]
    let _ = crate::ported::params::setsparam("RBUFFER", &rbuf);             // c:zleparams[2]
    let _ = crate::ported::params::setiparam(
        "CURSOR",
        ZLECS.load(std::sync::atomic::Ordering::Relaxed) as i64,
    );                                                                       // c:zleparams[3]
    let _ = crate::ported::params::setiparam("NUMERIC", ZMULT.load(
        std::sync::atomic::Ordering::Relaxed,
    ) as i64);                                                               // c:zleparams[7]
    // $BUFFERLINES — count of newlines in BUFFER + 1.
    let lines = line.chars().filter(|c| *c == '\n').count() as i64 + 1;
    let _ = crate::ported::params::setiparam("BUFFERLINES", lines);          // c:zleparams[10]
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
pub fn zleunsetfn(pm: &mut crate::ported::zsh_h::param, exp: i32) {          // c:237
    crate::ported::params::stdunsetfn(pm, exp);                              // c:237
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
    pub fn set_buffer(s: &str) {                                 // c:245
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = s.chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn get_buffer() -> String {                                    // c:258
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect()
    }

    /// `$CURSOR=pos` setter — clamped to buffer length.
    /// Port of `set_cursor(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=(pos) vs C=(pm, x)
    pub fn set_cursor(pos: usize) {                              // c:267
        crate::ported::zle::zle_main::ZLECS.store(pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$CURSOR` accessor — current cursor position (0-indexed).
    /// Port of `get_cursor(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_cursor() -> usize {                                     // c:281
        crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// `$MARK=pos` setter — clamp to buffer length.
    /// Port of `set_mark(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=(pos) vs C=(pm, x)
    pub fn set_mark(pos: usize) {                                // c:299
        crate::ported::zle::zle_main::MARK.store(pos.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
    }

    /// `$MARK` accessor — current mark position.
    /// Port of `get_mark(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_mark() -> usize {                                       // c:311
        crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst)
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
pub fn set_region_active(                                                    // c:318
    x: i64,
) {
    // c:320 — `region_active = (int)!!x`. !!x: 0→0, anything else→1.
    crate::ported::zle::zle_main::REGION_ACTIVE.store(if x != 0 { 1 } else { 0 }, std::sync::atomic::Ordering::SeqCst);
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
pub fn get_region_active() -> i64 {   // c:325
    crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) as i64                                                 // c:325 return region_active
}

    /// `$LBUFFER=s` setter — replace text before the cursor; cursor
    /// lands at the new lbuffer's end.
    /// Port of `set_lbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
    pub fn set_lbuffer(s: &str) {                                // c:332
        let rbuf: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..].iter().collect();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = s.chars().chain(rbuf.chars()).collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(s.chars().count(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$LBUFFER` accessor — text before the cursor.
    /// Port of `get_lbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_lbuffer() -> String {                                   // c:355
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].iter().collect()
    }

    /// `$RBUFFER=s` setter — replace text after the cursor.
    /// Port of `set_rbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
    pub fn set_rbuffer(s: &str) {                                // c:364
        let lbuf: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].iter().collect();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = lbuf.chars().chain(s.chars()).collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// `$RBUFFER` accessor — text after the cursor.
    /// Port of `get_rbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_rbuffer() -> String {                                   // c:384
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..].iter().collect()
    }

/// Port of `get_prebuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c:394.
pub fn get_prebuffer() -> String {    // c:394
    // C body c:396-410 — `if (!stackhist) return ztrdup("");
    //                     dputs(...prepended buffer...)`. Returns the
    //                     stacked-line buffer (multi-line input not
    //                     yet committed to current zleline). Without
    //                     stackhist tracking we return empty.
    String::new()
}

/// Port of `get_widget(UNUSED(Param pm))` from Src/Zle/zle_params.c:414.
pub fn get_widget() -> String {       // c:414
    // c:421 — `return bindk ? bindk->nam : ""`.
    crate::ported::zle::zle_main::BINDK.lock().unwrap().as_ref().map(|t| t.nam.clone()).unwrap_or_default()
}

/// Port of `get_widgetfunc(UNUSED(Param pm))` from Src/Zle/zle_params.c:421.
pub fn get_widgetfunc() -> String {   // c:421
    use crate::ported::zle::zle_h::WidgetImpl as WidgetFunc;
    // c:423-430 — read bindk->widget. C union dispatches:
    //   WIDGET_INT  → ".internal"  (c:426-427)
    //   WIDGET_NCOMP → comp.func   (c:428-429)
    //   else → fnnam               (c:430)
    let bindk_guard = crate::ported::zle::zle_main::BINDK.lock().unwrap();
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
    // WidgetFunc::Comp variant); collapse to the User-fn case.
    match &w.u {
        WidgetFunc::UserFunc(name) => name.clone(),
        WidgetFunc::Internal(_) => ".internal".to_string(),
        _ => ".internal".to_string(),
    }
}

/// Port of `get_widgetstyle(UNUSED(Param pm))` from Src/Zle/zle_params.c:435.
pub fn get_widgetstyle() -> String {  // c:435

    // c:437-444 — read bindk->widget. INT → ".internal"; NCOMP →
    // comp.wid (the underlying widget name); else "".
    let bindk_guard = crate::ported::zle::zle_main::BINDK.lock().unwrap();
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
    String::new()                                                            // c:444
}

/// Port of `get_lwidget(UNUSED(Param pm))` from Src/Zle/zle_params.c:449.
pub fn get_lwidget() -> String {      // c:449
    // c:449 — `return (lbindk ? lbindk->nam : "")`.
    crate::ported::zle::zle_main::LBINDK.lock().unwrap().as_ref().map(|t| t.nam.clone()).unwrap_or_default()
}

    /// `$KEYMAP` accessor — currently-active keymap name.
    /// Port of `get_keymap(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_keymap() -> String {                                    // c:456
        crate::ported::zle::zle_keymap::curkeymapname().clone()
    }

/// Port of `get_keys(UNUSED(Param pm))` from Src/Zle/zle_params.c:463.
pub fn get_keys() -> Vec<u8> {        // c:463
    // c:470 — `return keybuf`. The active keymap-walk byte buffer.
    crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clone()
}

/// Port of `get_keys_queued_count(UNUSED(Param pm))` from Src/Zle/zle_params.c:470.
pub fn get_keys_queued_count() -> i64 {  // c:470
    // c:470 — `return kungetct`. Bytes pending in the unget queue.
    crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().len() as i64
}

/// Port of `set_numeric(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c:477.
pub fn set_numeric(x: i64) {   // c:477
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:479 — `zmult = x`. zmult is zmod.mult.
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = x as i32;
    // c:480 — `zmod.flags = MOD_MULT`. Replaces the whole flags
    // bitfield with just MOD_MULT (not OR — the C is a plain `=`).
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = MOD_MULT;
}

    /// `$NUMERIC` accessor — numeric prefix when set.
    /// Port of `get_numeric(UNUSED(Param pm))` from Src/Zle/zle_params.c which
    /// returns `zmod.mult` only when `MOD_MULT` is set, otherwise
    /// the parameter is unset.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_numeric() -> Option<i32> {                              // c:485
        if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & super::zle_h::MOD_MULT != 0 {
            Some(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult)
        } else {
            None
        }
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
pub fn unset_numeric(exp: i32) { // c:492
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if exp != 0 {                                                            // c:494
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;                             // c:496
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;                                                   // c:497
    }
}

/// Port of `set_histno(UNUSED(Param pm), zlong x)` from Src/Zle/zle_params.c:503.
pub fn set_histno(x: i64) {     // c:503
    // c:503-509 — `Histent he = quietgethist(x); if (!he) return;
    //              zle_setline()`.
    // zshrs uses History.cursor as the active history index. Clamp
    // to entries.len() when x is out of range (matches the
    // quietgethist NULL-result early-return).
    let idx = x.max(0) as usize;
    if idx <= crate::ported::zle::zle_main::history().lock().unwrap().entries.len() {
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = idx;
    }
}

/// Port of `get_histno(UNUSED(Param pm))` from Src/Zle/zle_params.c:514.
pub fn get_histno() -> i64 {          // c:514
    // c:514 — `return histline`. zshrs tracks the editing history
    // line via the History.cursor field (offset into entries Vec).
    crate::ported::zle::zle_main::history().lock().unwrap().cursor as i64
}

    /// `$BUFFERLINES` accessor — number of newline-separated lines.
    /// Port of `get_bufferlines(UNUSED(Param pm))` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_bufferlines() -> usize {                                // c:521
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().filter(|&&c| c == '\n').count() + 1
    }

    /// `$PENDING` accessor — bytes waiting in the input queue.
    /// Port of `get_pending(UNUSED(Param pm))` from Src/Zle/zle_params.c which
    /// returns `kungetct` (the unget-buffer fill).
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_pending() -> usize {                                    // c:528
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
pub fn get_recursive() -> i64 {       // c:535
    crate::ported::zle::zle_main::ZLE_RECURSIVE.load(std::sync::atomic::Ordering::SeqCst) as i64                                                 // c:535 return zle_recursive
}

/// Port of `get_yankstart(UNUSED(Param pm))` from Src/Zle/zle_params.c:542.
pub fn get_yankstart() -> i64 {       // c:542
    // c:542 — `return yankb`.
    crate::ported::zle::zle_main::YANKB.load(std::sync::atomic::Ordering::SeqCst) as i64
}

/// Port of `get_yankend(UNUSED(Param pm))` from Src/Zle/zle_params.c:549.
pub fn get_yankend() -> i64 {         // c:549
    // c:542 — `return yanke`.
    crate::ported::zle::zle_main::YANKE.load(std::sync::atomic::Ordering::SeqCst) as i64
}

/// Port of `get_yankactive(UNUSED(Param pm))` from Src/Zle/zle_params.c:556.
pub fn get_yankactive() -> i64 {      // c:556
    // c:549 — `return !!(lastcmd & ZLE_YANK) + !!(lastcmd & ZLE_YANKAFTER)`.
    let last = crate::ported::zle::zle_main::LASTCMD.load(std::sync::atomic::Ordering::SeqCst) as i32;
    let yank      = ((last & ZLE_YANK)      != 0) as i64;
    let yankafter = ((last & ZLE_YANKAFTER) != 0) as i64;
    yank + yankafter
}

/// Port of `set_yankstart(UNUSED(Param pm), zlong i)` from Src/Zle/zle_params.c:563.
pub fn set_yankstart(i: i64) {  // c:563
    // c:563 — `yankb = i`.
    crate::ported::zle::zle_main::YANKB.store(i.max(0) as usize, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `set_yankend(UNUSED(Param pm), zlong i)` from Src/Zle/zle_params.c:570.
pub fn set_yankend(i: i64) {    // c:570
    // c:563 — `yanke = i`.
    crate::ported::zle::zle_main::YANKE.store(i.max(0) as usize, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `get_isearchmatchstart(UNUSED(Param pm))` from Src/Zle/zle_params.c:577.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchstart() -> i64 {                                      // c:577
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_STARTPOS.load(Ordering::Relaxed) as i64  // c:579
}

/// Port of `get_isearchmatchend(UNUSED(Param pm))` from Src/Zle/zle_params.c:584.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchend() -> i64 {                                        // c:584
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ENDPOS.load(Ordering::Relaxed) as i64  // c:577
}

/// Port of `get_isearchmatchactive(UNUSED(Param pm))` from Src/Zle/zle_params.c:591.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_isearchmatchactive() -> i64 {                                     // c:591
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_hist::ISEARCH_ACTIVE.load(Ordering::Relaxed) as i64  // c:577
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
pub fn get_suffixstart() -> i64 {     // c:598
    use std::sync::atomic::Ordering;
    let suffixlen = crate::ported::zle::zle_misc::SUFFIXLEN.load(Ordering::Relaxed);
    (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) as i64) - (suffixlen as i64)                                  // c:600 zlecs - suffixlen
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
pub fn get_suffixend() -> i64 {       // c:605
    crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) as i64                                                         // c:605 return zlecs
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
pub fn get_suffixactive() -> i64 {                                           // c:612
    use std::sync::atomic::Ordering;
    crate::ported::zle::zle_misc::SUFFIXLEN.load(Ordering::Relaxed) as i64   // c:614 return suffixlen
}

    /// `$CUTBUFFER` accessor — most-recent kill-ring entry.
    /// Port of `get_cutbuffer(UNUSED(Param pm))` from Src/Zle/zle_params.c which
    /// reads `cutbuf` (the unnamed kill register).
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_cutbuffer() -> String {                                 // c:619
        crate::ported::zle::zle_main::KILLRING.lock().unwrap()
            .front()
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// `$CUTBUFFER=s` setter — overwrite the front of the kill ring.
    /// Port of `set_cutbuffer(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c.
    /// WARNING: param names don't match C — Rust=(s) vs C=(pm, x)
    pub fn set_cutbuffer(s: &str) {                              // c:629
        let chars: Vec<char> = s.chars().collect();
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().is_empty() {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(chars);
        } else {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap()[0] = chars;
        }
    }

/// Port of `unset_cutbuffer(Param pm, int exp)` from Src/Zle/zle_params.c:647.
pub fn unset_cutbuffer(exp: i32) {  // c:647
    // c:647-655 — `if (exp) { stdunsetfn; if (cutbuf.buf) { free; NULL; len=0 } }`.
    if exp != 0 {
        // zshrs uses VecDeque for the kill ring; the "primary" cut
        // buffer is the front entry. Clearing means popping it.
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_front();
    }
}

/// Port of `set_killring(UNUSED(Param pm), char **x)` from Src/Zle/zle_params.c:661.
pub fn set_killring(x: Option<&[String]>) {  // c:661
    // c:661-672 — `if (kring) { free each kptr->buf; zfree(kring) }`.
    // Then either rebuild from `x` or leave NULL.
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().clear();
    if let Some(arr) = x {
        for entry in arr {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_back(entry.chars().collect());
        }
    }
}

/// Port of `get_killring(UNUSED(Param pm))` from Src/Zle/zle_params.c:705.
pub fn get_killring() -> Vec<String> {  // c:705
    // c:705-733 — return kring entries with most-recently-killed
    // first. Empty entries returned as "" so the array length always
    // equals kringsize. zshrs holds the kill ring as
    // VecDeque<ZleString> where push_front puts newest at index 0,
    // so we iterate forward.
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().iter()
        .map(|entry| entry.iter().collect::<String>())
        .collect()
}

/// Port of `unset_killring(Param pm, int exp)` from Src/Zle/zle_params.c:741.
pub fn unset_killring(exp: i32) {  // c:741
    // c:741-746 — `if (exp) { set_killring(NULL); stdunsetfn(...) }`.
    if exp != 0 {
        set_killring(None);
        // stdunsetfn handles param-table bookkeeping — substrate.
    }
}

/// Port of `set_register(Param pm, char *value)` from Src/Zle/zle_params.c:751.
/// WARNING: param names don't match C — Rust=(zle, name, value) vs C=(pm, value)
pub fn set_register(name: char, value: &str) -> i32 {  // c:751
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
    if (idx as usize) < crate::ported::zle::zle_main::vibuf().lock().unwrap().len() {
        crate::ported::zle::zle_main::vibuf().lock().unwrap()[idx as usize] = value.chars().collect();
    }
    0
}

/// Port of `unset_register(Param pm, UNUSED(int exp))` from Src/Zle/zle_params.c:777.
/// WARNING: param names don't match C — Rust=(zle, name, _exp) vs C=(pm, exp)
pub fn unset_register(name: char, _exp: i32) {  // c:777
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
pub fn scan_registers(_t: i32, _flags: i32) {                                // c:784
}

/// Port of `get_registers(UNUSED(HashTable ht), const char *name)` from Src/Zle/zle_params.c:807.
pub fn get_registers(name: &str) -> Option<String> {  // c:807
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
        return None;                                                         // c:822-824 (vbuf==-1)
    };
    // c:798 — `pm->u.str = zlelineasstring(vibuf[i].buf, ...)`.
    if (idx as usize) < crate::ported::zle::zle_main::vibuf().lock().unwrap().len() {
        Some(crate::ported::zle::zle_main::vibuf().lock().unwrap()[idx as usize].iter().collect::<String>())
    } else {
        None
    }
}

/// Port of `set_registers(Param pm, HashTable ht)` from Src/Zle/zle_params.c:833.
/// WARNING: param names don't match C — Rust=(zle) vs C=(pm, ht)
pub fn set_registers(// c:833
                     map: &std::collections::HashMap<String, String>) {
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
pub fn unset_registers(exp: i32) { // c:857
    // C body c:859-870 — `if (exp) { for (i...) { vibuf[i].buf=NULL;
    //                              vibuf[i].len = 0; } stdunsetfn(...) }`.
    if exp != 0 {
        for buf in crate::ported::zle::zle_main::vibuf().lock().unwrap().iter_mut() {
            buf.clear();
        }
    }
}

/// Port of `set_prepost(ZLE_STRING_T *textvar, int *lenvar, char *x)` from Src/Zle/zle_params.c:865.
pub fn set_prepost(textvar: &mut String, lenvar: &mut usize, x: Option<&str>) {  // c:865
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
pub fn get_prepost(text: &str, len: usize) -> String {                       // c:879
    // c:879 — `return zlelineasstring(text, len, 0, NULL, NULL, 1)`.
    // In Rust the caller already owns a String; just truncate to len.
    text.chars().take(len).collect()
}

/// Port of `set_predisplay(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c:886.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn set_predisplay(x: Option<&str>) {                                     // c:886
    use crate::ported::zle::zle_misc::PREDISPLAY;
    use std::sync::Mutex;
    let g = PREDISPLAY.get_or_init(|| Mutex::new(String::new()));
    let mut buf = g.lock().unwrap();
    buf.clear();
    if let Some(s) = x {
        buf.push_str(s);
    }
}

/// Port of `get_predisplay(UNUSED(Param pm))` from Src/Zle/zle_params.c:893.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_predisplay() -> String {                                          // c:893
    use crate::ported::zle::zle_misc::PREDISPLAY;
    use std::sync::Mutex;
    PREDISPLAY.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `set_postdisplay(UNUSED(Param pm), char *x)` from Src/Zle/zle_params.c:900.
/// WARNING: param names don't match C — Rust=(x) vs C=(pm, x)
pub fn set_postdisplay(x: Option<&str>) {                                    // c:900
    use crate::ported::zle::zle_misc::POSTDISPLAY;
    use std::sync::Mutex;
    let g = POSTDISPLAY.get_or_init(|| Mutex::new(String::new()));
    let mut buf = g.lock().unwrap();
    buf.clear();
    if let Some(s) = x {
        buf.push_str(s);
    }
}

/// Port of `get_postdisplay(UNUSED(Param pm))` from Src/Zle/zle_params.c:907.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_postdisplay() -> String {                                         // c:907
    use crate::ported::zle::zle_misc::POSTDISPLAY;
    use std::sync::Mutex;
    // c:909 — `return get_prepost(postdisplay, postdisplaylen)` →
    // zlelineasstring(...). Return the raw String.
    POSTDISPLAY.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}


/// Port of `free_prepostdisplay()` from Src/Zle/zle_params.c:914.
pub fn free_prepostdisplay() {                                               // c:914
    use crate::ported::zle::zle_misc::{POSTDISPLAY, PREDISPLAY};
    use std::sync::Mutex;
    // c:916-917 — `if (predisplaylen) set_prepost(&predisplay, &predisplaylen, NULL)`.
    PREDISPLAY.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
    // c:918-919 — same for postdisplay.
    POSTDISPLAY.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
}

/// Port of `get_lasearch(UNUSED(Param pm))` from Src/Zle/zle_params.c:924.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_lasearch() -> String {                                            // c:924
    use crate::ported::zle::zle_misc::PREVIOUS_ABORTED_SEARCH;
    use std::sync::Mutex;
    // c:933-928 — `previous_aborted_search ? : ""`.
    PREVIOUS_ABORTED_SEARCH.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_lsearch(UNUSED(Param pm))` from Src/Zle/zle_params.c:933.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_lsearch() -> String {                                             // c:933
    use crate::ported::zle::zle_misc::PREVIOUS_SEARCH;
    use std::sync::Mutex;
    // c:935-937 — `previous_search ? : ""`.
    PREVIOUS_SEARCH.get_or_init(|| Mutex::new(String::new()))
        .lock().unwrap().clone()
}

/// Port of `get_context(UNUSED(Param pm))` from Src/Zle/zle_params.c:942.
pub fn get_context() -> &'static str {  // c:942
    use crate::ported::zsh_h::{ZLCON_LINE_CONT, ZLCON_SELECT, ZLCON_VARED};
    // c:944-958 — switch on zlecontext → "cont" / "select" / "vared" / "line".
    match crate::ported::zle::zle_main::ZLECONTEXT.load(std::sync::atomic::Ordering::SeqCst) {
        x if x == ZLCON_LINE_CONT => "cont",                                  // c:945-946
        x if x == ZLCON_SELECT    => "select",                                // c:949-950
        x if x == ZLCON_VARED     => "vared",                                 // c:953-954
        _                         => "line",                                  // c:957-958 default
    }
}

    /// `$ZLE_STATE` accessor — "insert"|"overwrite" + ":" + keymap.
    /// Port of `get_zle_state(UNUSED(Param pm))` from Src/Zle/zle_params.c. The C
    /// source emits a space-separated list of state words; our
    /// minimal version covers the two most-consulted fields.
    /// WARNING: param names don't match C — Rust=() vs C=(pm)
    pub fn get_zle_state() -> String {
        let mut state = String::new();

        if (crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0) {
            state.push_str("insert");
        } else {
            state.push_str("overwrite");
        }

        // Add keymap info
        state.push(':');
        state.push_str(&crate::ported::zle::zle_keymap::curkeymapname());

        state
    }

    /// `$ZLE_STATE` insert/overwrite component — true for insert.
    /// Sub-port of `get_zle_state(UNUSED(Param pm))` (Src/Zle/zle_params.c) which
    /// emits "insert" / "overwrite" + " " + "vicmd" / "main".
    pub fn is_insert_mode() -> bool {
        (crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst) != 0)
    }

    /// `$REGION_ACTIVE` accessor — non-zero when a visual selection
    /// is active.
    /// Port of `get_region_active(UNUSED(Param pm))` from Src/Zle/zle_params.c. The
    /// C source returns 1/2 (charwise/linewise); our simplified
    /// boolean compares mark vs cursor.
    pub fn is_region_active() -> bool {
        crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)
    }

#[cfg(test)]
mod region_active_tests {
    use super::*;

    #[test]
    fn get_region_active_reads_field() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:327 — `return region_active`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_region_active(), 0);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_region_active(), 1);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_region_active(), 2);
    }

    #[test]
    fn set_region_active_double_bang_idiom() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:320 — `region_active = (int)!!x`. Any non-zero → 1; zero → 0.
        crate::ported::zle::zle_main::zle_reset();
        set_region_active(0);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
        set_region_active(1);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        set_region_active(99);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        set_region_active(-1);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        set_region_active(0);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod trap_tests {
    use crate::ported::zle::zle_main::{zleaftertrap, zlebeforetrap};

    #[test]
    fn zlebeforetrap_returns_zero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2110 — `return 0` always.
        assert_eq!(zlebeforetrap(), 0);
    }

    #[test]
    fn zleaftertrap_returns_zero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:2119 — `return 0` always.
        assert_eq!(zleaftertrap(), 0);
    }
}

#[cfg(test)]
mod numeric_tests {
    use super::*;
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};

    #[test]
    fn set_numeric_sets_mult_and_replaces_flags() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:479-480 — `zmult=x; zmod.flags = MOD_MULT` (assignment,
        // not OR). Pre-existing flags get wiped.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_TMULT | MOD_NEG;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        set_numeric(7);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, 7);
        // Only MULT remains; TMULT and NEG are gone.
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_MULT != 0);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT, 0);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NEG, 0);
    }

    #[test]
    fn unset_numeric_resets_when_exp_nonzero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:494-498 — only resets when exp != 0.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 5;
        unset_numeric(1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, 1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags, 0);
    }

    #[test]
    fn unset_numeric_noop_when_exp_zero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:494 — `if (exp)` skips when exp == 0.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 5;
        unset_numeric(0);
        // Unchanged.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, 5);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_MULT != 0);
    }
}

#[cfg(test)]
mod suffix_tests {
    use super::*;
    use crate::ported::zle::zle_misc::SUFFIXLEN;
    use std::sync::atomic::Ordering;

    #[test]
    fn get_suffixactive_reads_suffixlen() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:614 — `return suffixlen`.
        SUFFIXLEN.store(7, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 7);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixactive(), 0);
    }

    #[test]
    fn get_suffixend_reads_zlecs() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:607 — `return zlecs`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZLECS.store(11, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_suffixend(), 11);
    }

    #[test]
    fn get_suffixstart_subtracts_suffixlen() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:600 — `return zlecs - suffixlen`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZLECS.store(20, std::sync::atomic::Ordering::SeqCst);
        SUFFIXLEN.store(5, Ordering::SeqCst);
        assert_eq!(get_suffixstart(), 15);
        SUFFIXLEN.store(0, Ordering::SeqCst);
        assert_eq!(get_suffixstart(), 20);
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::ported::zle::zle_thingy::Thingy;

    #[test]
    fn get_widget_reads_bindk_nam() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:416 — `return bindk ? bindk->nam : ""`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::BINDK.lock().unwrap() = Some(Thingy::new("self-insert"));
        assert_eq!(get_widget(), "self-insert");
    }

    #[test]
    fn get_widget_empty_when_no_bindk() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:416 — `bindk` NULL → empty string.
        crate::ported::zle::zle_main::zle_reset();
        assert_eq!(get_widget(), "");
    }

    #[test]
    fn get_lwidget_reads_lbindk_nam() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:451 — `return (lbindk ? lbindk->nam : "")`.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::LBINDK.lock().unwrap() = Some(Thingy::new("forward-char"));
        assert_eq!(get_lwidget(), "forward-char");
    }

    #[test]
    fn get_lwidget_empty_when_no_lbindk() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(get_lwidget(), "");
    }

    #[test]
    fn get_recursive_reads_zle_recursive_field() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:537 — `return zle_recursive`.
        crate::ported::zle::zle_main::zle_reset();
        crate::ported::zle::zle_main::ZLE_RECURSIVE.store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_recursive(), 0);
        crate::ported::zle::zle_main::ZLE_RECURSIVE.store(5, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_recursive(), 5);
    }
}

#[cfg(test)]
mod isearch_tests {
    use super::*;
    use crate::ported::zle::zle_hist::{ISEARCH_ACTIVE, ISEARCH_ENDPOS, ISEARCH_STARTPOS};
    use std::sync::atomic::Ordering;

    #[test]
    fn get_isearchmatchactive_reads_global() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:593 — `return isearch_active`.
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 0);
        ISEARCH_ACTIVE.store(1, Ordering::SeqCst);
        assert_eq!(get_isearchmatchactive(), 1);
        ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchstart_reads_global() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:579 — `return isearch_startpos`.
        ISEARCH_STARTPOS.store(7, Ordering::SeqCst);
        assert_eq!(get_isearchmatchstart(), 7);
        ISEARCH_STARTPOS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn get_isearchmatchend_reads_global() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = 7;
        assert_eq!(get_histno(), 7);
    }

    #[test]
    fn get_keys_returns_keybuf_clone() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_keymap::keybuf.lock().unwrap() = vec![0x1b, b'a'];
        assert_eq!(get_keys(), vec![0x1b, b'a']);
    }

    #[test]
    fn get_keys_queued_count_returns_unget_len() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().push_back(b'a');
        crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().push_back(b'b');
        crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().push_back(b'c');
        assert_eq!(get_keys_queued_count(), 3);
    }

    #[test]
    fn get_yankstart_yankend_read_fields() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_main::YANKB.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::YANKE.store(8, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(get_yankstart(), 3);
        assert_eq!(get_yankend(), 8);
    }

    #[test]
    fn set_yankstart_yankend_write_fields() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        set_yankstart(5);
        set_yankend(11);
        assert_eq!(crate::ported::zle::zle_main::YANKB.load(std::sync::atomic::Ordering::SeqCst), 5);
        assert_eq!(crate::ported::zle::zle_main::YANKE.load(std::sync::atomic::Ordering::SeqCst), 11);
    }
}

#[cfg(test)]
mod keybuf_tests {
    use crate::ported::zle::zle_keymap::{addkeybuf, freekeynode, KeyBinding};

    #[test]
    fn addkeybuf_plain_byte() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clear();
        addkeybuf(b'a' as i32);
        assert_eq!(*crate::ported::zle::zle_keymap::keybuf.lock().unwrap(), vec![b'a']);
    }

    #[test]
    fn addkeybuf_meta_quoted() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // 0xa0 needs Meta-quoting → 0x83 then (0xa0 ^ 0x20) = 0x80
        crate::ported::zle::zle_keymap::keybuf.lock().unwrap().clear();
        addkeybuf(0xa0);
        assert_eq!(*crate::ported::zle::zle_keymap::keybuf.lock().unwrap(), vec![0x83, 0x80]);
    }

    #[test]
    fn freekeynode_consumes_binding() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
    use crate::ported::zsh_h::{ZLCON_LINE_START, ZLCON_LINE_CONT, ZLCON_SELECT, ZLCON_VARED};

    #[test]
    fn get_set_predisplay_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:885,892 — round-trip set→get.
        set_predisplay(Some("[hint] "));
        assert_eq!(get_predisplay(), "[hint] ");
        set_predisplay(None);
        assert_eq!(get_predisplay(), "");
    }

    #[test]
    fn get_set_postdisplay_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        set_postdisplay(Some("trailer"));
        assert_eq!(get_postdisplay(), "trailer");
        set_postdisplay(None);
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn free_prepostdisplay_clears_both() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        set_predisplay(Some("a"));
        set_postdisplay(Some("b"));
        free_prepostdisplay();
        assert_eq!(get_predisplay(), "");
        assert_eq!(get_postdisplay(), "");
    }

    #[test]
    fn get_context_branches() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        use crate::ported::zle::zle_main::ZLECONTEXT;
        use std::sync::atomic::Ordering;
        crate::ported::zle::zle_main::zle_reset();
        ZLECONTEXT.store(ZLCON_LINE_START, Ordering::SeqCst); assert_eq!(get_context(), "line");
        ZLECONTEXT.store(ZLCON_LINE_CONT,  Ordering::SeqCst); assert_eq!(get_context(), "cont");
        ZLECONTEXT.store(ZLCON_SELECT,     Ordering::SeqCst); assert_eq!(get_context(), "select");
        ZLECONTEXT.store(ZLCON_VARED,      Ordering::SeqCst); assert_eq!(get_context(), "vared");
    }

    #[test]
    fn get_lasearch_lsearch_default_empty() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Globals default to empty Mutex<String>.
        // (Other tests may have set them, so we explicitly reset.)
        use crate::ported::zle::zle_misc::{PREVIOUS_ABORTED_SEARCH, PREVIOUS_SEARCH};
        use std::sync::Mutex;
        PREVIOUS_ABORTED_SEARCH.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
        PREVIOUS_SEARCH.get_or_init(|| Mutex::new(String::new())).lock().unwrap().clear();
        assert_eq!(get_lasearch(), "");
        assert_eq!(get_lsearch(), "");
    }

    #[test]
    fn get_prepost_truncates_to_len() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:881 — zlelineasstring(text, len, ...).
        assert_eq!(get_prepost("abcdef", 3), "abc");
        assert_eq!(get_prepost("xyz", 99), "xyz"); // len > content
    }

    #[test]
    fn set_prepost_writes_and_clears() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
    use crate::ported::zle::zle_h::{widget as Widget, WidgetImpl as WidgetFunc};
    use crate::ported::zle::zle_thingy::Thingy;
    use std::sync::Arc;


    #[test]
    fn set_get_register_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Register 'a' (idx 0).
        set_register('a', "hello");
        let s: String = crate::ported::zle::zle_main::vibuf().lock().unwrap()[0].iter().collect();
        assert_eq!(s, "hello");
        // get_registers reads back the same.
        assert_eq!(get_registers("a"), Some("hello".to_string()));
    }

    #[test]
    fn set_register_digit_uses_offset_26() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Register '0' → idx 26.
        set_register('0', "zero");
        let s: String = crate::ported::zle::zle_main::vibuf().lock().unwrap()[26].iter().collect();
        assert_eq!(s, "zero");
        assert_eq!(get_registers("0"), Some("zero".to_string()));
    }

    #[test]
    fn set_register_invalid_returns_one() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(set_register('!', "x"), 1);
    }

    #[test]
    fn unset_register_clears_buffer() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        set_register('a', "hi");
        unset_register('a', 1);
        assert_eq!(get_registers("a"), Some(String::new()));
    }

    #[test]
    fn set_get_killring_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let entries = vec!["first".to_string(), "second".to_string()];
        set_killring(Some(&entries));
        let got = get_killring();
        assert_eq!(got, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn unset_killring_clears_when_exp_nonzero() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let entries = vec!["x".to_string()];
        set_killring(Some(&entries));
        unset_killring(1);
        assert!(get_killring().is_empty());
    }

    #[test]
    fn set_histno_clamps_to_entries_len() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::zle::zle_main::history().lock().unwrap().entries.push(crate::ported::zle::zle_hist::HistEntry {
            line: "ls".to_string(), num: 1, time: None,
        });
        crate::ported::zle::zle_main::history().lock().unwrap().entries.push(crate::ported::zle::zle_hist::HistEntry {
            line: "cd".to_string(), num: 2, time: None,
        });
        set_histno(1);
        assert_eq!(crate::ported::zle::zle_main::history().lock().unwrap().cursor, 1);
        // Beyond-end clamp: x > entries.len() → no change (early
        // return mirrors C's `quietgethist returns NULL → return`).
        crate::ported::zle::zle_main::history().lock().unwrap().cursor = 7;
        set_histno(99);
        assert_eq!(crate::ported::zle::zle_main::history().lock().unwrap().cursor, 7);
    }
}
