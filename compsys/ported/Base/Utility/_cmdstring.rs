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
        // _cmdstring is `_command_names $@` (NOT externals_only).
        // Full mode emits all 8 tag categories — verify the named
        // ones appear.
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
}
