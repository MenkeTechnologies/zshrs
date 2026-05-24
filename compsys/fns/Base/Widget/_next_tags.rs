//! Port of `_next_tags` — move to next tag set. Moved from
//! `compsys/functions.rs`.

use crate::base::MainCompleteState;

/// _next_tags - Move to next tag set
pub fn _next_tags(state: &mut MainCompleteState) -> bool {
    state.tags.next()
}
