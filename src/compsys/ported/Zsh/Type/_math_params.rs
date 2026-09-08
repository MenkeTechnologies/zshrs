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

use crate::compsys::ported::shared::set_sh_lineno;
use crate::ported::exec::dispatch_function_call;

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
    set_sh_lineno(3);
    let r = dispatch_function_call(
        "_parameters",
        &["-g".to_string(), "(integer|float)*".to_string()],
    )
    .unwrap_or(1);
    if r == 0 {
        return 0;
    }
    // sh:3 tail  || _parameters
    set_sh_lineno(3);
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
