//! Port of `_correct_word` from `Completion/Base/Widget/_correct_word`.
//!
//! Full upstream body (15 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xc
//! sh: 2
//! sh: 3  # Simple completion front-end implementing spelling correction.
//! sh: 4  # The maximum number of errors is set quite high, and
//! sh: 5  # the numeric prefix can be used to specify a different value.
//! sh: 6
//! sh: 7  local curcontext="$curcontext"
//! sh: 8
//! sh: 9  if [[ -z "$curcontext" ]]; then
//! sh:10    curcontext="correct-word:::"
//! sh:11  else
//! sh:12    curcontext="correct-word:${curcontext#*:}"
//! sh:13  fi
//! sh:14
//! sh:15  _main_complete _correct
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};

/// `_correct_word` — front-end widget for spell-correction
/// completion via `_main_complete _correct`.
pub fn _correct_word() -> i32 {
    let saved = getsparam("curcontext").unwrap_or_default();
    let new_ctx = if saved.is_empty() {
        "correct-word:::".to_string()
    } else {
        let tail = saved.splitn(2, ':').nth(1).unwrap_or("");
        format!("correct-word:{}", tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    let r = dispatch_function_call("_main_complete", &["_correct".to_string()]).unwrap_or(1);

    let _ = setsparam("curcontext", &saved);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_correct_word(), 1);
    }
}
