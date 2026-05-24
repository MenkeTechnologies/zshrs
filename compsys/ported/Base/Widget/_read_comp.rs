//! Port of `_read_comp` — read completions from file. Moved from
//! `compsys/functions.rs`. Renamed from `read_comp` to mirror zsh
//! shell function name `_read_comp`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _read_comp - Read completions from file
pub fn _read_comp(state: &mut CompletionState, file: &str) -> bool {
    let contents = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let prefix = state.params.prefix.clone();
    let mut matched = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with(&prefix) {
            state.add_match(Completion::new(line), None);
            matched = true;
        }
    }

    matched
}
