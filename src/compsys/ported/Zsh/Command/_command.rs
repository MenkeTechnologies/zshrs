//! Port of `_command` from `Completion/Zsh/Command/_command`.
//!
//! Full upstream body (master, 7 lines verbatim):
//! ```text
//! sh:1  #compdef command
//! sh:2
//! sh:3  _arguments -S : \
//! sh:4    '-v[indicate result of command search]:*:command:_path_commands' \
//! sh:5    '-V[show result of command search in verbose form]:*:command:_path_commands' \
//! sh:6    '(-)-p[use default PATH to find command]' \
//! sh:7    '*:: : _normal -p $service'
//! ```
//!
//! zsh 5.9.1/5.9.2 had a bare `_arguments \` at sh:3; master passes `-S :`
//! so a `--` ends the option words. A drop-in run as `zsh` keeps 5.9.2's.

use crate::ported::exec::dispatch_function_call;

/// `_command` — `command` builtin completion: pure `_arguments`
/// dispatch (sibling shell fn). Returns 1 without an executor.
pub fn _command() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_command");
    let mut argv: Vec<String> = Vec::new();
    if !crate::extensions::emulation_startup::emulating_592() {
        argv.extend(["-S".to_string(), ":".to_string()]); // sh:3
    }
    argv.extend([
        "-v[indicate result of command search]:*:command:_path_commands".to_string(),
        "-V[show result of command search in verbose form]:*:command:_path_commands"
            .to_string(),
        "(-)-p[use default PATH to find command]".to_string(),
        "*:: : _normal -p $service".to_string(),
    ]);
    dispatch_function_call("_arguments", &argv).unwrap_or(1)
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
