//! Port of `_tilde_files` — complete files with tilde expansion. Moved
//! from `compsys/library.rs`. Renamed from `tilde_files` to mirror zsh
//! shell function name `_tilde_files`.

use crate::compcore::CompletionState;

use super::_files::{files_execute, FilesOpts};

/// _tilde_files - Complete files with tilde expansion
pub fn _tilde_files(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    if prefix.starts_with('~') {
        // Expand tilde
        if let Ok(home) = std::env::var("HOME") {
            let expanded = if prefix == "~" {
                home.clone()
            } else if let Some(after_tilde) = prefix.strip_prefix("~/") {
                format!("{}/{}", home, after_tilde)
            } else {
                // ~user form - would need to look up user
                return false;
            };

            // Update state prefix for completion
            let old_prefix = state.params.prefix.clone();
            state.params.prefix = expanded;
            state.params.iprefix = "~".to_string();

            let result = files_execute(state, &FilesOpts::default());

            // Restore
            state.params.prefix = old_prefix;
            state.params.iprefix.clear();

            return result;
        }
    }

    false
}
