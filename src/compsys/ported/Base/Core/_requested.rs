//! Port of `_requested` from `Completion/Base/Core/_requested`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt
//! sh: 4
//! sh: 5  __gopt=()
//! sh: 6  zparseopts -D -a __gopt 1 2 V J x
//! sh: 7
//! sh: 8  if comptags -R "$1"; then
//! sh: 9    if [[ $# -gt 3 ]]; then
//! sh:10      _all_labels - "$__gopt[@]" "$@" || return 1
//! sh:11    elif [[ $# -gt 1 ]]; then
//! sh:12      _description "$__gopt[@]" "$@"
//! sh:13    fi
//! sh:14    return 0
//! sh:15  else
//! sh:16    return 1
//! sh:17  fi
//! ```
//!
//! Calls real `bin_comptags -R` + real `bin_zparseopts`. Delegates
//! to sibling ports `_all_labels::_all_labels` and
//! `_description::_description` for the dispatch arms.

use super::_all_labels::_all_labels;
use super::_description::_description;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::zle::computil::bin_comptags;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:6 — bridge to real `bin_zparseopts -D -a __gopt 1 2 V J x`
/// via `-v <name>`.
fn run_gopt(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("__gopt", Vec::new());
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
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("__gopt").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
}

/// `_requested` — check if tag `$1` was requested by the current
/// completion context. Returns 0 when requested (after dispatching
/// to `_all_labels` or `_description` as appropriate), 1 otherwise.
pub fn _requested(args: &[String]) -> i32 {
    // sh:5-6
    let (argv, gopt) = run_gopt(args);

    // sh:8  comptags -R "$1"
    let arg1 = argv.first().cloned().unwrap_or_default();
    if bin_comptags("comptags", &["-R".to_string(), arg1], &make_ops(), 0) != 0 {
        // sh:16
        return 1;
    }

    // sh:9
    if argv.len() > 3 {
        // sh:10  _all_labels - "$__gopt[@]" "$@" || return 1
        let mut all_args: Vec<String> = vec!["-".to_string()];
        all_args.extend(gopt.iter().cloned());
        all_args.extend(argv.iter().cloned());
        if _all_labels(&all_args) != 0 {
            return 1;
        }
    } else if argv.len() > 1 {
        // sh:12  _description "$__gopt[@]" "$@"
        let mut desc_args: Vec<String> = gopt;
        desc_args.extend(argv);
        let _ = _description(&desc_args);
    }
    // sh:14
    0
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
    fn unrequested_tag_returns_one() {
        // sh:16 — comptags -R fails for tags never registered.
        let r = with_incompfunc(|| {
            _requested(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn parses_gopt_via_zparseopts() {
        // sh:6 — `-V` is boolean, strips from argv into gopt.
        let _g = crate::test_util::global_state_lock();
        let (rem, gopt) = run_gopt(&[
            "-V".to_string(),
            "mytag".to_string(),
            "n".to_string(),
            "d".to_string(),
        ]);
        assert_eq!(gopt, vec!["-V"]);
        assert_eq!(rem, vec!["mytag", "n", "d"]);
    }
}
