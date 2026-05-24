//! Port of `_absolute_command_paths` — complete commands by absolute path.
//!
//! Moved verbatim from `compsys/library.rs` to mirror zsh's one-file-per-
//! function layout. Local shell reference: zsh upstream
//! `Completion/Unix/Command/_absolute_command_paths`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::is_executable;

/// _absolute_command_paths - Complete commands with absolute paths
pub fn _absolute_command_paths(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    // Search PATH for executables
    if let Ok(path_var) = std::env::var("PATH") {
        state.begin_group("commands", true);

        for dir in path_var.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    if name_str.starts_with(&prefix) {
                        // Return absolute path
                        let full_path = entry.path();
                        if is_executable(&full_path) {
                            state.add_match(
                                Completion::new(full_path.to_string_lossy().to_string()),
                                Some("commands"),
                            );
                        }
                    }
                }
            }
        }

        state.end_group();
        state.nmatches > 0
    } else {
        false
    }
}
