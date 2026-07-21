//! Port of `_x_name` from `Completion/X/Type/_x_name`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local x="$argv[(I)-X]"
//! sh:4
//! sh:5  if (( x )); then
//! sh:6    _message -r "$argv[x + 1]"
//! sh:7  else
//! sh:8    _message -e names 'name'
//! sh:9  fi
//! ```
//!
//! `$argv[(I)-X]` is the 1-based index of the *last* element of `$argv`
//! equal to the literal `-X` (0 when absent — zsh arithmetic-context
//! falsy). When present, the following element (`$argv[x + 1]`) is a
//! raw `_message -r` format string; otherwise fall back to the
//! `names` tag with description `name` via `_message -e`.

use crate::compsys::ported::_message::_message;

/// sh:3 — 0-based Rust index of the last `"-X"` element in `args`
/// (mirrors zsh's 1-based `(I)` reverse-search subscript, minus one).
fn find_last_dash_x(args: &[String]) -> Option<usize> {
    args.iter().rposition(|a| a == "-X")
}

/// `_x_name` — offer either a raw `_message -r` format (when invoked
/// with a trailing `-X FORMAT` pair, as `_arguments`' `x:` action does)
/// or the generic `names`/`name` message.
pub fn _x_name(args: &[String]) -> i32 {
    // sh:3, sh:5
    match find_last_dash_x(args) {
        Some(i) => {
            // sh:6  _message -r "$argv[x + 1]"
            let fmt = args.get(i + 1).cloned().unwrap_or_default();
            _message(&["-r".to_string(), fmt])
        }
        None => {
            // sh:8  _message -e names 'name'
            _message(&[
                "-e".to_string(),
                "names".to_string(),
                "name".to_string(),
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_last_dash_x_absent_is_none() {
        assert_eq!(find_last_dash_x(&["-a".to_string(), "-b".to_string()]), None);
    }

    #[test]
    fn find_last_dash_x_finds_last_occurrence() {
        // sh:3 — `(I)` is a reverse search: the *last* matching index.
        let args = vec![
            "-X".to_string(),
            "first".to_string(),
            "-X".to_string(),
            "second".to_string(),
        ];
        assert_eq!(find_last_dash_x(&args), Some(2));
    }

    #[test]
    fn find_last_dash_x_trailing_with_no_value() {
        // sh:6 — `$argv[x + 1]` on an out-of-range zsh subscript expands
        // to empty; mirrored by `args.get(i + 1)` returning `None` ->
        // `unwrap_or_default()` = "".
        let args = vec!["-X".to_string()];
        assert_eq!(find_last_dash_x(&args), Some(0));
    }

    #[test]
    fn returns_one_without_completion_context() {
        // sh:8-9 — default branch calls `_message -e names 'name'`,
        // which requires an active `_tags` context; absent one it
        // returns 1 (mirrors _baudrates' no-context convention).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_x_name(&[]), 1);
    }

    #[test]
    fn dash_x_branch_routes_into_message_raw_mode() {
        // sh:5-6 — with `-X FORMAT` present, `_x_name` takes the raw
        // branch: `_message -r "$argv[x + 1]"`. That enters `_message`'s
        // DEFAULT (non `-e`) branch, whose `_tags messages || return 1`
        // gate (sh:30 in `_message`) runs BEFORE the `-r` raw handling
        // (sh:32) — raw mode does NOT bypass it. Without a completion
        // context the gate fails, so the call returns 1 (mirrors
        // `_message`'s own `default_mode_requires_messages_tag`).
        //
        // The `-e` else-branch (sh:8) would unconditionally set
        // `_comp_mesg=yes`; the raw branch returns at the gate before
        // sh:44, leaving it untouched. Asserting it stays cleared proves
        // routing hit the raw path, not the `-e names 'name'` fallback.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let _ = crate::ported::params::setsparam("_comp_mesg", "");
        assert_eq!(
            _x_name(&["-X".to_string(), "%d".to_string()]),
            1
        );
        assert_ne!(
            crate::ported::params::getsparam("_comp_mesg").as_deref(),
            Some("yes"),
            "raw `-r` branch must not run the `-e` mode's `_comp_mesg=yes` (sh:8)"
        );
    }
}
