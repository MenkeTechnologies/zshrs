//! Port of `_generic` — generic completion widget.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_generic`
//! (system copy `/opt/homebrew/share/zsh/functions/_generic`).
//!
//! Upstream shell source (full):
//! ```text
//!  3  if [[ -n $ZSH_TRACE_GENERIC_WIDGET ]]; then
//!  4    local widget=$ZSH_TRACE_GENERIC_WIDGET
//!  6    $widget _generic
//!  7    return
//!  8  fi
//! 10  local curcontext="${curcontext:-}"
//! 12  if [[ -z "$curcontext" ]]; then
//! 13    curcontext="${WIDGET}:::"
//! 14  else
//! 15    curcontext="${WIDGET}:${curcontext#*:}"
//! 16  fi
//! 18  _main_complete "$@"
//! ```
//!
//! Simplified Rust port: skips the ZSH_TRACE_GENERIC_WIDGET trace
//! hook and the curcontext rewrite. Takes the action closure as a
//! direct argument — the user's widget binding plugs in its own
//! completer instead of relying on `_main_complete "$@"` to forward
//! `$@`.

use crate::base::{CompleterResult, MainCompleteState};

/// _generic - Generic completion widget
pub fn _generic(
    state: &mut MainCompleteState,
    action: impl FnOnce(&mut MainCompleteState) -> CompleterResult,
) -> CompleterResult {
    action(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_action_unconditionally() {
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::Matched),
            CompleterResult::Matched
        ));
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::NoMatch),
            CompleterResult::NoMatch
        ));
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _generic(&mut state, |_| CompleterResult::Skip),
            CompleterResult::Skip
        ));
    }

    #[test]
    fn action_observes_provided_state() {
        let mut state = MainCompleteState::new("hello", 5);
        state.ctx.context = ":complete::test:".into();
        let seen_ctx = std::cell::Cell::new(String::new());
        _generic(&mut state, |s| {
            seen_ctx.set(s.ctx.context.clone());
            CompleterResult::Matched
        });
        assert_eq!(seen_ctx.into_inner(), ":complete::test:");
    }

    #[test]
    fn action_can_mutate_state() {
        use crate::completion::Completion;
        let mut state = MainCompleteState::new("", 0);
        _generic(&mut state, |s| {
            s.comp.add_match(Completion::new("via-generic"), None);
            CompleterResult::Matched
        });
        let names: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"via-generic".to_string()));
    }
}
