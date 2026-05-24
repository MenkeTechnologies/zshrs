//! Port of `_menu` — menu completion mode.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_menu`
//! (system copy `/opt/homebrew/share/zsh/functions/_menu`).
//!
//! Upstream shell source:
//! ```text
//!  3  [[ _matcher_num -gt 1 ]] && return 1
//! 11  if [[ -n "$compstate[old_list]" ]]; then
//! 13    # We have an old list, keep it and insert the next match.
//! 15    compstate[old_list]=keep
//! 16    compstate[insert]=$((compstate[old_insert]+1))
//! 17  else
//! 19    # No old list, make completion insert the first match.
//! 20    compstate[insert]=1
//! 21  fi
//! 23  return 1
//! ```
//!
//! Simplified Rust port: drops the `_matcher_num > 1` early-exit
//! and old-list handling (those require the full menu-state
//! machinery) and unconditionally arms menu mode via
//! `compstate[insert]=menu`. Users get the menu cycle on next Tab.

use crate::compcore::CompletionState;

/// _menu - Menu completion mode
pub fn _menu(state: &mut CompletionState) -> bool {
    // shell:20 — compstate[insert]=1 (we use `menu` here so the
    // user's auto-menu binding picks it up uniformly; ZLE both
    // forms trigger menu mode).
    state.params.compstate.insert = "menu".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_compstate_insert_to_menu() {
        let mut state = CompletionState::new();
        assert!(_menu(&mut state));
        assert_eq!(state.params.compstate.insert, "menu");
    }
}
