//! Port of `_obsd_architectures` from `Completion/BSD/Type/_obsd_architectures`.
//!
//! Full upstream body (6 lines):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _description architectures expl 'architecture'
//! sh: 6  compadd "$@" "$expl[@]" alpha amd64 arm64 armv7 hppa i386 landisk loongson luna88k macppc octeon sparc64
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::params::getaparam;
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

/// sh:6 — static list of OpenBSD architecture names.
const ARCHITECTURES: &[&str] = &[
    "alpha", "amd64", "arm64", "armv7", "hppa", "i386", "landisk", "loongson", "luna88k", "macppc",
    "octeon", "sparc64",
];

/// `_obsd_architectures` — complete OpenBSD architecture names.
pub fn _obsd_architectures(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_obsd_architectures");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_obsd_architectures` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:5  _description architectures expl 'architecture'
    let _ = _description(&[
        "architectures".to_string(),
        "expl".to_string(),
        "architecture".to_string(),
    ]);

    // sh:6  compadd "$@" "$expl[@]" alpha amd64 arm64 armv7 hppa i386 landisk loongson luna88k macppc octeon sparc64
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.extend(ARCHITECTURES.iter().map(|s| s.to_string()));
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures_list_matches_upstream() {
        assert_eq!(
            ARCHITECTURES,
            &[
                "alpha", "amd64", "arm64", "armv7", "hppa", "i386", "landisk", "loongson",
                "luna88k", "macppc", "octeon", "sparc64",
            ]
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_obsd_architectures(&[]), 1);
    }
}
