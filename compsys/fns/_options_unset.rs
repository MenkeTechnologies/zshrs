//! Port of `_options_unset` — complete currently unset options. Moved
//! from `compsys/library.rs`. Renamed from `options_unset` to mirror
//! zsh shell function name `_options_unset`.

use crate::compcore::CompletionState;

use super::_options::_options;

/// _options_unset - Complete currently unset options
pub fn _options_unset(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let unset_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| !*is_set)
        .copied()
        .collect();
    _options(state, &unset_opts)
}
