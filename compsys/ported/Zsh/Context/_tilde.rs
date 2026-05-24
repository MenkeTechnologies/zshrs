//! Port of `_tilde` — `-tilde-` context handler (after typing `~`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_tilde`.
//!
//! Upstream shell source (the whole 32-line file):
//! ```text
//! #compdef -tilde-
//!
//! [[ -n "$compstate[quote]" ]] && return 1
//!
//! local expl suf ret=1
//!
//! if [[ "$SUFFIX" = */* ]]; then
//!   ISUFFIX="/${SUFFIX#*/}$ISUFFIX"
//!   SUFFIX="${SUFFIX%%/*}"
//!   suf=(-S '')
//! else
//!   suf=(-qS/)
//! fi
//!
//! _tags users named-directories directory-stack
//!
//! while _tags; do
//!   _requested users expl 'user' compadd "$suf[@]" - "${(@k)userdirs}"
//!   _requested named-directories expl 'named directory' compadd "$suf[@]" -k nameddirs
//!   _requested directory-stack expl 'directory stack' _directory_stack
//!   (( ret )) || return 0
//! done
//!
//! return ret
//! ```
//!
//! Strict Rust port: faithful 1:1 — dispatches via `_tags` + the
//! three `_requested` per-tag branches. Caller supplies user
//! homedirs, named dirs, and dirstack.

use crate::base::MainCompleteState;
use crate::completion::{Completion, CompletionFlags};
use crate::ported::_directory_stack::_directory_stack;
use crate::ported::_requested::{RequestedAction, _requested};

/// `_tilde` — `-tilde-` context dispatcher.
///
/// `quoting` — true iff `$compstate[quote]` is non-empty (we're in
/// a quoted context).
/// `userdirs` — map of `user → homedir` (`${(k)userdirs}` keys).
/// `nameddirs` — map of `name → dir` (`-k nameddirs` keys).
/// `dirstack` — `$dirstack` rendered with `~` substitution.
/// `pushdminus` — pushdminus option flag for `_directory_stack`.
pub fn _tilde(
    state: &mut MainCompleteState,
    quoting: bool,
    userdirs: &[String],
    nameddirs: &[String],
    dirstack: &[String],
    pushdminus: bool,
) -> bool {
    // shell:3 — `[[ -n $compstate[quote] ]] && return 1`
    if quoting {
        return false;
    }

    // shell:7-12 — split SUFFIX at `/` if present.
    let has_slash_in_suffix = state.comp.params.suffix.contains('/');
    if has_slash_in_suffix {
        let suffix = state.comp.params.suffix.clone();
        let slash = suffix.find('/').unwrap();
        let pre = &suffix[..slash];
        let rest = &suffix[slash..]; // includes the `/`
        state.comp.params.isuffix = format!("{}{}", rest, state.comp.params.isuffix);
        state.comp.params.suffix = pre.to_string();
    }

    // suf flag — `-S ''` if SUFFIX had `/`, else `-qS/`.
    let (suf_str, nospace) = if has_slash_in_suffix {
        (Some(""), true)
    } else {
        (Some("/"), true)
    };

    // shell:14 — `_tags users named-directories directory-stack`
    // Initialise the tag manager (replace any prior state).
    state.tags = crate::base::TagManager::new();
    state.tags.init(&[
        "users".into(),
        "named-directories".into(),
        "directory-stack".into(),
    ]);
    state.tags.add_try(&[
        "users".into(),
        "named-directories".into(),
        "directory-stack".into(),
    ]);
    if !state.tags.start() {
        return false;
    }

    let mut ret = false;
    let prefix = state.comp.params.prefix.clone();
    // shell:16-21 — `while _tags; do …; (( ret )) || return 0 done`
    loop {
        // _requested users → emit each user with suf flag
        if _requested(&mut state.tags, &[String::from("users")]).is_some() {
            state.comp.begin_group("users", true);
            for u in userdirs {
                if !u.starts_with(&prefix) {
                    continue;
                }
                let mut c = Completion::new(u.clone());
                if let Some(s) = suf_str {
                    c.suf = Some(s.to_string());
                }
                if nospace {
                    c.flags |= CompletionFlags::NOSPACE;
                }
                state.comp.add_match(c, Some("users"));
                ret = true;
            }
            state.comp.end_group();
        }
        // _requested named-directories
        if _requested(&mut state.tags, &[String::from("named-directories")]).is_some() {
            state.comp.begin_group("named-directories", true);
            for n in nameddirs {
                if !n.starts_with(&prefix) {
                    continue;
                }
                let mut c = Completion::new(n.clone());
                if let Some(s) = suf_str {
                    c.suf = Some(s.to_string());
                }
                if nospace {
                    c.flags |= CompletionFlags::NOSPACE;
                }
                state.comp.add_match(c, Some("named-directories"));
                ret = true;
            }
            state.comp.end_group();
        }
        // _requested directory-stack → _directory_stack
        if matches!(
            _requested(&mut state.tags, &[String::from("directory-stack")])
                .map(|t| t.1),
            Some(RequestedAction::GateOnly)
                | Some(RequestedAction::Describe)
                | Some(RequestedAction::AllLabels)
        ) {
            if _directory_stack(&mut state.comp, dirstack, pushdminus) {
                ret = true;
            }
        }
        // `(( ret )) || return 0` — shell idiom: return success on
        // first iteration that yielded matches; otherwise advance.
        if ret {
            return true;
        }
        if !state.tags.next() {
            break;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_short_circuits_with_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_tilde(&mut state, true, &[], &[], &[], false));
    }

    #[test]
    fn emits_users_with_slash_suffix() {
        let mut state = MainCompleteState::new("", 0);
        let users = vec!["alice".to_string(), "bob".to_string()];
        let _ = _tilde(&mut state, false, &users, &[], &[], false);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .filter(|g| g.name == "users")
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"bob"));
    }

    #[test]
    fn suffix_with_slash_splits_into_isuffix() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.suffix = "subdir/file".into();
        let users = vec!["alice".to_string()];
        let _ = _tilde(&mut state, false, &users, &[], &[], false);
        // shell:7-10 — SUFFIX="subdir", ISUFFIX="/file".
        assert_eq!(state.comp.params.suffix, "subdir");
        assert_eq!(state.comp.params.isuffix, "/file");
    }

    #[test]
    fn emits_named_directories() {
        let mut state = MainCompleteState::new("", 0);
        let named = vec!["work".to_string(), "docs".to_string()];
        let _ = _tilde(&mut state, false, &[], &named, &[], false);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .filter(|g| g.name == "named-directories")
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"work"));
        assert!(names.contains(&"docs"));
    }

    #[test]
    fn empty_inputs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_tilde(&mut state, false, &[], &[], &[], false));
    }

    #[test]
    fn prefix_filter_applies() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ali".into();
        let users = vec!["alice".to_string(), "bob".to_string()];
        let _ = _tilde(&mut state, false, &users, &[], &[], false);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alice"));
        assert!(!names.contains(&"bob"));
    }
}
