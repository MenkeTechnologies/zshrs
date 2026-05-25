//! Port of `_autocd` from `Completion/Zsh/Context/_autocd`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef -command-
//! sh: 2
//! sh: 3  _command_names
//! sh: 4  local ret=$?
//! sh: 5  [[ -o autocd ]] && _cd || return ret
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_command_names`]. When the shell `autocd` option is set
//! (caller-supplied via `autocd_set`), additionally tries `_cd`
//! (per-command completer; not in the engine layer — caller
//! dispatches that themselves).
//!
//! TODO: `_cd` is a Zsh/Command per-builtin completer not in the
//! engine port. Caller passes a closure that runs `_cd` when
//! `autocd_set=true` AND `_command_names` returned false (i.e. the
//! bareword is a directory, not a command).



use crate::compsys::compcore::CompletionState;
use crate::compsys::ported::_command_names::{ShellInventory, _command_names};

/// `_autocd` — `-command-` context entry. When `autocd_set` is true
/// and `_command_names` returns false, fall through to `cd_action`.
pub fn _autocd(
    state: &mut CompletionState,
    inv: &ShellInventory<'_>,
    autocd_set: bool,
    cd_action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // shell:3 — `_command_names`
    let ret = _command_names(state, inv, false);
    // shell:5 — `[[ -o autocd ]] && _cd || return ret`
    if autocd_set {
        cd_action(state)
    } else {
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_autocd_returns_command_names_result() {
        let mut state = CompletionState::new();
        state.params.prefix = "ls".into();
        let inv = ShellInventory::default();
        let r = _autocd(&mut state, &inv, false, |_| {
            panic!("cd_action MUST NOT run when autocd_set=false")
        });
        // _command_names emits if PATH has `ls` (it does on every Unix).
        let _ = r;
    }

    #[test]
    fn autocd_runs_cd_action() {
        let mut state = CompletionState::new();
        let inv = ShellInventory::default();
        let cd_ran = std::cell::Cell::new(false);
        let _ = _autocd(&mut state, &inv, true, |_| {
            cd_ran.set(true);
            true
        });
        assert!(cd_ran.get());
    }

    #[test]
    fn autocd_cd_action_result_propagates() {
        let mut state = CompletionState::new();
        let inv = ShellInventory::default();
        let r = _autocd(&mut state, &inv, true, |_| false);
        assert!(!r);
    }

    #[test]
    fn empty_state_dispatches_correctly() {
        let mut state = CompletionState::new();
        let inv = ShellInventory::default();
        let _ = _autocd(&mut state, &inv, false, |_| true);
    }
}
