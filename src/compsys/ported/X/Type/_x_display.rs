//! Port of `_x_display` from `Completion/X/Type/_x_display`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef -value-,DISPLAY,-default-
//! sh: 3  _tags displays && _hosts -S ':0 ' -r :
//! ```
//!
//! `_tags displays && _hosts -S ':0 ' -r :` registers the `displays`
//! tag with `_tags`, and only on success (tag accepted for this
//! context) delegates to `_hosts` to offer hostnames suffixed with
//! `:0 ` (the default X display/screen number) with `:` treated as an
//! ignored-character (`-r`) separator so completion doesn't stop at
//! the trailing colon.

use crate::compsys::ported::_hosts::_hosts;
use crate::compsys::ported::_tags::_tags;

/// `_x_display` — complete an X `DISPLAY` value (`host:0`) via `_hosts`,
/// gated on the `displays` tag being accepted by `_tags`.
pub fn _x_display(args: &[String]) -> i32 {
    let _ = args; // sh: the shell fn body never references "$@".

    // sh:3  _tags displays && _hosts -S ':0 ' -r :
    let tags_ret = _tags(&["displays".to_string()]);
    if tags_ret != 0 {
        return tags_ret;
    }
    _hosts(&[
        "-S".to_string(),
        ":0 ".to_string(),
        "-r".to_string(),
        ":".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// `_tags`/`_hosts` guard their underlying builtins/context on
    /// `INCOMPFUNC == 1`; mirror the sibling ports' test convention.
    fn with_incompfunc<T, F: FnOnce() -> T>(f: F) -> T {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn short_circuits_on_tags_failure_without_calling_hosts() {
        // sh:3 `&&` — with no completion context wired, `_tags displays`
        // (argv is non-empty, so this exercises the registration path,
        // not the sh:67 `-N` next-set path) and `_x_display` must
        // propagate whatever `_tags` returns in this same environment
        // without reaching `_hosts` (which would otherwise touch the
        // filesystem / spawn getent). Compared within a single
        // `with_incompfunc` invocation so both calls observe identical
        // guard/context state.
        let (x_display_ret, tags_ret) = with_incompfunc(|| {
            let x = _x_display(&[]);
            let t = _tags(&["displays".to_string()]);
            (x, t)
        });
        assert_eq!(x_display_ret, tags_ret);
    }

    #[test]
    fn ignores_extra_args_never_forwarded_upstream() {
        // sh: the shell function body never references "$@" — any args
        // passed to `_x_display` are dropped, matching upstream. Both
        // calls run inside one `with_incompfunc` invocation so guard
        // state is identical for the comparison.
        let (r1, r2) = with_incompfunc(|| {
            let a = _x_display(&[]);
            let b = _x_display(&["ignored".to_string(), "-also-ignored".to_string()]);
            (a, b)
        });
        assert_eq!(r1, r2);
    }
}
