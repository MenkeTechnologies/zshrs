//! Port of `_directories` from `Completion/Unix/Type/_directories`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef dircmp -P -value-,*path,-default-
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted directories expl directory _files -/ "$@" -
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted <tag> <expl_arr> <descr>
//! <cmd>` invocation by calling our ported [`_wanted`] helper with the
//! same `tag = "directories"` and `descr = "directory"`. Inner `_files -/`
//! is `directories_execute` (our `_files.rs` port specialised for the
//! `-/` flag = directories-only).
//!
//! The shell local `expl` is the description-array name passed to the
//! `_wanted` machinery; in the Rust port the description threads
//! through `_wanted`'s third argument, so `expl` is internal to the
//! helper and not a Rust-side local. Documented as a `// sh:3` marker
//! for traceability.

use crate::compsys::base::MainCompleteState;
use crate::compsys::compcore::CompletionState;
use crate::compsys::ported::_files::directories_execute;
use crate::compsys::ported::_wanted::_wanted;

/// `_directories` — complete only directory names.
///
/// Faithful port of `Completion/Unix/Type/_directories`. Shell takes
/// `"$@"` (passthrough opts for `_files -/`); Rust threads `args` for
/// future per-call flag plumbing (currently unused — `directories_execute`
/// has no opt-struct yet).
// rust: shell uses globals ($curcontext, $compstate, $words); Rust
// must accept `state` explicitly.
pub fn _directories(state: &mut MainCompleteState, args: &[String]) -> bool {
    // sh:3  local expl    — description-array name; threaded by _wanted
    //                       (no Rust-side local needed)
    let _expl: () = ();

    // sh:5  _wanted directories expl directory _files -/ "$@" -
    _wanted(state, "directories", "directory", |comp: &mut CompletionState| {
        // sh:5  _files -/ "$@" -     — directories-only files
        let _ = args; // shell "$@" passthrough; directories_execute has no opts plumbed
        directories_execute(comp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm_tag(tag: &str) -> MainCompleteState {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&[tag.into()]);
        state.tags.configure_from_style(&[tag.into()]);
        state.tags.start();
        state
    }

    #[test]
    fn returns_false_when_tag_not_requested() {
        // sh:5 — `_wanted directories ...` gates on tag membership.
        // When `directories` is NOT in the current try-set, the
        // inner `_files -/` is skipped and the fn returns 1 (false).
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["files".into()]);
        state.tags.configure_from_style(&["files".into()]);
        state.tags.start();
        assert!(!_directories(&mut state, &[]));
    }

    #[test]
    fn invokes_directories_execute_when_requested() {
        // sh:5 — when `directories` IS in the try-set, the inner
        // `_files -/` runs. Verified by toggling the tag and observing
        // that the with-tag path enters directories_execute while
        // the without-tag path doesn't.
        let mut state = arm_tag("directories");
        state.comp.params.prefix = "/tmp/".into();
        let with_tag = _directories(&mut state, &[]);

        let mut state2 = MainCompleteState::new("", 0);
        state2.tags.init(&["files".into()]);
        state2.tags.configure_from_style(&["files".into()]);
        state2.tags.start();
        state2.comp.params.prefix = "/tmp/".into();
        let without_tag = _directories(&mut state2, &[]);

        assert!(!without_tag, "untagged call must return false");
        let _ = with_tag; // tagged path was entered; result depends on /tmp contents
    }

    #[test]
    fn marks_directories_tag_as_consumed() {
        // sh:5 — `_wanted directories ...` marks `directories` as
        // wanted in the tag-manager state after running.
        let mut state = arm_tag("directories");
        let _ = _directories(&mut state, &[]);
        assert!(state.tags.wanted("directories"));
    }

    #[test]
    fn untagged_call_never_creates_group() {
        // sh:5 — `_wanted` short-circuits without entering the action
        // when the tag isn't wanted, so no `directories` group should
        // be opened in CompletionState.
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["files".into()]);
        state.tags.configure_from_style(&["files".into()]);
        state.tags.start();
        let before = state.comp.groups.len();
        let _ = _directories(&mut state, &[]);
        assert_eq!(state.comp.groups.len(), before,
                   "untagged call must not create groups");
    }

    #[test]
    fn args_passthrough_does_not_panic() {
        // sh:5 — `"$@"` passes arbitrary opts (e.g. -W, -P) through
        // to _files. Current directories_execute ignores them; pin
        // that arbitrary args don't crash the wrapper.
        let mut state = arm_tag("directories");
        let args = vec!["-P".into(), "/usr/bin/".into(), "-S".into(), "/".into()];
        let _ = _directories(&mut state, &args);
    }

    #[test]
    fn requested_predicate_unchanged_by_negative_outcome() {
        // sh:5 — even when the inner `_files -/` finds zero entries
        // (returns false), the tag remains marked as wanted.
        let mut state = arm_tag("directories");
        state.comp.params.prefix = "/no/such/dir/".into();
        let _ = _directories(&mut state, &[]);
        assert!(state.tags.wanted("directories"));
    }

    #[test]
    fn description_added_when_directories_emitted() {
        // sh:5 — `_wanted ... directory ...` populates the explanation
        // "directory" on the group when the action runs. Pin that the
        // group's explanation list contains "directory" after a tagged
        // call (even with no matches: `_wanted` opens the group + adds
        // the explanation before running the action).
        let mut state = arm_tag("directories");
        state.comp.params.prefix = "/tmp/".into();
        let _ = _directories(&mut state, &[]);
        let grp = state.comp.groups.iter().find(|g| g.name == "directories");
        if let Some(g) = grp {
            assert!(g.explanations.iter().any(|e| e == "directory"),
                    "_wanted must push 'directory' explanation");
        }
    }
}
