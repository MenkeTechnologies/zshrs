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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_topic_with_topic_dash_desc_disp() {
        let mut state = CompletionState::new();
        let entries = vec![
            ("foo".into(), "the foo cmd".into()),
            ("bar".into(), "the bar cmd".into()),
        ];
        assert!(_complete_help(&mut state, &entries));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str["foo"], "foo -- the foo cmd");
        assert_eq!(by_str["bar"], "bar -- the bar cmd");
    }

    #[test]
    fn empty_entries_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_complete_help(&mut state, &[]));
    }
}
