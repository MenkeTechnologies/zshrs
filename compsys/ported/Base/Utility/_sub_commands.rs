//! Port of `_sub_commands` — complete subcommands. Moved from
//! `compsys/functions.rs`.

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
