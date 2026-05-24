//! Port of `_prefix` — complete with prefix handling.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_prefix`
//! (system copy `/opt/homebrew/share/zsh/functions/_prefix`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  [[ _matcher_num -gt 1 || -z "$SUFFIX" ]] && return 1
//!  5  local comp curcontext="$curcontext" tmp suf="$SUFFIX"
//! 16  if zstyle -t ":completion:${curcontext}:" add-space; then
//! 17    ISUFFIX=" $SUFFIX"
//! 18  else
//! 19    ISUFFIX="$SUFFIX"
//! 20  fi
//! 22  SUFFIX=
//! ```
//!
//! The shell version moves SUFFIX into ISUFFIX (the "ignored suffix",
//! preserved on the line but excluded from completion matching),
//! then runs the rest of the completer pipeline against bare PREFIX.
//!
//! Strict Rust port: honors `matcher_num > 1 || empty SUFFIX → bail`
//! gate AND the `add-space` style. Moves SUFFIX into ISUFFIX (with
//! optional leading space) for the action's duration, clears SUFFIX,
//! then restores both on return.

use crate::compcore::CompletionState;

/// _prefix - Complete with prefix handling.
///
/// `matcher_num` mirrors `_matcher_num`; `add_space` mirrors the
/// `add-space` zstyle resolved to bool by the caller.
pub fn _prefix(
    state: &mut CompletionState,
    matcher_num: usize,
    add_space: bool,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // shell:3 — bail when past first matcher OR no suffix.
    if matcher_num > 1 || state.params.suffix.is_empty() {
        return false;
    }
    // shell:16-22 — move SUFFIX → ISUFFIX (with optional leading
    // space), then clear SUFFIX.
    let saved_suffix = state.params.suffix.clone();
    let saved_isuffix = state.params.isuffix.clone();
    state.params.isuffix = if add_space {
        format!(" {}", saved_suffix)
    } else {
        saved_suffix.clone()
    };
    state.params.suffix.clear();

    let result = action(state);

    // Restore both fields.
    state.params.suffix = saved_suffix;
    state.params.isuffix = saved_isuffix;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_cleared_during_action_and_restored_after() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed = std::cell::Cell::new(String::new());
        let result = _prefix(&mut state, 1, false, |s| {
            observed.set(s.params.suffix.clone());
            true
        });
        assert!(result);
        assert_eq!(observed.into_inner(), "");
        assert_eq!(state.params.suffix, "BACK");
    }

    #[test]
    fn propagates_action_return_value() {
        let mut state = CompletionState::new();
        state.params.suffix = "x".into();
        assert!(!_prefix(&mut state, 1, false, |_| false));
        assert!(_prefix(&mut state, 1, false, |_| true));
    }

    #[test]
    fn empty_suffix_bails_per_shell_gate() {
        let mut state = CompletionState::new();
        let called = std::cell::Cell::new(false);
        let r = _prefix(&mut state, 1, false, |_| {
            called.set(true);
            true
        });
        assert!(!r);
        assert!(!called.get(), "action must NOT run when suffix is empty");
    }

    #[test]
    fn past_first_matcher_bails() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let r = _prefix(&mut state, 2, false, |_| true);
        assert!(!r);
    }

    #[test]
    fn add_space_prepends_space_to_isuffix() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed_isuffix = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, true, |s| {
            observed_isuffix.set(s.params.isuffix.clone());
            true
        });
        assert_eq!(observed_isuffix.into_inner(), " BACK");
    }

    #[test]
    fn no_add_space_isuffix_equals_suffix_verbatim() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, false, |s| {
            observed.set(s.params.isuffix.clone());
            true
        });
        assert_eq!(observed.into_inner(), "BACK");
    }

    #[test]
    fn isuffix_restored_to_original_after_action() {
        let mut state = CompletionState::new();
        state.params.suffix = "S".into();
        state.params.isuffix = "ORIGINAL".into();
        _prefix(&mut state, 1, false, |_| true);
        assert_eq!(state.params.isuffix, "ORIGINAL");
    }

    #[test]
    fn action_can_emit_matches() {
        use crate::completion::Completion;
        let mut state = CompletionState::new();
        state.params.suffix = "X".into();
        _prefix(&mut state, 1, false, |s| {
            s.add_match(Completion::new("emit"), None);
            true
        });
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"emit".to_string()));
        assert_eq!(state.params.suffix, "X");
    }

    #[test]
    fn prefix_field_untouched() {
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        state.params.suffix = "-svn".into();
        let observed_prefix = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, false, |s| {
            observed_prefix.set(s.params.prefix.clone());
            true
        });
        assert_eq!(observed_prefix.into_inner(), "git");
        assert_eq!(state.params.prefix, "git");
    }
}
