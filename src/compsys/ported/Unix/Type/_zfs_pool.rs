//! Port of `_zfs_pool` from `Completion/Unix/Type/_zfs_pool`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  compadd "$@" - $(zpool list -H -o name)
//! ```

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

/// `_zfs_pool` — complete ZFS pool names from `zpool list -H -o name`.
pub fn _zfs_pool(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_zfs_pool");
    // sh:3  $(zpool list -H -o name)
    let names: Vec<String> = std::process::Command::new("zpool")
        .args(["list", "-H", "-o", "name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    // sh:3  compadd "$@" - $names
    let mut cadd: Vec<String> = args.to_vec();
    cadd.push("-".to_string());
    cadd.extend(names);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        // Outside a completion function bin_compadd refuses (returns 1),
        // regardless of whether `zpool` exists.
        assert_eq!(_zfs_pool(&[]), 1);
    }
}
