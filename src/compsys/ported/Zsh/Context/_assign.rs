//! Port of `_assign` from `Completion/Zsh/Context/_assign`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef -assign-parameter-
//! sh:2
//! sh:3  _parameters -g "^*readonly*" -S ''
//! ```
//!
//! sh:3 is a plain COMMAND WORD, so it resolves the way `execcmd` resolves
//! one — shfunc/port/plugin (`Src/exec.c:3105-3109`), then builtin, then
//! `$PATH`, then `Src/exec.c:903`'s `command not found` and c:908's status
//! 127 — which is what `shared::dispatch_action_command` does. On this host
//! the name is genuinely unresolvable: `$fpath`'s first `_parameters`
//! (`~/.zpwr/autoload/comp_utils/_parameters`) is UNTAGGED, so `compinit`
//! sh:507-526 registers nothing for it and the Rust port stands aside for the
//! file (router.rs `has_fpath_override`).

use crate::compsys::ported::shared::dispatch_action_command;

/// `_assign` — `-assign-parameter-` context completion: list
/// writable (non-readonly) parameters with no suffix.
pub fn _assign() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_assign");
    dispatch_action_command(
        "_parameters",
        &[
            "-g".to_string(),
            "^*readonly*".to_string(),
            "-S".to_string(),
            "".to_string(),
        ],
        3,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no executor wired `_parameters` resolves to no shell function, no
    /// builtin and nothing on `$PATH` — the `Src/exec.c:903` case — so the
    /// port reports `command not found` and returns c:908's 127 instead of
    /// swallowing the diagnostic behind a silent 1.
    #[test]
    fn unresolvable_parameters_reports_command_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_assign(), 127);
    }
}
