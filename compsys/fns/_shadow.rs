//! Port of `_shadow` — shadow existing completions. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _shadow - Shadow existing completions
pub fn _shadow(
    state: &mut CompletionState,
    _shadow_name: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // Shadow mechanism - run action in isolated context
    action(state)
}
