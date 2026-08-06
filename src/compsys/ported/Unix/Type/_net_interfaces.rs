//! Port of `_net_interfaces` from `Completion/Unix/Type/_net_interfaces`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh:1  #compdef ifup ifdown
//! sh:3  local expl
//! sh:4  local -a net_intf_disp net_intf_list
//! sh:6  _find_net_interfaces
//! sh:8  _wanted interfaces expl 'network interface' \
//! sh:9      compadd "$@" "$net_intf_disp[@]" - "${(@)net_intf_list%%:*}"
//! ```
//!
//! `_find_net_interfaces` (sibling) populates `net_intf_disp` /
//! `net_intf_list`; dispatched via the sibling path. `${(@)net_intf_list%%:*}`
//! strips the `:`-suffixed metadata off each list element.

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getaparam;

/// `_net_interfaces` — complete network interface names (ifup/ifdown).
pub fn _net_interfaces(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_net_interfaces");
    // sh:6  _find_net_interfaces
    let _ = dispatch_function_call("_find_net_interfaces", &[]);
    let disp = getaparam("net_intf_disp").unwrap_or_default();
    let list = getaparam("net_intf_list").unwrap_or_default();
    // sh:9  "${(@)net_intf_list%%:*}" — drop the `:`-suffixed metadata.
    let names: Vec<String> = list
        .iter()
        .map(|e| e.split(':').next().unwrap_or(e).to_string())
        .collect();
    // sh:8-9  _wanted interfaces expl 'network interface' compadd …
    let mut w = vec![
        "interfaces".to_string(),
        "expl".to_string(),
        "network interface".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.extend(disp);
    w.push("-".to_string());
    w.extend(names);
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_net_interfaces(&[]), 1);
    }
}
