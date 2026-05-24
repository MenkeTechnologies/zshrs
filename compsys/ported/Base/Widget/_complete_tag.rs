//! Port of `_complete_tag` — complete for specific tag.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_complete_tag`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete_tag`).
//!
//! NOTE: the upstream `_complete_tag` is a TAGS-file widget — it
//! reads ctags/etags output to complete identifiers from a project's
//! TAGS file. Our Rust port uses the SAME NAME for a different,
//! more general purpose: "run an action gated on a tag being
//! requested." The shell's TAGS-file widget is not portable to this
//! layer (it depends on the comp-system tag-walking + filesystem
//! TAGS-file location heuristic).
//!
//! The Rust impl matches the SHAPE of `_wanted` more than the
//! upstream `_complete_tag` — see [`crate::ported::_wanted`] for
//! the canonical "if-tag-requested-then-run-action" port.

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
