//! Port of `_directories` from `Completion/Unix/Type/_directories`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef dircmp -P -value-,*path,-default-
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted directories expl directory _files -/ "$@" -
//! ```
//!
//! `_files -/` — _files with the directory-only filter. _files is
//! a sibling shell fn (not ported); dispatches via `_wanted`'s
//! action-chunk path which routes through `exec accessors`.

use crate::compsys::ported::_wanted::_wanted;

/// Reach `_directories` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_directories "${suf[@]}" && ret=0` (Completion/bashcompinit sh:38) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_directories_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _directories(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_directories", args, || _directories_impl(args))
}

/// `_directories` — directory-only completion via `_files -/`.
pub fn _directories_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_directories");
    // sh:5
    let mut wanted_argv: Vec<String> = vec![
        "directories".to_string(),
        "expl".to_string(),
        "directory".to_string(),
        "_files".to_string(),
        "-/".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
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
        let r = _directories_impl(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
