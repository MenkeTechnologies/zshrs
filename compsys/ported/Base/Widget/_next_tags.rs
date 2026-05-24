//! Port of `_next_tags` — move to next tag set.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_next_tags`
//! (system copy `/opt/homebrew/share/zsh/functions/_next_tags`).
//!
//! Upstream shell source is a `list-choices` widget that swaps the
//! `_all_labels` and `_next_label` fns with wrappers that advance
//! the active tag set, then re-runs completion:
//! ```text
//!  5  _next_tags() {
//!  6    eval "$_comp_setup"
//!  8    local ins ops="$PREFIX$SUFFIX"
//! 10    unfunction _all_labels _next_label
//! 12    _all_labels() { …advance + dispatch… }
//! ```
//!
//! Simplified Rust port: drops the fn-shadowing trick (Rust can't
//! re-bind named fns at runtime) and just calls `TagManager::next()`
//! directly — which IS the advance the shadowed fns ultimately do.

use crate::base::MainCompleteState;

/// _next_tags - Move to next tag set
pub fn _next_tags(state: &mut MainCompleteState) -> bool {
    state.tags.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_tag_manager_next() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["a".into(), "b".into()]);
        state.tags.add_try(&["a".into()]);
        state.tags.add_try(&["b".into()]);
        state.tags.start();
        assert!(_next_tags(&mut state), "first call advances to set 2");
        assert!(!_next_tags(&mut state), "second call → no more sets");
    }
}
