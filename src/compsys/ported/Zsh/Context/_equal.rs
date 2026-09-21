//! Port of `_equal` from `Completion/Zsh/Context/_equal`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #compdef -equal-
//! sh: 2
//! sh: 3  local -a match mbegin mend
//! sh: 4
//! sh: 5  if _have_glob_qual $PREFIX; then
//! sh: 6    compset -p ${#match[1]}
//! sh: 7    compset -S '[^\)\|\~]#(|\))'
//! sh: 8    _globquals
//! sh: 9  else
//! sh:10    _path_commands
//! sh:11  fi
//! ```
//!
//! `_have_glob_qual`, `_globquals`, `_path_commands` are sibling
//! shell fns — dispatch via `exec accessors`. `compset` calls go to the
//! real builtin.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_equal` — `=cmd` / `=(...)` context completion: glob-qualifier
/// expansion if `_have_glob_qual` matches, otherwise path-command
/// completion.
pub fn _equal() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_equal");
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:5  if _have_glob_qual $PREFIX
    // sh:5 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    if dispatch_action_command("_have_glob_qual", &[prefix], 5) == 0 {
        // sh:6  compset -p ${#match[1]}
        let match_arr = getaparam("match").unwrap_or_default();
        let match1_len = match_arr.first().map(|s| s.len()).unwrap_or(0);
        let _ = bin_compset(
            "compset",
            &["-p".to_string(), match1_len.to_string()],
            &make_ops(),
            0,
        );
        // sh:7  compset -S '[^\)\|\~]#(|\))'
        let _ = bin_compset(
            "compset",
            &["-S".to_string(), "[^\\)\\|\\~]#(|\\))".to_string()],
            &make_ops(),
            0,
        );
        // sh:8
        // sh:8 is a COMMAND WORD, so `dispatch_action_command`
        // (shared.rs:1407) resolves it exactly as `execcmd` does:
        // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
        // `$PATH`, then c:903's `command not found` with c:908's 127. The
        // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
        // that resolved nowhere returned in silence.
        dispatch_action_command("_globquals", &[], 8)
    } else {
        // sh:10
        // sh:10 is a COMMAND WORD, so `dispatch_action_command`
        // (shared.rs:1407) resolves it exactly as `execcmd` does:
        // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
        // `$PATH`, then c:903's `command not found` with c:908's 127. The
        // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
        // that resolved nowhere returned in silence.
        dispatch_action_command("_path_commands", &[], 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127. It used to return a silent 1.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_equal(), 127);
    }
}
