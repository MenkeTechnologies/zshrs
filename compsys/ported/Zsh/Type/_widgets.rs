//! Port of `_widgets` (zsh Completion/Zle/_widgets, 9 lines).
//!
//! Local shell reference: upstream
//! `Completion/Zle/_widgets` (not vendored locally — system copy at
//! `/opt/homebrew/share/zsh/functions/_widgets`).
//!
//! Two semantics the original stub got wrong:
//!   1. `widgets[(R)pat]` matches against widget VALUE (the
//!      implementation kind: `builtin`, `user:_complete_help`,
//!      `completion`, `redisplay`), not against the widget NAME.
//!      Returns matching KEYS via `${(@k)…}`.
//!   2. The `-M 'r:|-=* r:|=*'` matchspec means `-` acts as a segment
//!      separator and a typed `XYZ` can match any segment. So `bs`
//!      anchors at start of word OR after a `-`, matching
//!      `backward-skip-line` etc.

use crate::compcore::CompletionState;
use crate::completion::Completion;

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
}
