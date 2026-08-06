//! Port of `_user_math_func` from `Completion/Zsh/Type/_user_math_func`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4  local -a funcs
//! sh:5
//! sh:6  funcs=(${${${(f)"$(functions -M)"}##functions -M }%% *})
//! sh:7
//! sh:8  _wanted user-math-functions expl 'user math function' \
//! sh:9      compadd -S '(' -q "$@" -a funcs
//! ```
//!
//! sh:6 — parses `functions -M` output lines, extracting just the
//!   name field. We bypass the text-parsing step by reading the
//!   live `MATHFUNCS` registry directly and filtering for
//!   `MFF_USERFUNC` (the same predicate that `functions -M` uses
//!   to decide what to list, at `builtin.rs:4471`).

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::module::MATHFUNCS;
use crate::ported::params::setaparam;
use crate::ported::zsh_h::MFF_USERFUNC;

/// `_user_math_func` — complete user-defined math function names
/// (those added via `functions -M`).
pub fn _user_math_func(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_user_math_func");
    // sh:6 — collect names of user-flagged math functions.
    let funcs: Vec<String> = MATHFUNCS
        .lock()
        .map(|tab| {
            tab.iter()
                .filter(|m| (m.flags & MFF_USERFUNC) != 0)
                .map(|m| m.name.clone())
                .collect()
        })
        .unwrap_or_default();

    // Publish under the shell-side name `funcs` so `compadd -a funcs`
    // picks them up.
    setaparam("funcs", funcs);

    // sh:8-9
    let mut wanted_argv: Vec<String> = vec![
        "user-math-functions".to_string(),
        "expl".to_string(),
        "user math function".to_string(),
        "compadd".to_string(),
        "-S".to_string(),
        "(".to_string(),
        "-q".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("funcs".to_string());
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_nonzero_with_no_user_funcs() {
        // sh:9 — no user math funcs registered AND no tag → _wanted
        //   returns 1.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _user_math_func(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn publishes_funcs_array() {
        // sh:6 — verify the `funcs` array gets published (even when
        //   empty, the publish itself happens).
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _user_math_func(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let funcs = crate::ported::params::getaparam("funcs");
        assert!(funcs.is_some(), "funcs array should be published");
    }
}
