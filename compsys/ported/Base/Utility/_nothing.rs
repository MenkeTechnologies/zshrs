//! Port of `_nothing` — add no completions (but don't fail).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_nothing`
//! (system copy `/opt/homebrew/share/zsh/functions/_nothing`).
//!
//! Upstream shell source (the WHOLE file):
//! ```text
//!  1  #compdef true false log times clear logname whoami sync
//!  3  _message 'no argument or option'
//! ```
//!
//! Upstream is bound as the completion for commands that genuinely
//! take no args (`true`, `false`, `whoami`, etc.) — it emits the
//! "no argument or option" message via `_message`.
//!
//! Strict Rust port: dispatches to `_message` directly. The shell
//! `_message` itself returns success regardless of whether matches
//! were added; we mirror by always returning true.

use crate::compcore::CompletionState;
use crate::ported::_message::_message;
use crate::zstyle::ZStyleStore;

/// `_nothing` — emit the "no argument or option" message and signal
/// the completer ran successfully.
pub fn _nothing(state: &mut CompletionState, styles: &ZStyleStore, context: &str) -> bool {
    // shell:3 — `_message 'no argument or option'`. tag defaults to
    // the empty curtag at this layer.
    _message(state, styles, context, "messages", "no argument or option");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_no_argument_or_option_message() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        assert!(_nothing(&mut state, &styles, ""));
        // _message attaches the rendered string as an explanation
        // on the "messages" group.
        let exps: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(
            exps.iter().any(|s| s.contains("no argument or option")),
            "expected the upstream message somewhere in explanations; got {exps:?}"
        );
    }

    #[test]
    fn always_returns_true() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        assert!(_nothing(&mut state, &styles, ""));
        assert!(_nothing(&mut state, &styles, ":completion::foo:"));
    }

    #[test]
    fn does_not_add_normal_matches() {
        // The fn emits a message, not match candidates. nmatches
        // should stay at zero.
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        _nothing(&mut state, &styles, "");
        assert_eq!(state.nmatches, 0);
    }

    #[test]
    fn does_not_touch_compstate() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        state.params.compstate.insert = "menu".into();
        state.params.compstate.list = "list packed".into();
        _nothing(&mut state, &styles, "");
        assert_eq!(state.params.compstate.insert, "menu");
        assert_eq!(state.params.compstate.list, "list packed");
    }

    #[test]
    fn does_not_touch_prefix_suffix() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        state.params.prefix = "git".into();
        state.params.suffix = "@HEAD".into();
        _nothing(&mut state, &styles, "");
        assert_eq!(state.params.prefix, "git");
        assert_eq!(state.params.suffix, "@HEAD");
    }

    #[test]
    fn respects_messages_format_style_when_set() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        // _description should pick up the "messages" format style.
        styles.set(
            ":completion:::test:messages",
            "format",
            vec!["[%d]".into()],
            false,
        );
        _nothing(&mut state, &styles, ":test:");
        let exps: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        // The %d placeholder should expand to the message text.
        assert!(
            exps.iter().any(|s| s.contains("no argument or option")),
            "format-style %d substitution should still contain the body; got {exps:?}"
        );
    }
}
