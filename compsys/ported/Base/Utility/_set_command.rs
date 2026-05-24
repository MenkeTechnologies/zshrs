//! Port of `_set_command` — set the command being completed.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_set_command`
//! (system copy `/opt/homebrew/share/zsh/functions/_set_command`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  # Sets parameters _comp_command1, _comp_command2 and _comp_command
//!  9  command="$words[1]"
//! 13  if (( $+builtins[$command] + $+functions[$command] )); then
//! 14    _comp_command1="$command"
//! 15    _comp_command="$_comp_command1"
//! 16  elif [[ "$command[1]" = '=' ]]; then
//! 17    eval _comp_command2\=$command
//! ```
//!
//! Upstream sets THREE parameters (_comp_command, _comp_command1,
//! _comp_command2) using slightly different rules per command shape.
//!
//! Simplified Rust port: records just `lastcomp[command] = words[0]`
//! — sufficient for the common single-command lookup; the dual-form
//! _comp_command1/2 distinction was used by legacy compctl
//! callsites that we don't carry.

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
