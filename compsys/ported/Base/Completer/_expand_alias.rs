//! Port of `_expand_alias` — expand aliases.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_expand_alias`
//! (system copy `/opt/homebrew/share/zsh/functions/_expand_alias`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  9  if [[ "$funcstack[2]" = _prefix ]]; then
//! 10    word="$IPREFIX$PREFIX$SUFFIX"
//! 12    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! 22  zstyle -s ":completion:${curcontext}:${what}" regular sel
//! 32  zstyle -s ":completion:${curcontext}:${what}" global sel
//! 41  if [[ -n "$pre" ]]; then  …emit replacement…
//! ```
//!
//! Shell honors `regular` / `global` zstyles + the `_prefix`
//! funcstack detection. Simplified Rust port: takes alias map
//! directly (caller pulled from `aliastab_lock()`), uses
//! `current_word()` (= PREFIX+SUFFIX, matching shell:10
//! `_prefix`-parent case), and emits the expansion with NOSPACE
//! flag so the inserted text isn't re-split into words.

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
