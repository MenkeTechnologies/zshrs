//! Port of `_shadow` / `_unshadow` from
//! `Completion/Base/Utility/_shadow`.
//!
//! Full upstream body (97 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:35  _shadow() {
//! sh:38    local -A fsfx=( -s … )
//! sh:42    zparseopts -K -A fsfx -D s:
//! sh:43    for fname; do
//! sh:44      shadowname=${fname}@${fsfx[-s]}
//! sh:45      if (( ${+functions[$shadowname]} )); then continue
//! sh:46      elif (( ${+functions[$fname]} )); then functions -c -- $fname $shadowname
//! sh:50      elif (( ${+builtins[$fname]} )); then alias-shadow
//! sh:53      else alias-cmd
//! sh:54      fi
//! sh:56    done
//! sh:60    builtin set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}
//! sh:62  }
//! sh:65  _unshadow() {
//! sh:73    while [[ ${.shadow.stack[1]?no shadows} != -- ]]; do
//! sh:74      fname=…; shadowname=…
//! sh:76      if function defined: unfunction; restore from shadowname
//! sh:88    done
//! sh:92    return
//! sh:94  ((.shadow.depth++/--))
//! sh:96  (( ARGC )) && _shadow … "$@"
//! ```
//!
//! The shadowing dance copies a function's body into a backup name,
//! then user redefines it. `_unshadow` restores. Faithful port
//! requires the shell's function table; we model a simplified
//! string-name stack in shell-side params.

use crate::ported::params::{getaparam, setaparam};

const STACK_PARAM: &str = ".shadow.stack";
const DEPTH_PARAM: &str = ".shadow.depth";

/// `_shadow` — push function names onto the shadow stack so a
/// subsequent `_unshadow` can pop them. The actual function-table
/// copy is delegated to the shell's `functions -c`; we record the
/// names for unshadow's bookkeeping.
pub fn _shadow(args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }
    let depth: i64 = getaparam(DEPTH_PARAM)
        .and_then(|v| v.first().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let suffix = format!("shadow_{}", depth + 1);

    let mut stack = getaparam(STACK_PARAM).unwrap_or_default();
    // Push: suffix + fnames + sentinel
    stack.insert(0, suffix);
    for f in args {
        stack.insert(1, format!("f@{}", f));
    }
    stack.insert(args.len() + 1, "--".to_string());
    setaparam(STACK_PARAM, stack);
    setaparam(DEPTH_PARAM, vec![(depth + 1).to_string()]);
    0
}

/// `_unshadow` — pop the topmost shadow-stack frame.
pub fn _unshadow() -> i32 {
    let mut stack = getaparam(STACK_PARAM).unwrap_or_default();
    if stack.is_empty() {
        return 1;
    }
    // Pop the suffix entry
    stack.remove(0);
    // Pop entries until we hit the sentinel `--`
    while !stack.is_empty() && stack[0] != "--" {
        stack.remove(0);
    }
    // Drop sentinel
    if !stack.is_empty() && stack[0] == "--" {
        stack.remove(0);
    }
    setaparam(STACK_PARAM, stack);
    let depth: i64 = getaparam(DEPTH_PARAM)
        .and_then(|v| v.first().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    setaparam(DEPTH_PARAM, vec![(depth - 1).max(0).to_string()]);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_then_unshadow_balances_depth() {
        let _g = crate::test_util::global_state_lock();
        setaparam(STACK_PARAM, Vec::new());
        setaparam(DEPTH_PARAM, Vec::new());
        let _ = _shadow(&["myfn".to_string()]);
        let depth1: i64 = getaparam(DEPTH_PARAM)
            .and_then(|v| v.first().and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        assert_eq!(depth1, 1);
        let _ = _unshadow();
        let depth2: i64 = getaparam(DEPTH_PARAM)
            .and_then(|v| v.first().and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        assert_eq!(depth2, 0);
    }

    #[test]
    fn unshadow_on_empty_stack_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setaparam(STACK_PARAM, Vec::new());
        setaparam(DEPTH_PARAM, Vec::new());
        assert_eq!(_unshadow(), 1);
    }
}
