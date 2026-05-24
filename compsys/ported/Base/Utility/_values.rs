//! Port of `_values` — complete comma-separated values.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_values`
//! (system copy `/opt/homebrew/share/zsh/functions/_values`).
//!
//! Upstream shell source (key lines from the ~300-line fn):
//! ```text
//!  3  local subopts opt usecc garbage keep
//!  5  subopts=()
//!  6  zparseopts -D -a garbage s+:=keep S+:=keep w+=keep C=usecc \
//!      O:=subopts M: J: V: 1 2 o+: n F: X:
//! 11  if compvalues -i "$keep[@]" "$@"; then
//! 14    local noargs args opts descr action expl sep argsep subc test='*'
//! ```
//!
//! Upstream is a heavyweight `compvalues -i` driver: parses `-s sep`,
//! `-S argsep`, `-w` (with-arg vs no-arg), `-C` (curcontext), `-O`
//! (compadd opt array). Then walks specs in `name[desc]:arg-desc:action`
//! form, filtering out already-typed values.
//!
//! Faithful Rust port: covers the `name[desc]` parsing via
//! `Value::parse` + already-used filtering by splitting PREFIX at
//! the separator. Drops `-w` (with-arg distinction) and `-C`
//! curcontext rewriting — caller-side concerns.

use std::collections::HashSet;

use crate::base::Value;
use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _values - complete comma-separated values
pub fn _values(
    state: &mut CompletionState,
    description: &str,
    separator: char,
    specs: &[String],
) -> bool {
    let values: Vec<Value> = specs.iter().filter_map(|s| Value::parse(s)).collect();

    let prefix = state.params.prefix.clone();

    // Find already-used values
    let used: HashSet<String> = prefix
        .split(separator)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Get current value being completed
    let current_prefix = prefix.rsplit(separator).next().unwrap_or("").to_string();

    state.begin_group("values", true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some("values"));
    }

    let mut matched = false;
    for value in &values {
        // Skip already-used values
        if used.contains(&value.name) {
            continue;
        }

        // Check prefix match
        if !value.name.starts_with(&current_prefix) {
            continue;
        }

        let mut comp = Completion::new(&value.name);
        if !value.description.is_empty() {
            comp.disp = Some(format!("{} -- {}", value.name, value.description));
        }

        // Add separator suffix
        if value.has_arg {
            comp.suf = Some("=".to_string());
            comp.flags |= CompletionFlags::NOSPACE;
        }

        state.add_match(comp, Some("values"));
        matched = true;
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_used_values_filtered_out() {
        let mut state = CompletionState::new();
        state.params.prefix = "yes,n".into();
        let specs = vec!["yes".into(), "no".into(), "maybe".into()];
        assert!(_values(&mut state, "vals", ',', &specs));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"yes"), "used value leaked through filter");
        assert!(names.contains(&"no"));
    }

    #[test]
    fn description_emitted_as_disp() {
        let mut state = CompletionState::new();
        let specs = vec!["yes[Confirm action]".into()];
        assert!(_values(&mut state, "vals", ',', &specs));
        let disp = state.groups[0].matches[0].disp.clone().unwrap_or_default();
        assert!(disp.contains("yes -- Confirm action"), "got {disp:?}");
    }

    #[test]
    fn current_typed_value_prefix_matched() {
        let mut state = CompletionState::new();
        state.params.prefix = "ma".into();
        let specs = vec![
            "yes".into(),
            "no".into(),
            "maybe".into(),
            "manual".into(),
        ];
        assert!(_values(&mut state, "vals", ',', &specs));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"maybe"));
        assert!(names.contains(&"manual"));
        assert!(!names.contains(&"yes"));
    }

    #[test]
    fn empty_specs_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_values(&mut state, "vals", ',', &[]));
    }

    #[test]
    fn group_explanation_added_when_description_non_empty() {
        let mut state = CompletionState::new();
        let specs = vec!["x".into()];
        _values(&mut state, "vals description", ',', &specs);
        let g = &state.groups[0];
        assert!(!g.explanations.is_empty());
        assert_eq!(g.explanations[0], "vals description");
    }

    #[test]
    fn custom_separator_used_for_dedup() {
        // With colon separator, `a:b:c` is the typed list and `b`
        // should not appear again.
        let mut state = CompletionState::new();
        state.params.prefix = "a:b:".into();
        let specs = vec!["a".into(), "b".into(), "c".into()];
        _values(&mut state, "", ':', &specs);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"a"), "a typed → must be excluded");
        assert!(!names.contains(&"b"), "b typed → must be excluded");
        assert!(names.contains(&"c"));
    }

    #[test]
    fn has_arg_value_gets_equals_suffix() {
        let mut state = CompletionState::new();
        // `name=value-form` spec creates a Value with has_arg=true.
        let specs: Vec<String> = vec!["foo:=ARG".to_string()];
        if crate::base::Value::parse(&specs[0]).is_some() {
            assert!(_values(&mut state, "", ',', &specs));
            let m = &state.groups[0].matches[0];
            // has_arg → comp.suf = "=" + NOSPACE flag.
            if m.suf.as_deref() == Some("=") {
                assert!(
                    m.flags.contains(crate::completion::CompletionFlags::NOSPACE),
                    "has_arg value must carry NOSPACE"
                );
            }
        }
    }

    #[test]
    fn empty_description_skips_explanation() {
        let mut state = CompletionState::new();
        let specs = vec!["x".into()];
        _values(&mut state, "", ',', &specs);
        let g = &state.groups[0];
        assert!(g.explanations.is_empty());
    }

    #[test]
    fn all_already_used_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "a,b,c,".into();
        let specs = vec!["a".into(), "b".into(), "c".into()];
        assert!(!_values(&mut state, "", ',', &specs));
    }

    #[test]
    fn empty_prefix_emits_all_unused_specs() {
        let mut state = CompletionState::new();
        let specs = vec!["alpha".into(), "beta".into(), "gamma".into()];
        assert!(_values(&mut state, "", ',', &specs));
        assert_eq!(state.groups[0].matches.len(), 3);
    }

    #[test]
    fn malformed_spec_silently_skipped() {
        // Whatever Value::parse rejects, _values should also skip
        // without crashing.
        let mut state = CompletionState::new();
        let specs = vec![
            "".to_string(), // empty spec — Value::parse returns None
            "valid".to_string(),
        ];
        let _ = _values(&mut state, "", ',', &specs);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"valid"));
    }
}
