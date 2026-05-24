//! Port of `_globquals` — complete glob-qualifier chars inside `(…)`.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_globquals`.
//!
//! Upstream is a ~120-line state machine. The user-visible
//! candidate set is the documented glob-qualifier letter list.
//!
//! Strict Rust port: emits each documented qualifier char with a
//! short description. Doesn't model the state machine's parameter-
//! consumption (`f<mode>`, `u<user>`, `g<group>`, `s<sb>`, etc.) at
//! this layer — caller handles secondary input.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Documented glob qualifiers (zsh `(.qual)` form).
pub const GLOB_QUALIFIERS: &[(&str, &str)] = &[
    ("/", "directories"),
    ("F", "non-empty dirs"),
    (".", "regular files"),
    ("@", "symbolic links"),
    ("=", "sockets"),
    ("p", "named pipes (FIFOs)"),
    ("*", "executable plain files"),
    ("r", "owner-readable"),
    ("w", "owner-writable"),
    ("x", "owner-executable"),
    ("A", "group-readable"),
    ("I", "group-writable"),
    ("E", "group-executable"),
    ("R", "world-readable"),
    ("W", "world-writable"),
    ("X", "world-executable"),
    ("s", "setuid (set-UID) files"),
    ("S", "setgid (set-GID) files"),
    ("t", "sticky-bit files"),
    ("d", "device files (block/char)"),
    ("U", "owned by current UID"),
    ("G", "owned by current GID"),
    ("u", "owned by named user"),
    ("g", "owned by named group"),
    ("a", "accessed in N days"),
    ("m", "modified in N days"),
    ("c", "inode-changed in N days"),
    ("L", "size larger than"),
    ("N", "no glob qualifier (clear)"),
    ("o", "sort order"),
    ("O", "reverse sort order"),
    ("D", "include dotfiles"),
    ("[", "begin range / pattern"),
    (",", "logical OR between qualifiers"),
    ("^", "negate the following qualifier"),
];

/// `_globquals` — emit glob qualifier chars.
pub fn _globquals(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("glob-qualifiers", true);
    let mut any = false;
    for (q, desc) in GLOB_QUALIFIERS {
        if !q.starts_with(&*prefix) {
            continue;
        }
        let mut comp = Completion::new(*q);
        comp.disp = Some(format!("{} -- {}", q, desc));
        state.add_match(comp, Some("glob-qualifiers"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_qualifiers_emitted() {
        let mut state = CompletionState::new();
        let _ = _globquals(&mut state);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        for q in ["/", ".", "@", "*", "U", "D", "o", "O"] {
            assert!(names.contains(&q), "missing qualifier `{q}`");
        }
    }

    #[test]
    fn prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "U".into();
        let _ = _globquals(&mut state);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["U"]);
    }

    #[test]
    fn group_name_is_glob_qualifiers() {
        let mut state = CompletionState::new();
        let _ = _globquals(&mut state);
        assert!(state.groups.iter().any(|g| g.name == "glob-qualifiers"));
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "zzz".into();
        assert!(!_globquals(&mut state));
    }

    #[test]
    fn count_at_least_30_qualifiers() {
        let mut state = CompletionState::new();
        let _ = _globquals(&mut state);
        assert!(state.groups[0].matches.len() >= 30);
    }

    #[test]
    fn each_has_disp_description() {
        let mut state = CompletionState::new();
        let _ = _globquals(&mut state);
        for m in &state.groups[0].matches {
            assert!(m.disp.as_deref().unwrap_or("").contains(" -- "));
        }
    }
}
