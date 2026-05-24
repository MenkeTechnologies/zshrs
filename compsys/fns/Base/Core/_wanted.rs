//! Port of `_wanted` — check if tag is wanted and complete. Moved from
//! `compsys/functions.rs`. Renamed from `wanted` to mirror zsh shell
//! function name `_wanted`.

use crate::base::MainCompleteState;
use crate::compcore::CompletionState;

/// _wanted - Check if tag is wanted and complete
pub fn _wanted(
    state: &mut MainCompleteState,
    tag: &str,
    description: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    if !state.tags.requested(tag) {
        return false;
    }

    state.comp.begin_group(tag, true);
    if !description.is_empty() {
        state
            .comp
            .add_explanation(description.to_string(), Some(tag));
    }

    let result = action(&mut state.comp);

    state.comp.end_group();
    result
}
