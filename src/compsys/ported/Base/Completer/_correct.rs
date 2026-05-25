//! Port of `_correct` from `Completion/Base/Completer/_correct`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is mainly a wrapper around the more general `_approximate'.
//! sh: 4  # By setting `compstate[pattern_match]' to something unequal to `*' and
//! sh: 5  # then calling `_approximate', we get only corrections, not all strings
//! sh: 6  # with the corrected prefix and something after it.
//! sh: 7  #
//! sh: 8  # Supported configuration keys are the same as for `_approximate', only
//! sh: 9  # starting with `correct'.
//! sh:10
//! sh:11  local ret=1 opm="$compstate[pattern_match]"
//! sh:12
//! sh:13  compstate[pattern_match]='-'
//! sh:14
//! sh:15  _approximate && ret=0
//! sh:16
//! sh:17  compstate[pattern_match]="$opm"
//! sh:18
//! sh:19  return ret
//! ```
//!
//! Faithful Rust port: `_approximate(state, 1)` — pinning max_errors
//! to 1 mirrors shell's "only corrections" semantic (the compstate
//! [pattern_match]='-' trick prevents pattern-match acceptance,
//! leaving only Levenshtein-1 matches).



use crate::compsys::base::{CompleterResult, MainCompleteState};

use super::_approximate::_approximate;

/// _correct - Spelling correction
pub fn _correct(state: &mut MainCompleteState) -> CompleterResult {
    // Same as approximate with error=1
    _approximate(state, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::completion::Completion;

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

    #[test]
    fn single_char_prefix_bails_like_approximate() {
        // _approximate gates on `|PREFIX+SUFFIX| > 1`. Since
        // _correct delegates, the same gate applies.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "g".into();
        state.comp.add_match(Completion::new("git"), None);
        assert!(matches!(_correct(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn delegates_max_errors_1_not_higher() {
        // Pin: _correct accepts dist-1 but NOT dist-2 typos. If the
        // delegation were `_approximate(2)`, "xyz" would match "abc"
        // (dist 3) too — we want max_errors=1 strict.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ab".into();
        state.comp.add_match(Completion::new("xy"), None);
        // dist("ab", "xy") = 2 → out of max_errors=1.
        assert!(matches!(_correct(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn correct_does_not_emit_from_pattern_match_path() {
        // The upstream uses compstate[pattern_match]='-' to suppress
        // glob acceptance. We can't directly assert that property
        // at the Rust layer, but we CAN pin that a candidate that
        // would only have matched via pattern (not Levenshtein) is
        // not surfaced. Use a glob char in the prefix: with dist=1
        // it has to match a candidate within 1 edit of literally
        // the chars "f*" — only "fa", "fb" etc. would qualify.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "f*".into();
        state.comp.add_match(Completion::new("file"), None);
        // edit_distance("f*", "file") = 3 → too far for _correct.
        assert!(matches!(_correct(&mut state), CompleterResult::NoMatch));
    }
}
