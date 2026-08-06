//! Port of `_limits` from `Completion/Zsh/Type/_limits`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef unlimit
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted limits expl 'process limit' compadd "$@" - ${${(f)"$(limit)"}%% *}
//! ```
//!
//! sh:5's `$(limit)` shell-out enumerates the configured rlimits.
//! We bypass the fork/parse and read the same authoritative table
//! (`known_resources`) the real `bin_limit` consults — see
//! `src/ported/builtins/rlimits.rs:92`.

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::builtins::rlimits::known_resources;

/// `_limits` — `unlimit` command completion: list process-resource
/// limit names.
pub fn _limits(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_limits");
    // sh:5 — first column of `limit` output = resource name.
    let names: Vec<String> = known_resources.iter().map(|r| r.name.to_string()).collect();

    let mut wanted_argv: Vec<String> = vec![
        "limits".to_string(),
        "expl".to_string(),
        "process limit".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(names);
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _limits(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn enumerates_known_resources_nonempty() {
        // Confirm the authoritative table has entries (platform-
        //   dependent on Linux; on macOS the table also has values).
        assert!(!known_resources.is_empty());
    }
}
