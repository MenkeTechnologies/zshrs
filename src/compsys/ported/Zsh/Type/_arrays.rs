//! Port of `_arrays` from `Completion/Zsh/Type/_arrays`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef shift
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted arrays expl array _parameters "$@" -g '*array*'
//! ```
//!
//! `_wanted ... _parameters` — _wanted's action is the literal
//! command `_parameters "$@" -g '*array*'`. We dispatch the action
//! chunk through `_all_labels`'s normal action-dispatch path, which
//! routes shell-fn calls via `crate::ported::exec::dispatch_function_call`.

use crate::compsys::ported::_wanted::_wanted;

/// Reach `_arrays` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_arrays` (Completion/Zsh/Context/_brace_parameter sh:193) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_arrays_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _arrays(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_arrays", args, || _arrays_impl(args))
}

/// `_arrays` — `shift` command completion: list array-typed
/// parameters via `_parameters -g '*array*'`.
pub fn _arrays_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_arrays");
    // sh:5
    let mut wanted_argv: Vec<String> = vec![
        "arrays".to_string(),
        "expl".to_string(),
        "array".to_string(),
        "_parameters".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-g".to_string());
    wanted_argv.push("*array*".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _arrays_impl(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
