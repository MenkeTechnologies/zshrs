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

use crate::compsys::ported::shared::dispatch_action_command;
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
        // sh:4-5 — a plain COMMAND WORD: `execcmd` resolves shfunc/port/
        // plugin (`Src/exec.c:3105-3109`), then builtin, then `$PATH`, and a
        // name that resolves nowhere reaches c:903 `command not found` with
        // c:908 status 127. `dispatch_action_command` (shared.rs:1407) is that
        // resolution; it also publishes the caller line, which `FnScope`
        // zeroed on entry (`Src/exec.c:1429`).
        return dispatch_action_command("_history_modifiers", &["p".to_string()], 4);
    }
    // sh:8 — same, and reachable on this host: `$fpath`'s first
    // `_parameters` (`~/.zpwr/autoload/comp_utils/_parameters`) is UNTAGGED,
    // so `compinit` sh:507-526 registers nothing for the name and the Rust
    // port stands aside for the file (router.rs `has_fpath_override`). zsh
    // prints `_parameter:8: command not found: _parameters`; before this the
    // port returned a silent 1.
    dispatch_action_command("_parameters", &["-e".to_string()], 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no executor wired, `_parameters` is neither a shell function, nor
    /// a builtin, nor on `$PATH` — the `Src/exec.c:903` case — so the port
    /// reports `command not found` and returns c:908's 127. It used to
    /// swallow the diagnostic and return 1.
    #[test]
    fn unresolvable_parameters_reports_command_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_parameter(), 127);
    }
}
