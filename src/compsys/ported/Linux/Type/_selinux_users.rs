//! Port of `_selinux_users` from `Completion/Linux/Type/_selinux_users`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a seusers expl
//! sh: 5  seusers=( ${(f)"$(_call_program selinux-users seinfo --flat -u)"} )
//! sh: 6  (( $#seusers )) || seusers=( guest_u root staff_u sysadm_u system_u unconfined_u user_u )
//! sh: 7  _description selinux-users expl "selinux user"
//! sh: 8  compadd "$@" "$expl[@]" -a seusers
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam};
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

/// sh:6 — default table of SELinux user identities used when
/// `seinfo --flat -u` produced no output.
const DEFAULT_SEUSERS: &[&str] = &[
    "guest_u",
    "root",
    "staff_u",
    "sysadm_u",
    "system_u",
    "unconfined_u",
    "user_u",
];

/// `_selinux_users` — complete SELinux user identities via `seinfo --flat -u`,
/// falling back to the well-known default identity set.
pub fn _selinux_users(args: &[String]) -> i32 {
    // sh:5  seusers=( ${(f)"$(_call_program selinux-users seinfo --flat -u)"} )
    let _ = _call_program(&[
        "selinux-users".to_string(),
        "seinfo --flat -u".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    let mut seusers: Vec<String> = out.lines().map(str::to_string).collect();

    // sh:6  (( $#seusers )) || seusers=( guest_u root staff_u sysadm_u system_u unconfined_u user_u )
    if seusers.is_empty() {
        seusers = DEFAULT_SEUSERS.iter().map(|s| s.to_string()).collect();
    }

    // sh:7  _description selinux-users expl "selinux user"
    let _ = _description(&[
        "selinux-users".to_string(),
        "expl".to_string(),
        "selinux user".to_string(),
    ]);

    // sh:8  compadd "$@" "$expl[@]" -a seusers
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-a".to_string());
    cadd.push("seusers".to_string());
    let _ = crate::ported::params::setaparam("seusers", seusers);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_seusers_used_when_no_output() {
        let lines: Vec<String> = "".lines().map(str::to_string).collect();
        assert!(lines.is_empty());
        let fallback: Vec<String> = DEFAULT_SEUSERS.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            fallback,
            vec![
                "guest_u",
                "root",
                "staff_u",
                "sysadm_u",
                "system_u",
                "unconfined_u",
                "user_u",
            ]
        );
    }

    #[test]
    fn seinfo_output_lines_split_and_kept_when_nonempty() {
        let out = "guest_u\nstaff_u\nsysadm_u\n";
        let seusers: Vec<String> = out.lines().map(str::to_string).collect();
        assert_eq!(seusers, vec!["guest_u", "staff_u", "sysadm_u"]);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_selinux_users(&[]), 1);
    }
}
