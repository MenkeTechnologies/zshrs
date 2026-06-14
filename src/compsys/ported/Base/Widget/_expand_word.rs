//! Port of `_expand_word` from `Completion/Base/Widget/_expand_word`.
//!
//! Full upstream body (13 lines verbatim):
//! ```text
//! sh: 1  #compdef -K _expand_word complete-word \C-xe _list_expansions list-choices \C-xd
//! sh: 2
//! sh: 3  # Simple completion front-end implementing expansion.
//! sh: 4
//! sh: 5  local curcontext="$curcontext"
//! sh: 6
//! sh: 7  if [[ -z "$curcontext" ]]; then
//! sh: 8    curcontext="expand-word:::"
//! sh: 9  else
//! sh:10    curcontext="expand-word:${curcontext#*:}"
//! sh:11  fi
//! sh:12
//! sh:13  _main_complete _expand
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};

/// `_expand_word` — front-end widget: set context to `expand-word`
/// and dispatch `_main_complete _expand`.
pub fn _expand_word() -> i32 {
    // sh:5 snapshot for shell-local restore
    let saved = getsparam("curcontext").unwrap_or_default();

    // sh:7-11
    let new_ctx = if saved.is_empty() {
        "expand-word:::".to_string()
    } else {
        let tail = saved.splitn(2, ':').nth(1).unwrap_or("");
        format!("expand-word:{}", tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    // sh:13
    let r = dispatch_function_call("_main_complete", &["_expand".to_string()]).unwrap_or(1);

    // Restore shell-local scoping
    let _ = setsparam("curcontext", &saved);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_expand_word(), 1);
    }

    #[test]
    fn restores_curcontext_after_call() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "original:ctx:foo:bar");
        let _ = _expand_word();
        assert_eq!(
            getsparam("curcontext").as_deref(),
            Some("original:ctx:foo:bar")
        );
    }
}
