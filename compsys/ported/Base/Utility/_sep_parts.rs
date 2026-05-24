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

    // Walk the prefix consuming one positional separator at a time.
    // sep_chars[0] separates array[0] from array[1], sep_chars[1]
    // separates array[1] from array[2], etc.
    let mut array_idx = 0;
    let mut cursor = 0;
    while array_idx < sep_chars.len() {
        let sep = sep_chars[array_idx];
        if let Some(pos) = prefix[cursor..].find(sep) {
            cursor += pos + sep.len_utf8();
            array_idx += 1;
        } else {
            break;
        }
    }

    if array_idx >= arrays.len() {
        return false;
    }

    // The unconsumed tail of `prefix` is the user's typed text for
    // the current segment.
    let current_prefix = prefix[cursor..].to_string();

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

    #[test]
    fn next_separator_attached_as_suffix_when_more_arrays_follow() {
        // First-segment match should carry the next separator as
        // `suf` + NOSPACE so the user continues onto the second
        // segment without losing the delimiter.
        let mut state = CompletionState::new();
        let arrays = vec![
            vec!["alice".into(), "bob".into()],
            vec!["host1".into(), "host2".into()],
        ];
        assert!(_sep_parts(&mut state, "@", &arrays));
        let c = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .find(|c| c.str_ == "alice")
            .expect("alice present");
        assert_eq!(c.suf.as_deref(), Some("@"));
        assert!(c.flags.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn last_array_does_not_attach_suffix() {
        // After the final separator, completion of the last segment
        // should NOT add a separator after itself.
        let mut state = CompletionState::new();
        state.params.prefix = "alice@".into();
        let arrays = vec![
            vec!["alice".into()],
            vec!["host1".into(), "host2".into()],
        ];
        assert!(_sep_parts(&mut state, "@", &arrays));
        let c = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .find(|c| c.str_ == "host1")
            .expect("host1 present");
        assert!(c.suf.is_none(), "last segment should not have suf");
    }

    #[test]
    fn three_segments_with_colon_separator() {
        // user:host:port style — three arrays separated by colons.
        // The `separators` string has one CHAR PER ARRAY BOUNDARY:
        // two arrays need 1 char, three arrays need 2 chars.
        let mut state = CompletionState::new();
        state.params.prefix = "alice:web:".into();
        let arrays = vec![
            vec!["alice".into()],
            vec!["web".into(), "api".into()],
            vec!["80".into(), "443".into()],
        ];
        assert!(_sep_parts(&mut state, "::", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        // We're past two `:` separators → third array (ports).
        assert!(names.contains(&"80".to_string()));
        assert!(names.contains(&"443".to_string()));
    }

    #[test]
    fn no_matching_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-not".into();
        let arrays = vec![vec!["alpha".into(), "beta".into()]];
        assert!(!_sep_parts(&mut state, "/", &arrays));
    }

    #[test]
    fn prefix_past_all_arrays_returns_false() {
        // Two arrays, but the prefix already has THREE separators →
        // we'd be looking for an array at index 3 which doesn't exist.
        let mut state = CompletionState::new();
        state.params.prefix = "a/b/c/".into();
        let arrays = vec![
            vec!["a".into()],
            vec!["b".into()],
        ];
        assert!(!_sep_parts(&mut state, "/", &arrays));
    }
}
