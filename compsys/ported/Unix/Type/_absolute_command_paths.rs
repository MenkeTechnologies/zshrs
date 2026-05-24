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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_absolute_paths() {
        // Inject /bin into PATH so the scan has known content.
        let orig = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "/bin");
        let mut state = CompletionState::new();
        // Use bare letter prefix; `l` finds `ls`, `ln`, … in /bin.
        state.params.prefix = "l".into();
        let _ = _absolute_command_paths(&mut state);
        std::env::set_var("PATH", &orig);

        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        for n in &names {
            assert!(
                n.starts_with('/'),
                "_absolute_command_paths must emit absolute paths; got `{n}`"
            );
        }
    }

    #[test]
    fn empty_path_dirs_emit_no_matches() {
        // Set PATH to a definitely-nonexistent dir; the read_dir
        // call fails and no matches are added.
        let orig = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "/no/such/path/at/all/for/zshrs/test");
        let mut state = CompletionState::new();
        state.params.prefix = "anything".into();
        let result = _absolute_command_paths(&mut state);
        std::env::set_var("PATH", &orig);
        assert!(!result, "nonexistent PATH dir → no matches → false");
    }
}
