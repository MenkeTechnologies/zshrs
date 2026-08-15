//! Port of `_set_command` from `Completion/Base/Utility/_set_command`.
//!
//! Full upstream body (31 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 6  local command
//! sh: 8  command="$words[1]"
//! sh:10  [[ -z "$command" ]] && return
//! sh:12  if (( $+builtins[$command] + $+functions[$command] )); then
//! sh:13    _comp_command1="$command"
//! sh:14    _comp_command="$_comp_command1"
//! sh:15  elif [[ "$command[1]" = '=' ]]; then
//! sh:16    eval _comp_command2\=$command
//! sh:17    _comp_command1="$command[2,-1]"
//! sh:18    _comp_command="$_comp_command2"
//! sh:19  elif [[ "$command" = ..#/* ]]; then
//! sh:20    _comp_command1="${PWD}/$command"
//! sh:21    _comp_command2="${command:t}"
//! sh:22    _comp_command="$_comp_command2"
//! sh:23  elif [[ "$command" = */* ]]; then
//! sh:24    _comp_command1="$command"
//! sh:25    _comp_command2="${command:t}"
//! sh:26    _comp_command="$_comp_command2"
//! sh:27  else
//! sh:28    _comp_command1="$command"
//! sh:29    _comp_command2="$commands[$command]"
//! sh:30    _comp_command="$_comp_command1"
//! sh:31  fi
//! ```
//!
//! Reads `$words[1]`, classifies the command (builtin/function vs
//! `=cmd` vs absolute path vs basename), and publishes the three
//! `_comp_command{,1,2}` params for downstream consumers.

use crate::ported::params::{getaparam, getsparam, setsparam};

/// Reach `_set_command` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_set_command` (Completion/Zsh/Context/_redirect sh:5) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_set_command_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _set_command() -> i32 {
    crate::compsys::ported::shared::call_compfn("_set_command", &[], || _set_command_impl())
}

/// `_set_command` — classify `$words[1]` and publish
/// `_comp_command`, `_comp_command1`, `_comp_command2`. Returns 0
/// on success, 1 when `$words[1]` is empty.
pub fn _set_command_impl() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_set_command");
    let words = getaparam("words").unwrap_or_default();
    // sh:8
    let command = words.first().cloned().unwrap_or_default();
    // sh:7
    if command.is_empty() {
        return 1;
    }

    // sh:12 — builtin OR function lookup (we approximate: check the
    //   shfunc table; builtins are also enumerable but we keep it
    //   simple, falling through to the path-classify branches when
    //   not a known function).
    let is_function = crate::ported::utils::getshfunc(&command).is_some();
    let is_builtin = is_known_builtin(&command);
    if is_function || is_builtin {
        // sh:13-14
        let _ = setsparam("_comp_command1", &command);
        let _ = setsparam("_comp_command", &command);
        return 0;
    }

    // sh:12  `=cmd` — `eval _comp_command2\=$command` runs equals-expansion,
    // which is the same `$PATH` command lookup `$commands[...]` performs
    // (`Src/Modules/parameter.c:213 getpmcommand`).
    if command.starts_with('=') {
        let bare = &command[1..];
        // c:Src/Modules/parameter.c:213 `getpmcommand` — the getfn
        // behind the `commands` special hash (`PARTAB` row
        // parameter.rs:4391). `commands` is NOT paramtab-hashed
        // storage, so the previous `getaparam("commands")` returned
        // None and every bare-name lookup here yielded "".
        // See the sh:29 note below for why the fetch's `loadparamnode`
        // side effect has to be requested explicitly.
        crate::vm_helper::mark_module_param_used("commands");
        let resolved: String =
            crate::ported::modules::parameter::getpmcommand(std::ptr::null_mut(), bare)
                .and_then(|pm| pm.u_str.clone())
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

    // sh:23 — `*/*` containing slash
    if command.contains('/') {
        let _ = setsparam("_comp_command1", &command);
        let tail = basename(&command);
        let _ = setsparam("_comp_command2", &tail);
        let _ = setsparam("_comp_command", &tail);
        return 0;
    }

    // sh:27-30 — bare name, lookup in $commands
    // c:Src/Modules/parameter.c:213 `getpmcommand` — same `commands`
    // special-hash getfn as the `=cmd` branch above. This is the value
    // `_redirect` (Completion/Zsh/Context/_redirect:13-14) puts FIRST in
    // `strs`, so an empty `_comp_command2` cost the full-path dispatch
    // key: zsh builds `-redirect-,<,/bin/cat`, zshrs built
    // `-redirect-,<,cat`.
    //
    // sh:29 writes `$commands[$command]`, so in zsh the subscript goes
    // through `fetchvalue` → `getparamnode` (Src/params.c:588-595) →
    // `loadparamnode` (c:563-585), which CLEARS the `zsh/parameter`
    // PM_AUTOLOAD stub and installs the real special node. That side
    // effect is observable: `local -A +h commands` preserves the special
    // only once the stub has been loaded, so `_command_names`'s
    // `local -A +h commands` behaves differently before and after this
    // line has run. `_normal` calls `_set_command` only when
    // `CURRENT != 1`, which is exactly why zsh's command-position
    // completion sees a plain local assoc and argument-position
    // completion sees the live command table. Calling the getfn
    // directly skips the fetch, so the load has to be requested here.
    crate::vm_helper::mark_module_param_used("commands");
    let cmd2: String =
        crate::ported::modules::parameter::getpmcommand(std::ptr::null_mut(), &command)
            .and_then(|pm| pm.u_str.clone())
            .unwrap_or_default();
    let _ = setsparam("_comp_command1", &command);
    let _ = setsparam("_comp_command2", &cmd2);
    let _ = setsparam("_comp_command", &command);
    0
}

/// sh:12's `$+builtins[$command]` test — consult the LIVE builtin table.
/// The previous port used a hardcoded ~34-name allowlist that misclassified
/// every builtin outside it (`print`, `typeset`, `zle`, `setopt`, `zstyle`,
/// `compadd`, `zmodload`, `whence`, `getopts`, …) as non-builtins, so
/// `_set_command` failed to recognise them as the command word. Bug #657.
fn is_known_builtin(name: &str) -> bool {
    crate::ported::builtin::createbuiltintable().contains_key(name)
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
        assert_eq!(_set_command_impl(), 1);
    }

    #[test]
    fn slash_path_uses_basename() {
        // sh:23-26
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["/usr/bin/ls".to_string()]);
        let _ = _set_command_impl();
        assert_eq!(getsparam("_comp_command1").as_deref(), Some("/usr/bin/ls"));
        assert_eq!(getsparam("_comp_command2").as_deref(), Some("ls"));
        assert_eq!(getsparam("_comp_command").as_deref(), Some("ls"));
    }

    #[test]
    fn known_builtin_takes_builtin_branch() {
        // sh:12-14
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["cd".to_string()]);
        let _ = _set_command_impl();
        assert_eq!(getsparam("_comp_command1").as_deref(), Some("cd"));
        assert_eq!(getsparam("_comp_command").as_deref(), Some("cd"));
    }

    // ========================================================
    // is_known_builtin — recognition matrix
    // ========================================================

    #[test]
    fn known_builtins_include_navigation_and_io() {
        for name in ["cd", "echo", "pwd", "printf", "read"] {
            assert!(
                is_known_builtin(name),
                "expected {} to register as builtin",
                name
            );
        }
    }

    #[test]
    fn known_builtins_include_set_and_unset_family() {
        for name in ["set", "unset", "export", "alias", "unalias"] {
            assert!(is_known_builtin(name), "{} missing from builtin set", name);
        }
    }

    #[test]
    fn known_builtins_include_control_flow() {
        for name in ["return", "break", "continue", "trap"] {
            assert!(is_known_builtin(name), "{} missing from builtin set", name);
        }
    }

    #[test]
    fn known_builtins_include_test_aliases() {
        // Both `test` and the bracketed form `[` are listed.
        assert!(is_known_builtin("test"));
        assert!(is_known_builtin("["));
    }

    #[test]
    fn known_builtins_includes_source_and_dot_alias() {
        // `source` and `.` are both registered as builtin per sh:9.
        assert!(is_known_builtin("source"));
        assert!(is_known_builtin("."));
    }

    #[test]
    fn unknown_command_is_not_a_builtin() {
        assert!(!is_known_builtin("ls"));
        assert!(!is_known_builtin("grep"));
        assert!(!is_known_builtin(""));
        assert!(!is_known_builtin("foo-bar-baz"));
    }

    #[test]
    fn builtin_name_is_case_sensitive() {
        // POSIX builtin lookup is case-sensitive — uppercase aliases
        // are NOT auto-recognized.
        assert!(!is_known_builtin("CD"));
        assert!(!is_known_builtin("Echo"));
    }

    // ========================================================
    // basename — `${command:t}` analog
    // ========================================================

    #[test]
    fn basename_strips_leading_directory_components() {
        assert_eq!(basename("/usr/bin/ls"), "ls");
        assert_eq!(basename("foo/bar/baz"), "baz");
    }

    #[test]
    fn basename_passes_through_when_no_slash() {
        assert_eq!(basename("plain"), "plain");
    }

    #[test]
    fn basename_empty_input_returns_empty() {
        assert_eq!(basename(""), "");
    }

    #[test]
    fn basename_trailing_slash_yields_empty_string() {
        // rsplit('/').next() on `"foo/"` returns the empty tail.
        assert_eq!(basename("foo/"), "");
    }

    #[test]
    fn basename_handles_root_only() {
        assert_eq!(basename("/"), "");
    }

    // ========================================================
    // _set_command — control-flow branches
    // ========================================================

    #[test]
    fn relative_dotdot_path_uses_pwd_prefix_and_basename() {
        // sh:16-19  command begins with `../`
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PWD", "/here");
        setaparam("words", vec!["../tools/runme".to_string()]);
        let _ = _set_command_impl();
        assert_eq!(
            getsparam("_comp_command1").as_deref(),
            Some("/here/../tools/runme")
        );
        assert_eq!(getsparam("_comp_command2").as_deref(), Some("runme"));
        assert_eq!(getsparam("_comp_command").as_deref(), Some("runme"));
    }
}
