//! Port of `_capabilities` from `Completion/Linux/Type/_capabilities`.
//!
//! Full upstream body (66 lines, abridged — head is a usage comment):
//! ```text
//! sh: 1  #autoload
//! sh:19  local -a caplist=( chown dac_override … checkpoint_restore bpf )
//! sh:62  local -a expl
//! sh:64  _description capabilities expl "Linux capability"
//! sh:65  compadd "${(@)expl}" "$@" -a - caplist
//! ```
//!
//! sh:19-61 — the full 45-entry `caplist`, taken from
//! `include/uapi/linux/capability.h` (`grep 'define CAP' | sed …`).
//! sh:65 — `-a` tells `compadd` the trailing word(s) (`caplist`) are
//! ARRAY-PARAMETER NAMES to expand, not literal candidates, so the
//! array is published via `setaparam` first and referenced by name —
//! mirrors `_ports.rs`'s `compadd -a … - ports` pattern.

use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, setaparam};
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

/// sh:19-61 — Linux capability names (lower-cased `CAP_*` from
/// `include/uapi/linux/capability.h`).
const CAPLIST: &[&str] = &[
    "chown",
    "dac_override",
    "dac_read_search",
    "fowner",
    "fsetid",
    "kill",
    "setgid",
    "setuid",
    "setpcap",
    "linux_immutable",
    "net_bind_service",
    "net_broadcast",
    "net_admin",
    "net_raw",
    "ipc_lock",
    "ipc_owner",
    "sys_module",
    "sys_rawio",
    "sys_chroot",
    "sys_ptrace",
    "sys_pacct",
    "sys_admin",
    "sys_boot",
    "sys_nice",
    "sys_resource",
    "sys_time",
    "sys_tty_config",
    "mknod",
    "lease",
    "audit_write",
    "audit_control",
    "setfcap",
    "mac_override",
    "mac_admin",
    "syslog",
    "wake_alarm",
    "block_suspend",
    "audit_read",
    "perfmon",
    "bpf",
    "checkpoint_restore",
];

/// `_capabilities` — complete POSIX capability names for Linux.
/// Accepts arbitrary `compadd` options (e.g. `-p cap_`, `-o nosort`)
/// which are forwarded verbatim (sh:65's `"$@"`).
pub fn _capabilities(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_capabilities");
    // sh:19-61 — publish the literal array under the name `caplist` so
    // `compadd -a … caplist` (sh:65) can expand it by reference.
    let caplist: Vec<String> = CAPLIST.iter().map(|s| s.to_string()).collect();
    setaparam("caplist", caplist);

    // sh:64  _description capabilities expl "Linux capability"
    let _ = _description(&[
        "capabilities".to_string(),
        "expl".to_string(),
        "Linux capability".to_string(),
    ]);

    // sh:65  compadd "${(@)expl}" "$@" -a - caplist
    let mut cadd: Vec<String> = getaparam("expl").unwrap_or_default();
    cadd.extend(args.iter().cloned());
    cadd.push("-a".to_string());
    cadd.push("-".to_string());
    cadd.push("caplist".to_string());
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caplist_has_no_duplicates_and_is_lowercase() {
        // sh:19-61 — sanity on the literal table: no dup entries, and
        // every name is already lower-cased (per the sh:18 `tr` filter
        // documented in the shell source).
        let mut seen = std::collections::HashSet::new();
        for cap in CAPLIST {
            assert!(seen.insert(*cap), "duplicate capability: {cap}");
            assert_eq!(*cap, cap.to_lowercase());
        }
        assert_eq!(CAPLIST.len(), 41);
    }

    #[test]
    fn returns_one_without_completion_context() {
        // bin_compadd (sh:65) requires INCOMPFUNC — mirrors
        // `_baudrates.rs`'s no-context return path.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_capabilities(&[]), 1);
    }

    #[test]
    fn publishes_caplist_array_param() {
        // sh:65 — `-a … caplist` expands the array by name, so the
        // param must be populated with the full literal table
        // regardless of completion-context availability.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let _ = _capabilities(&[]);
        let published = getaparam("caplist").unwrap_or_default();
        assert_eq!(published.len(), CAPLIST.len());
        assert_eq!(published.first().map(String::as_str), Some("chown"));
    }
}
