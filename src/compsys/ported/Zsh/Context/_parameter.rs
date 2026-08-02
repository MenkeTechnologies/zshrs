//! Port of `_parameter` from `Completion/Zsh/Context/_parameter`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #compdef -parameter-
//! sh:2
//! sh:3  if compset -P '*:'; then
//! sh:4    _history_modifiers p
//! sh:5    return
//! sh:6  fi
//! sh:7
//! sh:8  _parameters -e
//! ```
//!
//! `compset -P '*:'` shifts past a leading `prefix:` match. When
//! present, dispatch `_history_modifiers p`; else `_parameters -e`.

use crate::ported::exec::dispatch_function_call;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_parameter` — `${...}` parameter-expansion context completion.
pub fn _parameter() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_parameter");
    // sh:3  if compset -P '*:'
    if bin_compset(
        "compset",
        &["-P".to_string(), "*:".to_string()],
        &make_ops(),
        0,
    ) == 0
    {
        // sh:4-5
        return dispatch_function_call("_history_modifiers", &["p".to_string()]).unwrap_or(1);
    }
    // sh:8
    dispatch_function_call("_parameters", &["-e".to_string()]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_parameter(), 1);
    }
}
