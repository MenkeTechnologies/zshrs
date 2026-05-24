//! Port of `_cmdstring` — complete a command string (for eval, etc.).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_cmdstring`
//! (system copy `/opt/homebrew/share/zsh/functions/_cmdstring`).
//!
//! Upstream shell source (full 6 lines):
//! ```text
//!  4  compset -q
//!  5  _normal
//! ```
//!
//! Upstream calls `compset -q` to treat the current word as a
//! quoted shell argument, then dispatches to `_normal` (which
//! handles command-vs-argument selection).
//!
//! Simplified Rust port: skips the `compset -q` quote-stripping
//! (handled at the ZLE layer) and dispatches to `_command_names`
//! in full mode — equivalent user behavior for the most common
//! eval-arg case.

use crate::compcore::CompletionState;

use super::_command_names::{_command_names, ShellInventory};

/// _cmdstring - Complete a command string (for eval, etc.)
pub fn _cmdstring(state: &mut CompletionState, inv: &ShellInventory<'_>) -> bool {
    // Complete as if it were a command line
    _command_names(state, inv, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_command_names_full_mode() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let groups: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        for tag in ["commands", "builtins", "functions", "aliases"] {
            assert!(
                groups.contains(&tag),
                "_cmdstring must delegate to full _command_names; missing tag {tag}"
            );
        }
    }

    #[test]
    fn emits_builtin_match_with_prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "tr".into();
        let builtins = vec!["true".into(), "trap".into(), "exit".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"true".to_string()));
        assert!(names.contains(&"trap".to_string()));
        assert!(!names.contains(&"exit".to_string()));
    }

    #[test]
    fn empty_inventory_still_emits_groups() {
        let mut state = CompletionState::new();
        let inv = ShellInventory::default();
        let _ = _cmdstring(&mut state, &inv);
        // Full-mode _command_names creates the tag groups even when
        // empty.
        let groups: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"commands"));
    }

    #[test]
    fn aliases_appear_in_result() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let aliases = vec!["ll".into(), "la".into()];
        let inv = ShellInventory {
            aliases: &aliases,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"ll".to_string()));
    }
}
