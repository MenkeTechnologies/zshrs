//! Port of `_prefix` — complete with prefix handling. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _prefix - Complete with prefix handling
pub fn _prefix(
    state: &mut CompletionState,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // Save suffix, complete prefix only, restore
    let saved_suffix = state.params.suffix.clone();
    state.params.suffix.clear();

    let result = action(state);

    state.params.suffix = saved_suffix;
    result
}
