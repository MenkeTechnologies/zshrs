//! Port of `_volume_groups` from `Completion/AIX/Type/_volume_groups`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted volumegroups expl 'volume group' compadd "$@" - $(lsvg)
//! ```

use crate::compsys::ported::_wanted::_wanted;

/// `_volume_groups` — complete AIX LVM volume-group names (`lsvg`).
pub fn _volume_groups(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_volume_groups");
    // sh:5  $(lsvg)
    let groups: Vec<String> = std::process::Command::new("lsvg")
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

    // sh:5  _wanted volumegroups expl 'volume group' compadd "$@" - <groups>
    let mut w = vec![
        "volumegroups".to_string(),
        "expl".to_string(),
        "volume group".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(groups);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_volume_groups(&[]), 1);
    }
}
