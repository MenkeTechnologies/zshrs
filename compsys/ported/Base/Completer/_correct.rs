//! Port of `_correct` — spelling correction. Moved from
//! `compsys/functions.rs`. Renamed from `correct` to mirror zsh shell
//! function name `_correct`.

use crate::base::{CompleterResult, MainCompleteState};

use super::_approximate::_approximate;

/// _correct - Spelling correction
pub fn _correct(state: &mut MainCompleteState) -> CompleterResult {
    // Same as approximate with error=1
    _approximate(state, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    #[test]
    fn one_typo_corrected() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "comit".into();
        state.comp.add_match(Completion::new("commit"), None);
        assert!(matches!(_correct(&mut state), CompleterResult::Matched));
    }

    #[test]
    fn two_typos_not_corrected() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "comit".into();
        state.comp.add_match(Completion::new("checkout"), None);
        assert!(matches!(_correct(&mut state), CompleterResult::NoMatch));
    }
}
