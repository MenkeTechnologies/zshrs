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

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getsparam;

/// `_bash_completions` — Bash-style keybinding completion router.
/// Reads the last char of `$KEYS` to pick a completer.
pub fn _bash_completions() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_bash_completions");
    let keys = getsparam("KEYS").unwrap_or_default();
    let key = keys.chars().last().unwrap_or(' ');

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
        _ => return 1,
    };

    dispatch_function_call("_main_complete", &argv).unwrap_or(1)
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
