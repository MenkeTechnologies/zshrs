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

use crate::compsys::ported::shared::dispatch_action_command;

/// `_math_params` — complete parameter names usable in math contexts
/// (integer/float typed). Falls back to unfiltered `_parameters` if
/// the filtered call yields nothing.
///
/// Both calls publish the upstream line `3` first. `FnScope::enter` zeroes
/// `lineno` on the way in (`shared.rs:817-821`, standing in for
/// `Src/exec.c:1429`'s `oldlineno = lineno`), and nothing restores it before
/// the callee's frame is pushed — `doshfunc` records the CALLER's `lineno` at
/// push time (c:Src/exec.c:6013), which is what `$functrace` and
/// `$funcfiletrace` read back. Without this the two parameters report
/// `_math_params:0` where zsh reports `_math_params:3`. Measured with
/// `comptab_parity.py --case 'let /usr/share/zsh/5.9/f' --keys tab`, whose
/// listing prints those parameters' values:
///   zsh  : `_math_params:3 _alternative:63 _math:12 (eval):1 …`
///   zshrs: `_math_params:0 _alternative:63 _math:0  (eval):1 …`
/// The same value prefixes any diagnostic the callee raises
/// (`Src/utils.c:301-305`).
pub fn _math_params() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_math_params");
    // sh:3  _parameters -g '(integer|float)*'
    //
    // Both halves of the `||` list are plain COMMAND WORDS, so each resolves
    // the way `execcmd` resolves one — shfunc/port/plugin
    // (`Src/exec.c:3105-3109`), then builtin, then `$PATH`, and a name that
    // resolves nowhere reaches c:903's `command not found` with status 127.
    // `dispatch_action_command` (shared.rs:1407) IS that resolution, and it
    // publishes the caller line before anything can diagnose.
    //
    // This is not hypothetical here: `$fpath`'s first `_parameters` on this
    // host is `~/.zpwr/autoload/comp_utils/_parameters`, whose first word is
    // a bare `#`, so `compinit` registers NOTHING for the name (compinit
    // sh:507-526 dispatches on `#compdef`/`#autoload` only) and the Rust port
    // stands aside for the file (router.rs `has_fpath_override`). Measured on
    // `let <TAB>` (`_math` sh:12's `math-parameters:...:_math_params`):
    //     zsh    `_math_params:3: command not found: _parameters`  (twice —
    //            the failed first word runs the `||` tail, which also fails)
    //     zshrs  (nothing at all)
    let r = dispatch_action_command(
        "_parameters",
        &["-g".to_string(), "(integer|float)*".to_string()],
        3,
    );
    if r == 0 {
        return 0;
    }
    // sh:3 tail  || _parameters
    dispatch_action_command("_parameters", &[], 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no executor wired, `_parameters` resolves to no shell function,
    /// no builtin and nothing on `$PATH`, which is exactly the `Src/exec.c:903`
    /// case: the shell reports `command not found` and the command's status is
    /// c:908's 127. Both halves of sh:3's `||` list take that arm, so the
    /// second one's status is what `_math_params` returns. Before the fix this
    /// was a silent `1`, i.e. the diagnostic zsh prints here was swallowed.
    #[test]
    fn unresolvable_parameters_reports_command_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_math_params(), 127);
    }
}
