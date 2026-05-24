//! Port of `_next_tags` — move to next tag set. Moved from
//! `compsys/functions.rs`.

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
