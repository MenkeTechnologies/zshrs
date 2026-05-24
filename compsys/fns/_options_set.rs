//! Port of `_options_set` — complete currently set options. Moved from
//! `compsys/library.rs`. Renamed from `options_set` to mirror zsh shell
//! function name `_options_set`.

use crate::compcore::CompletionState;

use super::_options::_options;

/// _options_set - Complete currently set options
pub fn _options_set(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let set_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| *is_set)
        .copied()
        .collect();
    _options(state, &set_opts)
}
