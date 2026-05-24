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
//! Simplified Rust port: takes a `(name, description)` slice and
//! emits each with the standard `name -- desc` disp format,
//! prefix-filtered. Drops the position-2 gate (caller-side
//! responsibility) and the "no more arguments" message.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _sub_commands - Complete subcommands
pub fn _sub_commands(
    state: &mut CompletionState,
    commands: &[(String, String)], // (name, description)
) -> bool {
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

    #[test]
    fn emits_subcommand_with_disp() {
        let mut state = CompletionState::new();
        let cmds = vec![
            ("commit".into(), "Create commit".into()),
            ("push".into(), "Push to remote".into()),
        ];
        assert!(_sub_commands(&mut state, &cmds));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str["commit"], "commit -- Create commit");
        assert_eq!(by_str["push"], "push -- Push to remote");
    }

    #[test]
    fn prefix_filters_subcommands() {
        let mut state = CompletionState::new();
        state.params.prefix = "co".into();
        let cmds = vec![("commit".into(), "".into()), ("push".into(), "".into())];
        assert!(_sub_commands(&mut state, &cmds));
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
        let cmds = vec![("x".into(), "".into())];
        assert!(_sub_commands(&mut state, &cmds));
        assert_eq!(state.groups[0].matches[0].disp, None);
    }
}
