//! Port of `_users_on` from `Completion/Unix/Type/_users_on`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #compdef write
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  if (( $+commands[users] )); then
//! sh: 6    _wanted users expl 'users logged on' \
//! sh: 7        compadd "$@" - $(_call_program users users) && return 0
//! sh: 8  else
//! sh: 9    # Other methods of finding out users logged on should be added here
//! sh:10    return 1
//! sh:11  fi
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam};

/// sh:5 — `(( $+commands[users] ))`: is `users` a hashed command?
fn has_command(name: &str) -> bool {
    getaparam("commands")
        .unwrap_or_default()
        .chunks(2)
        .any(|kv| kv.first().map(|k| k == name).unwrap_or(false))
}

/// `_users_on` — complete the names of users currently logged on, from
/// the output of the `users` command.
pub fn _users_on(args: &[String]) -> i32 {
    // sh:5
    if !has_command("users") {
        // sh:10
        return 1;
    }
    // sh:7  $(_call_program users users) — output lands in $REPLY.
    let _ = _call_program(&["users".to_string(), "users".to_string()]);
    let logged_on: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();
    // sh:6-7  _wanted users expl 'users logged on' compadd "$@" - $logged_on
    let mut w: Vec<String> = vec![
        "users".to_string(),
        "expl".to_string(),
        "users logged on".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(logged_on);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_users_command() {
        // sh:5,10 — no `commands[users]` hashed → return 1.
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam("commands", Vec::new());
        assert_eq!(_users_on(&[]), 1);
    }
}
