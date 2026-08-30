//! Port of `_global_tags` from `Completion/Unix/Type/_global_tags`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local expl tags
//! sh:5  tags=( $(_call_program global-tags global --completion $PREFIX 2>/dev/null) )
//! sh:7  _wanted global-tags expl 'tag' compadd -M 'm:{a-zA-Z}={A-Za-z}' -a "$@" - tags
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// `_global_tags` — complete GNU GLOBAL tags via `global --completion`.
pub fn _global_tags(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_global_tags");
    // sh:5 — run the helper, split its stdout into words.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let _ = call_program_capture(&[
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

    /// `_wanted` registers its OWN tag, so the "without registered tags"
    /// premise never holds: with the `doshfunc`-frame shift (see
    /// `Base/Core/_wanted.rs:45-54`) `_wanted` registers and `_all_labels`
    /// adds matches, making 0 the correct return. Confirmed against real
    /// zsh 5.9.2 driven through a PTY inside a live completion widget —
    /// all of these return 0, not 1. The old name and `assert_eq!(r, 1)`
    /// encoded the pre-shift answer.
    ///
    /// `reset_completion_state` is what makes that answer STABLE. The
    /// assertion used to pass alone and fail inside a full run because a
    /// leftover `$PREFIX` from an earlier test filtered out every candidate
    /// `compadd` was offered, so `compadd` returned 1 for a tag set that had
    /// been registered perfectly well — see that helper for the mechanism.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        // Pin the INPUT. sh:5 runs `_call_program global-tags global
        // --completion $PREFIX`, so the candidate list is whatever GNU
        // GLOBAL prints on this host — nothing on a box without `global`,
        // or outside a directory carrying a GTAGS database. `_call_program`
        // takes its command line from the `command` style when one is set
        // (`Completion/Base/Utility/_call_program:26`, ported at
        // `_call_program.rs:74-101`), which is the upstream-sanctioned way
        // to hand it a fixed list. `reset_completion_state` unsets
        // `$curcontext`, so sh:5's style context is `:completion::global-tags`.
        crate::test_util::set_test_zstyle(":completion::global-tags", "command", "echo alpha beta");
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _global_tags(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 0);
    }
}
