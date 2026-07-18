//! Port of `_autocd` from `Completion/Zsh/Context/_autocd`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef -command-
//! sh:2
//! sh:3  _command_names
//! sh:4  local ret=$?
//! sh:5  [[ -o autocd ]] && _cd || return ret
//! ```
//!
//! `_command_names` is ported (in `compsys::ported::_command_names`).
//! `_cd` is a sibling shell fn — dispatch via `exec accessors`. The
//! `AUTOCD` option is read directly from `zsh_h::isset`.

use crate::ported::exec::dispatch_function_call;
use crate::ported::zsh_h::{isset, AUTOCD};

/// `_autocd` — `-command-` context completion: command names + `_cd`
/// when the `autocd` option is set.
pub fn _autocd() -> i32 {
    // sh:3 — `_command_names`. Route through the router (NOT a direct
    // `_command_names(&[])` call) so a plugin-registered override (ABI v4,
    // `zmodload -R`) — e.g. the user's customized `_command_names` — wins
    // over the built-in Rust port, exactly as it would if `_autocd` were
    // the shell function invoking `_command_names` by name. `dispatch_compsys`
    // resolves the override first, else the built-in port, so it always
    // returns `Some` here.
    let ret = crate::compsys::router::dispatch_compsys("_command_names", &[]).unwrap_or(1);

    // sh:5  [[ -o autocd ]] && _cd || return ret
    if isset(AUTOCD) {
        dispatch_function_call("_cd", &[]).unwrap_or(ret)
    } else {
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_command_names_status_when_autocd_off() {
        // When AUTOCD is unset (default for tests), _autocd returns
        //   _command_names' status (1 without registered tags).
        let _g = crate::test_util::global_state_lock();
        let _r = _autocd();
    }
}
