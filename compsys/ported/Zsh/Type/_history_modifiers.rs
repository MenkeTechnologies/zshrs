//! Port of `_history_modifiers` — complete history-expansion / glob
//! qualifier / parameter modifier chars (`:h`, `:t`, `:r`, …).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_history_modifiers`.
//!
//! Upstream shell source (~30 lines, key dispatch):
//! ```text
//! local type=$1
//! # type:  h=history, q=glob qualifier, p=parameter
//! # case $char in
//! #   ([hretpqQxlu&]) single character modifiers
//! #   (s|gs) substitution
//! #   ...
//! ```
//!
//! Strict Rust port: emits the documented single-char modifiers
//! (and `gs`, `s`) as candidates. The `type` arg selects which
//! subset; for `h` (history) all modifiers are available; for
//! `p` (parameter) the substitution `s`/`gs` is also available;
//! for `q` (glob qualifier) only single-char.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Canonical single-char modifiers (all contexts).
pub const SINGLE_CHAR_MODIFIERS: &[(&str, &str)] = &[
    ("h", "head (remove last path segment)"),
    ("r", "remove extension"),
    ("e", "leave extension only"),
    ("t", "tail (basename)"),
    ("p", "print only"),
    ("q", "quote whitespace"),
    ("Q", "remove quoting"),
    ("x", "split into words at whitespace"),
    ("l", "lowercase"),
    ("u", "uppercase"),
    ("&", "repeat last substitution"),
];

/// Multi-char modifiers (history + parameter context only).
pub const MULTI_CHAR_MODIFIERS: &[(&str, &str)] = &[
    ("s", "substitution: s/pattern/replacement/"),
    ("gs", "global substitution: gs/pattern/replacement/"),
];

/// Modifier context selector — mirrors the shell `$1` arg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierContext {
    /// History expansion (`!foo:h`).
    History,
    /// Glob qualifier (`*(.h)`).
    GlobQualifier,
    /// Parameter expansion (`${foo:h}`).
    Parameter,
}

/// `_history_modifiers` — emit modifier chars for the given context.
pub fn _history_modifiers(state: &mut CompletionState, ctx: ModifierContext) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("modifiers", true);
    let mut any = false;
    for (m, desc) in SINGLE_CHAR_MODIFIERS {
        if !m.starts_with(&*prefix) {
            continue;
        }
        let mut comp = Completion::new(*m);
        comp.disp = Some(format!("{} -- {}", m, desc));
        state.add_match(comp, Some("modifiers"));
        any = true;
    }
    if matches!(ctx, ModifierContext::History | ModifierContext::Parameter) {
        for (m, desc) in MULTI_CHAR_MODIFIERS {
            if !m.starts_with(&*prefix) {
                continue;
            }
            let mut comp = Completion::new(*m);
            comp.disp = Some(format!("{} -- {}", m, desc));
            state.add_match(comp, Some("modifiers"));
            any = true;
        }
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_context_emits_singles_and_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h"));
        assert!(names.contains(&"t"));
        assert!(names.contains(&"r"));
        assert!(names.contains(&"e"));
        assert!(names.contains(&"s"));
        assert!(names.contains(&"gs"));
    }

    #[test]
    fn glob_qualifier_context_excludes_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::GlobQualifier);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h"));
        assert!(!names.contains(&"s"));
        assert!(!names.contains(&"gs"));
    }

    #[test]
    fn parameter_context_includes_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::Parameter);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"s"));
        assert!(names.contains(&"gs"));
    }

    #[test]
    fn prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "g".into();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["gs"]);
    }

    #[test]
    fn each_emit_has_descriptive_disp() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        for m in &state.groups[0].matches {
            let d = m.disp.as_deref().unwrap_or("");
            assert!(d.contains(" -- "), "missing disp separator on `{}`", m.str_);
        }
    }
}
