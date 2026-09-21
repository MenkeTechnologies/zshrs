//! Port of `_precommand` from `Completion/Zsh/Command/_precommand`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh:1  #compdef - nohup eval time rusage noglob nocorrect catchsegv aoss hilite eatmydata
//! sh:2
//! sh:3  shift words
//! sh:4  (( CURRENT-- ))
//! sh:5
//! sh:6  _normal -p $service
//! ```
//!
//! `shift words` drops the first element of the `$words` array;
//! `CURRENT--` decrements the cursor position. Then dispatches
//! `_normal -p $service` (treats remaining argv as a fresh command
//! invocation).

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};

/// `_precommand` — prefix-command completion: strip the prefix from
/// `$words` + `$CURRENT`, then run `_normal -p $service`.
pub fn _precommand() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_precommand");
    // sh:3  shift words
    if let Some(mut words) = getaparam("words") {
        if !words.is_empty() {
            words.remove(0);
            setaparam("words", words);
        }
    }
    // sh:4  (( CURRENT-- ))
    let current: i64 = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let _ = setsparam("CURRENT", &(current - 1).to_string());

    // sh:6  _normal -p $service
    let service = getsparam("service").unwrap_or_default();
    // sh:6 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    dispatch_action_command("_normal", &["-p".to_string(), service], 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_words_and_decrements_current() {
        let _g = crate::test_util::global_state_lock();
        setaparam(
            "words",
            vec!["nohup".to_string(), "ls".to_string(), "-la".to_string()],
        );
        let _ = setsparam("CURRENT", "3");
        let _r = _precommand();
        let words = getaparam("words").unwrap_or_default();
        let current = getsparam("CURRENT").unwrap_or_default();
        assert_eq!(words, vec!["ls", "-la"]);
        assert_eq!(current, "2");
    }
}
