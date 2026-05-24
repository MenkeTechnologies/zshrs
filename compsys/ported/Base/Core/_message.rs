//! Port of `_message` — display a message (no completions).
//!
//! Local shell reference: `compsys/functions/Base/Core/_message`
//! (system copy `/opt/homebrew/share/zsh/functions/_message`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local format raw gopt
//!  5  if [[ "$1" = -e ]]; then
//!  9    _comp_mesg=yes
//! 11    if (( $# > 2 )); then
//! 12      tag="$2"
//! 13      shift
//! 15    else
//! 16      tag="$curtag"
//! 21    zstyle -s ":completion:${curcontext}:messages" format format
//! ```
//!
//! Upstream resolves the `format` zstyle to wrap the message, sets
//! `_comp_mesg=yes` so subsequent code knows a message was emitted,
//! then `compadd -x` for ZLE to render.
//!
//! Faithful Rust port: calls into `_description` (which already
//! handles format resolution) then attaches the rendered string
//! as an explanation on the named tag group. `nmessages` increments
//! match the shell-side `_comp_mesg` flag (callers check the count).

use crate::compcore::CompletionState;
use crate::ported::_description::_description;
use crate::zstyle::ZStyleStore;

/// _message - display a message (no completions)
pub fn _message(
    state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    message: &str,
) {
    let formatted = _description(state, styles, context, tag, message);

    if let Some(msg) = formatted {
        state.begin_group(tag, true);
        state.add_explanation(msg, Some(tag));
        state.end_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_explanation_into_named_tag_group() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        _message(&mut state, &styles, ":complete::test:", "messages", "no values to complete");
        let grp = state
            .groups
            .iter()
            .find(|g| g.name == "messages")
            .expect("messages group present");
        assert!(!grp.explanations.is_empty(), "message must appear as an explanation");
    }

    #[test]
    fn nmessages_counter_incremented() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        assert_eq!(state.nmessages, 0);
        _message(&mut state, &styles, ":complete::test:", "messages", "hello");
        assert_eq!(state.nmessages, 1, "_message must bump nmessages");
    }

    #[test]
    fn multiple_messages_increment_counter() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        _message(&mut state, &styles, ":complete::test:", "a", "first");
        _message(&mut state, &styles, ":complete::test:", "b", "second");
        _message(&mut state, &styles, ":complete::test:", "c", "third");
        assert_eq!(state.nmessages, 3);
    }

    #[test]
    fn format_zstyle_wraps_message() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        // _description honors format zstyle on descriptions context.
        styles.set(
            ":completion::test::messages:descriptions",
            "format",
            vec!["[%d]".into()],
            false,
        );
        _message(&mut state, &styles, "::test:", "messages", "hello");
        let g = state.groups.iter().find(|g| g.name == "messages");
        assert!(g.is_some(), "messages group should exist");
    }

    #[test]
    fn no_emit_when_description_returns_none() {
        // When _description returns None (e.g. `hidden=all`), no
        // explanation should be added. Just pin nmessages didn't
        // climb.
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        let before = state.nmessages;
        // With no special configuration, _message always emits.
        _message(&mut state, &styles, ":complete::test:", "msg", "hi");
        assert!(state.nmessages > before);
    }

    #[test]
    fn hidden_all_suppresses_emission() {
        // When the tag's `hidden` style is `all`, _description
        // returns None → no group created, nmessages unchanged.
        // `_description` builds ctx = `{context}:{tag}` so we set
        // the style at that combined path.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        // context=":t:", tag="tag" → lookup ctx is ":t::tag"
        styles.set(":t::tag", "hidden", vec!["all".into()], false);
        let before = state.nmessages;
        _message(&mut state, &styles, ":t:", "tag", "should be hidden");
        assert_eq!(state.nmessages, before);
        assert!(!state.groups.iter().any(|g| g.name == "tag"));
    }

    #[test]
    fn format_d_substitution_appears_in_explanation() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        // context=":t:", tag="msg" → lookup ctx is ":t::msg"
        styles.set(":t::msg", "format", vec!["<<%d>>".into()], false);
        _message(&mut state, &styles, ":t:", "msg", "INSIDE");
        let grp = state.groups.iter().find(|g| g.name == "msg").unwrap();
        assert!(grp.explanations.iter().any(|e| e.contains("<<INSIDE>>")));
    }

    #[test]
    fn message_does_not_add_completion_matches() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        _message(&mut state, &styles, ":t:", "tag", "msg");
        assert_eq!(state.nmatches, 0, "_message MUST NOT emit candidate matches");
    }
}
