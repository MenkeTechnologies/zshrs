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
//! Strict Rust port: implements both branches.
//!
//! 1. If `trace_widget` is non-empty, dispatch the trace widget via
//!    `_call_function` (mirrors `$ZSH_TRACE_GENERIC_WIDGET`).
//! 2. Otherwise rewrite `curcontext` to `widget:rest-of-context`
//!    (shell:12-16) and then invoke `action` (the caller's
//!    `_main_complete` equivalent — the closure form lets the
//!    caller wire whichever completer chain they want).

use crate::base::{CompleterResult, MainCompleteState};
use crate::ported::_call_function::_call_function;

/// _generic - Generic completion widget.
///
/// `widget` is the ZLE widget name (`$WIDGET`). `trace_widget` is
/// the value of `$ZSH_TRACE_GENERIC_WIDGET` (empty if unset).
pub fn _generic(
    state: &mut MainCompleteState,
    widget: &str,
    trace_widget: &str,
    action: impl FnOnce(&mut MainCompleteState) -> CompleterResult,
) -> CompleterResult {
    // shell:3-8 — trace hook fast path.
    if !trace_widget.is_empty() {
        let _ = _call_function(state, trace_widget);
        return CompleterResult::NoMatch;
    }

    // shell:10-16 — curcontext rewrite.
    state.ctx.context = if state.ctx.context.is_empty() {
        format!("{}:::", widget)
    } else {
        // ${curcontext#*:} strips through first colon, keeping suffix.
        let suffix = state
            .ctx
            .context
            .splitn(2, ':')
            .nth(1)
            .unwrap_or("");
        format!("{}:{}", widget, suffix)
    };

    // shell:18 — _main_complete "$@" (delegated via action).
    action(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_curcontext_set_to_widget_with_triple_colons() {
        // shell:12-13 — `curcontext="${WIDGET}:::"`.
        let mut state = MainCompleteState::new("", 0);
        let _ = _generic(&mut state, "complete-word", "", |_| {
            CompleterResult::NoMatch
        });
        assert_eq!(state.ctx.context, "complete-word:::");
    }

    #[test]
    fn non_empty_curcontext_keeps_suffix_after_first_colon() {
        // shell:14-15 — `curcontext="${WIDGET}:${curcontext#*:}"`.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = "old:complete::cmd:".into();
        let _ = _generic(&mut state, "list-choices", "", |_| {
            CompleterResult::NoMatch
        });
        assert_eq!(state.ctx.context, "list-choices:complete::cmd:");
    }

    #[test]
    fn trace_widget_dispatches_via_call_function_and_returns_no_match() {
        use crate::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        FIRED.store(false, Ordering::SeqCst);
        register(
            "my-tracer",
            Box::new(|_| {
                FIRED.store(true, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        let r = _generic(&mut state, "complete-word", "my-tracer", |_| {
            panic!("action MUST NOT run when trace_widget is set");
        });
        unregister("my-tracer");
        assert!(matches!(r, CompleterResult::NoMatch));
        assert!(FIRED.load(Ordering::SeqCst));
    }

    #[test]
    fn action_observes_rewritten_curcontext() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = "OLD:foo::bar:".into();
        let seen = std::cell::Cell::new(String::new());
        let _ = _generic(&mut state, "complete-word", "", |s| {
            seen.set(s.ctx.context.clone());
            CompleterResult::Matched
        });
        assert_eq!(seen.into_inner(), "complete-word:foo::bar:");
    }

    #[test]
    fn action_result_propagates() {
        // Each variant round-trips.
        for r in [
            CompleterResult::Matched,
            CompleterResult::NoMatch,
            CompleterResult::Skip,
        ] {
            let mut state = MainCompleteState::new("", 0);
            // Match by variant only — Skip/NoMatch don't carry data.
            let returned = _generic(&mut state, "w", "", |_| match r {
                CompleterResult::Matched => CompleterResult::Matched,
                CompleterResult::NoMatch => CompleterResult::NoMatch,
                CompleterResult::Skip => CompleterResult::Skip,
            });
            assert_eq!(
                std::mem::discriminant(&returned),
                std::mem::discriminant(&r)
            );
        }
    }

    #[test]
    fn action_can_mutate_state() {
        use crate::completion::Completion;
        let mut state = MainCompleteState::new("", 0);
        let _ = _generic(&mut state, "w", "", |s| {
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

    #[test]
    fn trace_widget_unregistered_still_returns_no_match() {
        // When ZSH_TRACE_GENERIC_WIDGET is set but the widget isn't
        // registered, the trace branch still fires (silently) and
        // returns NoMatch.
        let mut state = MainCompleteState::new("", 0);
        let r = _generic(&mut state, "complete-word", "not-a-registered-fn", |_| {
            panic!("action MUST NOT run when trace_widget is set");
        });
        assert!(matches!(r, CompleterResult::NoMatch));
    }

    #[test]
    fn curcontext_rewrite_preserves_double_colon_segments() {
        // `:foo:bar::baz:` — splitn(2, ':') gives prefix="" and
        // suffix="foo:bar::baz:". After rewrite: "w:foo:bar::baz:"
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":foo:bar::baz:".into();
        let _ = _generic(&mut state, "w", "", |_| CompleterResult::NoMatch);
        assert_eq!(state.ctx.context, "w:foo:bar::baz:");
    }

    #[test]
    fn curcontext_no_colon_in_input_still_works() {
        // Context with no `:` — splitn(2, ':').nth(1) is None,
        // suffix = "". Result: "w:"
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = "nocolon".into();
        let _ = _generic(&mut state, "w", "", |_| CompleterResult::NoMatch);
        assert_eq!(state.ctx.context, "w:");
    }
}
