//! Port of `_sub_commands` — complete subcommands.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_sub_commands`
//! (system copy `/opt/homebrew/share/zsh/functions/_sub_commands`).
//!
//! Upstream shell source (the whole 9-line fn):
//! ```text
//!  3  local expl
//!  5  if [[ CURRENT -eq 2 ]]; then
//!  6    _wanted commands expl command compadd "$@"
//!  7  else
//!  8    _message 'no more arguments'
//!  9  fi
//! ```
//!
//! Upstream emits the supplied commands when at position 2 (right
//! after the main command name), or "no more arguments" otherwise.
//!
//! Strict Rust port: honors the `current==2` position gate
//! (shell:5). When at position 2, emits the supplied commands;
//! otherwise dispatches `_message 'no more arguments'`.

use crate::compcore::CompletionState;
use crate::completion::Completion;
use crate::ported::_message::_message;
use crate::zstyle::ZStyleStore;

/// _sub_commands - Complete subcommands.
///
/// `position` mirrors `$CURRENT` from the shell. `styles` + `context`
/// are forwarded to `_message` for the off-position case (so the
/// `messages` format style is respected).
pub fn _sub_commands(
    state: &mut CompletionState,
    commands: &[(String, String)],
    position: usize,
    styles: &ZStyleStore,
    context: &str,
) -> bool {
    // shell:5 — `[[ CURRENT -eq 2 ]]`
    if position != 2 {
        // shell:8 — `_message 'no more arguments'`
        _message(state, styles, context, "messages", "no more arguments");
        return false;
    }

    let prefix = state.params.prefix.clone();
    state.begin_group("subcommands", true);

    let mut matched = false;
    for (name, desc) in commands {
        if name.starts_with(&prefix) {
            let mut comp = Completion::new(name);
            if !desc.is_empty() {
                comp.disp = Some(format!("{} -- {}", name, desc));
            }
            state.add_match(comp, Some("subcommands"));
            matched = true;
        }
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_styles() -> ZStyleStore {
        ZStyleStore::new()
    }

    #[test]
    fn position_2_emits_subcommand_with_disp() {
        let mut state = CompletionState::new();
        let cmds = vec![
            ("commit".into(), "Create commit".into()),
            ("push".into(), "Push to remote".into()),
        ];
        let s = empty_styles();
        assert!(_sub_commands(&mut state, &cmds, 2, &s, ""));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str["commit"], "commit -- Create commit");
        assert_eq!(by_str["push"], "push -- Push to remote");
    }

    #[test]
    fn off_position_emits_no_more_arguments_message() {
        let mut state = CompletionState::new();
        let s = empty_styles();
        // position 3 → off → message dispatched, returns false.
        assert!(!_sub_commands(&mut state, &[("x".into(), "".into())], 3, &s, ""));
        let exps: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(
            exps.iter().any(|s| s.contains("no more arguments")),
            "expected the upstream message; got {exps:?}"
        );
    }

    #[test]
    fn position_1_off_does_not_emit_subcommands() {
        let mut state = CompletionState::new();
        let s = empty_styles();
        let _ = _sub_commands(&mut state, &[("foo".into(), "".into())], 1, &s, "");
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"foo"));
    }

    #[test]
    fn prefix_filters_subcommands() {
        let mut state = CompletionState::new();
        state.params.prefix = "co".into();
        let s = empty_styles();
        let cmds = vec![("commit".into(), "".into()), ("push".into(), "".into())];
        assert!(_sub_commands(&mut state, &cmds, 2, &s, ""));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["commit"]);
    }

    #[test]
    fn empty_description_omits_disp() {
        let mut state = CompletionState::new();
        let s = empty_styles();
        let cmds = vec![("x".into(), "".into())];
        assert!(_sub_commands(&mut state, &cmds, 2, &s, ""));
        assert_eq!(state.groups[0].matches[0].disp, None);
    }

    #[test]
    fn empty_commands_at_position_2_returns_false() {
        let mut state = CompletionState::new();
        let s = empty_styles();
        assert!(!_sub_commands(&mut state, &[], 2, &s, ""));
    }
}
