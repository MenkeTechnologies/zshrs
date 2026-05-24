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
//! Strict Rust port: walks the patterns at completion-time (skipping
//! the compile step), first matching pattern wins. Emits the
//! description as the group explanation AND dispatches the action
//! via `_call_function` so registered Rust callbacks (or shell-fn
//! shims) actually fire when a pattern hits.

use crate::base::MainCompleteState;
use crate::ported::_call_function::_call_function;

/// _regex_arguments - Complete using regex-based argument specs.
///
/// `name` is forwarded as the generated-fn name (matches upstream's
/// `$1`); not used for dispatch at this layer but kept in the
/// signature for parity with the shell call site. `patterns` is a
/// slice of `(regex, description, action_fn_name)` triples.
pub fn _regex_arguments(
    state: &mut MainCompleteState,
    _name: &str,
    patterns: &[(String, String, String)],
) -> bool {
    let current = state.comp.params.current_word();

    for (pattern, desc, action) in patterns {
        let Ok(re) = regex_lite::Regex::new(pattern) else {
            continue;
        };
        if !re.is_match(&current) {
            continue;
        }
        state.comp.begin_group("regex", true);
        state.comp.add_explanation(desc.clone(), Some("regex"));
        state.comp.end_group();
        // Dispatch the registered action. The shell version `eval`s
        // the action string; we resolve `action` as a fn name and
        // dispatch through `_call_function`. Empty action = no-op.
        if !action.is_empty() {
            let _ = _call_function(state, action);
        }
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_matching_pattern_wins() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc123".into();
        let patterns = vec![
            ("^xyz".into(), "no match".into(), "".into()),
            ("^abc".into(), "matches abc-prefix".into(), "".into()),
            ("^.*".into(), "matches anything".into(), "".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        let exps: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        assert!(
            exps.contains(&"matches abc-prefix".to_string()),
            "got {exps:?}"
        );
    }

    #[test]
    fn no_pattern_matches_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc".into();
        let patterns = vec![("^xyz".into(), "no".into(), "".into())];
        assert!(!_regex_arguments(&mut state, "x", &patterns));
    }

    #[test]
    fn invalid_regex_pattern_skipped() {
        let mut state = MainCompleteState::new("", 0);
        let patterns = vec![
            ("[bad-regex".into(), "bad".into(), "".into()),
            (".*".into(), "good".into(), "".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
    }

    #[test]
    fn empty_patterns_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "anything".into();
        assert!(!_regex_arguments(&mut state, "x", &[]));
    }

    #[test]
    fn anchored_end_regex_works() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc.txt".into();
        let patterns = vec![
            ("\\.txt$".into(), "text file".into(), "".into()),
            (".*".into(), "fallback".into(), "".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        let exps: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        assert!(exps.contains(&"text file".to_string()), "got {exps:?}");
    }

    #[test]
    fn group_name_is_regex_for_matched_dispatch() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "x".into();
        let patterns = vec![(".".into(), "explanation".into(), "act".into())];
        assert!(_regex_arguments(&mut state, "fnname", &patterns));
        assert!(state.comp.groups.iter().any(|g| g.name == "regex"));
    }

    #[test]
    fn current_word_not_prefix_drives_matching() {
        let mut state = MainCompleteState::new("", 0);
        // Empty words → current_word() is empty.
        let patterns = vec![
            ("^x$".into(), "x-only".into(), "".into()),
            ("^$".into(), "empty".into(), "".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        let exps: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        assert!(
            exps.contains(&"empty".to_string()),
            "empty current_word should match `^$`; got {exps:?}"
        );
    }

    #[test]
    fn registered_action_fires_for_matching_pattern() {
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rxargs_action",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "go".into();
        let patterns = vec![("^go".into(), "go-prefix".into(), "_rxargs_action".into())];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        unregister("_rxargs_action");
        assert_eq!(
            FIRED.load(Ordering::SeqCst),
            1,
            "registered action must fire exactly once on match"
        );
    }

    #[test]
    fn action_does_not_fire_when_no_pattern_matches() {
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rxargs_skip",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc".into();
        let patterns = vec![("^XX$".into(), "no".into(), "_rxargs_skip".into())];
        assert!(!_regex_arguments(&mut state, "x", &patterns));
        unregister("_rxargs_skip");
        assert_eq!(FIRED.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unregistered_action_name_does_not_panic() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "x".into();
        let patterns = vec![(".".into(), "desc".into(), "not-registered".into())];
        // Should NOT panic — `_call_function` returns false for
        // unknown names.
        assert!(_regex_arguments(&mut state, "x", &patterns));
    }

    #[test]
    fn second_matching_action_does_not_fire() {
        // First-match-wins — the second pattern's action MUST NOT
        // fire even though its regex would also match.
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        FIRED.store(false, Ordering::SeqCst);
        register(
            "_rxargs_second",
            Box::new(|_| {
                FIRED.store(true, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "abc".into();
        let patterns = vec![
            ("^abc".into(), "first".into(), "".into()),
            (".*".into(), "second".into(), "_rxargs_second".into()),
        ];
        assert!(_regex_arguments(&mut state, "x", &patterns));
        unregister("_rxargs_second");
        assert!(!FIRED.load(Ordering::SeqCst));
    }
}
