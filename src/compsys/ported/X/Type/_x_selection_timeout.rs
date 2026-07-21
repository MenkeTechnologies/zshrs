//! Port of `_x_selection_timeout` from `Completion/X/Type/_x_selection_timeout`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local x="$argv[(I)-X]"
//! sh: 4
//! sh: 5  if (( x )); then
//! sh: 6    _message -r "$argv[x + 1]"
//! sh: 7  else
//! sh: 8    _message -e values 'selection timeout'
//! sh: 9  fi
//! ```
//!
//! `$argv[(I)-X]` is the zsh reverse-index-find operator: the highest
//! index `i` such that `$argv[i]` equals the literal string `-X`, or 0
//! if absent. When present, the element immediately following `-X` is
//! forwarded verbatim as the raw message text to `_message -r`.

use crate::compsys::ported::_message::_message;

/// sh:3 — `$argv[(I)-X]`: highest 1-based index of the literal `-X` in
/// `args`, or `None` if absent (zsh: `0`, falsy in `(( x ))`).
fn find_dash_x_index(args: &[String]) -> Option<usize> {
    args.iter().rposition(|a| a == "-X")
}

/// `_x_selection_timeout` — complete an X selection-timeout value: if
/// `-X MSG` was passed in `$@`, emit `MSG` verbatim as the completion
/// message; otherwise fall back to the generic "selection timeout"
/// values message.
pub fn _x_selection_timeout(args: &[String]) -> i32 {
    // sh:3
    let x = find_dash_x_index(args);

    // sh:5-9
    if let Some(i) = x {
        // sh:6  _message -r "$argv[x + 1]"
        let msg = args.get(i + 1).cloned().unwrap_or_default();
        _message(&["-r".to_string(), msg])
    } else {
        // sh:8  _message -e values 'selection timeout'
        _message(&[
            "-e".to_string(),
            "values".to_string(),
            "selection timeout".to_string(),
        ])
    }
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
    fn find_dash_x_index_returns_last_match() {
        // sh:3 — `(I)` is reverse search: last occurrence wins.
        let args = vec![
            "-X".to_string(),
            "first".to_string(),
            "-X".to_string(),
            "second".to_string(),
        ];
        assert_eq!(find_dash_x_index(&args), Some(2));
    }

    #[test]
    fn find_dash_x_index_none_when_absent() {
        let args = vec!["-Y".to_string(), "val".to_string()];
        assert_eq!(find_dash_x_index(&args), None);
    }

    #[test]
    fn dash_x_present_routes_to_message_raw_mode() {
        // sh:5-6 — with `-X` present, `_message -r` is invoked forwarding
        //   the raw text. No `messages` tag registered -> `_tags messages
        //   || return 1` inside `_message` (sh:30) fires.
        let r = with_incompfunc(|| {
            _x_selection_timeout(&["-X".to_string(), "custom timeout".to_string()])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn dash_x_absent_routes_to_message_dash_e_values() {
        // sh:8 — falls back to `_message -e values 'selection timeout'`,
        //   which (per `_message` sh:24) returns 1 without any
        //   registered `values` spec/completion context.
        let r = with_incompfunc(|| _x_selection_timeout(&[]));
        assert_eq!(r, 1);
    }
}
