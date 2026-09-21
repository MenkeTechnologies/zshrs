//! Port of `_correct_word` from `Completion/Base/Widget/_correct_word`.
//!
//! Full upstream body (15 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xc
//! sh: 2
//! sh: 3  # Simple completion front-end implementing spelling correction.
//! sh: 4  # The maximum number of errors is set quite high, and
//! sh: 5  # the numeric prefix can be used to specify a different value.
//! sh: 6
//! sh: 7  local curcontext="$curcontext"
//! sh: 8
//! sh: 9  if [[ -z "$curcontext" ]]; then
//! sh:10    curcontext="correct-word:::"
//! sh:11  else
//! sh:12    curcontext="correct-word:${curcontext#*:}"
//! sh:13  fi
//! sh:14
//! sh:15  _main_complete _correct
//! ```

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::{getsparam, setsparam};

/// `_correct_word` — front-end widget for spell-correction
/// completion via `_main_complete _correct`.
pub fn _correct_word() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_correct_word");
    let saved = getsparam("curcontext").unwrap_or_default();
    let new_ctx = if saved.is_empty() {
        "correct-word:::".to_string()
    } else {
        let tail = saved.splitn(2, ':').nth(1).unwrap_or("");
        format!("correct-word:{}", tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    // sh:15 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    let r = dispatch_action_command("_main_complete", &["_correct".to_string()], 15);

    let _ = setsparam("curcontext", &saved);
    r
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
        assert_eq!(_correct_word(), 127);
    }
}
