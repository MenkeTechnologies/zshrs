//! Port of `_sub_commands` from `Completion/Base/Utility/_sub_commands`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  if [[ CURRENT -eq 2 ]]; then
//! sh:6    _wanted commands expl command compadd "$@"
//! sh:7  else
//! sh:8    _message 'no more arguments'
//! sh:9  fi
//! ```

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;

/// `_sub_commands` — complete the FIRST sub-command argument only
/// (CURRENT == 2 in 1-based shell terms); emit a "no more arguments"
/// message thereafter.
pub fn _sub_commands(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_sub_commands");
    // sh:5
    let current: i64 = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if current == 2 {
        // sh:6
        let mut wanted_argv: Vec<String> = vec![
            "commands".to_string(),
            "expl".to_string(),
            "command".to_string(),
            "compadd".to_string(),
        ];
        wanted_argv.extend(args.iter().cloned());
        _wanted(&wanted_argv)
    } else {
        // sh:8
        _message(&["no more arguments".to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn current_two_takes_wanted_path() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = setsparam("CURRENT", "2");
        let r = _sub_commands(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1); // no tags → _wanted returns 1
    }

    #[test]
    fn current_other_emits_message() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = setsparam("CURRENT", "5");
        let _r = _sub_commands(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
