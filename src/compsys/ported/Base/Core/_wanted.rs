//! Port of `_wanted` from `Completion/Base/Core/_wanted`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local -a __targs __gopt
//! sh: 4
//! sh: 5  zparseopts -D -a __gopt 1 2 V J x C:=__targs
//! sh: 6
//! sh: 7  _tags "$__targs[@]" "$1"
//! sh: 8
//! sh: 9  while _tags; do
//! sh:10    _all_labels "$__gopt[@]" "$@" && return 0
//! sh:11  done
//! sh:12
//! sh:13  return 1
//! ```
//!
//! Calls real `bin_zparseopts` with extended spec `C:=__targs`
//! (value-taking `-C <subcontext>` flag stored in `__targs`).
//! Delegates to sibling ports `_tags::_tags` and
//! `_all_labels::_all_labels` for the iteration.

use super::_all_labels::_all_labels;
use super::_tags::_tags;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:5 — bridge to real `bin_zparseopts -D -a __gopt 1 2 V J x
/// C:=__targs` via `-v <name>` (named array source). Returns
/// `(remaining_argv, __targs, __gopt)`.
fn run_zparseopts_wanted(args: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("__gopt", Vec::new());
    setaparam("__targs", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "__gopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V".to_string(),
            "J".to_string(),
            "x".to_string(),
            "C:=__targs".to_string(),
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("__gopt").unwrap_or_default();
    let targs = getaparam("__targs").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, targs, gopt)
}

/// `_wanted` — register tag `$1` and (if requested) loop through
/// `_tags` calling `_all_labels` per round. Returns 0 if any
/// iteration succeeded, 1 if none.
pub fn _wanted(args: &[String]) -> i32 {
    // sh:3-5
    let (argv, targs, gopt) = run_zparseopts_wanted(args);

    // sh:7  _tags "$__targs[@]" "$1"
    let arg1 = argv.first().cloned().unwrap_or_default();
    let mut tags_args: Vec<String> = targs.clone();
    tags_args.push(arg1);
    let _ = _tags(&tags_args);

    // sh:9-11  while _tags; do _all_labels … && return 0; done
    loop {
        if _tags(&[]) != 0 {
            // sh:9 — _tags's switch-to-next-tag-set form returns
            //   non-zero when no more tag sets exist.
            break;
        }
        let mut labels_args: Vec<String> = gopt.clone();
        labels_args.extend(argv.iter().cloned());
        if _all_labels(&labels_args) == 0 {
            // sh:10
            return 0;
        }
    }

    // sh:13
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    fn with_incompfunc<T, F: FnOnce() -> T>(f: F) -> T {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn no_matching_tagset_returns_one() {
        // sh:13 — when _tags's switch-to-next never produces a
        //   match (no comptags state pre-loaded), the while loop
        //   short-circuits and we return 1.
        let r = with_incompfunc(|| {
            _wanted(&[
                "mytag".to_string(),
                "name".to_string(),
                "descr".to_string(),
                "compadd".to_string(),
                "alpha".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn c_flag_takes_value_into_targs() {
        // sh:5 — `C:=__targs` means -C is value-taking; the value
        //   accumulates into __targs alongside the literal `-C`.
        let _g = crate::test_util::global_state_lock();
        let (rem, targs, gopt) = run_zparseopts_wanted(&[
            "-C".to_string(),
            "subctx".to_string(),
            "tagname".to_string(),
        ]);
        assert_eq!(targs, vec!["-C", "subctx"]);
        assert_eq!(gopt, Vec::<String>::new());
        assert_eq!(rem, vec!["tagname"]);
    }

    #[test]
    fn boolean_flags_into_gopt() {
        // sh:5 — `1 2 V J x` boolean, stored into __gopt; rest stays
        //   in argv.
        let _g = crate::test_util::global_state_lock();
        let (rem, targs, gopt) =
            run_zparseopts_wanted(&["-V".to_string(), "-1".to_string(), "mytag".to_string()]);
        assert_eq!(targs, Vec::<String>::new());
        assert_eq!(gopt, vec!["-V", "-1"]);
        assert_eq!(rem, vec!["mytag"]);
    }
}
