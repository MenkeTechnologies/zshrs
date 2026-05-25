//! Port of `_widgets` from `Completion/Zsh/Type/_widgets`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl pattern
//! sh: 4
//! sh: 5  pattern=( -g \* )
//! sh: 6  zparseopts -D -K -E g:=pattern
//! sh: 7
//! sh: 8  _description widgets expl widget
//! sh: 9  compadd "$@" "$expl[@]" -M 'r:|-=* r:|=*' - "${(@k)widgets[(R)${pattern[2]}]}"
//! ```



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

use super::shared::glob_matches;

pub fn _widgets(
    state: &mut CompletionState,
    widgets: &[(String, String)],
    kind_pattern: Option<&str>,
) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("widgets", true);
    for (name, kind) in widgets {
        // Filter by KIND glob (shell `[(R)pat]` is value-matching).
        if let Some(pat) = kind_pattern {
            if !glob_matches(pat, kind) {
                continue;
            }
        }
        // Match user-typed PREFIX against either start-of-name or
        // start-of-segment-after-hyphen (mimics `r:|-=*`).
        if !prefix.is_empty()
            && !name.starts_with(&prefix)
            && !name.split('-').any(|seg| seg.starts_with(&prefix))
        {
            continue;
        }
        state.add_match(Completion::new(name.clone()), Some("widgets"));
    }
    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_kind_not_name() {
        let mut state = CompletionState::new();
        let ws = vec![
            ("backward-char".into(), "builtin".into()),
            ("_complete_help".into(), "completion".into()),
            ("my-widget".into(), "user:my_widget".into()),
        ];
        _widgets(&mut state, &ws, Some("user:*"));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["my-widget"]);
    }

    #[test]
    fn matchspec_matches_after_hyphen() {
        let mut state = CompletionState::new();
        state.params.prefix = "ski".into();
        let ws = vec![
            ("backward-skip-line".into(), "builtin".into()),
            ("forward-word".into(), "builtin".into()),
        ];
        _widgets(&mut state, &ws, None);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["backward-skip-line"]);
    }

    #[test]
    fn empty_prefix_emits_all_widgets() {
        let mut state = CompletionState::new();
        let ws = vec![
            ("forward-word".into(), "builtin".into()),
            ("backward-word".into(), "builtin".into()),
        ];
        _widgets(&mut state, &ws, None);
        assert_eq!(state.nmatches, 2);
    }

    #[test]
    fn empty_widget_list_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_widgets(&mut state, &[], None));
    }

    #[test]
    fn kind_glob_supports_star_wildcard() {
        let mut state = CompletionState::new();
        let ws = vec![
            ("forward-word".into(), "builtin".into()),
            ("_complete_help".into(), "completion".into()),
            ("my-widget".into(), "user:_complete".into()),
            ("redraw".into(), "redisplay".into()),
        ];
        // `*letion*` matches "completion".
        _widgets(&mut state, &ws, Some("*letion*"));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"_complete_help"));
        assert!(!names.contains(&"forward-word"));
        assert!(!names.contains(&"redraw"));
    }

    #[test]
    fn prefix_matches_start_of_name() {
        let mut state = CompletionState::new();
        state.params.prefix = "back".into();
        let ws = vec![
            ("backward-word".into(), "builtin".into()),
            ("forward-word".into(), "builtin".into()),
        ];
        _widgets(&mut state, &ws, None);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["backward-word"]);
    }

    #[test]
    fn middle_segment_after_hyphen_matches() {
        // `cha` should match `backward-char` (segment after first `-`).
        let mut state = CompletionState::new();
        state.params.prefix = "cha".into();
        let ws = vec![
            ("backward-char".into(), "builtin".into()),
            ("forward-word".into(), "builtin".into()),
            ("nothing".into(), "builtin".into()),
        ];
        _widgets(&mut state, &ws, None);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"backward-char"));
        assert!(!names.contains(&"forward-word"));
        assert!(!names.contains(&"nothing"));
    }

    #[test]
    fn underscore_prefixed_completion_widget_passes() {
        // _complete_help has kind `completion`; prefix _com matches.
        let mut state = CompletionState::new();
        state.params.prefix = "_com".into();
        let ws = vec![
            ("_complete_help".into(), "completion".into()),
            ("user-fn".into(), "user:fn".into()),
        ];
        _widgets(&mut state, &ws, None);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"_complete_help"));
    }

    #[test]
    fn no_matching_kind_returns_false() {
        let mut state = CompletionState::new();
        let ws = vec![("forward-word".into(), "builtin".into())];
        // Kind `nonexistent` matches nothing.
        assert!(!_widgets(&mut state, &ws, Some("nonexistent")));
    }

    #[test]
    fn kind_filter_combined_with_name_prefix() {
        let mut state = CompletionState::new();
        state.params.prefix = "back".into();
        let ws = vec![
            ("backward-word".into(), "builtin".into()),
            ("backward-user".into(), "user:x".into()),
            ("forward-word".into(), "builtin".into()),
        ];
        _widgets(&mut state, &ws, Some("builtin"));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["backward-word"]);
    }

    #[test]
    fn duplicates_in_input_preserved_in_output() {
        // _widgets doesn't dedup — pass-through. (compsys's compadd
        // table is what dedups in the full pipeline.)
        let mut state = CompletionState::new();
        let ws = vec![
            ("repeat".into(), "builtin".into()),
            ("repeat".into(), "builtin".into()),
        ];
        let _ = _widgets(&mut state, &ws, None);
        let count = state.groups[0]
            .matches
            .iter()
            .filter(|c| c.str_ == "repeat")
            .count();
        // _widgets emits whatever was given; dedup is upstream.
        assert_eq!(count, 2);
    }
}
