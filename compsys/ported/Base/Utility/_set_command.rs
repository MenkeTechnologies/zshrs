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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_first_word_as_command_in_lastcomp() {
        let mut state = MainCompleteState::new("git status", 10);
        state.comp.params.words = vec!["git".into(), "status".into()];
        _set_command(&mut state);
        assert_eq!(state.lastcomp.get("command").map(String::as_str), Some("git"));
    }

    #[test]
    fn empty_words_does_not_insert() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.words.clear();
        _set_command(&mut state);
        assert!(state.lastcomp.get("command").is_none());
    }
}
