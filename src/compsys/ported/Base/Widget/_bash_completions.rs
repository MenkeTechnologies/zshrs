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

    /// Run `_bash_completions` with `$KEYS` = `keys`, published the way a
    /// completion call publishes it, and return its status plus whatever it
    /// wrote to fd 2.
    ///
    /// `$KEYS` is read-only to scripts (zle_params.c:152), so it cannot be
    /// assigned; it comes from `keybuf` through `makezleparams` — called with
    /// `ro = 1` at compcore.c:820 for every completion function — inside the
    /// scope `endparamscope` closes again (c:839).
    ///
    /// No executor runs under `cargo test`, so neither `_main_complete` nor
    /// `_message` resolves: every arm ends in c:903's `command not found` with
    /// c:908's 127, and the diagnostic carries the ARM's own `sh:` line. That
    /// line is the observable that tells the arms apart.
    fn run_with_keys(keys: &[u8]) -> (i32, String) {
        use std::io::Read;

        *crate::ported::zle::zle_keymap::keybuf.lock().unwrap() = keys.to_vec();
        crate::ported::utils::inc_locallevel(); // c:startparamscope
        crate::ported::zle::zle_params::makezleparams(1); // c:820

        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe failed");
        let saved_stderr = unsafe { libc::dup(2) };
        assert!(saved_stderr >= 0, "dup(2) failed");
        assert!(unsafe { libc::dup2(fds[1], 2) } >= 0, "dup2 onto fd 2 failed");

        let status = _bash_completions();

        unsafe {
            libc::dup2(saved_stderr, 2);
            libc::close(saved_stderr);
            libc::close(fds[1]);
        }
        let mut err = String::new();
        let mut reader = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fds[0]) };
        let _ = reader.read_to_string(&mut err);

        crate::ported::params::endparamscope(); // c:839
        crate::ported::utils::errflag.store(0, std::sync::atomic::Ordering::Relaxed);
        (status, err)
    }

    /// sh:44 — a key the `case` does not list goes to `_message`, not to
    /// `_main_complete`.
    #[test]
    fn unknown_key_takes_the_message_arm() {
        let _g = crate::test_util::global_state_lock();
        let _z = crate::ported::zle::zle_main::zle_test_setup();
        let (status, err) = run_with_keys(b"abc");
        assert_eq!(status, 127, "c:908 — `_message` resolves nowhere without an executor");
        assert!(
            err.contains(":44: command not found: _message"),
            "the last key `c` is not understood, so sh:44 runs `_message`; got `{err}`"
        );
    }

    /// sh:33 — `ESC !` completes command names through `_main_complete`.
    #[test]
    fn bang_key_takes_the_command_names_arm() {
        let _g = crate::test_util::global_state_lock();
        let _z = crate::ported::zle::zle_main::zle_test_setup();
        let (status, err) = run_with_keys(b"\x1b!");
        assert_eq!(status, 127, "c:908 — `_main_complete` resolves nowhere without an executor");
        assert!(
            err.contains(":33: command not found: _main_complete"),
            "the last key `!` selects sh:33; got `{err}`"
        );
    }
}
