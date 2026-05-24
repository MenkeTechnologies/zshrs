//! Port of `_message` — display a message (no completions).
//!
//! Extracted from `compsys/base.rs` (was `pub fn message`, lines
//! ~840-854). Renamed to `_message` to match the upstream zsh shell
//! function name at `Completion/Base/Core/_message`.

use crate::compcore::CompletionState;
use crate::ported::_description::_description;
use crate::zstyle::ZStyleStore;

/// _message - display a message (no completions)
pub fn _message(
    state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    message: &str,
) {
    let formatted = _description(state, styles, context, tag, message);

    if let Some(msg) = formatted {
        state.begin_group(tag, true);
        state.add_explanation(msg, Some(tag));
        state.end_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_explanation_into_named_tag_group() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        _message(&mut state, &styles, ":complete::test:", "messages", "no values to complete");
        let grp = state
            .groups
            .iter()
            .find(|g| g.name == "messages")
            .expect("messages group present");
        assert!(!grp.explanations.is_empty(), "message must appear as an explanation");
    }

    #[test]
    fn nmessages_counter_incremented() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        assert_eq!(state.nmessages, 0);
        _message(&mut state, &styles, ":complete::test:", "messages", "hello");
        assert_eq!(state.nmessages, 1, "_message must bump nmessages");
    }
}
