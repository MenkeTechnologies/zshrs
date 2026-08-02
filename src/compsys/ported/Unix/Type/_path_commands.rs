//! Port of `_path_commands` from
//! `Completion/Unix/Type/_path_commands`.
//!
//! Full upstream body (125 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  3  _path_commands_caching_policy() { … cache age check … }
//! sh: 20  _call_whatis() { … whatis -s 1,6,8 OS-dependent … }
//! sh: 51  _path_commands() {
//! sh: 54    if zstyle -t … commands extra-verbose; then
//! sh: 55-90  build $_command_descriptions assoc from whatis, emit
//! sh: 81      _wanted commands expl 'external command' compadd -ld descs -a dcmds
//! sh: 83      _wanted commands expl 'external command' compadd "$@" -a cmds
//! sh: 91    else
//! sh: 92      _wanted commands expl 'external command' compadd "$@" -k commands
//! sh: 93    fi
//! sh:124  }
//! ```
//!
//! The whatis-cache descriptive-display path is heavy; this port
//! implements the common (non-extra-verbose) leg: enumerate
//! `$commands` (the shell-side hashed-commands assoc) via `_wanted
//! commands expl 'external command' compadd -k commands`.

use crate::compsys::ported::_wanted::_wanted;

/// `_path_commands` — complete names of external commands from
/// `$commands` (zsh's PATH cache).
pub fn _path_commands(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_path_commands");
    // sh:92 (skip extra-verbose branch)
    let mut wanted_argv: Vec<String> = vec![
        "commands".to_string(),
        "expl".to_string(),
        "external command".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-k".to_string());
    wanted_argv.push("commands".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        assert_eq!(_path_commands(&[]), 1);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
