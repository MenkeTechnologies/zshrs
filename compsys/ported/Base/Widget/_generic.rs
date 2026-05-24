//! Port of `_generic` — generic completion widget. Moved from
//! `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};

/// _generic - Generic completion widget
pub fn _generic(
    state: &mut MainCompleteState,
    action: impl FnOnce(&mut MainCompleteState) -> CompleterResult,
) -> CompleterResult {
    action(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_action_unconditionally() {
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::Matched),
            CompleterResult::Matched
        ));
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::NoMatch),
            CompleterResult::NoMatch
        ));
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::Skip),
            CompleterResult::Skip
        ));
    }
}
