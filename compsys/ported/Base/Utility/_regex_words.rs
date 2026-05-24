//! Port of `_regex_words` — complete words matching regex.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_regex_words`
//! (system copy `/opt/homebrew/share/zsh/functions/_regex_words`).
//!
//! Upstream is a 52-line wrapper that builds an `_regex_arguments`
//! invocation from a list of `word:desc:action` specs. Key lines:
//! ```text
//!  3  _regex_words() {
//!  6    local name=$1 description=$2
//!  7    shift 2
//! 14    local spec specs i j word desc action
//! 24    for spec in "$@"; do
//! 25      word=${spec%%:*}
//! 26      desc=${spec#*:}; desc=${desc%%:*}
//! 27      action=${spec#*:*:}
//! ```
//!
//! Strict Rust port: takes `(word, description, action)` triples.
//! The action is a Rust fn registered under a name (mirrors shell's
//! `action` arg position, which can be a shell expression or
//! `_action_name`). When the user selects a matching word AT
//! completion time, the registered action fires via
//! `_call_function`. Emission semantics: prefix-filter each word,
//! attach `word -- description` disp, return true iff any survived.

use crate::base::MainCompleteState;
use crate::completion::Completion;
use crate::ported::_call_function::_call_function;

/// One row of the _regex_words spec table.
#[derive(Clone, Debug)]
pub struct RegexWordsSpec {
    pub word: String,
    pub description: String,
    /// Optional registered action fn name. When non-empty, the
    /// fn is dispatched (via `_call_function`) the moment the spec
    /// is emitted as a match — mirroring upstream which generates
    /// `_regex_arguments` invocations that may immediately recurse.
    pub action: String,
}

/// _regex_words - Complete words matching regex.
pub fn _regex_words(
    state: &mut MainCompleteState,
    tag: &str,
    description: &str,
    specs: &[RegexWordsSpec],
) -> bool {
    let prefix = state.comp.params.prefix.clone();

    state.comp.begin_group(tag, true);
    if !description.is_empty() {
        state
            .comp
            .add_explanation(description.to_string(), Some(tag));
    }

    let mut matched = false;
    let mut actions_to_run: Vec<String> = Vec::new();
    for spec in specs {
        if spec.word.starts_with(&prefix) {
            let mut comp = Completion::new(&spec.word);
            if !spec.description.is_empty() {
                comp.disp = Some(format!("{} -- {}", spec.word, spec.description));
            }
            state.comp.add_match(comp, Some(tag));
            matched = true;
            if !spec.action.is_empty() {
                actions_to_run.push(spec.action.clone());
            }
        }
    }
    state.comp.end_group();

    for fnname in actions_to_run {
        let _ = _call_function(state, &fnname);
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(w: &str, d: &str, a: &str) -> RegexWordsSpec {
        RegexWordsSpec {
            word: w.into(),
            description: d.into(),
            action: a.into(),
        }
    }

    #[test]
    fn prefix_filter_and_disp_format() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "co".into();
        let specs = vec![
            spec("commit", "Create commit", ""),
            spec("push", "Push to remote", ""),
        ];
        assert!(_regex_words(&mut state, "words", "verb", &specs));
        let by_str: std::collections::HashMap<&str, &str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str.get("commit"), Some(&"commit -- Create commit"));
        assert!(!by_str.contains_key("push"));
    }

    #[test]
    fn empty_specs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_regex_words(&mut state, "words", "verb", &[]));
    }

    #[test]
    fn empty_description_emits_word_without_disp() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("up", "", "")];
        assert!(_regex_words(&mut state, "words", "verb", &specs));
        assert_eq!(state.comp.groups[0].matches[0].disp, None);
    }

    #[test]
    fn empty_tag_description_skips_explanation() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("x", "x desc", "")];
        _regex_words(&mut state, "words", "", &specs);
        let g = state.comp.groups.iter().find(|g| g.name == "words").unwrap();
        assert!(g.explanations.is_empty());
    }

    #[test]
    fn empty_prefix_emits_all_words() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![
            spec("a", "first", ""),
            spec("b", "second", ""),
            spec("c", "third", ""),
        ];
        assert!(_regex_words(&mut state, "words", "test", &specs));
        assert_eq!(state.comp.groups[0].matches.len(), 3);
    }

    #[test]
    fn tag_name_used_as_group_name() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("x", "", "")];
        _regex_words(&mut state, "my-tag", "", &specs);
        assert!(state.comp.groups.iter().any(|g| g.name == "my-tag"));
    }

    #[test]
    fn registered_action_fires_when_matching_spec_emits() {
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rwspec_act",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "do".into();
        let specs = vec![spec("do-the-thing", "the thing", "_rwspec_act")];
        let _ = _regex_words(&mut state, "tag", "verb", &specs);
        unregister("_rwspec_act");
        assert_eq!(
            FIRED.load(Ordering::SeqCst),
            1,
            "registered action should fire exactly once per matching spec"
        );
    }

    #[test]
    fn action_does_not_fire_for_unmatched_specs() {
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rwspec_skip",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "different-prefix-xyz".into();
        let specs = vec![spec("not-going-to-match", "", "_rwspec_skip")];
        let _ = _regex_words(&mut state, "tag", "", &specs);
        unregister("_rwspec_skip");
        assert_eq!(
            FIRED.load(Ordering::SeqCst),
            0,
            "non-matching spec must NOT fire its action"
        );
    }
}
