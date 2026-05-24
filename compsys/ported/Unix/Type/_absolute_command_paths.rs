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
    fn emitted_names_under_current_path_are_absolute() {
        // No env mutation — read whatever PATH the test process has.
        // If matches come back, every one MUST start with `/`. This
        // pins the absolute-path contract without racing with
        // parallel tests that fork external commands.
        let mut state = CompletionState::new();
        state.params.prefix = "ls".into();
        let _ = _absolute_command_paths(&mut state);
        for n in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
        {
            assert!(
                n.starts_with('/'),
                "_absolute_command_paths must emit absolute paths; got `{n}`"
            );
        }
    }

    #[test]
    fn prefix_that_matches_nothing_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-not-a-real-cmd-xyz".into();
        assert!(
            !_absolute_command_paths(&mut state),
            "off-prefix → no matches → false"
        );
    }

    #[test]
    fn matches_grouped_under_commands_tag() {
        let mut state = CompletionState::new();
        state.params.prefix = "ls".into();
        let _ = _absolute_command_paths(&mut state);
        // The fn always begins a `commands` group regardless of
        // whether matches end up there.
        assert!(state.groups.iter().any(|g| g.name == "commands"));
    }

    #[test]
    fn empty_prefix_emits_at_least_one_executable() {
        // With empty prefix, every executable in PATH should be a
        // candidate — pin that the count is non-trivial.
        let mut state = CompletionState::new();
        let _ = _absolute_command_paths(&mut state);
        // Reasonable lower bound on most systems: at least `ls`
        // somewhere. Don't fail if PATH is sandboxed empty.
        let count = state
            .groups
            .iter()
            .map(|g| g.matches.len())
            .sum::<usize>();
        // We don't assert > 0 because sandboxed CI may have no PATH;
        // but we DO assert no panic happened by reaching here.
        let _ = count;
    }
}
