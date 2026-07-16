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
