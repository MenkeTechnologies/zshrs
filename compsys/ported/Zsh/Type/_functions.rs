//! Port of `_functions` — complete shell function names.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_functions`.
//!
//! Upstream shell source (7 lines):
//! ```text
//!  3  local expl ffilt
//!  5  zstyle -t ":completion:${curcontext}:functions" prefix-needed && \
//!  6   [[ $PREFIX != [_.]* ]] && \
//!  7   ffilt='[(I)[^_.]*]'
//!  9  _wanted functions expl 'shell function' compadd -k "$@" - "functions$ffilt"
//! ```
//!
//! `prefix-needed` semantics: when truthy AND user hasn't typed
//! something starting with `_` or `.`, hide names that DO start
//! with `_` or `.` (i.e. only show "public" functions).
//!
//! Strict Rust port: caller injects the function name list (since
//! compsys can't see the parent's `$functions` assoc).

use crate::base::MainCompleteState;
use crate::completion::Completion;

/// `_functions` — complete function names with optional
/// `prefix-needed` filter on the `functions` tag.
pub fn _functions(state: &mut MainCompleteState, function_names: &[String]) -> bool {
    // shell:5-7 — `prefix-needed` zstyle gate (must precede the
    // _wanted call because it builds `ffilt`, then the `_wanted`
    // call uses it).
    let prefix = state.comp.params.prefix.clone();
    let ctx = format!(":completion:{}:functions", state.ctx.context);
    let prefix_needed = state
        .styles
        .lookup_values(&ctx, "prefix-needed")
        .and_then(|v| v.first().cloned())
        .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
        .unwrap_or(false);
    let hide_internal =
        prefix_needed && !(prefix.starts_with('_') || prefix.starts_with('.'));

    // shell:9 — `_wanted functions expl 'shell function' compadd …`
    crate::ported::_wanted::_wanted(state, "functions", "shell function", |s| {
        let mut any = false;
        for name in function_names {
            if hide_internal && (name.starts_with('_') || name.starts_with('.')) {
                continue;
            }
            if !name.starts_with(&prefix) {
                continue;
            }
            s.add_match(Completion::new(name.clone()), Some("functions"));
            any = true;
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["functions".into()]);
        state.tags.add_try(&["functions".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_all_function_names_with_no_prefix_needed() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let fns = vec!["foo".to_string(), "_bar".to_string(), ".baz".to_string()];
        assert!(_functions(&mut state, &fns));
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn prefix_needed_hides_underscore_and_dot_functions() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.ctx.context = ":t:".into();
        state.styles.set(
            ":completion::t::functions",
            "prefix-needed",
            vec!["true".into()],
            false,
        );
        let fns = vec![
            "foo".to_string(),
            "_bar".to_string(),
            ".baz".to_string(),
            "publicfn".to_string(),
        ];
        let _ = _functions(&mut state, &fns);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"publicfn"));
        assert!(!names.contains(&"_bar"));
        assert!(!names.contains(&".baz"));
    }

    #[test]
    fn typed_underscore_prefix_bypasses_filter() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "_".into();
        state.ctx.context = ":t:".into();
        state.styles.set(
            ":completion::t::functions",
            "prefix-needed",
            vec!["true".into()],
            false,
        );
        let fns = vec!["_bar".to_string(), "foo".to_string()];
        let _ = _functions(&mut state, &fns);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // User typed `_` → filter disabled, _bar shows.
        assert!(names.contains(&"_bar"));
        assert!(!names.contains(&"foo"), "foo doesn't start with _");
    }

    #[test]
    fn typed_dot_prefix_bypasses_filter() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = ".".into();
        state.ctx.context = ":t:".into();
        state.styles.set(
            ":completion::t::functions",
            "prefix-needed",
            vec!["true".into()],
            false,
        );
        let fns = vec![".baz".to_string(), "foo".to_string()];
        let _ = _functions(&mut state, &fns);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&".baz"));
    }

    #[test]
    fn untagged_call_returns_false_via_wanted_gate() {
        // No tag seeded → _wanted gates everything off.
        let mut state = MainCompleteState::new("", 0);
        let fns = vec!["foo".to_string()];
        assert!(!_functions(&mut state, &fns));
    }

    #[test]
    fn empty_function_list_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        assert!(!_functions(&mut state, &[]));
    }
}
