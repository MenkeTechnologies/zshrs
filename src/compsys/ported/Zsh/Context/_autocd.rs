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
//! `_command_names` is ported (in `compsys::ported::_command_names`) and
//! `_cd` is a sibling shell fn; both are reached through
//! `shared::dispatch_action_command`, which resolves a command word exactly
//! as `execcmd` does — function/port/plugin, then builtin, then `$PATH`,
//! then `Src/exec.c:903`'s `command not found`. The `AUTOCD` option is read
//! directly from `zsh_h::isset`.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::zsh_h::{isset, AUTOCD};

/// `_autocd` — `-command-` context completion: command names + `_cd`
/// when the `autocd` option is set.
pub fn _autocd() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_autocd");
    // sh:3 — `_command_names` is a COMMAND WORD, so it resolves the way
    // `execcmd` resolves one: a shell function (a real definition, an
    // `autoload`/`compinit` stub, a plugin override, or the Rust port
    // standing in for the stock `$fpath` file), then a builtin, then `$PATH`,
    // and a name that resolves nowhere reaches `Src/exec.c:903`'s
    // `zerr("command not found: %s", arg0)` with status 127.
    // `dispatch_action_command` (shared.rs:1407) IS that resolution, and it
    // publishes the calling line first — `FnScope` zeroes `lineno` for every
    // port body (shared.rs), so without it the diagnostic and any frame
    // `_command_names` pushes record `_autocd:0` where zsh reads `_autocd:3`.
    //
    // What it replaces:
    //     dispatch_compsys(..).or_else(|| dispatch_function_call(..)).unwrap_or(1)
    // — the same function/port/plugin arms, but with NO not-found arm: a host
    // where `_command_names` resolves to nothing got `ret = 1` and complete
    // SILENCE. That is measurable here, because `compinit` stubs a `$fpath`
    // file only when it carries a `#compdef`/`#autoload` TAG (compinit
    // sh:533-548) and this host's first `$fpath` hit,
    // `~/.zpwr/autoload/comp_utils/_command_names`, is untagged — so the port
    // steps aside for it (router.rs `has_fpath_override`) and nothing defines
    // the name:
    //     zsh    `_autocd:3: command not found: _command_names`   rc 127
    //     zshrs  (nothing at all)                                  rc 1
    // `_normal`'s `CURRENT == 1` branch evals `$_comps[-command-]`, which is
    // `_autocd`, so every second-command position took that path: `nohup
    // <TAB>`, `git bisect run <TAB>` (`_git-bisect` sh:290 `*:: : _normal`)
    // and every other `_precommand`-shaped rest-arg rendered a diagnostic on
    // one side and an empty screen on the other.
    let ret = dispatch_action_command("_command_names", &[], 3);

    // sh:5  [[ -o autocd ]] && _cd || return ret
    if isset(AUTOCD) {
        // `A && B || C`: a FAILING `_cd` runs `return ret`, so `_cd`'s own
        // nonzero status never reaches the caller; only its success falls off
        // the end of the file, with the `&&` list's status, 0. The previous
        // `.unwrap_or(ret)` returned `_cd`'s status verbatim.
        if dispatch_action_command("_cd", &[], 5) == 0 {
            0
        } else {
            ret
        }
    } else {
        // sh:5 — `[[ -o autocd ]]` is false, so `&&` skips `_cd` and `||`
        // runs `return ret`.
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
