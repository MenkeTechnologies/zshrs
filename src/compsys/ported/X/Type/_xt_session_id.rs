//! Port of `_xt_session_id` from `Completion/X/Type/_xt_session_id`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  _message -e ids 'session ID'
//! ```

use crate::compsys::ported::_message::_message;

/// `_xt_session_id` — complete an X toolkit session-management ID:
/// unconditionally delegates to `_message -e ids 'session ID'` (sh:3).
pub fn _xt_session_id(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_xt_session_id");
    // sh:3
    _message(&[
        "-e".to_string(),
        "ids".to_string(),
        "session ID".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// `_message`/`_tags` guard their underlying builtins on
    /// `INCOMPFUNC == 1`; mirror the sibling ports' test convention.
    fn with_incompfunc<F: FnOnce() -> i32>(f: F) -> i32 {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn delegates_to_message_dash_e_ids() {
        // sh:3 — `_message -e ids 'session ID'`. Without a registered
        //   `ids` tag/completion context, `_message`'s `-e` mode
        //   (sh:24 in `_message`) returns 1 (initial `ret=1`, never
        //   flipped since `_next_label` yields no matches).
        let r = with_incompfunc(|| _xt_session_id(&[]));
        // `_message` registers its tag at its OWN nesting level (comptags is
        // indexed by locallevel), so it succeeds and returns 0 even with no
        // tag offered by a caller — verified against `zsh -f` + compinit,
        // where a completer body of `_message -e titles t; print rc=$?`
        // prints `rc=0`.
        assert_eq!(r, 0);
    }

    #[test]
    fn ignores_extraneous_args() {
        // sh:3 — args passed to `_xt_session_id` itself are irrelevant;
        //   it never inspects `$@`, always calling the fixed
        //   `_message -e ids 'session ID'` invocation.
        let r = with_incompfunc(|| _xt_session_id(&["-X".to_string(), "ignored".to_string()]));
        // `_message` registers its tag at its OWN nesting level (comptags is
        // indexed by locallevel), so it succeeds and returns 0 even with no
        // tag offered by a caller — verified against `zsh -f` + compinit,
        // where a completer body of `_message -e titles t; print rc=$?`
        // prints `rc=0`.
        assert_eq!(r, 0);
    }

    #[test]
    fn sets_comp_mesg_flag() {
        // `_message` sh:8 sets `_comp_mesg=yes` unconditionally in
        //   `-e` mode, regardless of match outcome.
        let _ = with_incompfunc(|| {
            let _ = crate::ported::params::setsparam("_comp_mesg", "");
            _xt_session_id(&[])
        });
        assert_eq!(
            crate::ported::params::getsparam("_comp_mesg").as_deref(),
            Some("yes")
        );
    }
}
