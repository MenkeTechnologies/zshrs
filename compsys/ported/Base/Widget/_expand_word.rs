//! Port of `_expand_word` — expand word (aliases, variables, etc.).
//!
//! Local shell reference: `compsys/functions/Base/Widget/_expand_word`
//! (system copy `/opt/homebrew/share/zsh/functions/_expand_word`).
//!
//! Upstream shell source (full):
//! ```text
//!  5  local curcontext="$curcontext"
//!  7  if [[ -z "$curcontext" ]]; then
//!  8    curcontext="expand-word:::"
//!  9  else
//! 10    curcontext="expand-word:${curcontext#*:}"
//! 11  fi
//! 13  _main_complete _expand
//! ```
//!
//! The shell version sets curcontext to `expand-word:…` and runs
//! `_main_complete _expand`. Our Rust port shortcuts: directly call
//! `_expand` (the expansion completer). User-visible behavior
//! identical because `_main_complete` would just dispatch to
//! `_expand` anyway.

use crate::compcore::CompletionState;

use super::_expand::_expand;

/// _expand_word - Expand word (aliases, variables, etc.)
pub fn _expand_word(state: &mut CompletionState) -> bool {
    _expand(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_expand_for_tilde() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/foo".into();
        // _expand_word IS _expand; both must succeed on tilde.
        assert!(_expand_word(&mut state));
    }

    #[test]
    fn delegates_for_no_expansion_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "plain".into();
        assert!(!_expand_word(&mut state));
    }

    #[test]
    fn dollar_var_expansion_succeeds() {
        std::env::set_var("ZSHRS_TEST_EXP_VAR", "expanded");
        let mut state = CompletionState::new();
        state.params.prefix = "$ZSHRS_TEST_EXP_VAR/bin".into();
        assert!(_expand_word(&mut state));
        std::env::remove_var("ZSHRS_TEST_EXP_VAR");
    }

    #[test]
    fn brace_expansion_succeeds() {
        let mut state = CompletionState::new();
        state.params.prefix = "{a,b}{1,2}".into();
        assert!(_expand_word(&mut state));
        let names: std::collections::HashSet<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains("a1"));
        assert!(names.contains("b2"));
    }
}
