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

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};

/// `_precommand` — prefix-command completion: strip the prefix from
/// `$words` + `$CURRENT`, then run `_normal -p $service`.
pub fn _precommand() -> i32 {
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
    dispatch_function_call("_normal", &["-p".to_string(), service]).unwrap_or(1)
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
