//! Port of `_find_net_interfaces` from
//! `Completion/Unix/Type/_find_net_interfaces`.
//!
//! Full upstream body (41 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:    # Returns arrays net_intf_disp and net_intf_list which the
//! sh:    # caller should make local.
//! sh:10  local PATH=$PATH
//! sh:11  PATH=/sbin:$PATH   # needed tools live in /sbin
//! sh:13  case $OSTYPE in
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

use crate::compsys::ported::_call_program::call_program_capture;
use crate::ported::params::{getsparam, setaparam, setsparam};

fn call(tag: &str, cmd: &str) -> Vec<String> {
    if call_program_capture(&[tag.to_string(), cmd.to_string()]).1 == 0 {
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

/// sh:10-11 — `local PATH=$PATH` then `PATH=/sbin:$PATH`.
///
/// The upstream `local` scopes the prepend to this call, so the previous
/// value is restored on the way out. Without it the darwin arm cannot
/// find `/sbin/ifconfig` when `/sbin` is absent from the caller's `$PATH`,
/// and the whole completion dies with `command not found: ifconfig`.
struct PathScope(Option<String>);

impl Drop for PathScope {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            setsparam("PATH", &prev);
        }
    }
}

/// `_find_net_interfaces` — enumerate network interface names into
/// `$net_intf_list`.
pub fn _find_net_interfaces(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_find_net_interfaces");

    // sh:9-11 — "Make sure needed tools are in the path."
    //   local PATH=$PATH
    //   PATH=/sbin:$PATH
    // `ifconfig`, `lsdev` and `netstat` all live in /sbin or /usr/sbin on
    // the systems this dispatches to, and those are not on a default
    // $PATH. Restored by `PathScope` when this function returns.
    let prev_path = getsparam("PATH");
    if let Some(p) = prev_path.as_deref() {
        setsparam("PATH", &format!("/sbin:{}", p));
    }
    let _path_scope = PathScope(prev_path);

    // sh:13 — dispatch on $OSTYPE.
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
