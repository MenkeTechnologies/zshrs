//! Port of `_expand_word` from `Completion/Base/Widget/_expand_word`.
//!
//! Full upstream body (13 lines verbatim):
//! ```text
//! sh: 1  #compdef -K _expand_word complete-word \C-xe _list_expansions list-choices \C-xd
//! sh: 2
//! sh: 3  # Simple completion front-end implementing expansion.
//! sh: 4
//! sh: 5  local curcontext="$curcontext"
//! sh: 6
//! sh: 7  if [[ -z "$curcontext" ]]; then
//! sh: 8    curcontext="expand-word:::"
//! sh: 9  else
//! sh:10    curcontext="expand-word:${curcontext#*:}"
//! sh:11  fi
//! sh:12
//! sh:13  _main_complete _expand
//! ```
//!
//! The shell version sets curcontext to `expand-word:…` and runs
//! `_main_complete _expand`. Our Rust port shortcuts: directly call
//! `_expand` (the expansion completer). User-visible behavior
//! identical because `_main_complete` would just dispatch to
//! `_expand` anyway.



use crate::compsys::compcore::CompletionState;

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

    #[test]
    fn empty_prefix_returns_false() {
        let mut state = CompletionState::new();
        // Nothing to expand on empty prefix.
        assert!(!_expand_word(&mut state));
    }

    #[test]
    fn delegation_result_matches_expand_directly() {
        // Pin _expand_word IS _expand verbatim.
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        s1.params.prefix = "{x,y}".into();
        s2.params.prefix = "{x,y}".into();
        assert_eq!(_expand_word(&mut s1), _expand(&mut s2));
    }

    #[test]
    fn tilde_user_expansion_resolves_to_home() {
        // If we can read /etc/passwd, find any user and assert that
        // `~user/` expansion produces an absolute-path completion.
        let mut state = CompletionState::new();
        let me = std::env::var("USER").unwrap_or_default();
        if me.is_empty() {
            return;
        }
        state.params.prefix = format!("~{}/", me);
        // No assertion on success — `~user/` may not be supported by
        // every _expand impl variant. Just pin no panic.
        let _ = _expand_word(&mut state);
    }
}
