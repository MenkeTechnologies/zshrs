//! Port of `_regex_arguments` — complete using regex-based argument
//! specs.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_regex_arguments`
//! (system copy `/opt/homebrew/share/zsh/functions/_regex_arguments`).
//!
//! Upstream is an 86-line spec-parser that turns a function-name +
//! list-of-pat→action-tuples into a callable state-machine. Key
//! lines:
//! ```text
//!  3  _regex_arguments() {
//!  6    local name=$1
//!  7    shift
//!  9    local spec specs all_descr i
//! 24    eval "$name() {
//! 25      local i offset=\$CURRENT \$#words match mbegin mend
//! ```
//!
//! Upstream compiles the patterns into a generated shell fn.
//!
//! Simplified Rust port: takes the patterns directly and walks
//! them at completion-time (no compile step). First pattern that
//! matches `current_word()` wins; emits the description as the
//! group explanation. Uses `regex_lite` so patterns are real PCRE,
//! not zsh extended-globs.

use crate::compcore::CompletionState;

/// _regex_arguments - Complete using regex-based argument specs
pub fn _regex_arguments(
    state: &mut CompletionState,
    _name: &str,
    patterns: &[(String, String, String)], // (pattern, description, action)
) -> bool {
    let current = state.params.current_word();

    for (pattern, desc, action) in patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            if re.is_match(&current) {
                // Would execute the action
                state.begin_group("regex", true);
                state.add_explanation(desc.clone(), Some("regex"));
                state.end_group();
                let _ = action;
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_matching_pattern_wins() {
        let mut state = CompletionState::new();
        state.params.prefix = "abc123".into();
        let patterns = vec![
            ("^xyz".into(), "no match".into(), "".into()),
            ("^abc".into(), "matches abc-prefix".into(), "".into()),
            ("^.*".into(), "matches anything".into(), "".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        // First MATCHING pattern wins → "matches abc-prefix" emitted.
        let exps: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        assert!(exps.contains(&"matches abc-prefix".to_string()), "got {exps:?}");
    }

    #[test]
    fn no_pattern_matches_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "abc".into();
        let patterns = vec![("^xyz".into(), "no".into(), "".into())];
        assert!(!_regex_arguments(&mut state, "x", &patterns));
    }

    #[test]
    fn invalid_regex_pattern_skipped() {
        let mut state = CompletionState::new();
        let patterns = vec![
            ("[bad-regex".into(), "bad".into(), "".into()),
            (".*".into(), "good".into(), "".into()),
        ];
        // Invalid is skipped; second pattern matches.
        assert!(_regex_arguments(&mut state, "x", &patterns));
    }
}
