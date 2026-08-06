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

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::description_byname;
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
/// A plain Rust call to the sibling port skips both of
/// [`crate::compsys::ported::shared::call_compfn`]'s effects: `$fpath` /
/// shfunc arbitration (the user's own copy of the function is inert) and
/// the `doshfunc` frame (no `FUNCSTACK` entry, and the callee's
/// `declare_locals` land in the CALLER's param scope instead of its own).
///
/// The direct call stays as the fallback: it runs only when neither a shell
/// function nor a registered port claims the name — i.e. in unit tests with
/// no executor installed.
pub fn selinux_roles_byname(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_selinux_roles", args, || _selinux_roles(args))
}

/// `_selinux_roles` — offer the list of SELinux roles reported by
/// `seinfo --flat -r`.
pub fn _selinux_roles(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_selinux_roles");
    // sh:5  seroles=( ${(f)"$(_call_program selinux-roles seinfo --flat -r)"} )
    let _ = _call_program(&[
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
    let _ = description_byname(&[
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
        assert_eq!(_selinux_roles(&[]), 1);
    }
}
