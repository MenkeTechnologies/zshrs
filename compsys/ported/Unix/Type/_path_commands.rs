//! Port of `_path_commands` — emit external executables from `$path`.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_path_commands`.
//!
//! Upstream shell source (key dispatch):
//! ```text
//! if [[ -n $need_desc ]]; then …whatis-cached descriptions… else
//!   _wanted commands expl 'external command' compadd "$@" -k commands
//! fi
//! ```
//!
//! `compadd -k commands` is shorthand for "emit every external
//! executable found in $path" — exactly what [`_command_names`] in
//! externals-only mode does.
//!
//! Strict Rust port: wraps the emission with [`_wanted`] on the
//! `commands` tag with `'external command'` description (matching
//! upstream verbatim), then dispatches to [`_command_names`] with
//! `externals_only=true`.

use crate::base::MainCompleteState;
use crate::ported::_command_names::{ShellInventory, _command_names};
use crate::ported::_wanted::_wanted;

/// `_path_commands` — emit external executables.
pub fn _path_commands(state: &mut MainCompleteState) -> bool {
    // shell: `_wanted commands expl 'external command' compadd "$@" -k commands`
    _wanted(state, "commands", "external command", |s| {
        let inv = ShellInventory::default();
        _command_names(s, &inv, true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["commands".into()]);
        state.tags.add_try(&["commands".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_path_executables_with_known_prefix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _path_commands(&mut state);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.iter().any(|n| *n == "ls" || n.starts_with("ls")));
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ls".into();
        assert!(!_path_commands(&mut state));
    }

    #[test]
    fn no_panic_with_empty_prefix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _path_commands(&mut state);
    }

    #[test]
    fn off_prefix_emits_no_matches() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "definitely-no-such-binary-zzz-xyz".into();
        let _ = _path_commands(&mut state);
        let matches: usize = state
            .comp
            .groups
            .iter()
            .filter(|g| g.name == "commands")
            .map(|g| g.matches.len())
            .sum();
        assert_eq!(matches, 0);
    }

    #[test]
    fn group_named_commands_with_external_command_description() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _path_commands(&mut state);
        let grp = state
            .comp
            .groups
            .iter()
            .find(|g| g.name == "commands")
            .expect("commands group");
        assert!(grp.explanations.iter().any(|e| e == "external command"));
    }
}
