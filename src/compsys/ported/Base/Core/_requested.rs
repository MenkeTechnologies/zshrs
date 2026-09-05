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
//! Calls real `bin_comptags -R` + real `bin_zparseopts`. Reaches
//! `_all_labels` and `_description` BY NAME (`_all_labels` /
//! `_description` → [`crate::compsys::ported::shared::call_compfn`])
//! for the dispatch arms, matching the sh body's bare command words: a
//! user's own copy earlier on `$fpath` wins, and each call gets its own
//! `doshfunc` frame — which the `comptags -A-` level arithmetic below
//! depends on.

use super::_all_labels::_all_labels_impl;
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
    crate::compsys::ported::shared::set_bridge_argv(src, args);
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
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
}

/// Reach `_requested` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `if _requested jobs; then` (Completion/Unix/Command/_lp
/// sh:144) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_requested_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _requested(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_requested", args, || _requested_impl(args))
}

/// `_requested` — check if tag `$1` was requested by the current
/// completion context. Returns 0 when requested (after dispatching
/// to `_all_labels` or `_description` as appropriate), 1 otherwise.
pub fn _requested_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_requested");
    // sh:3  local __gopt
    crate::compsys::ported::shared::declare_locals(&["__gopt"], 0);
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
        //
        // The leading `-` marks `__prev=-` inside _all_labels, i.e. its
        // `comptags -A-` reaches back ONE function-nesting level to where
        // `_tags` registered (C invokes _all_labels as a real shell
        // function, so it sits at locallevel+1 vs the tags' level). The
        // Rust port used to call the sibling `_all_labels` as a plain Rust
        // fn, which skips doshfunc's `inc_locallevel` (exec.rs:6131) — so it
        // ran at the SAME level as the registration and `-A-` (level-1)
        // missed, aborting the whole `_tags`/`while _tags`/`_requested`
        // idiom with "comptags: no tags registered". A hand-rolled
        // inc/dec_locallevel pair simulated the missing depth. Going BY NAME
        // now supplies the real frame instead: `_all_labels` →
        // `call_compfn` → `dispatch_function_call` → `doshfunc`, which does
        // the `inc_locallevel` itself AND lets a user's own `_all_labels`
        // earlier on `$fpath` win, exactly as the bare `_all_labels` command
        // word does in the sh body.
        let mut all_args: Vec<String> = vec!["-".to_string()];
        all_args.extend(gopt.iter().cloned());
        all_args.extend(argv.iter().cloned());
        let rc = crate::compsys::ported::shared::call_compfn("_all_labels", &all_args, || {
            // Fallback = no executor (unit tests). `call_compfn`'s fallback
            // opens no `doshfunc` frame, so hand-roll the one effect this
            // call depends on — see the module header.
            crate::ported::utils::inc_locallevel();
            let r = _all_labels_impl(&all_args);
            crate::ported::utils::dec_locallevel();
            r
        });
        if rc != 0 {
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
            _requested_impl(&[
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
