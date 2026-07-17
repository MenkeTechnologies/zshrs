//! ZLE special-param write-back sync (Rust-only adapter).
//!
//! C's `makezleparams` (Src/Zle/zle_params.c:194) installs GSU-backed
//! special params: a widget's `BUFFER=x` assignment applies to the live
//! editor IMMEDIATELY through `set_buffer`. The Rust port snapshots
//! values into the paramtab instead, so widget writes land there and
//! must be diff-applied back to the editor. This module holds the
//! snapshot and the sync helpers used at every observable boundary:
//! `zle <widget>` calls inside a widget body (zle_thingy::bin_zle_call)
//! and widget exit (zle_main::execzlefunc). Interleavings like
//! zsh-expand's `LBUFFER=expanded; zle self-insert` then behave as if
//! the writes were live.
//!
//! Replace with real GSU special-param hooks when params.rs grows the
//! substrate; this module then deletes wholesale.

use std::sync::Mutex;

/// Widget-scope snapshot of the synced ZLE special params as of the
/// last `makezleparams` publish. `None` while no widget param scope
/// is active.
#[derive(Clone)]
pub struct ZleParamSnapshot {
    buffer: String,
    lbuffer: String,
    rbuffer: String,
    cursor: i64,
    /// `$PREDISPLAY` / `$POSTDISPLAY` — display-overlay text before/
    /// after the buffer. zsh-autosuggestions writes POSTDISPLAY from
    /// its widget wrappers; without write-back sync the ghost text
    /// never reached the editor (C's set_postdisplay GSU applies it
    /// live, zle_params.c:900).
    predisplay: String,
    postdisplay: String,
    /// `$region_highlight` user entries in their string form.
    region_highlight: Vec<String>,
    /// `$MARK` / `$REGION_ACTIVE` — region state (C set_mark /
    /// set_region_active GSU, zle_params.c:161/168). Visual-mode
    /// widgets write these.
    mark: i64,
    region_active: i64,
    /// `$CUTBUFFER` / `$killring` (C zle_params.c:149/155).
    cutbuffer: String,
    killring: Vec<String>,
}

static ZLE_PARAM_SNAPSHOT: Mutex<Option<ZleParamSnapshot>> = Mutex::new(None);

/// Arm the snapshot with the values `makezleparams` just published.
#[allow(clippy::too_many_arguments)]
pub fn arm_snapshot(
    buffer: String,
    lbuffer: String,
    rbuffer: String,
    cursor: i64,
    predisplay: String,
    postdisplay: String,
    region_highlight: Vec<String>,
    mark: i64,
    region_active: i64,
    cutbuffer: String,
    killring: Vec<String>,
) {
    *ZLE_PARAM_SNAPSHOT.lock().unwrap() = Some(ZleParamSnapshot {
        buffer,
        lbuffer,
        rbuffer,
        cursor,
        predisplay,
        postdisplay,
        region_highlight,
        mark,
        region_active,
        cutbuffer,
        killring,
    });
}

/// Drop the widget-scope snapshot on widget exit so out-of-scope
/// `zle` calls don't re-apply stale diffs.
pub fn clear_snapshot() {
    *ZLE_PARAM_SNAPSHOT.lock().unwrap() = None;
}

/// True while a widget param scope is active (makezleparams ran and
/// the snapshot hasn't been cleared).
pub fn active() -> bool {
    ZLE_PARAM_SNAPSHOT.lock().unwrap().is_some()
}

thread_local! {
    /// Reentry guard for [`live_write`]'s paramtab re-publish (the
    /// setsparam calls below would re-enter assignsparam's ZLE arm).
    static IN_LIVE_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while [`live_write`] is re-publishing — assignsparam's ZLE
/// arm must not re-route those writes.
pub fn in_live_write() -> bool {
    IN_LIVE_WRITE.with(|c| c.get())
}

/// Live GSU-adapter write for the ZLE editing specials. C's
/// `makezleparams` (Src/Zle/zle_params.c:194) installs REAL GSU
/// setters: `LBUFFER=x` mutates the editor immediately and
/// `$RBUFFER`/`$BUFFER`/`$CURSOR` reads derive from the ONE
/// line+cursor state. The snapshot model here left four independent
/// paramtab copies, so sequential widget mutations read STALE peers —
/// zsh-expand's snippet path (`LBUFFER=template; CURSOR=n;
/// RBUFFER=${RBUFFER:len}`) scrambled multiline templates. This
/// routes each write through the live editor setter, then re-publishes
/// the whole derived family into the paramtab so subsequent in-widget
/// READS are coherent, and re-arms the snapshot so the end-of-widget
/// diff doesn't double-apply.
///
/// Returns true when `name` was handled (an editing special while a
/// widget scope is active).
pub fn live_write(name: &str, val: &str) -> bool {
    if !active() || in_live_write() {
        return false;
    }
    use crate::ported::zle::zle_params as zp;
    match name {
        "BUFFER" => {
            // c:set_buffer (zle_params.c:250) — cursor clamps to len.
            zp::set_buffer(val);
            let len = val.chars().count();
            if zp::get_cursor() > len {
                zp::set_cursor(len);
            }
        }
        "LBUFFER" => zp::set_lbuffer(val), // c:set_lbuffer (zle_params.c:280)
        "RBUFFER" => zp::set_rbuffer(val), // c:set_rbuffer (zle_params.c:310)
        "CURSOR" => {
            // c:set_cursor (zle_params.c:340) — clamp into [0, len].
            let len = zp::get_buffer().chars().count() as i64;
            let n = val.trim().parse::<i64>().unwrap_or(0).clamp(0, len);
            zp::set_cursor(n as usize);
        }
        _ => return false,
    }
    // Re-publish the derived family so in-widget reads see live state.
    IN_LIVE_WRITE.with(|c| c.set(true));
    let buffer = zp::get_buffer();
    let lbuffer = zp::get_lbuffer();
    let rbuffer = zp::get_rbuffer();
    let cursor = zp::get_cursor() as i64;
    let _ = crate::ported::params::setsparam("BUFFER", &buffer);
    let _ = crate::ported::params::setsparam("LBUFFER", &lbuffer);
    let _ = crate::ported::params::setsparam("RBUFFER", &rbuffer);
    let _ = crate::ported::params::setiparam("CURSOR", cursor);
    IN_LIVE_WRITE.with(|c| c.set(false));
    // Re-arm so sync_from_paramtab sees these values as the new base.
    if let Some(snap) = ZLE_PARAM_SNAPSHOT.lock().unwrap().as_mut() {
        snap.buffer = buffer;
        snap.lbuffer = lbuffer;
        snap.rbuffer = rbuffer;
        snap.cursor = cursor;
    }
    true
}

/// Apply widget mutations of $BUFFER/$LBUFFER/$RBUFFER/$CURSOR from
/// the paramtab to the live editor. $BUFFER wins over $LBUFFER/
/// $RBUFFER when both changed; an LBUFFER/RBUFFER edit places the
/// cursor at the join point (C set_lbuffer/set_rbuffer,
/// zle_params.c:280-320). No-op when no widget scope is active.
pub fn sync_from_paramtab() {
    let snap = ZLE_PARAM_SNAPSHOT.lock().unwrap().clone();
    let Some(snap) = snap else {
        return;
    };
    let post_buf = crate::ported::params::getsparam("BUFFER").unwrap_or_default();
    let post_lbuf = crate::ported::params::getsparam("LBUFFER").unwrap_or_default();
    let post_rbuf = crate::ported::params::getsparam("RBUFFER").unwrap_or_default();
    let post_cur = crate::ported::params::getiparam("CURSOR");
    if post_buf != snap.buffer {
        crate::ported::zle::zle_params::set_buffer(&post_buf);
    } else if post_lbuf != snap.lbuffer || post_rbuf != snap.rbuffer {
        let joined = format!("{}{}", post_lbuf, post_rbuf);
        crate::ported::zle::zle_params::set_buffer(&joined);
        crate::ported::zle::zle_params::set_cursor(post_lbuf.chars().count());
    }
    if post_cur != snap.cursor && post_cur >= 0 {
        crate::ported::zle::zle_params::set_cursor(post_cur as usize);
    }
    // Display overlays — C's set_predisplay/set_postdisplay GSU setters
    // (zle_params.c:886/900) apply widget writes live; diff-apply here.
    let post_predisp = crate::ported::params::getsparam("PREDISPLAY").unwrap_or_default();
    if post_predisp != snap.predisplay {
        crate::ported::zle::zle_params::set_predisplay(Some(&post_predisp));
    }
    let post_postdisp = crate::ported::params::getsparam("POSTDISPLAY").unwrap_or_default();
    if post_postdisp != snap.postdisplay {
        crate::ported::zle::zle_params::set_postdisplay(Some(&post_postdisp));
    }
    // `$region_highlight` — C's set_region_highlight GSU setter
    // (zle_refresh.c:488) parses the entries live on assignment.
    let post_rh = crate::ported::params::getaparam("region_highlight").unwrap_or_default();
    if post_rh != snap.region_highlight {
        crate::ported::zle::zle_refresh::set_region_highlight(Some(&post_rh));
    }
    // `$MARK` / `$REGION_ACTIVE` — C's set_mark / set_region_active
    // GSU setters (zle_params.c:161/168) apply widget writes live.
    let post_mark = crate::ported::params::getiparam("MARK");
    if post_mark != snap.mark && post_mark >= 0 {
        crate::ported::zle::zle_params::set_mark(post_mark as usize);
    }
    let post_ra = crate::ported::params::getiparam("REGION_ACTIVE");
    if post_ra != snap.region_active {
        let _ = crate::ported::zle::zle_params::set_region_active(post_ra);
    }
    // `$CUTBUFFER` / `$killring` (zle_params.c:149/155).
    let post_cutbuf = crate::ported::params::getsparam("CUTBUFFER").unwrap_or_default();
    if post_cutbuf != snap.cutbuffer {
        crate::ported::zle::zle_params::set_cutbuffer(&post_cutbuf);
    }
    let post_kr = crate::ported::params::getaparam("killring").unwrap_or_default();
    if post_kr != snap.killring {
        crate::ported::zle::zle_params::set_killring(Some(&post_kr));
    }
}

/// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART !!!
/// C's `execzlefunc` epilogue (Src/Zle/zle_main.c:1579-1595) runs once
/// at the single exit point. The Rust dispatch is split across
/// `zle_main::execute_widget` (zlecore's inline arm) and
/// `zle_main::execzlefunc` (nested / `zle <widget>` calls), each with
/// multiple returns, so the epilogue lives here and both call it with
/// their entry-time snapshot (`nestedvichg` = vichgflag at entry,
/// `isrepeat` = viinrepeat==3 at entry — zle_main.c:1423-1424).
///
/// c:1579-1593 — end the vi change this widget constituted:
///   * in vicmd mode, a successful change is promoted `lastvichg =
///     curvichg` (the `.`-repeat target); a failed one is discarded.
///   * out of vicmd mode the change continues recording through
///     insert mode (`vichgflag = 1`) until `vicmdmode()` ends it.
/// c:1594-1595 — a `.`-repeat that entered insert mode stays live
/// (`viinrepeat = 1`) so `vicmdmode()` can finish it.
pub fn end_vichg_frame(nestedvichg: i32, isrepeat: bool, ret: i32) {
    use crate::ported::zle::zle_vi::{CURVICHG, LASTVICHG, VICHGFLAG, VIINREPEAT};
    use std::sync::atomic::Ordering::SeqCst;
    // c:1580 — `if (vichgflag == 2 && !nestedvichg)`.
    if VICHGFLAG.load(SeqCst) == 2 && nestedvichg == 0 {
        let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
        if crate::ported::zle::zle_h::invicmdmode(&kn) {
            // c:1581
            if ret != 0 {
                // c:1582-1583 — `free(curvichg.buf);` — failed change.
                let mut cur = CURVICHG.lock().unwrap();
                cur.buf.clear();
                cur.bufsz = 0;
                cur.bufptr = 0;
            } else {
                // c:1585-1587 — `lastvichg = curvichg;` — a completed
                // vicmd-mode change becomes the `.`-repeat target.
                let mut last = LASTVICHG.lock().unwrap();
                let mut cur = CURVICHG.lock().unwrap();
                last.mod_ = cur.mod_.clone();
                last.buf = std::mem::take(&mut cur.buf);
                last.bufsz = cur.bufsz;
                last.bufptr = cur.bufptr;
                cur.bufsz = 0;
                cur.bufptr = 0; // c:1590 `curvichg.buf = NULL;`
            }
            VICHGFLAG.store(0, SeqCst); // c:1589
        } else {
            // c:1592 — vi change continues while in insert mode.
            VICHGFLAG.store(1, SeqCst);
        }
    }
    if isrepeat {
        // c:1594-1595 — `viinrepeat = !invicmdmode();`
        let kn = crate::ported::zle::zle_keymap::curkeymapname().clone();
        VIINREPEAT.store(
            if crate::ported::zle::zle_h::invicmdmode(&kn) {
                0
            } else {
                1
            },
            SeqCst,
        );
    }
}

#[cfg(test)]
mod tests {
    /// `LBUFFER+=X` inside a widget scope must APPEND to the live
    /// editor value, not assign the appended text (zsh-autopair's
    /// `_ap-self-insert` — `LBUFFER+=$1; RBUFFER="$2$RBUFFER"` —
    /// collapsed `echo hi "` to `""`). assignsparam's ZLE live-write
    /// arm fired with the raw `+=` operand before ASSPM_AUGMENT was
    /// applied; C concatenates before gsu setfn dispatch
    /// (Src/params.c:2742-2748).
    #[test]
    fn zle_special_augment_appends_to_live_value() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zle::zle_params as zp;
        zp::set_buffer("echo hi ");
        zp::set_cursor(8);
        // Arm a widget scope so live_write is active (makezleparams shape).
        super::arm_snapshot(
            "echo hi ".into(),
            "echo hi ".into(),
            String::new(),
            8,
            String::new(),
            String::new(),
            Vec::new(),
        );
        let _ = crate::ported::params::assignsparam(
            "LBUFFER",
            "\"",
            crate::ported::zsh_h::ASSPM_AUGMENT,
        );
        let _ = crate::ported::params::assignsparam(
            "RBUFFER",
            "\"",
            crate::ported::zsh_h::ASSPM_AUGMENT,
        );
        super::clear_snapshot();
        assert_eq!(zp::get_buffer(), "echo hi \"\"", "append, not assign");
        assert_eq!(zp::get_cursor(), 9, "cursor sits between the pair");
        zp::set_buffer("");
    }
}
