//! Port of `_correct` from `Completion/Base/Completer/_correct`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is mainly a wrapper around the more general `_approximate'.
//! sh: 4  # By setting `compstate[pattern_match]' to something unequal to `*' and
//! sh: 5  # then calling `_approximate', we get only corrections, not all strings
//! sh: 6  # with the corrected prefix and something after it.
//! sh: 7  #
//! sh: 8  # Supported configuration keys are the same as for `_approximate', only
//! sh: 9  # starting with `correct'.
//! sh:10
//! sh:11  local ret=1 opm="$compstate[pattern_match]"
//! sh:12
//! sh:13  compstate[pattern_match]='-'
//! sh:14
//! sh:15  _approximate && ret=0
//! sh:16
//! sh:17  compstate[pattern_match]="$opm"
//! sh:18
//! sh:19  return ret
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_correct` — spelling-correction completer: wraps `_approximate`
/// with `compstate[pattern_match]` swapped to `-` for the duration.
pub fn _correct() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_correct");
    // sh:11
    let mut ret: i32 = 1;
    let opm = get_compstate_str("pattern_match").unwrap_or_default();
    // sh:13
    set_compstate_str("pattern_match", "-");
    // sh:15
    if dispatch_function_call("_approximate", &[]).unwrap_or(1) == 0 {
        ret = 0;
    }
    // sh:17
    set_compstate_str("pattern_match", &opm);
    // sh:19
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_correct(), 1);
    }

    #[test]
    fn restores_pattern_match_after_run() {
        // sh:17 — original compstate[pattern_match] must be put back.
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("pattern_match", "original");
        let _ = _correct();
        assert_eq!(
            get_compstate_str("pattern_match").as_deref(),
            Some("original")
        );
    }
}
