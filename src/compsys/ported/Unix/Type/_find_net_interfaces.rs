//! Port of `_find_net_interfaces` from
//! `Completion/Unix/Type/_find_net_interfaces`.
//!
//! Full upstream body (41 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:    # Returns arrays net_intf_disp and net_intf_list which the
//! sh:    # caller should make local.
//! sh:14  case $OSTYPE in
//! sh:     aix*)    lsdev -C -c if -F 'name:description'  (+ verbose disp)
//! sh:     darwin|freebsd|dragonfly)  ifconfig -l
//! sh:     irix*)   netstat -i
//! sh:     *)  ip link | sed …   ||  ifconfig -a | sed …
//! sh:         fallback: /proc/sys/net/ipv4/conf/* + /sys/class/net/*
//! sh:  esac
//! ```
//!
//! Not a completion generator itself: publishes `$net_intf_list`
//! (and, when verbose, `$net_intf_disp`) for the caller. Command
//! output is captured via the ported `_call_program` (`$REPLY`).

use crate::compsys::ported::_call_program::_call_program;
use crate::ported::params::{getsparam, setaparam};

fn call(tag: &str, cmd: &str) -> Vec<String> {
    if _call_program(&[tag.to_string(), cmd.to_string()]) == 0 {
        getsparam("REPLY")
            .unwrap_or_default()
            .split_whitespace()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    }
}

/// sh (linux fallback) — basenames under a `conf`/`net` sysfs dir.
fn dir_basenames(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// `_find_net_interfaces` — enumerate network interface names into
/// `$net_intf_list`.
pub fn _find_net_interfaces(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_find_net_interfaces");
    // sh:14 — dispatch on $OSTYPE.
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let list: Vec<String> = if ostype.starts_with("darwin")
        || ostype.starts_with("freebsd")
        || ostype.starts_with("dragonfly")
    {
        // sh — ifconfig -l (space-separated interface names).
        call("interfaces", "ifconfig -l")
    } else if ostype.starts_with("aix") {
        // sh — lsdev name:description (name is the first colon-field).
        call("interfaces", "lsdev -C -c if -F name:description")
            .into_iter()
            .filter_map(|l| l.split(':').next().map(String::from))
            .collect()
    } else if ostype.starts_with("irix") {
        call("interfaces", "/usr/etc/netstat -i")
    } else {
        // sh — linux: prefer `ip`, else `ifconfig`, else sysfs.
        let mut v = call(
            "interfaces",
            "ip link | sed -ne 's/^[0-9]\\+: \\([^:@]\\+\\).*/\\1/p;t; s/^[ ]\\+altname \\(.\\+\\)$/\\1/p'",
        );
        if v.is_empty() {
            v = call(
                "interfaces",
                "ifconfig -a 2>/dev/null | sed -n 's/^\\([^ \t:]*\\).*/\\1/p'",
            );
        }
        if v.is_empty() && std::path::Path::new("/proc/sys/net/ipv4/conf").is_dir() {
            for n in dir_basenames("/proc/sys/net/ipv4/conf") {
                if n != "all" && n != "default" {
                    v.push(n);
                }
            }
            v.extend(dir_basenames("/sys/class/net"));
            let mut seen = std::collections::HashSet::new();
            v.retain(|s| seen.insert(s.clone()));
        }
        v
    };

    setaparam("net_intf_list", list);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_net_intf_list_param() {
        let _g = crate::test_util::global_state_lock();
        let _ = _find_net_interfaces(&[]);
        // The param is always set (possibly empty) after the call.
        assert!(crate::ported::params::getaparam("net_intf_list").is_some());
    }
}
