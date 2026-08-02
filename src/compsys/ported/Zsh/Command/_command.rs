//! Port of `_command` from `Completion/Zsh/Command/_command`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh:1  #compdef command
//! sh:2
//! sh:3  _arguments \
//! sh:4    '-v[indicate result of command search]:*:command:_path_commands' \
//! sh:5    '-V[show result of command search in verbose form]:*:command:_path_commands' \
//! sh:6    '(-)-p[use default PATH to find command]' \
//! sh:7    '*:: : _normal -p $service'
//! ```

use crate::ported::exec::dispatch_function_call;

/// `_command` — `command` builtin completion: pure `_arguments`
/// dispatch (sibling shell fn). Returns 1 without an executor.
pub fn _command() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_command");
    dispatch_function_call(
        "_arguments",
        &[
            "-v[indicate result of command search]:*:command:_path_commands".to_string(),
            "-V[show result of command search in verbose form]:*:command:_path_commands"
                .to_string(),
            "(-)-p[use default PATH to find command]".to_string(),
            "*:: : _normal -p $service".to_string(),
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
        assert_eq!(_command(), 1);
    }
}
