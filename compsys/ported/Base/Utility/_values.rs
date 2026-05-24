//! Port of `_values` — complete comma-separated values.
//!
//! Extracted from `compsys/base.rs` (was lines ~510-564). Mirrors zsh
//! upstream `Completion/Base/Utility/_values`. Specs are parsed in the
//! `name[description]:arg-desc:action` form via
//! `crate::base::Value::parse`; already-used values (split on the
//! supplied separator) are filtered out.

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
}
