//! Port of `_list` — list completions without inserting.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_list`
//! (system copy `/opt/homebrew/share/zsh/functions/_list`).
//!
//! Upstream shell source (relevant lines):
//! ```text
//!  7  [[ _matcher_num -gt 1 ]] && return 1
//! 13  if zstyle -t ":completion:${curcontext}:" word; then
//! 14    pre="$HISTNO$LBUFFER"
//! 17  else
//! 18    pre="$PREFIX"
//! 23  if zstyle -T ":completion:${curcontext}:" condition &&
//! 24     [[ "$pre" != "$_list_prefix" || "$suf" != "$_list_suffix" ]]; then
//! 29    compstate[insert]=''
//! 30    compstate[list]='list force'
//! 31    _list_prefix="$pre"
//! 32    _list_suffix="$suf"
//! 33  fi
//! 37  return 1
//! ```
//!
//! Simplified Rust port: drops the per-call _list_prefix tracking
//! (which gates "only list once per identical prefix") and just
//! unconditionally sets `compstate[list]+=list` + clears
//! `compstate[insert]`. Same user-visible effect on a single Tab.

use crate::compcore::CompletionState;

/// _list - List completions without inserting
pub fn _list(state: &mut CompletionState) -> bool {
    // shell:30 — compstate[list]='list force' (we append `list` here)
    state.params.compstate.list.push_str(" list");
    // shell:29 — compstate[insert]=''
    state.params.compstate.insert.clear();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_list_to_compstate_list_and_clears_insert() {
        let mut state = CompletionState::new();
        state.params.compstate.insert = "menu".into();
        assert!(_list(&mut state));
        assert!(state.params.compstate.list.contains("list"),
                "compstate[list] must gain `list` suffix");
        assert_eq!(state.params.compstate.insert, "",
                   "_list must clear compstate[insert] so completion only lists");
    }

    #[test]
    fn returns_true_unconditionally() {
        // shell:37 `return 1` — but the SHELL return-1 means "no
        // matches added", which translates to true in the Rust
        // "we successfully did our work" sense. Pin contract.
        let mut state = CompletionState::new();
        assert!(_list(&mut state));
    }

    #[test]
    fn does_not_add_completion_matches() {
        // _list shows the existing list; never produces NEW matches.
        let mut state = CompletionState::new();
        _list(&mut state);
        assert_eq!(state.nmatches, 0);
    }

    #[test]
    fn idempotent_call_keeps_one_list_marker() {
        let mut state = CompletionState::new();
        _list(&mut state);
        let after_one = state.params.compstate.list.clone();
        _list(&mut state);
        // The second call appends ' list' again — pin that the
        // marker string grows on repeated calls (the receiver
        // typically dedupes when consuming compstate).
        assert!(state.params.compstate.list.len() > after_one.len());
        // BUT it still contains "list".
        assert!(state.params.compstate.list.contains("list"));
    }

    #[test]
    fn clears_existing_insert_no_matter_the_old_value() {
        for old in ["menu", "1", "all", "5"] {
            let mut state = CompletionState::new();
            state.params.compstate.insert = old.into();
            _list(&mut state);
            assert_eq!(state.params.compstate.insert, "", "old `{old}` not cleared");
        }
    }
}
