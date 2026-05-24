//! Port of `_complete_help` — show completion help.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_complete_help`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete_help`).
//!
//! Upstream shell source — the real `_complete_help` is a complex
//! `complete-word` widget that introspects compsys internals to
//! show *what zstyle would do* for the current word. Key lines:
//! ```text
//!  3  _complete_help() {
//!  4    eval "$_comp_setup"
//!  6    local _sort_tags=_help_sort_tags text i j k tmp
//!  7    typeset -A help_funcs help_tags help_sfuncs help_styles
//!  9    local -H _help_scan_funcstack="main_complete|complete|…"
//! 12    {
//! 13      compadd() { return 1 }
//! 14      compcall() { _help_sort_tags use-compctl }
//! 15      zstyle() { …shadow zstyle to RECORD lookups… }
//! ```
//!
//! Simplified Rust port: takes pre-collected `(topic, description)`
//! pairs from the caller and emits each with `topic -- desc` disp
//! formatting. This covers the user-visible "show me the help
//! entries" mode without re-implementing zsh's compsys introspection
//! plumbing (which would require shadowing zstyle/compadd globally).

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _complete_help - Show completion help
pub fn _complete_help(state: &mut CompletionState, help_entries: &[(String, String)]) -> bool {
    state.begin_group("help", true);

    for (topic, desc) in help_entries {
        let mut comp = Completion::new(topic);
        comp.disp = Some(format!("{} -- {}", topic, desc));
        state.add_match(comp, Some("help"));
    }

    state.end_group();
    !help_entries.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_topic_with_topic_dash_desc_disp() {
        let mut state = CompletionState::new();
        let entries = vec![
            ("foo".into(), "the foo cmd".into()),
            ("bar".into(), "the bar cmd".into()),
        ];
        assert!(_complete_help(&mut state, &entries));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str["foo"], "foo -- the foo cmd");
        assert_eq!(by_str["bar"], "bar -- the bar cmd");
    }

    #[test]
    fn empty_entries_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_complete_help(&mut state, &[]));
    }
}
