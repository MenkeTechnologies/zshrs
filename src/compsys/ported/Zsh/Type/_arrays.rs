//! Port of `_arrays` from `Completion/Zsh/Type/_arrays`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef shift
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted arrays expl array _parameters "$@" -g '*array*'
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted <tag> <expl_arr>
//! <descr> <cmd>` invocation by calling our ported [`_wanted`] with
//! `tag = "arrays"` and `descr = "array"`. Inner `_parameters -g
//! '*array*'` is [`_parameters_with_opts`] with `pattern: "*array*"`.
//!
//! The shell local `expl` is the description-array name threaded into
//! `_wanted`'s machinery; in the Rust port that role is implicit —
//! `_wanted`'s third arg carries the description. Documented at
//! `// sh:3` for traceability.
//!
//! Signature divergence (`// rust:`): shell `_arrays` reads `$parameters`
//! (a zsh special parameter mapping name → type) via the `_parameters`
//! callee. The Rust leaf can't reach `$parameters` directly, so the
//! caller passes a `HashMap<String, String>` snapshot.

use std::collections::HashMap;

use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_parameters::{ParametersOpts, _parameters_with_opts};
use crate::compsys::ported::_wanted::_wanted;

/// `_arrays` — complete array-typed parameter names.
// rust: shell takes no args (uses globals); Rust takes state +
// $parameters snapshot.
pub fn _arrays(state: &mut MainCompleteState, params: &HashMap<String, String>) -> bool {
    // sh:3  local expl    — description-array name; managed internally
    //                       by _wanted (no Rust-side local needed)
    let _expl: () = ();

    // sh:5  _wanted arrays expl array _parameters "$@" -g '*array*'
    _wanted(state, "arrays", "array", |s| {
        // sh:5  _parameters "$@" -g '*array*'   — type-pattern filter
        // (shell `"$@"` is empty in practice — _arrays is only bound
        // to `shift` and `shift` takes no per-call opts)
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
    use crate::compsys::base::TagManager;

    fn seed_tags(state: &mut MainCompleteState, tag: &str) {
        state.tags = TagManager::new();
        state.tags.init(&[tag.to_string()]);
        state.tags.add_try(&[tag.to_string()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_only_array_typed_parameters() {
        // sh:5 — `_parameters -g '*array*'` filters by zsh type
        // pattern: any type whose string contains "array" matches
        // (so plain "array" AND "readonly-array" both pass; "scalar"
        // doesn't).
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("PATH_ARR".into(), "array".into());
        params.insert("PATH_STR".into(), "scalar".into());
        params.insert("READONLY_ARR".into(), "readonly-array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state.comp.groups.iter()
            .flat_map(|g| g.matches.iter()).map(|c| c.str_.as_str()).collect();
        assert!(names.contains(&"PATH_ARR"));
        assert!(names.contains(&"READONLY_ARR"));
        assert!(!names.contains(&"PATH_STR"));
    }

    #[test]
    fn untagged_call_skips_emission() {
        // sh:5 — `_wanted arrays ...` short-circuits when `arrays`
        // isn't in the requested tag set.
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
        // sh:5 — when nothing matches `*array*` the inner _parameters
        // returns 1, _wanted propagates → _arrays returns false.
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("X".into(), "scalar".into());
        params.insert("Y".into(), "integer".into());
        assert!(!_arrays(&mut state, &params));
    }

    #[test]
    fn associations_excluded_unless_named_array() {
        // sh:5 — `*array*` pattern: "association" doesn't contain "array",
        // so associations are excluded. Pin that explicitly.
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        let mut params = HashMap::new();
        params.insert("MAP".into(), "association".into());
        params.insert("ARR".into(), "array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state.comp.groups.iter()
            .flat_map(|g| g.matches.iter()).map(|c| c.str_.as_str()).collect();
        assert!(names.contains(&"ARR"));
        assert!(!names.contains(&"MAP"));
    }

    #[test]
    fn description_attached_to_arrays_group() {
        // sh:5 — `_wanted arrays expl array ...` attaches "array" as
        // the explanation on the `arrays`-tagged group.
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
        // sh:5 — when the user has typed a prefix, _parameters
        // honours it ON TOP of the `-g '*array*'` type filter.
        let mut state = MainCompleteState::new("", 0);
        seed_tags(&mut state, "arrays");
        state.comp.params.prefix = "P".into();
        let mut params = HashMap::new();
        params.insert("PATH_ARR".into(), "array".into());
        params.insert("ZSH_ARR".into(), "array".into());
        let _ = _arrays(&mut state, &params);
        let names: Vec<&str> = state.comp.groups.iter()
            .flat_map(|g| g.matches.iter()).map(|c| c.str_.as_str()).collect();
        assert!(names.contains(&"PATH_ARR"));
        assert!(!names.contains(&"ZSH_ARR"));
    }
}
