//! Port of `_complete_help` — show completion help. Moved from
//! `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _complete_help - Show completion help
pub fn _complete_help(state: &mut CompletionState, help_entries: &[(String, String)]) -> bool {
    state.begin_group("help", true);

    for (topic, desc) in help_entries {
        let mut comp = Completion::new(topic);
        comp.disp = Some(format!("{} -- {}", topic, desc));
        state.add_match(comp, Some("help"));
    }

    state.end_group();
    !help_entries.is_empty()
}
