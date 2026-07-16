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

/// (BUFFER, LBUFFER, RBUFFER, CURSOR) as of the last sync. `None`
/// while no widget param scope is active.
static ZLE_PARAM_SNAPSHOT: Mutex<Option<(String, String, String, i64)>> = Mutex::new(None);

/// Arm the snapshot with the values `makezleparams` just published.
pub fn arm_snapshot(buffer: String, lbuffer: String, rbuffer: String, cursor: i64) {
    *ZLE_PARAM_SNAPSHOT.lock().unwrap() = Some((buffer, lbuffer, rbuffer, cursor));
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
    let Some((pre_buf, pre_lbuf, pre_rbuf, pre_cur)) = snap else {
        return;
    };
    let post_buf = crate::ported::params::getsparam("BUFFER").unwrap_or_default();
    let post_lbuf = crate::ported::params::getsparam("LBUFFER").unwrap_or_default();
    let post_rbuf = crate::ported::params::getsparam("RBUFFER").unwrap_or_default();
    let post_cur = crate::ported::params::getiparam("CURSOR");
    if post_buf != pre_buf {
        crate::ported::zle::zle_params::set_buffer(&post_buf);
    } else if post_lbuf != pre_lbuf || post_rbuf != pre_rbuf {
        let joined = format!("{}{}", post_lbuf, post_rbuf);
        crate::ported::zle::zle_params::set_buffer(&joined);
        crate::ported::zle::zle_params::set_cursor(post_lbuf.chars().count());
    }
    if post_cur != pre_cur && post_cur >= 0 {
        crate::ported::zle::zle_params::set_cursor(post_cur as usize);
    }
}
