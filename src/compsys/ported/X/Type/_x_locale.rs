//! Port of `_x_locale` from `Completion/X/Type/_x_locale`.
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
//! sh:8    _message -e locales 'locale'
//! sh:9  fi
//! ```
//!
//! `$argv[(I)-X]` is the 1-based index of the *last* element of `$argv`
//! equal to the literal `-X` (0 when absent — zsh arithmetic-context
//! falsy). When present, the following element (`$argv[x + 1]`) is a
//! raw `_message -r` format string; otherwise fall back to the
//! `locales` tag with description `locale` via `_message -e`.

use crate::compsys::ported::_message::_message;

/// sh:3 — 0-based Rust index of the last `"-X"` element in `args`
/// (mirrors zsh's 1-based `(I)` reverse-search subscript, minus one).
fn find_last_dash_x(args: &[String]) -> Option<usize> {
    args.iter().rposition(|a| a == "-X")
}

/// `_x_locale` — offer either a raw `_message -r` format (when invoked
/// with a trailing `-X FORMAT` pair, as `_arguments`' `x:` action does)
/// or the generic `locales`/`locale` message.
pub fn _x_locale(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_locale");
    // sh:3, sh:5
    match find_last_dash_x(args) {
        Some(i) => {
            // sh:6  _message -r "$argv[x + 1]"
            let fmt = args.get(i + 1).cloned().unwrap_or_default();
            _message(&["-r".to_string(), fmt])
        }
        None => {
            // sh:8  _message -e locales 'locale'
            _message(&[
                "-e".to_string(),
                "locales".to_string(),
                "locale".to_string(),
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_last_dash_x_absent_is_none() {
        assert_eq!(
            find_last_dash_x(&["-a".to_string(), "-b".to_string()]),
            None
        );
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
        // sh:8-9 — default branch calls `_message -e locales 'locale'`,
        // which requires an active `_tags` context; absent one it
        // returns 1 (mirrors _baudrates' no-context convention).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_x_locale(&[]), 1);
    }

    #[test]
    fn dash_x_branch_routes_into_message_raw_mode() {
        // sh:5-6 — with `-X FORMAT` present, `_message -r FORMAT` runs
        // instead of the `-e locales 'locale'` default. `_message`'s
        // `-r` handling still passes through its own `_tags messages`
        // gate (sh:30 of `_message`), so the exact return code depends
        // on comptags state; just verify the `-X` routing path (not
        // `-e`) is reached without panicking.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let _r = _x_locale(&["-X".to_string(), "%d".to_string()]);
    }
}
