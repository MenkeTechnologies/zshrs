//! Port of `_next_tags` — move to next tag set.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_next_tags`
//! (system copy `/opt/homebrew/share/zsh/functions/_next_tags`).
//!
//! Upstream shell source is a `list-choices` widget that swaps the
//! `_all_labels` and `_next_label` fns with wrappers that advance
//! the active tag set, then re-runs completion:
//! ```text
//!  5  _next_tags() {
//!  6    eval "$_comp_setup"
//!  8    local ins ops="$PREFIX$SUFFIX"
//! 10    unfunction _all_labels _next_label
//! 12    _all_labels() { …advance + dispatch… }
//! ```
//!
//! Simplified Rust port: drops the fn-shadowing trick (Rust can't
//! re-bind named fns at runtime) and just calls `TagManager::next()`
//! directly — which IS the advance the shadowed fns ultimately do.

use crate::base::MainCompleteState;

/// _next_tags - Move to next tag set
pub fn _next_tags(state: &mut MainCompleteState) -> bool {
    state.tags.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_tag_manager_next() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["a".into(), "b".into()]);
        state.tags.add_try(&["a".into()]);
        state.tags.add_try(&["b".into()]);
        state.tags.start();
        assert!(_next_tags(&mut state), "first call advances to set 2");
        assert!(!_next_tags(&mut state), "second call → no more sets");
    }

    #[test]
    fn returns_false_when_no_more_tag_sets() {
        let mut state = MainCompleteState::new("", 0);
        // Empty tag manager → no sets to advance to.
        assert!(!_next_tags(&mut state));
    }

    #[test]
    fn after_advance_wanted_reflects_current_set() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["files".into(), "directories".into()]);
        state.tags.add_try(&["files".into()]);
        state.tags.add_try(&["directories".into()]);
        state.tags.start();
        assert!(state.tags.wanted("files"));
        _next_tags(&mut state);
        assert!(
            state.tags.wanted("directories"),
            "after _next_tags, directories must be the wanted tag"
        );
    }

    #[test]
    fn single_set_returns_false_on_first_call() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["only".into()]);
        state.tags.add_try(&["only".into()]);
        state.tags.start();
        assert!(!_next_tags(&mut state));
    }

    #[test]
    fn three_sets_advance_then_exhaust() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["a".into(), "b".into(), "c".into()]);
        state.tags.add_try(&["a".into()]);
        state.tags.add_try(&["b".into()]);
        state.tags.add_try(&["c".into()]);
        state.tags.start();
        assert!(_next_tags(&mut state), "a → b");
        assert!(_next_tags(&mut state), "b → c");
        assert!(!_next_tags(&mut state), "c → done");
    }

    #[test]
    fn next_tags_does_not_emit_matches() {
        // _next_tags only mutates the tag-set pointer; it doesn't
        // add completion candidates. Pin that no group is created.
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["x".into(), "y".into()]);
        state.tags.add_try(&["x".into()]);
        state.tags.add_try(&["y".into()]);
        state.tags.start();
        let before_groups = state.comp.groups.len();
        let before_n = state.comp.nmatches;
        _next_tags(&mut state);
        assert_eq!(state.comp.groups.len(), before_groups);
        assert_eq!(state.comp.nmatches, before_n);
    }

    #[test]
    fn idempotent_after_exhaustion() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["solo".into()]);
        state.tags.add_try(&["solo".into()]);
        state.tags.start();
        assert!(!_next_tags(&mut state));
        // Calling again must STILL return false (no panic, no state
        // change).
        assert!(!_next_tags(&mut state));
        assert!(!_next_tags(&mut state));
    }
}
