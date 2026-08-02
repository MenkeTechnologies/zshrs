//! Port of `_global_tags` from `Completion/Unix/Type/_global_tags`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local expl tags
//! sh:5  tags=( $(_call_program global-tags global --completion $PREFIX 2>/dev/null) )
//! sh:7  _wanted global-tags expl 'tag' compadd -M 'm:{a-zA-Z}={A-Za-z}' -a "$@" - tags
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// `_global_tags` — complete GNU GLOBAL tags via `global --completion`.
pub fn _global_tags(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_global_tags");
    // sh:5 — run the helper, split its stdout into words.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let _ = _call_program(&[
        "global-tags".to_string(),
        "global".to_string(),
        "--completion".to_string(),
        prefix,
    ]);
    let tags: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();
    setaparam("tags", tags);

    // sh:7
    let mut w: Vec<String> = vec![
        "global-tags".to_string(),
        "expl".to_string(),
        "tag".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z}".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("tags".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _global_tags(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
