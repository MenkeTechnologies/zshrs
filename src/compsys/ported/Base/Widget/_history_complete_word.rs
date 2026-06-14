//! Port of `_history_complete_word` from
//! `Completion/Base/Widget/_history_complete_word`.
//!
//! Full upstream body (121 lines, abridged):
//! ```text
//! sh: 1  #compdef -K _history-complete-older complete-word \e/ …
//! sh:18  _history_complete_word() {
//! sh:24    if [[ -z "$curcontext" ]]; then curcontext=history-words:::
//! sh:26    else curcontext="history-words${curcontext#*:}"
//! sh:30    direction=newer / older based on $WIDGET name
//! sh:34    stop = style :stop
//! sh:36    list-style governs compstate[list]
//! sh:38    if last widget was a history-completer && we have old state:
//! sh:39      navigate within existing list (older/newer/stop boundary)
//! sh:81    else _hist_stop=; _hist_old_prefix=$PREFIX; _history_complete_word_gen_matches
//! sh:86  }
//! sh:97  _history_complete_word_gen_matches() {
//! sh:99    _main_complete _history
//! sh:103   compute compstate[insert] direction-aware
//! sh:120 }
//! ```
//!
//! Navigation/state-management around the `_history` completer.
//! Reads `$WIDGET` to decide direction.

use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// Helper: dispatch `_main_complete _history` to seed matches.
fn gen_matches() -> i32 {
    let r = dispatch_function_call("_main_complete", &["_history".to_string()]).unwrap_or(1);
    let _hist_menu_length = get_compstate_str("nmatches").unwrap_or_default();
    let _ = setsparam("_hist_menu_length", &_hist_menu_length);
    r
}

/// `_history_complete_word` — history-word navigation widget.
pub fn _history_complete_word() -> i32 {
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

    let stop_on = testforstyle(&format!(":completion:{}:history-words", new_ctx), "stop") == 0;

    if !testforstyle(&format!(":completion:{}:history-words", new_ctx), "list") == 0 {
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
        if direction == "older" {
            if old_insert < menu_len {
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
            if old_insert > 1 {
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
    let _ = gen_matches();

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
        set_compstate_str("nmatches", "0");
        assert_eq!(_history_complete_word(), 1);
    }
}
