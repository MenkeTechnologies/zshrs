//! Port of `_oldlist` — use previous completion list.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_oldlist`
//! (system copy `/opt/homebrew/share/zsh/functions/_oldlist`).
//!
//! Upstream shell source (relevant lines):
//! ```text
//!  3  [[ _matcher_num -gt 1 || $_lastcomp[nmatches] -eq 0 ]] && return 1
//!  7  zstyle -s ":completion:${curcontext}:" old-list list
//! 16  if [[ -n $compstate[old_list] && $list != never &&
//! 17        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! 18    if [[ $WIDGETSTYLE = *list* && ( $list = always || $list != shown ) ]]; then
//! 19      compstate[old_list]=keep
//! 20      return 0
//! 21    elif [[ $list = *${_lastcomp[completer]}* ]]; then
//! 22      [[ "$_lastcomp[insert]" = unambig* ]] && compstate[to_end]=single
//! 23      compstate[old_list]=keep
//! ```
//!
//! Simplified Rust port: skips the `_matcher_num` / lastcomp /
//! WIDGETSTYLE branches (need full ZLE widget-context wiring) and
//! unconditionally arms `compstate[old_list]=keep` — the user-facing
//! behavior the `compstate[old_list]=keep` line ALL the branches
//! converge on.

use crate::compcore::CompletionState;

/// _oldlist - Use previous completion list
pub fn _oldlist(state: &mut CompletionState) -> bool {
    // shell:19 / shell:23 — compstate[old_list]=keep
    state.params.compstate.old_list = "keep".to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_old_list_to_keep() {
        let mut state = CompletionState::new();
        assert!(_oldlist(&mut state));
        assert_eq!(state.params.compstate.old_list, "keep",
                   "shell `compstate[old_list]=keep` must be reflected");
    }
}
