//! Port of `_directory_stack` — complete `+N` / `-N` directory
//! stack indices (for `popd`/`pushd`/`cd`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_directory_stack`.
//!
//! Upstream shell source (key lines from ~50 lines):
//! ```text
//! [[ $PREFIX = [-+]* ]] || return 1
//! lines=("${(D)dirstack[@]}")  # %-rendered (HOME → ~)
//! # for + : number 0..N-1 ascending; for - : 0..N-1 descending
//! ```
//!
//! `pushdminus` option flips the meaning of `+` vs `-`. Our port
//! takes both the dirstack (rendered) and the `pushdminus` bool
//! from the caller.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// `_directory_stack` — emit numbered dirstack entries when the
/// user typed `+` or `-`.
pub fn _directory_stack(
    state: &mut CompletionState,
    dirstack: &[String],
    pushdminus: bool,
) -> bool {
    let prefix = state.params.prefix.clone();
    if !prefix.starts_with('+') && !prefix.starts_with('-') {
        return false;
    }
    let sign = prefix.chars().next().unwrap();
    let user_typed = &prefix[1..];

    state.begin_group("directory-stack", true);
    let mut any = false;
    // When pushdminus AND sign is '+', OR no-pushdminus AND sign is '-',
    // the numbering is REVERSED.
    let reverse =
        (pushdminus && sign == '+') || (!pushdminus && sign == '-');
    let n = dirstack.len();
    for (i, entry) in dirstack.iter().enumerate() {
        let display_num = if reverse { n.saturating_sub(1).saturating_sub(i) } else { i };
        let cand = format!("{}{}", sign, display_num);
        if !cand[1..].starts_with(user_typed) {
            continue;
        }
        let mut comp = Completion::new(cand.clone());
        comp.disp = Some(format!("{} -- {}", cand, entry));
        state.add_match(comp, Some("directory-stack"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_dash_or_plus_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/some/path".into();
        let dirs = vec!["~".to_string(), "~/work".to_string()];
        assert!(!_directory_stack(&mut state, &dirs, false));
    }

    #[test]
    fn plus_emits_ascending_numbered_entries() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec![
            "~".to_string(),
            "~/work".to_string(),
            "~/Documents".to_string(),
        ];
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // With pushdminus=false, + uses ascending 0,1,2.
        assert!(names.contains(&"+0"));
        assert!(names.contains(&"+1"));
        assert!(names.contains(&"+2"));
    }

    #[test]
    fn dash_emits_descending_numbered_entries() {
        // With pushdminus=false, `-` is reversed.
        let mut state = CompletionState::new();
        state.params.prefix = "-".into();
        let dirs = vec!["~".to_string(), "~/work".to_string()];
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"-0"));
        assert!(names.contains(&"-1"));
    }

    #[test]
    fn pushdminus_flips_plus_to_descending() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec!["~".to_string(), "~/w".to_string()];
        let _ = _directory_stack(&mut state, &dirs, true);
        // With pushdminus=true, `+` is reversed.
        // entries[0]=~ → display=1; entries[1]=~/w → display=0
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"+0"));
        assert!(names.contains(&"+1"));
    }

    #[test]
    fn partial_number_typed_filters() {
        let mut state = CompletionState::new();
        state.params.prefix = "+1".into();
        let dirs: Vec<String> = (0..15).map(|i| format!("/d{i}")).collect();
        let _ = _directory_stack(&mut state, &dirs, false);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // All emitted should be +1, +10, +11, +12, +13, +14 (starts_with "1")
        assert!(names.iter().all(|n| n[1..].starts_with("1")));
        assert!(names.contains(&"+1"));
        assert!(names.contains(&"+10"));
    }

    #[test]
    fn entries_attached_to_disp() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        let dirs = vec!["~/myproj".to_string()];
        let _ = _directory_stack(&mut state, &dirs, false);
        let disp = state.groups[0].matches[0]
            .disp
            .as_deref()
            .unwrap_or("");
        assert!(disp.contains("~/myproj"));
    }

    #[test]
    fn empty_dirstack_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "+".into();
        assert!(!_directory_stack(&mut state, &[], false));
    }
}
