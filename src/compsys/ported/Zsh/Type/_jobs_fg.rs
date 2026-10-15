//! Port of `_jobs_fg` from `Completion/Zsh/Type/_jobs_fg`.
//!
//! Full upstream body (master, 8 lines verbatim):
//! ```text
//! sh:1  #compdef disown fg
//! sh:2
//! sh:3  local -a ca_args=( "$@" )
//! sh:4  local -a args=( '(-)*: :_jobs' )
//! sh:5
//! sh:6  [[ $service == disown ]] && args+=( '(: *)-a[disown all jobs]' )
//! sh:7
//! sh:8  _arguments -s -S -A '-*' -O ca_args : $args
//! ```
//!
//! zsh 5.9.1/5.9.2's body was the single line `_jobs "$@"` (no `disown -a`,
//! upstream 54585); a drop-in run as `zsh` (emulating_592) keeps it.

use crate::compsys::ported::shared::dispatch_action_command;

/// `_jobs_fg` — `fg` / `disown` completion. Exit code = `_arguments`'
/// (master) or `_jobs`' (5.9.2) exit; 127 when the command word resolves
/// nowhere.
pub fn _jobs_fg(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jobs_fg");
    if crate::extensions::emulation_startup::emulating_592() {
        // 5.9.2 sh:3 — `_jobs "$@"`, a COMMAND WORD: `dispatch_action_command`
        // (shared.rs:1407) resolves it as `execcmd` does, ending in c:903's
        // `command not found` with c:908's 127.
        return dispatch_action_command("_jobs", args, 3);
    }
    // sh:3 — `local -a ca_args=( "$@" )`: `_arguments -O ca_args` reads it back
    // by name, so publish it as a function-local array.
    crate::compsys::ported::shared::declare_locals(&["ca_args"], crate::compsys::ported::shared::PM_ARRAY);
    crate::ported::params::setaparam("ca_args", args.to_vec());
    // sh:4-8
    let mut argv: Vec<String> = ["-s", "-S", "-A", "-*", "-O", "ca_args", ":", "(-)*: :_jobs"]
        .map(String::from)
        .to_vec();
    if crate::ported::params::getsparam("service").as_deref() == Some("disown") {
        argv.push("(: *)-a[disown all jobs]".to_string()); // sh:6
    }
    dispatch_action_command("_arguments", &argv, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_jobs_fg(&[]), 127);
    }
}
