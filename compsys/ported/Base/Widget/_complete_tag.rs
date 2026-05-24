//! Port of `_complete_tag` — complete for specific tag. Moved from
//! `compsys/functions.rs`.

use crate::base::MainCompleteState;
use crate::compcore::CompletionState;

/// _complete_tag - Complete for specific tag
pub fn _complete_tag(
    state: &mut MainCompleteState,
    tag: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    if state.tags.requested(tag) {
        state.comp.begin_group(tag, true);
        let result = action(&mut state.comp);
        state.comp.end_group();
        result
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrequested_tag_skips_action() {
        let mut state = MainCompleteState::new("", 0);
        let ran = std::cell::Cell::new(false);
        let result = _complete_tag(&mut state, "values", |_| {
            ran.set(true);
            true
        });
        assert!(!result);
        assert!(!ran.get(), "action must NOT run when tag not requested");
    }

    #[test]
    fn requested_tag_runs_action_and_returns_its_result() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();
        assert!(_complete_tag(&mut state, "values", |_| true));
        // Group named after the tag was begun.
        assert!(state.comp.groups.iter().any(|g| g.name == "values"));
    }
}
