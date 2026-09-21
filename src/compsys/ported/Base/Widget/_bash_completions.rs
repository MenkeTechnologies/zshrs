//! Port of `_bash_completions` from
//! `Completion/Base/Widget/_bash_completions`.
//!
//! Full upstream body (46 lines verbatim, abridged):
//! ```text
//! sh: 1  #compdef -K _bash_complete-word complete-word \e~ _bash_list-choices list-choices ^X~
//! sh:28  eval "$_comp_setup"
//! sh:30  local key=$KEYS[-1] expl
//! sh:32  case $key in
//! sh:33    '!') _main_complete _command_names ;;
//! sh:35    '$') _main_complete - parameters _wanted parameters expl 'exported parameter' \
//! sh:36                                       _parameters -g '*export*' ;;
//! sh:38    '@') _main_complete _hosts ;;
//! sh:40    '/') _main_complete _files ;;
//! sh:42    '~') _main_complete _users ;;
//! sh:46  esac
//! ```
//!
//! Dispatches `_main_complete` with a key-specific completer chain
//! based on the last char of `$KEYS`. `_main_complete` is a sibling
//! shell fn — dispatched via `exec accessors`.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::getsparam;

/// `_bash_completions` — Bash-style keybinding completion router.
/// Reads the last char of `$KEYS` to pick a completer.
pub fn _bash_completions() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_bash_completions");
    let keys = getsparam("KEYS").unwrap_or_default();
    let key = keys.chars().last().unwrap_or(' ');

    // Each arm of sh:32-46's `case` writes its own `_main_complete` line, and
    // the line a diagnostic carries is the arm's, so carry it alongside the
    // argv rather than picking one for all five.
    let line: u64 = match key {
        '!' => 33,
        '$' => 35,
        '@' => 38,
        '/' => 40,
        '~' => 42,
        _ => 44,
    };
    let argv: Vec<String> = match key {
        '!' => vec!["_command_names".to_string()],
        '$' => vec![
            "-".to_string(),
            "parameters".to_string(),
            "_wanted".to_string(),
            "parameters".to_string(),
            "expl".to_string(),
            "exported parameter".to_string(),
            "_parameters".to_string(),
            "-g".to_string(),
            "*export*".to_string(),
        ],
        '@' => vec!["_hosts".to_string()],
        '/' => vec!["_files".to_string()],
        '~' => vec!["_users".to_string()],
        // sh:44 `*) _message "Key $key is not understood"` — the function's
        // status is `_message`'s, not a bare failure.
        _ => {
            // sh:44 is a COMMAND WORD like the others; `dispatch_action_command`
            // (shared.rs:1407) is `execcmd`'s resolution, ending in c:903's
            // `command not found` with c:908's 127 for a name that resolves
            // nowhere, where `.unwrap_or(1)` was silent.
            return dispatch_action_command(
                "_message",
                &[format!("Key {} is not understood", key)],
                line,
            )
        }
    };

    // sh:33/35/38/40/42 — the arm's own line is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    dispatch_action_command("_main_complete", &argv, line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn unknown_key_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("KEYS", "abc");
        assert_eq!(_bash_completions(), 1);
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("KEYS", "\\e!");
        assert_eq!(_bash_completions(), 1);
    }
}
