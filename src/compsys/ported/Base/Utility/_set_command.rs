//! Port of `_set_command` from `Completion/Base/Utility/_set_command`.
//!
//! Full upstream body (31 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local command
//! sh: 6  command="$words[1]"
//! sh: 7  [[ -z "$command" ]] && return
//! sh: 9  if (( $+builtins[$command] + $+functions[$command] )); then
//! sh:10    _comp_command1="$command"
//! sh:11    _comp_command="$_comp_command1"
//! sh:12  elif [[ "$command[1]" = '=' ]]; then
//! sh:13    eval _comp_command2\=$command
//! sh:14    _comp_command1="$command[2,-1]"
//! sh:15    _comp_command="$_comp_command2"
//! sh:16  elif [[ "$command" = ..#/* ]]; then
//! sh:17    _comp_command1="${PWD}/$command"
//! sh:18    _comp_command2="${command:t}"
//! sh:19    _comp_command="$_comp_command2"
//! sh:20  elif [[ "$command" = */* ]]; then
//! sh:21    _comp_command1="$command"
//! sh:22    _comp_command2="${command:t}"
//! sh:23    _comp_command="$_comp_command2"
//! sh:24  else
//! sh:25    _comp_command1="$command"
//! sh:26    _comp_command2="$commands[$command]"
//! sh:27    _comp_command="$_comp_command1"
//! sh:28  fi
//! ```
//!
//! Reads `$words[1]`, classifies the command (builtin/function vs
//! `=cmd` vs absolute path vs basename), and publishes the three
//! `_comp_command{,1,2}` params for downstream consumers.

use crate::ported::params::{getaparam, getsparam, setsparam};

/// `_set_command` — classify `$words[1]` and publish
/// `_comp_command`, `_comp_command1`, `_comp_command2`. Returns 0
/// on success, 1 when `$words[1]` is empty.
pub fn _set_command() -> i32 {
    let words = getaparam("words").unwrap_or_default();
    // sh:6
    let command = words.first().cloned().unwrap_or_default();
    // sh:7
    if command.is_empty() {
        return 1;
    }

    // sh:9 — builtin OR function lookup (we approximate: check the
    //   shfunc table; builtins are also enumerable but we keep it
    //   simple, falling through to the path-classify branches when
    //   not a known function).
    let is_function = crate::ported::utils::getshfunc(&command).is_some();
    let is_builtin = is_known_builtin(&command);
    if is_function || is_builtin {
        // sh:10-11
        let _ = setsparam("_comp_command1", &command);
        let _ = setsparam("_comp_command", &command);
        return 0;
    }

    // sh:12  `=cmd` — expand to full path via the `commands` assoc
    if command.starts_with('=') {
        let bare = &command[1..];
        let commands = getaparam("commands").unwrap_or_default();
        // commands is a flat array of key/value pairs in our port —
        //   look up `bare` linearly.
        let resolved: String = commands
            .chunks(2)
            .find(|kv| kv.first().map(|k| k == bare).unwrap_or(false))
            .and_then(|kv| kv.get(1).cloned())
            .unwrap_or_default();
        let _ = setsparam("_comp_command2", &resolved);
        let _ = setsparam("_comp_command1", bare);
        let _ = setsparam("_comp_command", &resolved);
        return 0;
    }

    // sh:16 — `..*/...` relative path
    if let Some(stripped) = command.strip_prefix("..") {
        if stripped.starts_with('/') {
            let pwd = getsparam("PWD").unwrap_or_default();
            let _ = setsparam("_comp_command1", &format!("{}/{}", pwd, command));
            let tail = basename(&command);
            let _ = setsparam("_comp_command2", &tail);
            let _ = setsparam("_comp_command", &tail);
            return 0;
        }
    }

    // sh:20 — `*/*` containing slash
    if command.contains('/') {
        let _ = setsparam("_comp_command1", &command);
        let tail = basename(&command);
        let _ = setsparam("_comp_command2", &tail);
        let _ = setsparam("_comp_command", &tail);
        return 0;
    }

    // sh:24-27 — bare name, lookup in $commands
    let commands = getaparam("commands").unwrap_or_default();
    let cmd2: String = commands
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == &command).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
        .unwrap_or_default();
    let _ = setsparam("_comp_command1", &command);
    let _ = setsparam("_comp_command2", &cmd2);
    let _ = setsparam("_comp_command", &command);
    0
}

/// sh:9's `$+builtins[$command]` test. We treat any non-shfunc name
/// that looks like a well-known shell builtin as such; the
/// authoritative test would read the active builtins table.
fn is_known_builtin(name: &str) -> bool {
    matches!(
        name,
        "cd" | "echo" | "pwd" | "exit" | "set" | "unset" | "export" | "alias"
            | "unalias" | "source" | "." | "eval" | "exec" | "test" | "[" | "true"
            | "false" | "printf" | "read" | "shift" | "type" | "command"
            | "builtin" | "let" | "return" | "break" | "continue" | "trap"
            | "wait" | "kill" | "jobs" | "fg" | "bg" | "umask" | "ulimit"
    )
}

/// `${command:t}` — basename.
fn basename(s: &str) -> String {
    s.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setaparam;

    #[test]
    fn empty_words_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setaparam("words", Vec::new());
        assert_eq!(_set_command(), 1);
    }

    #[test]
    fn slash_path_uses_basename() {
        // sh:20-23
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["/usr/bin/ls".to_string()]);
        let _ = _set_command();
        assert_eq!(getsparam("_comp_command1").as_deref(), Some("/usr/bin/ls"));
        assert_eq!(getsparam("_comp_command2").as_deref(), Some("ls"));
        assert_eq!(getsparam("_comp_command").as_deref(), Some("ls"));
    }

    #[test]
    fn known_builtin_takes_builtin_branch() {
        // sh:9-11
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["cd".to_string()]);
        let _ = _set_command();
        assert_eq!(getsparam("_comp_command1").as_deref(), Some("cd"));
        assert_eq!(getsparam("_comp_command").as_deref(), Some("cd"));
    }
}
