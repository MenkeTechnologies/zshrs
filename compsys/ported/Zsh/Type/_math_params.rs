//! Port of `_math_params` — complete numeric (integer/float) params.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_math_params`.
//!
//! Upstream shell source (the whole 2-line file):
//! ```text
//! _parameters -g '(integer|float)*' || _parameters
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_parameters_with_opts`] with the exact pattern shell uses;
//! on `false` falls back to [`_parameters`] no-filter, matching
//! the shell `||` operator.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::ported::_parameters::{ParametersOpts, _parameters, _parameters_with_opts};

/// `_math_params` — numeric params first, else fall back to all.
pub fn _math_params(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    // shell: `_parameters -g '(integer|float)*'`
    let any = _parameters_with_opts(
        state,
        params,
        &ParametersOpts {
            pattern: Some("(integer|float)*"),
            ..Default::default()
        },
    );
    if any {
        return true;
    }
    // shell: `|| _parameters`
    _parameters(state, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_and_float_params_emitted() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("COUNT".into(), "integer".into());
        p.insert("RATIO".into(), "float".into());
        p.insert("NAME".into(), "scalar".into());
        let _ = _math_params(&mut state, &p);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"COUNT".to_string()));
        assert!(names.contains(&"RATIO".to_string()));
        assert!(!names.contains(&"NAME".to_string()));
    }

    #[test]
    fn fallback_to_all_when_no_numeric_params() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("PATH".into(), "scalar".into());
        let _ = _math_params(&mut state, &p);
        let total: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert!(total > 0, "fallback to _parameters should emit something");
    }

    #[test]
    fn prefix_filter_applies() {
        let mut state = CompletionState::new();
        state.params.prefix = "C".into();
        let mut p = HashMap::new();
        p.insert("COUNT".into(), "integer".into());
        p.insert("RATIO".into(), "float".into());
        let _ = _math_params(&mut state, &p);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"COUNT".to_string()));
        assert!(!names.contains(&"RATIO".to_string()));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_math_params(&mut state, &HashMap::new()));
    }

    #[test]
    fn fallback_only_fires_when_primary_empty() {
        // Mixed bag: primary should fire and return true; fallback
        // must NOT also run and double-emit.
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("NUM".into(), "integer".into());
        p.insert("STR".into(), "scalar".into());
        let _ = _math_params(&mut state, &p);
        let count_str = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .filter(|c| c.str_ == "STR")
            .count();
        assert_eq!(count_str, 0, "fallback must NOT run when primary succeeded");
    }
}
