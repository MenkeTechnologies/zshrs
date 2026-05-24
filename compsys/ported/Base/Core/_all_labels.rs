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
