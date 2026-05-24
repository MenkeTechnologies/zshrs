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
//! Simplified Rust port: takes a `(word, description)` slice
//! directly (action elided — the caller wires that) and emits each
//! word prefix-filtered with `word -- description` disp formatting.
//! Skips the regex-construction step that the shell version uses
//! to build the underlying `_regex_arguments` invocation.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _regex_words - Complete words matching regex
pub fn _regex_words(
    state: &mut CompletionState,
    tag: &str,
    description: &str,
    specs: &[(String, String)], // (word, description)
) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    let mut matched = false;
    for (word, desc) in specs {
        if word.starts_with(&prefix) {
            let mut comp = Completion::new(word);
            if !desc.is_empty() {
                comp.disp = Some(format!("{} -- {}", word, desc));
            }
            state.add_match(comp, Some(tag));
            matched = true;
        }
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_filter_and_disp_format() {
        let mut state = CompletionState::new();
        state.params.prefix = "co".into();
        let specs = vec![
            ("commit".into(), "Create commit".into()),
            ("push".into(), "Push to remote".into()),
        ];
        assert!(_regex_words(&mut state, "words", "verb", &specs));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str.get("commit"), Some(&"commit -- Create commit"));
        assert!(!by_str.contains_key("push"));
    }
}
