//! Port of `_complete_tag` — complete for specific tag. Moved from
//! `compsys/functions.rs`.

use crate::base::MainCompleteState;
use crate::compcore::CompletionState;

/// _complete_tag - Complete for specific tag
pub fn _complete_tag(
    state: &mut MainCompleteState,
    tag: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    if state.tags.requested(tag) {
        state.comp.begin_group(tag, true);
        let result = action(&mut state.comp);
        state.comp.end_group();
        result
    } else {
        false
    }
}
