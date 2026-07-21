//! Port of `_x_resource` from `Completion/X/Type/_x_resource`.
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
//! sh: 8    _message -e resources 'resource'
//! sh: 9  fi
//! ```
//!
//! `$argv[(I)-X]` is a reverse (last-match) index search: the `(I)`
//! subscript flag returns the index of the *last* element equal to
//! `-X`, or `0` if none is found. When found, the raw message text is
//! the element immediately following it (`$argv[x + 1]`), passed
//! straight through to `_message -r`. Otherwise fall back to the
//! `resources` tag's default `'resource'` message.

use crate::compsys::ported::_message::_message;

/// sh:3 — `$argv[(I)-X]`: 1-based index of the *last* `-X` element in
/// `args`, or `None` if absent (mirrors zsh's `0` == "not found").
fn last_index_of_dash_x(args: &[String]) -> Option<usize> {
    args.iter().rposition(|a| a == "-X")
}

/// `_x_resource` — complete/annotate an X toolkit `-xrm`-style
/// resource specification. If invoked with a `-X <message>` pair
/// (caller-supplied literal message text), relay it verbatim via
/// `_message -r`; otherwise emit the generic `'resource'` message
/// under the `resources` tag.
pub fn _x_resource(args: &[String]) -> i32 {
    // sh:3
    let x = last_index_of_dash_x(args);

    // sh:5-9
    match x {
        Some(idx) => {
            // sh:6  _message -r "$argv[x + 1]"
            //   0-based: the element right after the found `-X`.
            let msg = args.get(idx + 1).cloned().unwrap_or_default();
            _message(&["-r".to_string(), msg])
        }
        None => {
            // sh:8  _message -e resources 'resource'
            _message(&[
                "-e".to_string(),
                "resources".to_string(),
                "resource".to_string(),
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_last_dash_x_index() {
        // sh:3 — `(I)` searches for the LAST match.
        let args = vec![
            "-X".to_string(),
            "first".to_string(),
            "-X".to_string(),
            "second".to_string(),
        ];
        assert_eq!(last_index_of_dash_x(&args), Some(2));
    }

    #[test]
    fn no_dash_x_returns_none() {
        let args = vec!["-Y".to_string(), "val".to_string()];
        assert_eq!(last_index_of_dash_x(&args), None);
    }

    #[test]
    fn dash_x_relays_raw_message() {
        // sh:6 — with `-X <msg>` present, `_message -r <msg>` is
        //   invoked (the raw-message relay branch, distinct from the
        //   `-e resources` fallback). `_message`'s default mode runs
        //   `_tags messages || return 1` (_message sh:30) BEFORE it
        //   ever reaches the `-r` handling (sh:32). Outside a
        //   completion context `comptags` errors ("can only be called
        //   from completion function"), so `_tags` fails and `_message`
        //   returns 1 — matching `_message`'s own
        //   `default_mode_requires_messages_tag` test.
        let _g = crate::test_util::global_state_lock();
        let r = _x_resource(&["-X".to_string(), "custom text".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn no_dash_x_falls_back_to_resources_tag() {
        // sh:8 — without `-X`, falls back to `_message -e resources
        //   'resource'`, which returns 1 when no specs match (mirrors
        //   `_message`'s own `dash_e_with_no_specs_returns_one` test).
        let _g = crate::test_util::global_state_lock();
        let r = _x_resource(&["-Y".to_string(), "irrelevant".to_string()]);
        assert_eq!(r, 1);
    }
}
