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

/// `_arrays` — `shift` command completion: list array-typed
/// parameters via `_parameters -g '*array*'`.
pub fn _arrays(args: &[String]) -> i32 {
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
        let r = _arrays(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
