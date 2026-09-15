//! Port of `_history_complete_word` from
//! `Completion/Base/Widget/_history_complete_word`.
//!
//! Full upstream body (121 lines, abridged):
//! ```text
//! sh:  1  #compdef -K _history-complete-older complete-word \e/ …
//! sh: 18  _history_complete_word() {
//! sh: 24    if [[ -z "$curcontext" ]]; then curcontext=history-words:::
//! sh: 26    else curcontext="history-words${curcontext#*:}"
//! sh: 30    direction=newer / older based on $WIDGET name
//! sh: 34    stop = style :stop
//! sh: 36    list-style governs compstate[list]
//! sh: 38    if last widget was a history-completer && we have old state:
//! sh: 39      navigate within existing list (older/newer/stop boundary)
//! sh: 81    else _hist_stop=; _hist_old_prefix=$PREFIX; _history_complete_word_gen_matches
//! sh: 93  }
//! sh: 97  _history_complete_word_gen_matches() {
//! sh: 99    _main_complete _history
//! sh:103   compute compstate[insert] direction-aware
//! sh:119 }
//! ```
//!
//! Navigation/state-management around the `_history` completer.
//! Reads `$WIDGET` to decide direction.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::shared::{zstyle_T, zstyle_t};
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// sh:95-119 `_history_complete_word_gen_matches`.
///
/// The port used to stop after sh:103, so `compstate[insert]` was never set
/// on a fresh completion: zsh inserts the newest history word (`true zpfEND`)
/// where zshrs only listed the two matches.
fn gen_matches(direction: &str) -> i32 {
    // sh:97
    let hist_stop = getsparam("_hist_stop").unwrap_or_default();
    if !hist_stop.is_empty() {
        let _ = setsparam("PREFIX", &getsparam("_hist_old_prefix").unwrap_or_default());
    }
    // sh:99
    let _ = dispatch_function_call("_main_complete", &["_history".to_string()]);
    // sh:101
    let ctx = getsparam("curcontext").unwrap_or_default();
    if zstyle_T(&format!(":completion:{}:history-words", ctx), "list") != 0 {
        set_compstate_str("list", "");
    }
    // sh:103
    let menu_len = get_compstate_str("nmatches").unwrap_or_default();
    let _ = setsparam("_hist_menu_length", &menu_len);
    // sh:105-114
    let lastcomp_insert =
        crate::compsys::ported::shared::assoc_get("_lastcomp", "insert").unwrap_or_default();
    if !lastcomp_insert.contains("unambig") {
        let n: i64 = menu_len.parse().unwrap_or(0);
        let stop_adj = if hist_stop.is_empty() { 0 } else { 1 };
        match direction {
            "newer" => set_compstate_str("insert", &(n - stop_adj).to_string()),
            "older" => set_compstate_str("insert", &(1 + stop_adj).to_string()),
            _ => {}
        }
    }
    // sh:116-118
    let _ = setsparam("_hist_stop", "");
    0
}

/// `_history_complete_word` — history-word navigation widget.
pub fn _history_complete_word() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_history_complete_word");
    let saved_ctx = getsparam("curcontext").unwrap_or_default();
    let new_ctx = if saved_ctx.is_empty() {
        "history-words:::".to_string()
    } else {
        let tail = saved_ctx.splitn(2, ':').nth(1).unwrap_or("");
        format!("history-words{}", tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    let widget = getsparam("WIDGET").unwrap_or_default();
    let direction = if widget.ends_with("newer") {
        "newer"
    } else {
        "older"
    };

    // sh:34 — `zstyle -t … stop && stop=yes`, a VALUE test; see [`zstyle_t`].
    let stop_on = zstyle_t(&format!(":completion:{}:history-words", new_ctx), "stop") == 0;

    // sh:36 — `zstyle -T … list || compstate[list]=''`: an UNSET style is
    //   true, so the list is cleared only when the style is set to a
    //   non-boolean value. See [`zstyle_T`].
    //
    //   The previous spelling was `if !testforstyle(…) == 0`, which Rust
    //   parses as `(!testforstyle(…)) == 0` with `!` the BITWISE NOT of an
    //   i32 — true only for a return of -1, which `testforstyle` never
    //   produces. The branch was therefore dead and `compstate[list]` was
    //   never cleared.
    if zstyle_T(&format!(":completion:{}:history-words", new_ctx), "list") != 0 {
        set_compstate_str("list", "");
    }

    let lastwidget = getsparam("LASTWIDGET").unwrap_or_default();
    let old_list = get_compstate_str("old_list").unwrap_or_default();
    let hist_stop = getsparam("_hist_stop").unwrap_or_default();
    let in_history_chain = lastwidget.starts_with("_history-complete-");
    let menu_len: i64 = getsparam("_hist_menu_length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let old_insert: i64 = get_compstate_str("old_insert")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if in_history_chain && (!old_list.is_empty() || !hist_stop.is_empty()) {
        let old_prefix = getsparam("_hist_old_prefix").unwrap_or_default();
        if direction == "older" {
            if hist_stop == "new" {
                // sh:41-45
                let _ = setsparam("PREFIX", &old_prefix);
                let _ = gen_matches(direction);
                set_compstate_str("insert", "2");
                let _ = setsparam("_hist_stop", "");
            } else if hist_stop == "old" {
                // sh:46-50
                let _ = setsparam("PREFIX", &old_prefix);
                let _ = gen_matches(direction);
                set_compstate_str("insert", "1");
                let _ = setsparam("_hist_stop", "");
            } else if old_insert < menu_len {
                set_compstate_str("old_list", "keep");
                set_compstate_str("insert", &(old_insert + 1).to_string());
            } else if stop_on {
                let _ = setsparam("_hist_stop", "old");
                let _ = _message(&["beginning of history reached".to_string()]);
                let _ = setsparam("curcontext", &saved_ctx);
                return 1;
            } else {
                set_compstate_str("old_list", "keep");
                set_compstate_str("insert", "1");
            }
        } else {
            // newer
            let nmatches = || -> i64 {
                get_compstate_str("nmatches")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0)
            };
            if hist_stop == "old" {
                // sh:63-67
                let _ = setsparam("PREFIX", &old_prefix);
                let _ = gen_matches(direction);
                set_compstate_str("insert", &(nmatches() - 1).to_string());
                let _ = setsparam("_hist_stop", "");
            } else if hist_stop == "new" {
                // sh:68-72
                let _ = setsparam("PREFIX", &old_prefix);
                let _ = gen_matches(direction);
                set_compstate_str("insert", &nmatches().to_string());
                let _ = setsparam("_hist_stop", "");
            } else if old_insert > 1 {
                set_compstate_str("old_list", "keep");
                set_compstate_str("insert", &(old_insert - 1).to_string());
            } else if stop_on {
                let _ = setsparam("_hist_stop", "new");
                let _ = _message(&["end of history reached".to_string()]);
                let _ = setsparam("curcontext", &saved_ctx);
                return 1;
            } else {
                set_compstate_str("old_list", "keep");
                set_compstate_str("insert", &menu_len.to_string());
            }
        }
        let _ = setsparam("curcontext", &saved_ctx);
        return 0;
    }

    // Fresh start
    let _ = setsparam("_hist_stop", "");
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let _ = setsparam("_hist_old_prefix", &prefix);
    let _ = gen_matches(direction);

    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let _ = setsparam("curcontext", &saved_ctx);
    if nm == 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "_history-complete-older");
        let _ = setsparam("LASTWIDGET", "");
        // `compstate[nmatches]` is a LIVE GSU integer (complete.c:1411)
        // backed by the `nmatches` counter, so writing it through
        // `set_compstate_str` is a no-op — the reader ignores the stored
        // hash for this one key. Zero the counter itself, or matches
        // added by an earlier completion test make this widget report
        // "found something" and return 0.
        crate::ported::zle::compcore::nmatches.store(0, std::sync::atomic::Ordering::Relaxed);
        set_compstate_str("nmatches", "0");
        assert_eq!(_history_complete_word(), 1);
    }
}
