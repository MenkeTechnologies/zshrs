//! Port of `_complete_help` from
//! `Completion/Base/Widget/_complete_help`.
//!
//! Full upstream body (92 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xh
//! sh: 3  _complete_help() {
//! sh:  5    eval "$_comp_setup"
//! sh:  7    local _sort_tags=_help_sort_tags …
//! sh: 10    _shadow compadd compcall zstyle
//! sh: 11    compadd() { return 1 }
//! sh: 12    compcall() { _help_sort_tags use-compctl }
//! sh: 14    zstyle() { … capture-via-funcstack-walk … }
//! sh: 50    ${1:-_main_complete}
//! sh: 51  } always {
//! sh: 52    _unshadow compadd compcall zstyle
//! sh: 53  }
//! sh: 56  _help_sort_tags() { … emit grouped tag/style report … }
//! ```
//!
//! Diagnostic widget that runs the completion machinery and prints
//! which tags, functions, and styles were consulted. Wraps the
//! invocation in `_shadow compadd compcall zstyle` so we can
//! capture the live calls and a closing `_unshadow` to restore.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_shadow::{_shadow, _unshadow};
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::zle::complete::set_compadd_trace;

/// `_complete_help` — diagnostic widget. Optional `$1` selects the
/// inner completer (default `_main_complete`).
pub fn _complete_help(args: &[String]) -> i32 {
    let target = args
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "_main_complete".to_string());

    // sh:10 — `_shadow compadd compcall zstyle`. Snapshot the three
    //   names so any prior user-side definitions get restored by the
    //   trailing `_unshadow` regardless of what the trace layer
    //   installs in between.
    let _ = _shadow(&[
        "compadd".to_string(),
        "compcall".to_string(),
        "zstyle".to_string(),
    ]);

    // sh:11 — `compadd() { return 1 }`. Process-wide flag flips
    //   `bin_compadd` into trace mode: every call is recorded into
    //   `$_complete_help_funcs` and returns 1 without adding matches.
    //   Clear the buffer first so a re-invocation starts fresh.
    setaparam("_complete_help_funcs", Vec::new());
    set_compadd_trace(true);

    let ret = dispatch_function_call(&target, &[]).unwrap_or(1);

    // sh:52 — restore. Always flip the trace flag back even if the
    //   inner call panicked / errored, then pop the shadow frame so
    //   the three names go back to their prior bindings.
    set_compadd_trace(false);
    let _ = _unshadow();

    // sh:24-49 — original `_help_sort_tags` formats a per-tag report
    //   from `funcstack`/`compstate`. We don't have a funcstack walk
    //   yet, so emit the captured `compadd` call list as a stand-in
    //   so the user at least sees which match-emitters fired and
    //   with what args. The output is `-r` (raw, no quoting).
    let funcs = getaparam("_complete_help_funcs").unwrap_or_default();
    let body = if funcs.is_empty() {
        format!("{}: no compadd calls captured", target)
    } else {
        let mut lines = vec![format!("tags in context :completion:{}:", target)];
        for f in &funcs {
            lines.push(format!("  compadd {}", f));
        }
        lines.join("\n")
    };
    let _ = _message(&["-r".to_string(), body]);
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{getaparam, setaparam};

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_complete_help(&[]), 1);
    }

    #[test]
    fn clears_funcs_buffer_before_dispatch() {
        // sh:11 — pre-populate `_complete_help_funcs` with stale
        //   entries, run the widget (no executor → returns 1), and
        //   confirm the buffer was reset before dispatch even though
        //   no compadd calls fired.
        let _g = crate::test_util::global_state_lock();
        setaparam("_complete_help_funcs", vec!["stale_entry".to_string()]);
        let _ = _complete_help(&[]);
        let after = getaparam("_complete_help_funcs").unwrap_or_default();
        assert!(
            !after.iter().any(|s| s == "stale_entry"),
            "stale entries must be cleared at widget entry"
        );
    }
}
