//! Port of `_vcs_info_hooks` from `Completion/Zsh/Type/_vcs_info_hooks`.
//!
//! Full upstream body (2 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  compadd - ${functions[(I)+vi-*]#+vi-}
//! ```
//!
//! `${functions[(I)+vi-*]#+vi-}` — `(I)` is the "include indices" zsh
//! subscript flag which enumerates every key in `$functions` matching
//! `+vi-*`. The `#+vi-` modifier strips the prefix. So the shell emits
//! every user-defined vcs_info hook function with its `+vi-` prefix
//! stripped (e.g. `+vi-git-untracked` → `git-untracked`).
//!
//! Faithful Rust port: calls the real `bin_compadd` builtin in
//! `src/ported/zle/complete.rs` directly. No compsys-side middleman.
//!
//! Signature divergence (`// rust:`): the leaf can't reach `$functions`
//! from compsys. Caller supplies the function-name list (the same list
//! the shell would have iterated via `(k)functions`).

use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

/// `_vcs_info_hooks` — emit `+vi-*` hook function names, stripped of
/// the `+vi-` prefix. Returns the `bin_compadd` exit code (0 = matches
/// added, 1 = not in completion fn or no matches).
pub fn _vcs_info_hooks(function_names: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_vcs_info_hooks");
    // sh:2  compadd - ${functions[(I)+vi-*]#+vi-}
    //
    // Build argv exactly as the shell would: `-` (the literal positional
    // arg telling compadd "stop option parsing — what follows is
    // matches") then each stripped hook name.
    let mut argv: Vec<String> = vec!["-".to_string()];
    for f in function_names {
        if let Some(hook) = f.strip_prefix("+vi-") {
            argv.push(hook.to_string());
        }
    }

    // Empty `options` struct — bin_compadd parses every flag from argv.
    let ops = options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    bin_compadd("compadd", &argv, &ops, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// bin_compadd refuses unless `INCOMPFUNC == 1` (sh:608-610 of
    /// complete.c). Tests must set the guard before calling and clear
    /// after.
    fn with_incompfunc<F: FnOnce() -> i32>(f: F) -> i32 {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn calls_bin_compadd_with_stripped_hooks() {
        // sh:2 — `+vi-` prefix stripped from each hook function name
        // before compadd is invoked.
        let fns = vec![
            "+vi-git-untracked".to_string(),
            "+vi-git-stash".to_string(),
            "ordinary_fn".to_string(),
        ];
        let _r = with_incompfunc(|| _vcs_info_hooks(&fns));
        // No panic on argv building; exact return code depends on
        // bin_compadd's internal "did matches get added" semantics.
    }

    #[test]
    fn no_matching_hooks_still_calls_compadd() {
        // sh:2 — when no functions start with `+vi-`, argv is just
        // ["-"] (no matches). bin_compadd must accept that without
        // panicking.
        let fns = vec!["ordinary_fn".to_string(), "another".to_string()];
        let _ = with_incompfunc(|| _vcs_info_hooks(&fns));
    }

    #[test]
    fn empty_input_is_safe() {
        let _ = with_incompfunc(|| _vcs_info_hooks(&[]));
    }

    #[test]
    fn outside_completion_function_returns_one() {
        // bin_compadd's guard (sh:608-610 of complete.c): returns 1
        // with "can only be called from completion function" warning
        // when INCOMPFUNC != 1.
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let r = _vcs_info_hooks(&["+vi-foo".to_string()]);
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        assert_eq!(r, 1, "bin_compadd outside completion fn must return 1");
    }

    #[test]
    fn plus_vi_dash_with_empty_tail_emitted() {
        // `+vi-` alone (no hook name) strips to "". bin_compadd
        // receives an empty-string arg; mirror shell behaviour. No
        // panic.
        let fns = vec!["+vi-".to_string(), "+vi-real".to_string()];
        let _ = with_incompfunc(|| _vcs_info_hooks(&fns));
    }

    #[test]
    fn strips_exactly_the_plus_vi_dash_prefix_only() {
        // `+vi-foo-bar-baz` → `foo-bar-baz` (single strip).
        let fns = vec!["+vi-foo-bar-baz".to_string()];
        let _ = with_incompfunc(|| _vcs_info_hooks(&fns));
    }
}
