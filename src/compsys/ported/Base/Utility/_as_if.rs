//! Port of `_as_if` from `Completion/Base/Utility/_as_if`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local words=("$words[@]") CURRENT=$CURRENT
//! sh: 3  local _comp_command1 _comp_command2 _comp_command
//! sh: 4
//! sh: 5  words[1]=("$@")
//! sh: 6  (( CURRENT += $# - 1 ))
//! sh: 7
//! sh: 8  _set_command
//! sh: 9
//! sh:10  _dispatch "$_comp_command" "$_comp_command1" "$_comp_command2" -default-
//! ```
//!
//! `words[1]=("$@")` REPLACES element 1 with the entire argv (zsh
//! array-element-as-array semantics — it splats). `CURRENT` shifts
//! by `$# - 1` to track the new position.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};

/// `_as_if` — re-dispatch completion AS IF the command line had
/// been replaced by `$@`. Used to simulate completion for a
/// different command (e.g. when `sudo` should defer to its first
/// non-option arg).
pub fn _as_if(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_as_if");
    // sh:2  snapshot words / CURRENT (we restore at end to mirror
    //   the shell's `local` scoping).
    let saved_words = getaparam("words").unwrap_or_default();
    let saved_current = getsparam("CURRENT").unwrap_or_default();

    // sh:5  words[1]=("$@") — replace element 1 with full argv,
    //   splatted in place (zsh array-element-is-array semantic).
    let mut new_words: Vec<String> = args.to_vec();
    if saved_words.len() > 1 {
        new_words.extend(saved_words.iter().skip(1).cloned());
    }
    setaparam("words", new_words);

    // sh:6  (( CURRENT += $# - 1 ))
    let cur: i64 = saved_current.parse().unwrap_or(0);
    let new_current = cur + (args.len() as i64) - 1;
    let _ = setsparam("CURRENT", &new_current.to_string());

    // sh:8  _set_command — sibling shell fn that derives
    //   `$_comp_command{,1,2}` from $words.
    let _ = dispatch_function_call("_set_command", &[]);

    // sh:10
    let cmd = getsparam("_comp_command").unwrap_or_default();
    let cmd1 = getsparam("_comp_command1").unwrap_or_default();
    let cmd2 = getsparam("_comp_command2").unwrap_or_default();
    let r = dispatch_function_call("_dispatch", &[cmd, cmd1, cmd2, "-default-".to_string()])
        .unwrap_or(1);

    // Restore shell-local scoping
    setaparam("words", saved_words);
    let _ = setsparam("CURRENT", &saved_current);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_as_if(&["foo".to_string()]), 1);
    }

    #[test]
    fn restores_words_and_current_after_call() {
        // sh:2 `local` scoping — we manually restore in Rust port.
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["original".to_string()]);
        let _ = setsparam("CURRENT", "5");
        let _ = _as_if(&["replacement".to_string()]);
        assert_eq!(getaparam("words").unwrap_or_default(), vec!["original"]);
        assert_eq!(getsparam("CURRENT").as_deref(), Some("5"));
    }
}
