//! Port of `_fbsd_architectures` from `Completion/BSD/Type/_fbsd_architectures`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _description architectures expl 'architecture'
//! sh: 6  compadd "$@" "$expl[@]" amd64 arm arm64 i386 mips powerpc riscv sparc64
//! ```

use crate::compsys::ported::_description::description_byname;
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

/// sh:6 — the fixed FreeBSD `MACHINE_ARCH` list.
const ARCHITECTURES: &[&str] = &[
    "amd64", "arm", "arm64", "i386", "mips", "powerpc", "riscv", "sparc64",
];

/// `_fbsd_architectures` — offer the fixed list of FreeBSD machine architectures.
pub fn _fbsd_architectures(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_fbsd_architectures");
    // sh:5  _description architectures expl 'architecture'
    let _ = description_byname(&[
        "architectures".to_string(),
        "expl".to_string(),
        "architecture".to_string(),
    ]);
    // sh:6  compadd "$@" "$expl[@]" amd64 arm arm64 i386 mips powerpc riscv sparc64
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
            &["amd64", "arm", "arm64", "i386", "mips", "powerpc", "riscv", "sparc64"]
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_fbsd_architectures(&[]), 1);
    }
}
