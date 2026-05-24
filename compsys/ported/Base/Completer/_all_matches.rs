//! Port of `_all_matches` — show all possible matches.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_all_matches`
//! (system copy `/opt/homebrew/share/zsh/functions/_all_matches`).
//!
//! Upstream shell source (line numbers from the system file):
//! ```text
//!  3  _all_matches() {
//!  4    local old
//!  6    zstyle -s ":completion:${curcontext}:" old-matches old
//!  8    if [[ "$old" = (only|true|yes|1|on) ]]; then
//! 10      if [[ -n "$compstate[old_list]" ]]; then
//! 11        compstate[insert]=all
//! 12        compstate[old_list]=keep
//! 13        return 0
//! 14      fi
//! 16      [[ "$old" = *only* ]] && return 1
//! 17    fi
//! 19    (( $comppostfuncs[(I)_all_matches_end] )) ||
//! 20        comppostfuncs=( "$comppostfuncs[@]" _all_matches_end )
//! 22    _all_matches_context=":completion:${curcontext}:"
//! 24    return 1
//! 25  }
//! ```
//!
//! Simplified Rust port: skips the old-matches zstyle / comppostfuncs
//! plumbing (those require the full compcore postfunc dispatch
//! infrastructure) and unconditionally sets `compstate[insert]=all`
//! to deliver the user-visible "show ALL" behavior. The fn returns
//! true so the caller's main_complete dispatch records that we
//! handled it.

use crate::compcore::CompletionState;

/// _all_matches - Show all possible matches
pub fn _all_matches(state: &mut CompletionState) -> bool {
    // shell:11 — compstate[insert]=all
    state.params.compstate.insert = "all".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_compstate_insert_to_all() {
        let mut state = CompletionState::new();
        assert!(_all_matches(&mut state));
        assert_eq!(state.params.compstate.insert, "all");
    }

    #[test]
    fn overwrites_existing_insert_value() {
        let mut state = CompletionState::new();
        state.params.compstate.insert = "menu".to_string();
        assert!(_all_matches(&mut state));
        assert_eq!(state.params.compstate.insert, "all");
    }
}
