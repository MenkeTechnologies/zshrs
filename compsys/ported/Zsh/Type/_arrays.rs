//! Port of `_arrays` — complete array parameter names.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_arrays`.
//!
//! Upstream shell source (the whole 5-line file):
//! ```text
//! #compdef shift
//!
//! local expl
//!
//! _wanted arrays expl array _parameters "$@" -g '*array*'
//! ```
//!
//! Strict Rust port: faithful 1:1 —
//!   1. `_wanted arrays expl array …` — call [`_wanted`] to gate
//!      on the `arrays` tag + push the `array` description.
//!   2. Inside the action: `_parameters -g '*array*'` via
//!      [`_parameters_with_opts`].

use std::collections::HashMap;

use crate::base::MainCompleteState;
use crate::ported::_parameters::{ParametersOpts, _parameters_with_opts};
use crate::ported::_wanted::_wanted;

/// `_arrays` — list array parameters by name.
pub fn _arrays(state: &mut MainCompleteState, params: &HashMap<String, String>) -> bool {
    // shell: `_wanted arrays expl array _parameters -g '*array*'`
    _wanted(state, "arrays", "array", |s| {
        _parameters_with_opts(
            s,
            params,
            &ParametersOpts {
                pattern: Some("*array*"),
                ..Default::default()
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::TagManager;

    fn seed_tags(state: &mut MainCompleteState, tag: &str) {
        state.tags = TagManager::new();
        state.tags.init(&[tag.to_string()]);
        state.tags.add_try(&[tag.to_string()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_only_array_typed_parameters() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("PATH_ARR".into(), "array".into());
        params.insert("PATH_STR".into(), "scalar".into());
        params.insert("READONLY_ARR".into(), "readonly-array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"PATH_ARR"));
        assert!(names.contains(&"READONLY_ARR"));
        assert!(!names.contains(&"PATH_STR"));
    }

    #[test]
    fn untagged_call_skips_emission() {
        // No tag seeded → `_wanted` gates everything off.
        let mut state = MainCompleteState::new("", 0);
        let mut params = HashMap::new();
        params.insert("ARR".into(), "array".into());
        assert!(!_arrays(&mut state, &params));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        assert!(!_arrays(&mut state, &HashMap::new()));
    }

    #[test]
    fn no_array_typed_params_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("X".into(), "scalar".into());
        params.insert("Y".into(), "integer".into());
        assert!(!_arrays(&mut state, &params));
    }

    #[test]
    fn associations_excluded_unless_named_array() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("MAP".into(), "association".into());
        params.insert("ARR".into(), "array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"ARR"));
        assert!(!names.contains(&"MAP"));
    }

    #[test]
    fn description_attached_to_arrays_group() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("ARR".into(), "array".into());
        let _ = _arrays(&mut state, &params);
        let grp = state.comp.groups.iter().find(|g| g.name == "arrays").unwrap();
        assert!(grp.explanations.iter().any(|e| e == "array"));
    }

    #[test]
    fn prefix_filter_combines_with_type_filter() {
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        state.comp.params.prefix = "P".into();
        let mut params = HashMap::new();
        params.insert("PATH_ARR".into(), "array".into());
        params.insert("ZSH_ARR".into(), "array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"PATH_ARR"));
        assert!(!names.contains(&"ZSH_ARR"));
    }
}
