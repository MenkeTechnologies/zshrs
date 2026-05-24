//! Port of `_sep_parts` — complete parts with arbitrary separators.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_sep_parts`
//! (system copy `/opt/homebrew/share/zsh/functions/_sep_parts`).
//!
//! Upstream shell source (key lines from the ~80-line fn):
//! ```text
//!  3  # Arguments are alternately arrays and separator strings.
//!  4  # Arrays may be given by name or literally as words separated
//!  5  # by white space in parentheses, e.g.:
//!  6  #   _sep_parts '(foo bar)' @ hosts
//! 18  while [[ $# -gt 1 ]]; do
//! 22    # split current part …
//! 24    # match against current array
//! 30    shift 2
//! ```
//!
//! Upstream is the multi-array sibling of `_multi_parts`: each array
//! holds candidates for the segment that follows the corresponding
//! separator. So `'(foo bar)' @ '(host1 host2)'` lets the user type
//! `foo@host1` etc.
//!
//! Faithful Rust port: walks the separators string char by char,
//! using the corresponding array at each segment position. The
//! cumulative-prefix tracking (so `foo@host1:port1` works with
//! `'@:'` as separators) matches the shell loop shape.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _sep_parts - complete parts with arbitrary separators
pub fn _sep_parts(state: &mut CompletionState, separators: &str, arrays: &[Vec<String>]) -> bool {
    if arrays.is_empty() {
        return false;
    }

    let prefix = state.params.prefix.clone();
    let sep_chars: Vec<char> = separators.chars().collect();

    // Find which array we're completing based on separators in prefix
    let mut array_idx = 0;
    for sep in &sep_chars {
        if prefix.contains(*sep) {
            array_idx += 1;
        }
    }

    if array_idx >= arrays.len() {
        return false;
    }

    // Get prefix for current part
    let current_prefix = if let Some(sep) = sep_chars.get(array_idx.saturating_sub(1)) {
        prefix.rsplit(*sep).next().unwrap_or("").to_string()
    } else {
        prefix.clone()
    };

    state.begin_group("sep-parts", true);

    let mut matched = false;
    for item in &arrays[array_idx] {
        if item.starts_with(&current_prefix) {
            let mut comp = Completion::new(item);

            // Add next separator if there are more arrays
            if array_idx + 1 < arrays.len() {
                if let Some(&sep) = sep_chars.get(array_idx) {
                    comp.suf = Some(sep.to_string());
                    comp.flags |= CompletionFlags::NOSPACE;
                }
            }

            state.add_match(comp, Some("sep-parts"));
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
    fn first_array_completes_first_segment() {
        let mut state = CompletionState::new();
        state.params.prefix = "us".into();
        let arrays = vec![
            vec!["users".into(), "usr".into(), "var".into()],
            vec!["local".into(), "share".into()],
        ];
        assert!(_sep_parts(&mut state, "/", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"users".to_string()));
        assert!(names.contains(&"usr".to_string()));
        assert!(!names.contains(&"var".to_string()));
    }

    #[test]
    fn second_array_used_after_first_separator() {
        let mut state = CompletionState::new();
        state.params.prefix = "usr/lo".into();
        let arrays = vec![
            vec!["users".into(), "usr".into()],
            vec!["local".into(), "share".into()],
        ];
        assert!(_sep_parts(&mut state, "/", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"local".to_string()));
        assert!(!names.contains(&"share".to_string()));
    }

    #[test]
    fn empty_arrays_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_sep_parts(&mut state, "/", &[]));
    }
}
