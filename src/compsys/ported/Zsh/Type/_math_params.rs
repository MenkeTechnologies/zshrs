//! Port of `_math_params` from `Completion/Zsh/Type/_math_params`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  _parameters -g '(integer|float)*' || _parameters
//! ```
//!
//! `_parameters` is a sibling shell function (not engine cluster);
//! dispatch via `exec accessors`. The shell `|| _parameters` retry runs
//! the unfiltered call when the filtered one finds nothing.

use crate::ported::exec::dispatch_function_call;

/// `_math_params` — complete parameter names usable in math contexts
/// (integer/float typed). Falls back to unfiltered `_parameters` if
/// the filtered call yields nothing.
pub fn _math_params() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_math_params");
    // sh:3  _parameters -g '(integer|float)*'
    let r = dispatch_function_call(
        "_parameters",
        &["-g".to_string(), "(integer|float)*".to_string()],
    )
    .unwrap_or(1);
    if r == 0 {
        return 0;
    }
    // sh:3 tail  || _parameters
    dispatch_function_call("_parameters", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_math_params(), 1);
    }
}
