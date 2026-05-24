//! Port of `_set_command` — set the command being completed. Moved
//! from `compsys/functions.rs`. Renamed from `set_command` to mirror
//! zsh shell function name `_set_command`.

use crate::base::MainCompleteState;

/// _set_command - Set the command being completed
pub fn _set_command(state: &mut MainCompleteState) {
    if !state.comp.params.words.is_empty() {
        let cmd = &state.comp.params.words[0];
        // Would set _comp_command1, _comp_command2, etc.
        state.lastcomp.insert("command".to_string(), cmd.clone());
    }
}
