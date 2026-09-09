//! Port of `_options` from `Completion/Zsh/Type/_options`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  # This should be used to complete all option names.
//! sh:4
//! sh:5  local expl
//! sh:6
//! sh:7  _wanted zsh-options expl 'zsh option' \
//! sh:8      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -k - options
//! ```
//!
//! `compadd -k options` reads keys of the shell-side `options`
//! associative array (the global option-name → on/off mapping).

use crate::compsys::ported::_wanted::_wanted;

/// Reach `_options` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_tags -C -o options && _options` (Completion/Zsh/Context/_condition sh:6) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_options_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _options(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_options", args, || _options_impl(args))
}

/// `_options` — complete all zsh option names.
pub fn _options_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_options");
    // sh:5 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_options` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:5 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:7-8
    let mut wanted_argv: Vec<String> = vec![
        "zsh-options".to_string(),
        "expl".to_string(),
        "zsh option".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-M".to_string());
    wanted_argv.push("B:[nN][oO]= M:_= M:{A-Z}={a-z}".to_string());
    wanted_argv.push("-k".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.push("options".to_string());
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
        // The lock serialises but restores nothing, and a `compadd`-driven
        // test parks a word in `$PREFIX` that makes every later candidate fail
        // to match — so this returned 0 in a full run and 1 on its own.
        crate::test_util::reset_completion_state();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _options_impl(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
