//! Port of `_requested` (zsh Completion/Base/Core/_requested, 18 lines).
//!
//! Upstream shell source: `/opt/homebrew/share/zsh/functions/_requested`
//! (system; vendored tree `compsys/functions/Base/Core/_requested` mirrors
//! the same file).
//!
//! Shell source (verbatim):
//! ```text
//! #autoload
//!
//! local __gopt
//!
//! __gopt=()
//! zparseopts -D -a __gopt 1 2 V J x
//!
//! if comptags -R "$1"; then
//!   if [[ $# -gt 3 ]]; then
//!     _all_labels - "$__gopt[@]" "$@" || return 1
//!   elif [[ $# -gt 1 ]]; then
//!     _description "$__gopt[@]" "$@"
//!   fi
//!   return 0
//! else
//!   return 1
//! fi
//! ```
//!
//! Three-clause dispatcher:
//!   - With 1 positional arg → bare "is this tag wanted?" check (the
//!     mode used by callers like `_files` and `_arguments` when they
//!     just want the gate without emitting matches).
//!   - With 2-3 positional args → call `_description` (gate + describe).
//!   - With ≥4 positional args → call `_all_labels` (gate + emit via
//!     the all-labels loop, which fans out across `tag-order` labels).
//!
//! Flag passthrough: `-1`, `-2`, `-V`, `-J`, `-x` are zparseopts'd
//! into `__gopt` and forwarded to `_all_labels` / `_description`. This
//! port mirrors that — parsed flags are returned alongside the gate
//! result so callers can forward them.
//!
//! The core "is tag wanted" decision is already implemented by
//! [`crate::base::TagManager::requested`] (see `compsys/base.rs:147`);
//! this port wraps it with the documented flag-parse + arg-count
//! dispatch semantics.

use crate::base::TagManager;

/// Flags parsed out of the leading `__gopt` zparseopts call.
///
/// Mirrors `zparseopts -D -a __gopt 1 2 V J x` exactly — these are
/// pass-through flags that `_description` / `_all_labels` recognise.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestedFlags {
    /// `-1` — `_description`/`_all_labels` "merge with previous group" flag.
    pub one: bool,
    /// `-2` — sibling of `-1`, similar mode.
    pub two: bool,
    /// `-V name` — explicit group name (carries an arg).
    pub v_group: Option<String>,
    /// `-J name` — sorted-group name (carries an arg).
    pub j_group: Option<String>,
    /// `-x` — expand (no arg).
    pub x: bool,
}

/// What `_requested` decided the caller should do next, based on
/// positional arg count after the leading tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestedAction {
    /// 1 positional arg (just the tag) — bare gate. Caller does the
    /// rest. The return tuple's `bool` says whether the tag is wanted.
    GateOnly,
    /// 2-3 positional args — caller should invoke `_description` with
    /// the parsed flags + the remaining args (`name`, optional
    /// `description`, … pass-through).
    Describe,
    /// ≥4 positional args — caller should invoke `_all_labels` with
    /// the flags + the remaining args.
    AllLabels,
}

/// _requested — parse `-1 -2 -V J -x` flags, then ask the tag manager.
///
/// `args` is the argv after the function name (so `args[0]` is the
/// tag). Returns `None` when the tag is NOT in the current try-set
/// (mirrors the upstream `comptags -R "$1"; … else return 1` branch).
///
/// Returns `Some((flags, action, positional))` when the tag IS active.
/// `positional` excludes the parsed flags; `positional[0]` is the tag.
pub fn _requested(
    tags: &mut TagManager,
    args: &[String],
) -> Option<(RequestedFlags, RequestedAction, Vec<String>)> {
    let mut flags = RequestedFlags::default();
    let mut positional: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-1" => flags.one = true,
            "-2" => flags.two = true,
            "-x" => flags.x = true,
            "-V" => {
                if i + 1 < args.len() {
                    flags.v_group = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-J" => {
                if i + 1 < args.len() {
                    flags.j_group = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let tag = match positional.first() {
        Some(t) => t.clone(),
        None => return None,
    };

    if !tags.requested(&tag) {
        return None;
    }

    let action = if positional.len() > 3 {
        RequestedAction::AllLabels
    } else if positional.len() > 1 {
        RequestedAction::Describe
    } else {
        RequestedAction::GateOnly
    };

    Some((flags, action, positional))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn make_tags() -> TagManager {
        let mut tm = TagManager::new();
        tm.init(&["files".to_string(), "directories".to_string()]);
        tm.add_try(&["files".to_string()]);
        assert!(tm.start());
        tm
    }

    #[test]
    fn returns_none_for_inactive_tag() {
        let mut tm = make_tags();
        assert!(_requested(&mut tm, &s(&["directories"])).is_none());
    }

    #[test]
    fn one_arg_is_gate_only() {
        let mut tm = make_tags();
        let (flags, action, pos) = _requested(&mut tm, &s(&["files"])).unwrap();
        assert_eq!(action, RequestedAction::GateOnly);
        assert_eq!(pos, vec!["files".to_string()]);
        assert_eq!(flags, RequestedFlags::default());
    }

    #[test]
    fn two_or_three_args_is_describe() {
        let mut tm = make_tags();
        let (_, action, _) = _requested(&mut tm, &s(&["files", "expl"])).unwrap();
        assert_eq!(action, RequestedAction::Describe);
        let mut tm2 = make_tags();
        let (_, action2, _) = _requested(&mut tm2, &s(&["files", "expl", "files"])).unwrap();
        assert_eq!(action2, RequestedAction::Describe);
    }

    #[test]
    fn four_plus_args_is_all_labels() {
        let mut tm = make_tags();
        let (_, action, pos) =
            _requested(&mut tm, &s(&["files", "expl", "files", "_files", "-/"])).unwrap();
        assert_eq!(action, RequestedAction::AllLabels);
        assert_eq!(pos.len(), 5);
    }

    #[test]
    fn parses_v_and_j_with_args_and_flag_booleans() {
        let mut tm = make_tags();
        let (flags, _, pos) =
            _requested(&mut tm, &s(&["-1", "-V", "grpV", "-x", "files", "expl"])).unwrap();
        assert!(flags.one);
        assert!(!flags.two);
        assert!(flags.x);
        assert_eq!(flags.v_group, Some("grpV".to_string()));
        assert_eq!(flags.j_group, None);
        // Flag-args are eaten — tag is still positional[0].
        assert_eq!(pos[0], "files");
        assert_eq!(pos[1], "expl");
    }

    #[test]
    fn empty_args_returns_none() {
        let mut tm = make_tags();
        assert!(_requested(&mut tm, &[]).is_none());
    }

    #[test]
    fn flags_only_no_positional_returns_none() {
        // Just flags, no tag → first positional doesn't exist.
        let mut tm = make_tags();
        assert!(_requested(&mut tm, &s(&["-1", "-2", "-x"])).is_none());
    }

    #[test]
    fn dash_J_carries_group_name_argument() {
        let mut tm = make_tags();
        let (flags, _, _) =
            _requested(&mut tm, &s(&["-J", "sortgroup", "files"])).unwrap();
        assert_eq!(flags.j_group.as_deref(), Some("sortgroup"));
    }

    #[test]
    fn dash_2_flag_recognized() {
        let mut tm = make_tags();
        let (flags, _, _) = _requested(&mut tm, &s(&["-2", "files"])).unwrap();
        assert!(flags.two);
        assert!(!flags.one);
    }

    #[test]
    fn unknown_flag_treated_as_positional() {
        let mut tm = make_tags();
        // `-z` is not in the zparseopts spec; should be treated as a
        // bare positional (which makes it the first arg = "tag name").
        // Since "-z" isn't a wanted tag, returns None.
        assert!(_requested(&mut tm, &s(&["-z", "files"])).is_none());
    }
}
