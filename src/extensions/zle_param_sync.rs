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
}

static ZLE_PARAM_SNAPSHOT: Mutex<Option<ZleParamSnapshot>> = Mutex::new(None);

/// Arm the snapshot with the values `makezleparams` just published.
pub fn arm_snapshot(
    buffer: String,
    lbuffer: String,
    rbuffer: String,
    cursor: i64,
    predisplay: String,
    postdisplay: String,
    region_highlight: Vec<String>,
) {
    *ZLE_PARAM_SNAPSHOT.lock().unwrap() = Some(ZleParamSnapshot {
        buffer,
        lbuffer,
        rbuffer,
        cursor,
        predisplay,
        postdisplay,
        region_highlight,
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
}
