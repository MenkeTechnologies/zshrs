//! Port of `_nbsd_architectures` from `Completion/BSD/Type/_nbsd_architectures`.
//!
//! Full upstream body (11 lines):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _description architectures expl 'architecture'
//! sh: 6  compadd "$@" "$expl[@]" amd64 evbarm evbmips evbppc hpcarm i386 sparc64 xen \
//! sh: 7    acorn32 algor alpha amiga amigappc arc atari bebox cats cesfic cobalt dreamcast \
//! sh: 8    emips epoc32 evbsh3 ews4800mips hp300 hppa hpcmips hpcsh ia64 ibmnws iyonix \
//! sh: 9    landisk luna68k mac68k macppc mipsco mmeye mvme68k mvmeppc netwinder news68k \
//! sh:10    newsmips next68k ofppc pmax prep rs6000 sandpoint sbmips sgimips shark sparc \
//! sh:11    sun2 sun3 vax x68k zaurus
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

/// sh:6-11 — the fixed table of NetBSD MACHINE_ARCH values.
const ARCHITECTURES: &[&str] = &[
    "amd64",
    "evbarm",
    "evbmips",
    "evbppc",
    "hpcarm",
    "i386",
    "sparc64",
    "xen",
    "acorn32",
    "algor",
    "alpha",
    "amiga",
    "amigappc",
    "arc",
    "atari",
    "bebox",
    "cats",
    "cesfic",
    "cobalt",
    "dreamcast",
    "emips",
    "epoc32",
    "evbsh3",
    "ews4800mips",
    "hp300",
    "hppa",
    "hpcmips",
    "hpcsh",
    "ia64",
    "ibmnws",
    "iyonix",
    "landisk",
    "luna68k",
    "mac68k",
    "macppc",
    "mipsco",
    "mmeye",
    "mvme68k",
    "mvmeppc",
    "netwinder",
    "news68k",
    "newsmips",
    "next68k",
    "ofppc",
    "pmax",
    "prep",
    "rs6000",
    "sandpoint",
    "sbmips",
    "sgimips",
    "shark",
    "sparc",
    "sun2",
    "sun3",
    "vax",
    "x68k",
    "zaurus",
];

/// `_nbsd_architectures` — offer the list of NetBSD MACHINE_ARCH values.
pub fn _nbsd_architectures(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_nbsd_architectures");
    // sh:5  _description architectures expl 'architecture'
    let _ = description_byname(&[
        "architectures".to_string(),
        "expl".to_string(),
        "architecture".to_string(),
    ]);
    // sh:6-11  compadd "$@" "$expl[@]" amd64 evbarm ... zaurus
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.extend(ARCHITECTURES.iter().map(|s| s.to_string()));
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures_table_has_no_dupes_and_matches_upstream_count() {
        let mut sorted = ARCHITECTURES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ARCHITECTURES.len());
        assert_eq!(ARCHITECTURES.len(), 57);
        assert!(ARCHITECTURES.contains(&"amd64"));
        assert!(ARCHITECTURES.contains(&"zaurus"));
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_nbsd_architectures(&[]), 1);
    }
}
