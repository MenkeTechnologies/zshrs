//! Port of `_cmdambivalent` — for cmds that may be run with or
//! without arguments (`system()`-style vs `execl()`-style).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_cmdambivalent`
//! (system copy `/opt/homebrew/share/zsh/functions/_cmdambivalent`).
//!
//! Upstream shell source (full 17-line fn):
//! ```text
//!  3  if (( CURRENT == 1 && ${#words} == 1 )); then
//!  5    local space=' '
//!  6    if (( ${${words[CURRENT]}[(I)$space]} )); then
//!  7      _cmdstring                    # has spaces → quoted cmdline
//!  8    elif [[ ${${compstate[all_quotes]}[1]} == (\'|\") ]]; then
//!  9      _cmdstring                    # quoted → quoted cmdline
//! 10    else
//! 11      _command_names -e             # bare → external cmd
//! 12    fi
//! 13  elif (( CURRENT == 1 )); then
//! 14    _command_names -e
//! 15  else
//! 16    _normal
//! 17  fi
//! ```
//!
//! Simplified Rust port: drops the space/quote heuristic
//! (caller-level concern) and just `current <= 1 → _command_names`
//! / `else → fall through`. Pinned by the
//! `argument_position_skips_command_names` test.

use crate::base::MainCompleteState;

use super::_command_names::{_command_names, ShellInventory};

/// _cmdambivalent - Handle commands that can be run with or without arguments
pub fn _cmdambivalent(state: &mut MainCompleteState, inv: &ShellInventory<'_>) -> bool {
    // If no arguments yet, complete as command
    if state.comp.params.current <= 1 {
        _command_names(&mut state.comp, inv, false)
    } else {
        // Otherwise use normal completion
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_position_dispatches_to_command_names() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdambivalent(&mut state, &inv);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(
            groups.contains(&"commands"),
            "expected _command_names dispatch in command position; got {groups:?}"
        );
    }

    #[test]
    fn argument_position_skips_command_names() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        let inv = ShellInventory::default();
        assert!(_cmdambivalent(&mut state, &inv));
        assert!(
            state.comp.groups.is_empty(),
            "argument position must NOT call _command_names"
        );
    }

    #[test]
    fn argument_position_returns_true_always() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 5;
        let inv = ShellInventory::default();
        assert!(
            _cmdambivalent(&mut state, &inv),
            "argument-position fall-through must return true"
        );
    }

    #[test]
    fn current_at_boundary_treated_as_command_position() {
        // current <= 1 is the command-position branch.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdambivalent(&mut state, &inv);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"commands"));
    }

    #[test]
    fn current_at_zero_also_treated_as_command_position() {
        // current=0 (edge case) → also command-position dispatch.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 0;
        let inv = ShellInventory::default();
        let _ = _cmdambivalent(&mut state, &inv);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        // _command_names was called → at least the `commands` group
        // gets created.
        assert!(groups.contains(&"commands"));
    }
}
