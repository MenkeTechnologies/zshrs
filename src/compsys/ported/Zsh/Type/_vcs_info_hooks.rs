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
//! `$functions` is NOT `shfunctab`: the subscript goes through the
//! `zsh/parameter` module's own scan, `scanpmfunctions`
//! (c:Src/Modules/parameter.c:530) -> `scanfunctions(..., dis = 0)`,
//! whose walk is gated at c:482
//!
//! ```c
//! if (dis ? (hn->flags & DISABLED) : !(hn->flags & DISABLED)) {
//! ```
//!
//! so a `disable -f +vi-foo` hook is absent from `$functions` (it moves to
//! `$dis_functions`, c:537-541) and must not be offered. The scan lives in
//! `shared::shfunc_names_with_prefix`, shared with the two other upstream
//! call sites that spell it the same way.

use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

/// `_vcs_info_hooks` — emit `+vi-*` hook function names, stripped of
/// the `+vi-` prefix. Returns the `bin_compadd` exit code (0 = matches
/// added, 1 = not in completion fn or no matches).
pub fn _vcs_info_hooks() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_vcs_info_hooks");
    // sh:2  compadd - ${functions[(I)+vi-*]#+vi-}
    //
    // `(I)+vi-*` enumerates the keys of `$functions` matching `+vi-*` —
    // the NOT-DISABLED half of `shfunctab` (c:482).
    let function_names = crate::compsys::ported::shared::shfunc_names_with_prefix("+vi-");
    // Build argv exactly as the shell would: `-` (the literal positional
    // arg telling compadd "stop option parsing — what follows is
    // matches") then each `#+vi-`-stripped hook name.
    let mut argv: Vec<String> = vec!["-".to_string()];
    for f in &function_names {
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
    use crate::ported::hashtable::shfunctab_lock;
    use crate::ported::zle::complete::INCOMPFUNC;
    use crate::ported::zsh_h::{hashnode, shfunc, DISABLED};
    use std::sync::atomic::Ordering;

    /// bin_compadd refuses unless `INCOMPFUNC == 1` (sh:608-610 of
    /// complete.c). Tests must set the guard before calling and clear
    /// after.
    fn with_incompfunc<F: FnOnce() -> i32>(f: F) -> i32 {
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    fn add_shfunc(nam: &str, disabled: bool) {
        let mut tab = shfunctab_lock().write().unwrap();
        tab.add(shfunc {
            node: hashnode {
                next: None,
                nam: nam.to_string(),
                flags: if disabled { DISABLED } else { 0 },
            },
            filename: None,
            lineno: 0,
            funcdef: None,
            redir: None,
            sticky: None,
            body: None,
            redir_text: None,
        });
    }

    /// sh:2's source is `${functions[(I)+vi-*]}`, the NOT-DISABLED half of
    /// `shfunctab` (c:Src/Modules/parameter.c:482). A `disable -f`'d hook
    /// belongs to `$dis_functions` (c:537-541) and must never be offered.
    #[test]
    fn scan_skips_disabled_hooks() {
        let _g = crate::test_util::global_state_lock();
        const ON: &str = "+vi-zzq-on";
        const OFF: &str = "+vi-zzq-off";
        add_shfunc(ON, false);
        add_shfunc(OFF, true);

        let names = crate::compsys::ported::shared::shfunc_names_with_prefix("+vi-");
        assert!(
            names.iter().any(|n| n == ON),
            "enabled hook missing from the `$functions` scan: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == OFF),
            "DISABLED hook offered — `$functions` never lists it (c:482): {:?}",
            names
        );

        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove(ON);
        tab.remove(OFF);
    }

    /// The port takes no arguments (upstream sh:2 takes none either): the
    /// hook names come from `$functions`, not from the caller.
    #[test]
    fn sources_hooks_from_shfunctab_not_arguments() {
        let _g = crate::test_util::global_state_lock();
        const ON: &str = "+vi-zzq-src";
        add_shfunc(ON, false);
        let names = crate::compsys::ported::shared::shfunc_names_with_prefix("+vi-");
        assert!(names.iter().any(|n| n == ON));
        // `#+vi-` strips exactly the prefix, leaving the hook name.
        assert_eq!(ON.strip_prefix("+vi-"), Some("zzq-src"));
        let _ = with_incompfunc(_vcs_info_hooks);
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove(ON);
    }

    #[test]
    fn outside_completion_function_returns_one() {
        // bin_compadd's guard (sh:608-610 of complete.c): returns 1
        // with "can only be called from completion function" warning
        // when INCOMPFUNC != 1.
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let r = _vcs_info_hooks();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        assert_eq!(r, 1, "bin_compadd outside completion fn must return 1");
    }
}
