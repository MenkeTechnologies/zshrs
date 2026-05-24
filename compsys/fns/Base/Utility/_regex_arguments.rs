//! Port of `_regex_arguments` — complete using regex-based argument
//! specs. Moved from `compsys/functions.rs`.

use crate::compcore::CompletionState;

/// _regex_arguments - Complete using regex-based argument specs
pub fn _regex_arguments(
    state: &mut CompletionState,
    _name: &str,
    patterns: &[(String, String, String)], // (pattern, description, action)
) -> bool {
    let current = state.params.current_word();

    for (pattern, desc, action) in patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            if re.is_match(&current) {
                // Would execute the action
                state.begin_group("regex", true);
                state.add_explanation(desc.clone(), Some("regex"));
                state.end_group();
                let _ = action;
                return true;
            }
        }
    }

    false
}
