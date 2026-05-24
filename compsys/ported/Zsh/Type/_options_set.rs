//! Port of `_options_set` — complete currently set options.
//!
//! Local shell reference: `compsys/functions/Zsh/Type/_options_set`
//! (system copy `/opt/homebrew/share/zsh/functions/_options_set`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local list expl
//!  5  list=( ${(@k)options[(R)on]} )
//!  7  _wanted options expl 'set option' compadd "$@" -a list
//! ```
//!
//! Upstream uses `${(@k)options[(R)on]}` to get the keys of the
//! shell `$options` array whose VALUE is "on" (i.e., set).
//!
//! Faithful Rust port: filters the caller's `&[(name, is_set)]`
//! slice to set-only entries and delegates to `_options` for the
//! actual emit (so disp formatting stays consistent).

use crate::compcore::CompletionState;

use super::_options::_options;

/// _options_set - Complete currently set options
pub fn _options_set(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let set_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| *is_set)
        .copied()
        .collect();
    _options(state, &set_opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_to_set_only() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("NULL_GLOB", false),
            ("PIPE_FAIL", true),
            ("NO_BANG_HIST", false),
        ];
        let ok = _options_set(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
        assert!(names.contains(&"PIPE_FAIL"));
        assert!(!names.contains(&"NULL_GLOB"), "unset option leaked through filter");
        assert!(!names.contains(&"NO_BANG_HIST"));
    }

    #[test]
    fn returns_false_when_no_set_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", false), ("B", false)];
        assert!(!_options_set(&mut state, &opts));
    }
}
