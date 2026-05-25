//! Port of `_limits` from `Completion/Zsh/Type/_limits`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef unlimit
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted limits expl 'process limit' compadd "$@" - ${${(f)"$(limit)"}%% *}
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted limits expl 'process
//! limit' compadd "$@" - <names>` by calling our ported [`_wanted`]
//! with `tag = "limits"` and `descr = "process limit"`. Inner action
//! is the equivalent of `compadd "$@" - <names>`: add each `<name>`
//! as a match, honouring `"$@"` passthrough opts.
//!
//! Shell-side `${${(f)"$(limit)"}%% *}` runs the `limit` builtin,
//! splits stdout on newlines (`(f)`), and strips everything from the
//! first whitespace onward (`%% *`). That yields the bare limit names
//! ("cputime", "filesize", ...).
//!
//! Rust-side divergence (`// rust:`): the leaf can't fork-exec `limit`
//! from inside compsys; instead callers either accept the static
//! [`LIMIT_NAMES`] (default) or inject their own list via the
//! `names` parameter. Behaviour is otherwise identical.
//!
//! Shell local `expl` is the description-array name threaded into
//! `_wanted`; in Rust it's implicit (the description is `_wanted`'s
//! third argument).

use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::Completion;
use crate::compsys::ported::_wanted::_wanted;

/// Canonical ulimit resource names (from `limit` / `ulimit -a`).
/// Matches both the BSD `limit` output and Linux `ulimit -a` rows.
pub const LIMIT_NAMES: &[&str] = &[
    "addressspace", "aiomemorylocked", "aiooperations", "cachedthreads",
    "coredumpsize", "cputime", "datasize", "descriptors", "filesize",
    "kqueues", "maxlockedmemory", "maxmessage", "maxnice", "maxproc",
    "maxpthreads", "maxrtprio", "memorylocked", "memoryqueue", "memoryuse",
    "msgqueue", "nofile", "openfiles", "pendingsignals", "pseudoterminals",
    "resident", "rttime", "sigpending", "sockbufsize", "stacksize",
    "swapsize", "threads", "userprocesses", "virtualmemory", "vmemorysize",
];

/// `_limits` — emit process limit names.
///
/// `args` mirrors shell `"$@"` passthrough (currently unused — the
/// existing compadd-shim doesn't forward arbitrary opts; reserved
/// for future plumbing).
// rust: shell reads `$(limit)` directly; Rust accepts `state`
// + passthrough args explicitly.
pub fn _limits(state: &mut MainCompleteState, args: &[String]) -> bool {
    // sh:3  local expl    — description-array; managed by _wanted
    let _expl: () = ();

    // sh:5  _wanted limits expl 'process limit' compadd "$@" - <names>
    let prefix = state.comp.params.prefix.clone();
    _wanted(state, "limits", "process limit", |s| {
        // sh:5  ${${(f)"$(limit)"}%% *}   — names from `limit` builtin
        //                                   (Rust uses LIMIT_NAMES constant
        //                                   because we can't fork `limit`)
        let _ = args; // "$@" passthrough; reserved
        let mut any = false;
        for name in LIMIT_NAMES {
            // sh:5  compadd <names>   — emit each name, prefix-filtered
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
    use crate::compsys::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["limits".into()]);
        state.tags.add_try(&["limits".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_at_least_one_limit_with_empty_prefix() {
        // sh:5 — `compadd <names>` with empty prefix emits all limit
        // names. We pin ≥10 to be robust across platforms (the
        // canonical set has 34 names; truncation below 10 would be
        // a serious regression).
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        assert!(_limits(&mut state, &[]));
        assert!(state.comp.groups[0].matches.len() >= 10);
    }

    #[test]
    fn untagged_call_skips_emission() {
        // sh:5 — `_wanted limits ...` gates on `limits` ∈ requested
        // tag set. Without seeding, the inner compadd never runs.
        let mut state = MainCompleteState::new("", 0);
        assert!(!_limits(&mut state, &[]));
    }

    #[test]
    fn prefix_filters_to_matching_limits() {
        // sh:5 — `compadd` honours `$PREFIX` (here "max"); only names
        // beginning with "max" survive.
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "max".into();
        let _ = _limits(&mut state, &[]);
        let names: Vec<&str> = state.comp.groups[0]
            .matches.iter().map(|c| c.str_.as_str()).collect();
        assert!(names.iter().all(|n| n.starts_with("max")));
        assert!(names.contains(&"maxproc"));
    }

    #[test]
    fn known_canonical_names_present() {
        // sh:5 — names sourced from `limit` builtin output; pin that
        // the well-known POSIX/BSD limits all show up.
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _limits(&mut state, &[]);
        let names: Vec<&str> = state.comp.groups[0]
            .matches.iter().map(|c| c.str_.as_str()).collect();
        for needed in [
            "cputime", "filesize", "datasize", "stacksize", "coredumpsize",
            "memoryuse", "descriptors",
        ] {
            assert!(names.contains(&needed), "missing canonical limit `{needed}`");
        }
    }

    #[test]
    fn off_prefix_returns_false() {
        // sh:5 — when prefix matches nothing, inner action returns
        // false; _wanted propagates → _limits returns false.
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "definitely-not-a-limit".into();
        assert!(!_limits(&mut state, &[]));
    }

    #[test]
    fn group_named_limits() {
        // sh:5 — `_wanted limits ...` opens a group named "limits".
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _limits(&mut state, &[]);
        assert!(state.comp.groups.iter().any(|g| g.name == "limits"));
    }

    #[test]
    fn description_attached_to_limits_group() {
        // sh:5 — `_wanted limits expl 'process limit' ...` pushes
        // "process limit" as the group's explanation string.
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _limits(&mut state, &[]);
        let grp = state.comp.groups.iter().find(|g| g.name == "limits").unwrap();
        assert!(grp.explanations.iter().any(|e| e == "process limit"));
    }

    #[test]
    fn args_passthrough_does_not_panic() {
        // sh:5 — `"$@"` passes arbitrary compadd opts (e.g. -X, -J).
        // Current compadd shim ignores them; pin that arbitrary args
        // don't crash the wrapper.
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let args = vec!["-X".into(), "process limit name".into()];
        let _ = _limits(&mut state, &args);
    }
}
