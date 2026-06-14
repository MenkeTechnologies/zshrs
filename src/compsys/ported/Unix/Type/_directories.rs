//! Port of `_directories` from `Completion/Unix/Type/_directories`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef dircmp -P -value-,*path,-default-
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted directories expl directory _files -/ "$@" -
//! ```
//!
//! `_files -/` — _files with the directory-only filter. _files is
//! a sibling shell fn (not ported); dispatches via `_wanted`'s
//! action-chunk path which routes through `exec accessors`.

use crate::compsys::ported::_wanted::_wanted;

/// `_directories` — directory-only completion via `_files -/`.
pub fn _directories(args: &[String]) -> i32 {
    // sh:5
    let mut wanted_argv: Vec<String> = vec![
        "directories".to_string(),
        "expl".to_string(),
        "directory".to_string(),
        "_files".to_string(),
        "-/".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _directories(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
