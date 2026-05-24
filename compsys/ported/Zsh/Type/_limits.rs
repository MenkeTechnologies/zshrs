//! Port of `_limits` — complete ulimit resource names.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_limits`.
//!
//! Upstream shell source (5 lines):
//! ```text
//! #compdef unlimit
//!
//! local expl
//!
//! _wanted limits expl 'process limit' compadd "$@" - ${${(f)"$(limit)"}%% *}
//! ```
//!
//! Runs `limit` (the zsh builtin) and parses each line's first
//! whitespace-separated word as the limit name. We use the
//! standard POSIX ulimit -a output format instead — same names.

use crate::base::MainCompleteState;
use crate::completion::Completion;
use crate::ported::_wanted::_wanted;

/// Canonical ulimit resource names (from `limit` / `ulimit -a`).
/// Matches both the BSD `limit` output and Linux `ulimit -a` rows.
pub const LIMIT_NAMES: &[&str] = &[
    "addressspace",
    "aiomemorylocked",
    "aiooperations",
    "cachedthreads",
    "coredumpsize",
    "cputime",
    "datasize",
    "descriptors",
    "filesize",
    "kqueues",
    "maxlockedmemory",
    "maxmessage",
    "maxnice",
    "maxproc",
    "maxpthreads",
    "maxrtprio",
    "memorylocked",
    "memoryqueue",
    "memoryuse",
    "msgqueue",
    "nofile",
    "openfiles",
    "pendingsignals",
    "pseudoterminals",
    "resident",
    "rttime",
    "sigpending",
    "sockbufsize",
    "stacksize",
    "swapsize",
    "threads",
    "userprocesses",
    "virtualmemory",
    "vmemorysize",
];

/// `_limits` — emit process limit names.
pub fn _limits(state: &mut MainCompleteState) -> bool {
    // shell: `_wanted limits expl 'process limit' compadd "$@" - ${(f)"$(limit)"}%% *}`
    let prefix = state.comp.params.prefix.clone();
    _wanted(state, "limits", "process limit", |s| {
        let mut any = false;
        for name in LIMIT_NAMES {
            if !name.starts_with(&*prefix) {
                continue;
            }
            s.add_match(Completion::new(*name), Some("limits"));
            any = true;
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["limits".into()]);
        state.tags.add_try(&["limits".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_at_least_one_limit_with_empty_prefix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        assert!(_limits(&mut state));
        assert!(state.comp.groups[0].matches.len() >= 10);
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_limits(&mut state));
    }

    #[test]
    fn prefix_filters_to_matching_limits() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "max".into();
        let _ = _limits(&mut state);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.iter().all(|n| n.starts_with("max")));
        assert!(names.contains(&"maxproc"));
    }

    #[test]
    fn known_canonical_names_present() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _limits(&mut state);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        for needed in [
            "cputime",
            "filesize",
            "datasize",
            "stacksize",
            "coredumpsize",
            "memoryuse",
            "descriptors",
        ] {
            assert!(names.contains(&needed), "missing canonical limit `{needed}`");
        }
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "definitely-not-a-limit".into();
        assert!(!_limits(&mut state));
    }

    #[test]
    fn group_named_limits() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _limits(&mut state);
        assert!(state.comp.groups.iter().any(|g| g.name == "limits"));
    }
}
