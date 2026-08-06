//! Port of `_nothing` from `Completion/Base/Utility/_nothing`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef true false log times clear logname whoami sync
//! sh:2
//! sh:3  _message 'no argument or option'
//! ```

use crate::compsys::ported::_message::message_byname;

/// `_nothing` — emit a message that the command takes no args.
pub fn _nothing() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_nothing");
    message_byname(&["no argument or option".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_message_status() {
        // sh:3 — _message's exit depends on whether `messages` tag
        //   is requested; without state it returns 1.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _r = _nothing();
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
