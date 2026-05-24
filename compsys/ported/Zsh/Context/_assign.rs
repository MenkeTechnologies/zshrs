//! Port of `_assign` — complete the LHS of `name=value` assignments.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_assign`.
//!
//! Upstream shell source (full 3 lines):
//! ```text
//! #compdef -assign-parameter-
//!
//! _parameters -g "^*readonly*" -S ''
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_parameters_with_opts`] with `pattern: "^*readonly*"` (extended-
//! glob negation) and `auto_suffix: ""` + `nospace: true` matching
//! upstream's `-S ''`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::ported::_parameters::{ParametersOpts, _parameters_with_opts};

/// `_assign` — emit writable parameter names.
pub fn _assign(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    // shell: `_parameters -g "^*readonly*" -S ''`
    _parameters_with_opts(
        state,
        params,
        &ParametersOpts {
            pattern: Some("^*readonly*"),
            auto_suffix: Some(""),
            nospace: true,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::CompletionFlags;

    #[test]
    fn readonly_parameters_excluded() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("HOME".into(), "scalar".into());
        p.insert("UID".into(), "readonly-integer".into());
        p.insert("PATH".into(), "scalar".into());
        let _ = _assign(&mut state, &p);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"HOME"));
        assert!(names.contains(&"PATH"));
        assert!(!names.contains(&"UID"));
    }

    #[test]
    fn each_emit_carries_nospace() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("X".into(), "scalar".into());
        let _ = _assign(&mut state, &p);
        assert!(state.groups[0].matches[0]
            .flags
            .contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_assign(&mut state, &HashMap::new()));
    }

    #[test]
    fn all_readonly_returns_false() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("UID".into(), "readonly-integer".into());
        p.insert("EUID".into(), "readonly-integer".into());
        assert!(!_assign(&mut state, &p));
    }

    #[test]
    fn prefix_filter_combines_with_readonly_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "U".into();
        let mut p = HashMap::new();
        p.insert("UID".into(), "readonly-integer".into());
        p.insert("USER".into(), "scalar".into());
        let _ = _assign(&mut state, &p);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["USER"]);
    }
}
