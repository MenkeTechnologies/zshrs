//! Port of `_jobs_fg` from `Completion/Zsh/Type/_jobs_fg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef disown fg
//! sh:2
//! sh:3  _jobs "$@"
//! ```

use crate::compsys::ported::shared::dispatch_action_command;

/// `_jobs_fg` — `fg` / `disown` completion: all jobs via plain
/// `_jobs`. Exit code = `_jobs` exit (1 when uncallable).
pub fn _jobs_fg(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jobs_fg");
    // sh:3 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    dispatch_action_command("_jobs", args, 3)
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
        assert_eq!(_jobs_fg(&[]), 127);
    }
}
