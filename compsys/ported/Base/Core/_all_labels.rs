//! Port of `_all_labels` — iterate over all labels for a tag.
//!
//! Extracted from `compsys/base.rs` (was lines ~344-367). Mirrors zsh
//! upstream `Completion/Base/Core/_all_labels`. Convenience wrapper
//! around `_next_label` that runs the supplied closure for each label
//! of the given tag and emits the description as a group explanation.

use crate::base::TagManager;
use crate::compcore::CompletionState;

/// _all_labels - iterate over all labels for a tag
pub fn _all_labels<F>(
    state: &mut CompletionState,
    tags: &mut TagManager,
    tag: &str,
    description: &str,
    mut f: F,
) -> bool
where
    F: FnMut(&mut CompletionState, &str) -> bool,
{
    if !tags.requested(tag) {
        return false;
    }

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    let result = f(state, tag);

    state.end_group();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tags(active: &[&str]) -> TagManager {
        let mut tm = TagManager::new();
        let all: Vec<String> = active.iter().map(|s| s.to_string()).collect();
        tm.init(&all);
        tm.add_try(&all);
        assert!(tm.start());
        tm
    }

    #[test]
    fn returns_false_when_tag_not_in_try_set() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        let invoked = std::cell::Cell::new(false);
        let ok = _all_labels(&mut state, &mut tm, "options", "opts", |_, _| {
            invoked.set(true);
            true
        });
        assert!(!ok);
        assert!(!invoked.get(), "closure must NOT run when tag inactive");
    }

    #[test]
    fn runs_closure_and_emits_explanation_when_active() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        let invoked = std::cell::Cell::new(false);
        let ok = _all_labels(&mut state, &mut tm, "files", "the files", |_, tag| {
            invoked.set(true);
            assert_eq!(tag, "files");
            true
        });
        assert!(ok);
        assert!(invoked.get());
    }
}
