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
