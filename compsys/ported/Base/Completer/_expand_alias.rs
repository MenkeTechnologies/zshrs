//! Port of `_expand_alias` — expand aliases. Moved from
//! `compsys/functions.rs`.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _expand_alias - Expand aliases
pub fn _expand_alias(state: &mut CompletionState, aliases: &HashMap<String, String>) -> bool {
    let word = state.params.current_word();

    if let Some(expansion) = aliases.get(&word) {
        let mut comp = Completion::new(expansion);
        comp.flags |= CompletionFlags::NOSPACE;
        state.add_match(comp, None);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_alias_emits_expansion_with_nospace() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut aliases = HashMap::new();
        aliases.insert("ll".into(), "ls -la".into());
        let ok = _expand_alias(&mut state, &aliases);
        assert!(ok);
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("a match");
        assert_eq!(m.str_, "ls -la");
        // NOSPACE flag is critical — without it, ZLE would append a
        // space after the expanded alias, which would re-trigger
        // word splitting and break compound aliases like `git st`.
        assert!(
            m.flags.contains(CompletionFlags::NOSPACE),
            "NOSPACE flag must be set on expanded alias"
        );
    }

    #[test]
    fn unknown_word_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "unknown".into();
        let aliases = HashMap::new();
        assert!(!_expand_alias(&mut state, &aliases));
    }

    #[test]
    fn full_current_word_used_not_just_prefix() {
        // current_word = prefix + suffix. Pin that alias lookup
        // joins both — otherwise a user typing `l|l` (cursor after
        // first l) would not match `ll`.
        let mut state = CompletionState::new();
        state.params.prefix = "l".into();
        state.params.suffix = "l".into();
        let mut aliases = HashMap::new();
        aliases.insert("ll".into(), "ls -la".into());
        assert!(_expand_alias(&mut state, &aliases));
    }
}
