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

/// `_selinux_roles` — offer the list of SELinux roles reported by
/// `seinfo --flat -r`.
pub fn _selinux_roles(args: &[String]) -> i32 {
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
        assert_eq!(_selinux_roles(&[]), 1);
    }
}
