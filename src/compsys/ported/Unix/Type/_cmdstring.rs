//! Port of `_cmdstring` from `Completion/Unix/Type/_cmdstring`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  # This is for a quoted argument that will be interpreted as a command.
//! sh:4
//! sh:5  compset -q
//! sh:6  _normal
//! ```

use crate::ported::exec::dispatch_function_call;
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

/// `_cmdstring` — completion for a quoted shell command argument.
/// Calls real `bin_compset -q` (unquote the current word into its
/// own context), then dispatches `_normal` (sibling shell fn).
pub fn _cmdstring() -> i32 {
    // sh:5  compset -q
    let _ = bin_compset("compset", &["-q".to_string()], &make_ops(), 0);
    // sh:6  _normal
    dispatch_function_call("_normal", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _cmdstring();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
