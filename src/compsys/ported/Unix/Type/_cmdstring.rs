//! Port of `_cmdstring` from `Completion/Unix/Type/_cmdstring`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  # This is for a quoted argument that will be interpreted as a command.
//! sh:4
//! sh:5  compset -q
//! sh:6  _normal
//! ```

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Reach `_cmdstring` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_cmdstring` (Completion/Unix/Type/_cmdambivalent sh:7) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_cmdstring_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _cmdstring() -> i32 {
    crate::compsys::ported::shared::call_compfn("_cmdstring", &[], || _cmdstring_impl())
}

/// `_cmdstring` — completion for a quoted shell command argument.
/// Calls real `bin_compset -q` (unquote the current word into its
/// own context), then dispatches `_normal` (sibling shell fn).
pub fn _cmdstring_impl() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_cmdstring");
    // sh:5  compset -q
    let _ = bin_compset("compset", &["-q".to_string()], &make_ops(), 0);
    // sh:6  _normal
    // sh:6 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    dispatch_action_command("_normal", &[], 6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127. It used to return a silent 1.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _cmdstring_impl();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 127);
    }
}
