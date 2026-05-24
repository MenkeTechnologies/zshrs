//! Port of `_correct` — spelling correction. Moved from
//! `compsys/functions.rs`. Renamed from `correct` to mirror zsh shell
//! function name `_correct`.

use crate::base::{CompleterResult, MainCompleteState};

use super::_approximate::_approximate;

/// _correct - Spelling correction
pub fn _correct(state: &mut MainCompleteState) -> CompleterResult {
    // Same as approximate with error=1
    _approximate(state, 1)
}
