//! Port of `_regex_words` — complete words matching regex. Moved from
//! `compsys/functions.rs`.

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
