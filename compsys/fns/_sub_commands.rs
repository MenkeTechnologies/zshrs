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
