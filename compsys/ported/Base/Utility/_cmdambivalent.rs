//! Port of `_cmdambivalent` — handle commands that can be run with or
//! without arguments. Moved from `compsys/library.rs`.

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
}
