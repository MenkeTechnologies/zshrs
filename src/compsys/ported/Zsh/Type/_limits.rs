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

use crate::compsys::ported::_wanted::_wanted;
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
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// `_wanted` registers its OWN tag, so the "without registered tags"
    /// premise never holds: with the `doshfunc`-frame shift (see
    /// `Base/Core/_wanted.rs:45-54`) `_wanted` registers and `_all_labels`
    /// adds matches, making 0 the correct return. Confirmed against real
    /// zsh 5.9.2 driven through a PTY inside a live completion widget —
    /// all of these return 0, not 1. The old name and `assert_eq!(r, 1)`
    /// encoded the pre-shift answer.
    ///
    /// The candidate list is `known_resources`, a compiled-in table, so
    /// nothing here reads the machine. What used to make this flaky was
    /// purely leaked completion state — see `reset_completion_state`.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _limits(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 0);
    }

    #[test]
    fn enumerates_known_resources_nonempty() {
        // Confirm the authoritative table has entries (platform-
        //   dependent on Linux; on macOS the table also has values).
        assert!(!known_resources.is_empty());
    }
}
