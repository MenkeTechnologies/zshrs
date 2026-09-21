//! Port of `_generic` from `Completion/Base/Widget/_generic`.
//!
//! Full upstream body (18 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  if [[ -n $ZSH_TRACE_GENERIC_WIDGET ]]; then
//! sh: 4    local widget=$ZSH_TRACE_GENERIC_WIDGET
//! sh: 5    unset ZSH_TRACE_GENERIC_WIDGET
//! sh: 6    $widget _generic
//! sh: 7    return
//! sh: 8  fi
//! sh: 9
//! sh:10  local curcontext="${curcontext:-}"
//! sh:11
//! sh:12  if [[ -z "$curcontext" ]]; then
//! sh:13    curcontext="${WIDGET}:::"
//! sh:14  else
//! sh:15    curcontext="${WIDGET}:${curcontext#*:}"
//! sh:16  fi
//! sh:17
//! sh:18  _main_complete "$@"
//! ```
//!
//! Reads `$ZSH_TRACE_GENERIC_WIDGET` (debugging hook) and `$WIDGET`
//! (current ZLE widget name) from shell-side params; dispatches the
//! trace widget OR `_main_complete` via `exec accessors`.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::{getsparam, setsparam, unsetparam};

/// `_generic` — generic completion-widget front-end that derives
/// `$curcontext` from `$WIDGET` and runs `_main_complete`.
pub fn _generic(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_generic");
    // sh:3-8  trace-widget hook
    let trace_widget = getsparam("ZSH_TRACE_GENERIC_WIDGET").unwrap_or_default();
    if !trace_widget.is_empty() {
        unsetparam("ZSH_TRACE_GENERIC_WIDGET");
        // sh:6 is a COMMAND WORD, so `dispatch_action_command`
        // (shared.rs:1407) resolves it exactly as `execcmd` does:
        // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
        // `$PATH`, then c:903's `command not found` with c:908's 127. The
        // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
        // that resolved nowhere returned in silence.
        return dispatch_action_command(&trace_widget, &["_generic".to_string()], 6);
    }

    // sh:10  local curcontext="${curcontext:-}"
    let saved = getsparam("curcontext").unwrap_or_default();
    let widget = getsparam("WIDGET").unwrap_or_default();

    // sh:12-16
    let new_ctx = if saved.is_empty() {
        format!("{}:::", widget)
    } else {
        let tail = saved.splitn(2, ':').nth(1).unwrap_or("");
        format!("{}:{}", widget, tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    // sh:18
    // sh:18 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    let r = dispatch_action_command("_main_complete", args, 18);
    let _ = setsparam("curcontext", &saved);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127. It used to return a silent 1.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("ZSH_TRACE_GENERIC_WIDGET", "");
        let _ = setsparam("WIDGET", "some-widget");
        assert_eq!(_generic(&[]), 127);
    }

    #[test]
    fn trace_widget_clears_env_and_dispatches_named_widget() {
        // sh:3-7 — when ZSH_TRACE_GENERIC_WIDGET is set, unset it
        //   and dispatch that widget instead of _main_complete.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("ZSH_TRACE_GENERIC_WIDGET", "_complete_debug");
        let _ = _generic(&[]);
        let after = getsparam("ZSH_TRACE_GENERIC_WIDGET").unwrap_or_default();
        assert!(after.is_empty());
    }
}
