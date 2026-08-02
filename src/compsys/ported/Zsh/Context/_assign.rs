//! Port of `_assign` from `Completion/Zsh/Context/_assign`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef -assign-parameter-
//! sh:2
//! sh:3  _parameters -g "^*readonly*" -S ''
//! ```
//!
//! Delegates to `_parameters` (sibling shell fn outside the engine
//! cluster) via `crate::ported::exec::dispatch_function_call`. Returns 1
//! when no executor is wired.

use crate::ported::exec::dispatch_function_call;

/// `_assign` — `-assign-parameter-` context completion: list
/// writable (non-readonly) parameters with no suffix.
pub fn _assign() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_assign");
    dispatch_function_call(
        "_parameters",
        &[
            "-g".to_string(),
            "^*readonly*".to_string(),
            "-S".to_string(),
            "".to_string(),
        ],
    )
    .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_assign(), 1);
    }
}
