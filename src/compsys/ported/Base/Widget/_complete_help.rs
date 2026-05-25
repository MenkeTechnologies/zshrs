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
//! which tags, functions, and styles were consulted. The
//! `_shadow`-based capture machinery isn't replicable in our port
//! without a real shell function table. We dispatch the inner fn
//! and emit a stub message.

use crate::compsys::ported::_message::_message;
use crate::ported::exec_hooks::dispatch_function_call;

/// `_complete_help` — diagnostic widget. Optional `$1` selects the
/// inner completer (default `_main_complete`).
pub fn _complete_help(args: &[String]) -> i32 {
    let target = args
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "_main_complete".to_string());
    let ret = dispatch_function_call(&target, &[]).unwrap_or(1);
    let _ = _message(&[
        "-r".to_string(),
        format!(
            "{}: tag/function/style trace requires _shadow integration",
            target
        ),
    ]);
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_complete_help(&[]), 1);
    }
}
