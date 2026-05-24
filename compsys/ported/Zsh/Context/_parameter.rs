//! Port of `_parameter` — complete parameter names (`-parameter-`
//! context, e.g. `$<TAB>`, `${<TAB>}`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_parameter`.
//!
//! Upstream shell source (the whole 2-line file):
//! ```text
//! #compdef -parameter-
//!
//! _parameters -e
//! ```
//!
//! `_parameters` upstream parses ONLY `-g` via `zparseopts`; every
//! other arg is forwarded to `compadd "$@"` as a passthrough. The
//! `-e` flag has no documented compadd meaning in current zsh and
//! is silently absorbed there; the user-visible behavior of
//! `_parameter` is therefore identical to bare `_parameters`.
//!
//! Strict Rust port: faithful 1:1 — single-line delegate to
//! [`_parameters`].

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::ported::_parameters::_parameters;

/// `_parameter` — `-parameter-` context dispatcher.
pub fn _parameter(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    // shell: `_parameters -e`
    _parameters(state, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_verbatim_to_parameters() {
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("HOME".into(), "scalar".into());
        p.insert("PATH".into(), "scalar".into());
        assert_eq!(_parameter(&mut s1, &p), _parameters(&mut s2, &p));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_parameter(&mut state, &HashMap::new()));
    }

    #[test]
    fn all_params_emitted() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("A".into(), "scalar".into());
        p.insert("B".into(), "scalar".into());
        p.insert("C".into(), "scalar".into());
        let _ = _parameter(&mut state, &p);
        assert_eq!(state.groups[0].matches.len(), 3);
    }

    #[test]
    fn prefix_filter_inherited_from_parameters() {
        let mut state = CompletionState::new();
        state.params.prefix = "PA".into();
        let mut p = HashMap::new();
        p.insert("PATH".into(), "scalar".into());
        p.insert("OTHER".into(), "scalar".into());
        let _ = _parameter(&mut state, &p);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["PATH"]);
    }
}
