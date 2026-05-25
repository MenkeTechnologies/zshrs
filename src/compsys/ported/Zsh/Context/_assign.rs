//! Port of `_assign` from `Completion/Zsh/Context/_assign`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef -assign-parameter-
//! sh: 2
//! sh: 3  _parameters -g "^*readonly*" -S ''
//! ```
//!
//! The shell function has no `local` declarations and no positional
//! parameters — it dispatches a single `_parameters` call with
//! `-g <pattern>` (extended-glob exclusion of read-only params) and
//! `-S ''` (empty auto-suffix, suppressing the default space).
//!
//! Signature divergence (`// rust:`): shell `_assign` takes no args
//! and reads `$words` / `$compstate` globals; Rust port threads
//! `state` + `params` (the hash of `$parameters`) explicitly because
//! compsys-Rust has no process-global equivalent of zsh's
//! `(P)$parameters`.



use std::collections::HashMap;

use crate::compsys::compcore::CompletionState;
use crate::compsys::ported::_parameters::{ParametersOpts, _parameters_with_opts};

/// `_assign` — emit writable parameter names for `name=value` LHS.
// rust: shell takes no args; Rust must accept state + params hash.
pub fn _assign(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    _parameters_with_opts(                                  // sh:3  _parameters
        state,
        params,
        &ParametersOpts {
            pattern: Some("^*readonly*"),                   // sh:3  -g "^*readonly*"
            auto_suffix: Some(""),                          // sh:3  -S ''
            nospace: true,                                  // sh:3  -S '' → nospace
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::completion::CompletionFlags;

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
