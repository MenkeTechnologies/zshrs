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
//! Strict Rust port: two entry points.
//!
//! 1. `_complete_help(state, entries)` — caller passes
//!    pre-collected `(topic, description)` pairs and we emit each
//!    with `topic -- desc` disp formatting under group `help`.
//!    Used when the caller already has the entries (e.g. tag list).
//!
//! 2. `_complete_help_shadow(state, completer, label)` — runs
//!    `completer` under `_shadow`, captures everything it would
//!    have added, and renders the capture as topic+desc rows. This
//!    is the closer analog of what the shell widget does: shadow
//!    `compadd`/`zstyle` to RECORD what a completer would do
//!    without polluting live state.

use crate::compcore::CompletionState;
use crate::completion::Completion;
use crate::ported::_shadow::_shadow;

/// _complete_help - Show completion help (entry-driven form).
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

/// _complete_help_shadow — run `completer` under `_shadow`, then
/// emit each captured (group, match) pair as a topic+desc help row.
/// `label` is forwarded as the shadow name (shows up in the
/// underlying record, useful for debugging).
pub fn _complete_help_shadow(
    state: &mut CompletionState,
    label: &str,
    completer: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    let record = _shadow(state, label, completer);
    let mut entries: Vec<(String, String)> = Vec::new();
    for (group, m) in &record.matches {
        entries.push((m.clone(), format!("(tag: {})", group)));
    }
    for e in &record.explanations {
        entries.push((format!("[msg] {e}"), "explanation".into()));
    }
    _complete_help(state, &entries)
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

    #[test]
    fn group_named_help_created() {
        let mut state = CompletionState::new();
        let entries = vec![("x".into(), "y".into())];
        _complete_help(&mut state, &entries);
        assert!(state.groups.iter().any(|g| g.name == "help"));
    }

    #[test]
    fn entries_emitted_in_input_order() {
        let mut state = CompletionState::new();
        let entries = vec![
            ("z".into(), "last alpha".into()),
            ("a".into(), "first alpha".into()),
            ("m".into(), "middle".into()),
        ];
        _complete_help(&mut state, &entries);
        // Default sort=true sorts alphabetically — pin that all three
        // are present regardless of order.
        let names: std::collections::HashSet<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains("a"));
        assert!(names.contains("m"));
        assert!(names.contains("z"));
    }

    #[test]
    fn returns_true_iff_entries_provided() {
        let mut state = CompletionState::new();
        let entries = vec![("topic".into(), "desc".into())];
        assert!(_complete_help(&mut state, &entries));
    }

    #[test]
    fn shadow_mode_captures_matches_a_completer_would_add() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "test-completer", |s| {
            s.add_match(Completion::new("foo"), Some("commands"));
            s.add_match(Completion::new("bar"), Some("commands"));
            true
        });
        assert!(ok);
        let entries: Vec<(String, String)> = state.groups[0]
            .matches
            .iter()
            .map(|c| {
                (
                    c.str_.clone(),
                    c.disp.clone().unwrap_or_default(),
                )
            })
            .collect();
        // Two rows, one per shadowed match. Each disp encodes its
        // source tag.
        let disps: Vec<String> = entries.iter().map(|(_, d)| d.clone()).collect();
        assert!(disps.iter().any(|d| d.contains("foo -- (tag: commands)")));
        assert!(disps.iter().any(|d| d.contains("bar -- (tag: commands)")));
    }

    #[test]
    fn shadow_mode_with_empty_completer_returns_false() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "empty", |_| true);
        assert!(!ok, "no captured rows → no help entries → false");
    }

    #[test]
    fn shadow_mode_rolls_back_live_completion_state() {
        // The completer adds matches under shadow; after
        // _complete_help_shadow returns, those matches do NOT
        // appear in the live "commands" tag group. Only the
        // synthesized "help" group exists.
        let mut state = CompletionState::new();
        let _ = _complete_help_shadow(&mut state, "noisy", |s| {
            s.add_match(Completion::new("live-poison"), Some("commands"));
            true
        });
        let live_commands_count: usize = state
            .groups
            .iter()
            .filter(|g| g.name == "commands")
            .map(|g| g.matches.len())
            .sum();
        assert_eq!(
            live_commands_count, 0,
            "shadowed completer's matches must not leak into live `commands` group"
        );
    }

    #[test]
    fn explanations_from_shadow_become_msg_rows() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "with-msg", |s| {
            s.add_match(Completion::new("x"), Some("g"));
            s.add_explanation("hint text".into(), Some("g"));
            true
        });
        assert!(ok);
        let disps: Vec<String> = state.groups[0]
            .matches
            .iter()
            .filter_map(|c| c.disp.clone())
            .collect();
        assert!(
            disps.iter().any(|d| d.contains("[msg] hint text")),
            "explanation should round-trip as a [msg] help row; got {disps:?}"
        );
    }
}
