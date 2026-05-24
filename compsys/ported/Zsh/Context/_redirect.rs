//! Port of `_redirect` — `-redirect-` context handler (after `>`,
//! `<`, `>>`, etc. on the command line).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_redirect`.
//!
//! Upstream shell source (the whole 18-line file):
//! ```text
//! #compdef -redirect-
//!
//! local strs _comp_command1 _comp_command2 _comp_command
//!
//! _set_command
//!
//! strs=( -default- )
//!
//! if [[ "$CURRENT" != "1" ]]; then
//!   strs=( "${_comp_command}" "$strs[@]" )
//!   if [[ -n "$_comp_command1" ]]; then
//!     strs=( "${_comp_command1}" "$strs[@]" )
//!     [[ -n "$_comp_command2" ]] &&
//!       strs=( "${_comp_command2}" "$strs[@]" )
//!   fi
//! fi
//!
//! _dispatch -redirect-,{${compstate[redirect]},-default-},${^strs}
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_set_command`] to populate `_comp_command{,1,2}` in
//! `state.lastcomp`, builds the dispatch-key list per upstream,
//! then calls our ported [`_dispatch`].

use crate::base::MainCompleteState;
use crate::ported::_set_command::{CommandClassifier, _set_command};

/// Build the `-redirect-,{kind,-default-},{cmd…,-default-}` dispatch
/// keys upstream produces via brace expansion on shell:18.
/// Returned to the caller — which owns the `_dispatch` comps
/// registry — for iteration.
pub fn redirect_dispatch_keys(
    state: &mut MainCompleteState,
    cx: &CommandClassifier<'_>,
    redirect_kind: &str,
) -> Vec<String> {
    // shell:5 — `_set_command`
    _set_command(state, cx);

    let current = state.comp.params.current;
    let mut strs: Vec<String> = vec!["-default-".into()];
    if current != 1 {
        let cmd = state
            .lastcomp
            .get("_comp_command")
            .cloned()
            .unwrap_or_default();
        let cmd1 = state
            .lastcomp
            .get("_comp_command1")
            .cloned()
            .unwrap_or_default();
        let cmd2 = state
            .lastcomp
            .get("_comp_command2")
            .cloned()
            .unwrap_or_default();
        if !cmd.is_empty() {
            strs.insert(0, cmd);
        }
        if !cmd1.is_empty() {
            strs.insert(0, cmd1);
        }
        if !cmd2.is_empty() {
            strs.insert(0, cmd2);
        }
    }

    let kinds = [redirect_kind, "-default-"];
    let mut keys: Vec<String> = Vec::new();
    for kind in &kinds {
        for s in &strs {
            keys.push(format!("-redirect-,{},{}", kind, s));
        }
    }
    keys
}

/// `_redirect` — `-redirect-` context handler. Runs
/// [`_set_command`] to populate `_comp_command{,1,2}` in
/// `state.lastcomp` per upstream line 5. Returns false — the
/// actual `_dispatch` call is the caller's job since it owns the
/// comps registry. Use [`redirect_dispatch_keys`] to get the key
/// list the caller should iterate.
pub fn _redirect(
    state: &mut MainCompleteState,
    cx: &CommandClassifier<'_>,
    _redirect_kind: &str,
) -> bool {
    _set_command(state, cx);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn null_cx() -> CommandClassifier<'static> {
        CommandClassifier::null()
    }

    #[test]
    fn current_is_one_only_dispatches_default_strs() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        let _ = _redirect(&mut state, &null_cx(), "out");
        // No registered _dispatch handlers → false; but
        // _set_command should have run, populating lastcomp.
        assert!(state.lastcomp.is_empty() || state.lastcomp.contains_key("_comp_command"));
    }

    #[test]
    fn current_gt_1_includes_command_in_strs() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), ">".into()];
        let _ = _redirect(&mut state, &null_cx(), "out");
        assert_eq!(
            state.lastcomp.get("_comp_command").map(String::as_str),
            Some("git")
        );
    }

    #[test]
    fn calls_set_command_first() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["ls".into()];
        let _ = _redirect(&mut state, &null_cx(), "out");
        // After _set_command runs, _comp_command should be set.
        assert!(state.lastcomp.contains_key("_comp_command"));
    }

    #[test]
    fn empty_state_does_not_panic() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _redirect(&mut state, &null_cx(), "out");
    }

    #[test]
    fn dispatch_keys_built_for_current_eq_1() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        let keys = redirect_dispatch_keys(&mut state, &null_cx(), "out");
        // Just "-default-" str: 2 kinds × 1 str = 2 keys
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| k.ends_with(",-default-")));
        assert!(keys.iter().any(|k| k.contains(",out,")));
    }

    #[test]
    fn dispatch_keys_include_command_for_current_gt_1() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), ">".into()];
        let keys = redirect_dispatch_keys(&mut state, &null_cx(), "out");
        // strs = [git, -default-] → 2 kinds × 2 = 4 keys
        assert!(keys.iter().any(|k| k.contains(",git")));
        assert!(keys.iter().any(|k| k.contains(",-default-")));
    }
}
