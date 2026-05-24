//! Port of `_generic` — generic completion widget. Moved from
//! `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};

/// _generic - Generic completion widget
pub fn _generic(
    state: &mut MainCompleteState,
    action: impl FnOnce(&mut MainCompleteState) -> CompleterResult,
) -> CompleterResult {
    action(state)
}
