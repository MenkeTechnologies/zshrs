//! Port of `_menu` — menu completion mode.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_menu`
//! (system copy `/opt/homebrew/share/zsh/functions/_menu`).
//!
//! Upstream shell source:
//! ```text
//!  3  [[ _matcher_num -gt 1 ]] && return 1
//! 11  if [[ -n "$compstate[old_list]" ]]; then
//! 15    compstate[old_list]=keep
//! 16    compstate[insert]=$((compstate[old_insert]+1))
//! 17  else
//! 20    compstate[insert]=1
//! 21  fi
//! 23  return 1
//! ```
//!
//! Strict Rust port: gates on `matcher_num > 1` (caller-supplied
//! since compsys's `CompletionState` doesn't expose it), then takes
//! the `old_list ? keep+bump : insert=1` branch and ALWAYS returns
//! false (shell `return 1`). The shell `return 1` is critical: it
//! signals to `_main_complete` that this completer DIDN'T provide
//! matches; the next completer (typically `_complete`) generates
//! them, and the inserted index drives which one fires.

use crate::compcore::CompletionState;

/// _menu - Menu completion mode.
///
/// `matcher_num` mirrors `_matcher_num` from the shell variable; the
/// caller (typically `_main_complete`) wires it.
pub fn _menu(state: &mut CompletionState, matcher_num: usize) -> bool {
    // shell:3 — `_matcher_num -gt 1` short-circuits to return 1.
    if matcher_num > 1 {
        return false;
    }

    let have_old_list = !state.params.compstate.old_list.is_empty();
    if have_old_list {
        state.params.compstate.old_list = "keep".to_string();
        let old_n: i64 = state
            .params
            .compstate
            .old_insert
            .parse()
            .unwrap_or(0);
        state.params.compstate.insert = (old_n + 1).to_string();
    } else {
        state.params.compstate.insert = "1".to_string();
    }

    // shell:23 — `return 1`. The shell return 1 from a completer
    // means "I didn't add matches, fall through". We mirror.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_first_matcher_bails() {
        // Default `insert` is `unambiguous`; bail must leave it
        // untouched (no mutation when matcher_num > 1).
        let mut state = CompletionState::new();
        let original_insert = state.params.compstate.insert.clone();
        assert!(!_menu(&mut state, 2));
        assert_eq!(state.params.compstate.insert, original_insert);
    }

    #[test]
    fn first_run_no_old_list_sets_insert_to_1() {
        // shell:20 — no old list → insert=1
        let mut state = CompletionState::new();
        let r = _menu(&mut state, 1);
        assert!(!r, "shell `return 1` → false");
        assert_eq!(state.params.compstate.insert, "1");
        assert_eq!(state.params.compstate.old_list, "");
    }

    #[test]
    fn second_run_with_old_list_bumps_insert() {
        // shell:15-16 — keep old list AND insert = old_insert+1
        let mut state = CompletionState::new();
        state.params.compstate.old_list = "shown".into();
        state.params.compstate.old_insert = "3".into();
        let r = _menu(&mut state, 1);
        assert!(!r);
        assert_eq!(state.params.compstate.old_list, "keep");
        assert_eq!(state.params.compstate.insert, "4");
    }

    #[test]
    fn old_list_with_empty_old_insert_starts_at_1() {
        let mut state = CompletionState::new();
        state.params.compstate.old_list = "shown".into();
        // old_insert empty → parses as 0 → +1 = 1
        let _ = _menu(&mut state, 1);
        assert_eq!(state.params.compstate.insert, "1");
    }

    #[test]
    fn never_adds_matches() {
        let mut state = CompletionState::new();
        let _ = _menu(&mut state, 1);
        assert_eq!(state.nmatches, 0);
    }

    #[test]
    fn matcher_zero_treated_as_first_pass() {
        let mut state = CompletionState::new();
        let _ = _menu(&mut state, 0);
        assert_eq!(state.params.compstate.insert, "1");
    }
}
