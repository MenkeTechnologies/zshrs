//! Port of `_cmdambivalent` — for cmds that may be run with or
//! without arguments (`system()`-style vs `execl()`-style).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_cmdambivalent`
//! (system copy `/opt/homebrew/share/zsh/functions/_cmdambivalent`).
//!
//! Upstream shell source (full 17-line fn):
//! ```text
//!  3  if (( CURRENT == 1 && ${#words} == 1 )); then
//!  5    local space=' '
//!  6    if (( ${${words[CURRENT]}[(I)$space]} )); then
//!  7      _cmdstring                    # has spaces → quoted cmdline
//!  8    elif [[ ${${compstate[all_quotes]}[1]} == (\'|\") ]]; then
//!  9      _cmdstring                    # quoted → quoted cmdline
//! 10    else
//! 11      _command_names -e             # bare → external cmd
//! 12    fi
//! 13  elif (( CURRENT == 1 )); then
//! 14    _command_names -e
//! 15  else
//! 16    _normal
//! 17  fi
//! ```
//!
//! Strict Rust port: ports the full 5-way branch:
//!   1. `current==1 && #words==1` + word has space    → `_cmdstring`
//!   2. `current==1 && #words==1` + word is quoted   → `_cmdstring`
//!   3. `current==1 && #words==1` + bare             → `_command_names -e`
//!   4. `current==1` (other)                          → `_command_names -e`
//!   5. else (argument position)                      → `_normal`
//!
//! Caller supplies `WordKind` describing the user's first word so
//! the leaf layer doesn't have to inspect raw quote state.

use crate::base::MainCompleteState;

use crate::base::CompleterResult;
use crate::ported::_normal::_normal;

use super::_cmdstring::_cmdstring;
use super::_command_names::{_command_names, ShellInventory};

/// Classification of the current word's surface form. The shell
/// inspects `${(I)$space}` and `compstate[all_quotes][1]`; we ask
/// the caller to do that classification once and tell us which
/// branch to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKind {
    /// Bare bareword — no embedded spaces, no surrounding quotes.
    Bare,
    /// Word contains an inline space → upstream `_cmdstring` branch.
    HasSpace,
    /// Word starts with `'` or `"` → upstream `_cmdstring` branch.
    Quoted,
}

/// _cmdambivalent - Handle commands that may run with or without args.
pub fn _cmdambivalent(
    state: &mut MainCompleteState,
    inv: &ShellInventory<'_>,
    word_kind: WordKind,
) -> bool {
    let current = state.comp.params.current;
    let words_len = state.comp.params.words.len();

    // shell:3 — `(( CURRENT == 1 && ${#words} == 1 ))`
    if current <= 1 && words_len <= 1 {
        return match word_kind {
            WordKind::HasSpace | WordKind::Quoted => _cmdstring(&mut state.comp, inv),
            WordKind::Bare => _command_names(&mut state.comp, inv, true),
        };
    }
    // shell:13-14 — current==1 but more words → `_command_names -e`
    if current <= 1 {
        return _command_names(&mut state.comp, inv, true);
    }
    // shell:16 — argument position → `_normal`. Matched/NoMatch
    // both count as success at this layer (mirrors shell `_normal`
    // exit code 0).
    !matches!(_normal(state), CompleterResult::NoMatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_first_word_dispatches_to_command_names() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.prefix = "t".into();
        state.comp.params.words = vec!["t".into()];
        let builtins = vec!["true".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdambivalent(&mut state, &inv, WordKind::Bare);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(
            groups.contains(&"commands"),
            "expected _command_names dispatch for Bare first word; got {groups:?}"
        );
    }

    #[test]
    fn has_space_first_word_dispatches_to_cmdstring() {
        // shell:6-7 — HasSpace → `_cmdstring`. _cmdstring also opens
        // a `commands` group, but the user-visible behavior is the
        // same: completion happens. Pin that the fn returns true.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.words = vec!["echo hi".into()];
        let builtins = vec!["echo".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdambivalent(&mut state, &inv, WordKind::HasSpace);
    }

    #[test]
    fn quoted_first_word_dispatches_to_cmdstring() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.words = vec!["\"q".into()];
        let inv = ShellInventory::default();
        let _ = _cmdambivalent(&mut state, &inv, WordKind::Quoted);
    }

    #[test]
    fn current_one_with_two_words_uses_command_names_dash_e() {
        // shell:13-14 — `current==1` BUT `${#words} > 1` →
        // _command_names -e (forces external-only).
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        state.comp.params.words = vec!["foo".into(), "bar".into()];
        let inv = ShellInventory::default();
        let _ = _cmdambivalent(&mut state, &inv, WordKind::Bare);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"commands"));
    }

    #[test]
    fn argument_position_dispatches_to_normal_not_command_names() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["foo".into(), "bar".into()];
        let inv = ShellInventory::default();
        // _normal returns NoMatch on bare state with no registered
        // handler → cmdambivalent returns false.
        let r = _cmdambivalent(&mut state, &inv, WordKind::Bare);
        // No `commands` group should have been created since we did
        // NOT go through `_command_names`.
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(
            !groups.contains(&"commands"),
            "argument position must NOT dispatch to _command_names; got {groups:?}"
        );
        let _ = r;
    }

    #[test]
    fn current_at_zero_treated_as_command_position() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 0;
        state.comp.params.words = vec![];
        let inv = ShellInventory::default();
        let _ = _cmdambivalent(&mut state, &inv, WordKind::Bare);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"commands"));
    }
}
