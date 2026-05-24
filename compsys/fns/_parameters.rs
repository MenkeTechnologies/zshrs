//! Port of `_parameters` — complete parameter (variable) names. Moved
//! from `compsys/library.rs`. Renamed from `parameters` to mirror zsh
//! shell function name `_parameters`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _parameters - Complete parameter (variable) names
pub fn _parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("parameters", true);

    for name in params.keys() {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("parameters"));
        }
    }

    state.end_group();
    state.nmatches > 0
}
