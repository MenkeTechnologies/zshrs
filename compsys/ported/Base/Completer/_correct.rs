//! Port of `_correct` — spelling correction.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_correct`
//! (system copy `/opt/homebrew/share/zsh/functions/_correct`).
//!
//! Upstream shell source (the whole 19-line wrapper):
//! ```text
//!  3  # This is mainly a wrapper around the more general `_approximate'.
//!  4  # By setting `compstate[pattern_match]' to something unequal to `*'
//!  5  # and then calling `_approximate', we get only corrections.
//! 11  local ret=1 opm="$compstate[pattern_match]"
//! 13  compstate[pattern_match]='-'
//! 15  _approximate && ret=0
//! 17  compstate[pattern_match]="$opm"
//! 19  return ret
//! ```
//!
//! Faithful Rust port: `_approximate(state, 1)` — pinning max_errors
//! to 1 mirrors shell's "only corrections" semantic (the compstate
//! [pattern_match]='-' trick prevents pattern-match acceptance,
//! leaving only Levenshtein-1 matches).

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

    #[test]
    fn exact_match_passes_max_errors_1() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "commit".into();
        state.comp.add_match(Completion::new("commit"), None);
        assert!(matches!(_correct(&mut state), CompleterResult::Matched));
    }

    #[test]
    fn empty_candidates_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "anything".into();
        assert!(matches!(_correct(&mut state), CompleterResult::NoMatch));
    }
}
