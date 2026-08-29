//! Port of `_x_visual` from `Completion/X/Type/_x_visual`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 6  local best="${argv[(r)-b]:+Best}"
//! sh: 7  argv[(i)-b]=()
//! sh: 9  _wanted visuals expl visual compadd "$@" -M 'm:{a-zA-Z}={A-Za-z}' - \
//! sh:10      $best DirectColor TrueColor PseudoColor StaticColor GrayScale StaticGray
//! ```

use crate::compsys::ported::_wanted::_wanted;

/// sh:6-7 — with `-b` present in `argv`, offer `Best` too; strip the
/// first (and, per `(i)` index-removal semantics, only the first)
/// literal `-b` from the argv that gets forwarded to `compadd`.
fn strip_dash_b(args: &[String]) -> (bool, Vec<String>) {
    let mut best = false;
    let mut rest = Vec::with_capacity(args.len());
    let mut removed = false;
    for a in args {
        if !removed && a == "-b" {
            best = true;
            removed = true;
            continue;
        }
        rest.push(a.clone());
    }
    (best, rest)
}

/// `_x_visual` — complete X visual-class names (`TrueColor`,
/// `PseudoColor`, …), optionally including `Best` when `-b` is given.
pub fn _x_visual(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_visual");
    // sh:6-7
    let (best, rest) = strip_dash_b(args);

    // sh:9-10  _wanted visuals expl visual compadd "$@" -M '...' - $best VISUALS...
    let mut w: Vec<String> = vec![
        "visuals".to_string(),
        "expl".to_string(),
        "visual".to_string(),
        "compadd".to_string(),
    ];
    w.extend(rest);
    w.push("-M".to_string());
    w.push("m:{a-zA-Z}={A-Za-z}".to_string());
    w.push("-".to_string());
    if best {
        w.push("Best".to_string());
    }
    for v in [
        "DirectColor",
        "TrueColor",
        "PseudoColor",
        "StaticColor",
        "GrayScale",
        "StaticGray",
    ] {
        w.push(v.to_string());
    }
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_dash_b_removes_only_first_occurrence() {
        let (best, rest) = strip_dash_b(&[
            "-J".to_string(),
            "grp".to_string(),
            "-b".to_string(),
            "-b".to_string(),
        ]);
        assert!(best);
        // sh:7 — `argv[(i)-b]=()` removes a single array slot (the
        // first index match), leaving any later "-b" untouched.
        assert_eq!(
            rest,
            vec!["-J".to_string(), "grp".to_string(), "-b".to_string()]
        );
    }

    #[test]
    fn strip_dash_b_absent_leaves_args_untouched() {
        let (best, rest) = strip_dash_b(&["-J".to_string(), "grp".to_string()]);
        assert!(!best);
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    /// `_wanted` registers its OWN tag, so the "without registered tags"
    /// premise never holds: with the `doshfunc`-frame shift (see
    /// `Base/Core/_wanted.rs:45-54`) `_wanted` registers and `_all_labels`
    /// adds matches, making 0 the correct return. Confirmed against real
    /// zsh 5.9.2 driven through a PTY inside a live completion widget —
    /// all of these return 0, not 1. The old name and `assert_eq!(r, 1)`
    /// encoded the pre-shift answer.
    ///
    /// `reset_completion_state` is what makes that answer STABLE. The
    /// assertion used to pass alone and fail inside a full run because a
    /// leftover `$PREFIX` from an earlier test filtered out every candidate
    /// `compadd` was offered, so `compadd` returned 1 for a tag set that had
    /// been registered perfectly well — see that helper for the mechanism.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _x_visual(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 0);
    }
}
