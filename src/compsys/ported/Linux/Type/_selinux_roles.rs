//! Port of `_selinux_roles` from `Completion/Linux/Type/_selinux_roles`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local -a seroles expl
//! sh:5  seroles=( ${(f)"$(_call_program selinux-roles seinfo --flat -r)"} )
//! sh:6  _description selinux-roles expl "selinux role"
//! sh:7  compadd "$@" "$expl[@]" -a seroles
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Reach `_selinux_roles` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_selinux_$parts[1] ${(P)parts[1]}` (Completion/Linux/Type/_selinux_contexts sh:18) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_selinux_roles_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _selinux_roles(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_selinux_roles", args, || {
        _selinux_roles_impl(args)
    })
}

/// `_selinux_roles` — offer the list of SELinux roles reported by
/// `seinfo --flat -r`.
pub fn _selinux_roles_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_selinux_roles");
    // sh:5  seroles=( ${(f)"$(_call_program selinux-roles seinfo --flat -r)"} )
    let _ = call_program_capture(&[
        "selinux-roles".to_string(),
        "seinfo".to_string(),
        "--flat".to_string(),
        "-r".to_string(),
    ]);
    let seroles: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    setaparam("seroles", seroles);

    // sh:6  _description selinux-roles expl "selinux role"
    let _ = _description(&[
        "selinux-roles".to_string(),
        "expl".to_string(),
        "selinux role".to_string(),
    ]);

    // sh:7  compadd "$@" "$expl[@]" -a seroles
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-a".to_string());
    cadd.push("seroles".to_string());
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_selinux_roles_impl(&[]), 1);
    }
}
