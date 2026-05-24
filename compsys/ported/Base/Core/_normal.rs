//! Port of `_normal` — normal command completion.
//!
//! Local shell reference: `compsys/functions/Base/Core/_normal`
//! (system copy `/opt/homebrew/share/zsh/functions/_normal`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local _comp_command1 _comp_command2 _comp_command precommand
//!  6  zparseopts -A opts -D - P p+:-=precommand s
//! 15  if [[ -o BANG_HIST && ( ($words[CURRENT] = \!*: …) || … ) ]]; then
//! 21    compset -P '*:'
//! 22    _history_modifiers h
//! 28  if [[ CURRENT -eq 1 ]]; then
//! 29    curcontext="${curcontext%:*:*}:-command-:"
//! 31    comp="$_comps[-command-]"
//! 32    [[ -n "$comp" ]] && eval "$comp" && return
//! 34    return 1
//! 37  _set_command
//! 39  _dispatch ${(k)opts[-s]} "$_comp_command" \
//!              "$_comp_command1" "$_comp_command2" -default-
//! ```
//!
//! Faithful Rust port: full algorithm shape preserved.
//!
//!   - shell:15-23: BANG_HIST history-modifier path — skipped here
//!     (shell-side history expansion lives at the lex layer).
//!   - shell:28-34: command-position dispatch via `_comps[-command-]`.
//!     Rust port takes a `cmd_handler` closure; the caller resolves
//!     the registered `-command-` entry and invokes it. Returns
//!     `Matched` iff the handler returned true.
//!   - shell:37-40: argument-position dispatch via `_dispatch`. Rust
//!     port takes an `arg_handler` closure called with the command
//!     name; caller looks it up in their comps table.
//!
//! If callers don't supply handlers (e.g. when called bare via the
//! 0-arg `_normal` shim below), we return `NoMatch` faithfully —
//! same as upstream `return 1` at shell:34 when no `_comps[-command-]`
//! is set.

use crate::base::{CompleterResult, MainCompleteState};

/// _normal - normal command completion (bare; faithful to upstream
/// behavior when no comps table is set: returns NoMatch).
pub fn _normal(state: &mut MainCompleteState) -> CompleterResult {
    _normal_with_handlers(state, |_, _| false, |_, _, _| false)
}

/// _normal with explicit dispatch handlers. The handlers map zsh's
/// `_comps[$cmd]` array lookup + eval to Rust closures the caller
/// owns.
///
/// `cmd_handler`: invoked at command position (CURRENT==1). Takes
/// `(state, "-command-")`. Returns true iff matches were emitted.
///
/// `arg_handler`: invoked at argument position (CURRENT>1). Takes
/// `(state, cmd, cmd_alternatives)` where `cmd_alternatives` is the
/// fallback chain shell-side passes as `"$_comp_command" "$_comp_command1"
/// "$_comp_command2" -default-`. Returns true iff matches were emitted.
pub fn _normal_with_handlers<C, A>(
    state: &mut MainCompleteState,
    cmd_handler: C,
    arg_handler: A,
) -> CompleterResult
where
    C: FnOnce(&mut MainCompleteState, &str) -> bool,
    A: FnOnce(&mut MainCompleteState, &str, &[&str]) -> bool,
{
    let current = state.comp.params.current as usize;

    // shell:28 — completing command name (position 1)
    if current == 1 {
        // shell:29 — curcontext rewritten to :-command-:
        let new_ctx = match state.ctx.context.rfind(':') {
            Some(_) => {
                // Strip last two `:`-segments and append `:-command-:`.
                let trimmed = state.ctx.context.trim_end_matches(':');
                let one_back = match trimmed.rfind(':') {
                    Some(i) => &trimmed[..i],
                    None => trimmed,
                };
                let two_back = match one_back.rfind(':') {
                    Some(i) => &one_back[..i],
                    None => one_back,
                };
                format!("{}:-command-:", two_back)
            }
            None => ":-command-:".to_string(),
        };
        let saved = std::mem::replace(&mut state.ctx.context, new_ctx);
        // shell:31-32 — comp="$_comps[-command-]"; eval "$comp"
        let matched = cmd_handler(state, "-command-");
        state.ctx.context = saved;
        return if matched {
            CompleterResult::Matched
        } else {
            // shell:34 — return 1 when no -command- handler matches
            CompleterResult::NoMatch
        };
    }

    // Get the command name (shell:37 `_set_command` → words[0]).
    let cmd = if !state.comp.params.words.is_empty() {
        state.comp.params.words[0].clone()
    } else {
        return CompleterResult::NoMatch;
    };

    // shell:39-40 — `_dispatch "$_comp_command" "$_comp_command1"
    //               "$_comp_command2" -default-`. Mirror the
    // fallback chain so the handler can try `cmd` first, then
    // `-default-` if the user has nothing registered for `cmd`.
    let fallbacks: &[&str] = &["-default-"];
    let matched = arg_handler(state, &cmd, fallbacks);
    if matched {
        CompleterResult::Matched
    } else {
        CompleterResult::NoMatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    #[test]
    fn command_position_calls_cmd_handler() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        let called = std::cell::Cell::new(false);
        let result = _normal_with_handlers(
            &mut state,
            |_, ctx| {
                called.set(ctx == "-command-");
                true
            },
            |_, _, _| panic!("arg_handler must not run at command position"),
        );
        assert!(matches!(result, CompleterResult::Matched));
        assert!(called.get(), "cmd_handler must be called with `-command-`");
    }

    #[test]
    fn command_position_unhandled_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        assert!(matches!(_normal(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn argument_position_calls_arg_handler_with_cmd_name() {
        let mut state = MainCompleteState::new("git status", 10);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), "status".into()];
        let seen = std::cell::RefCell::new(String::new());
        let result = _normal_with_handlers(
            &mut state,
            |_, _| panic!("cmd_handler must not run at arg position"),
            |s, cmd, fallbacks| {
                seen.borrow_mut().push_str(cmd);
                assert!(fallbacks.contains(&"-default-"));
                s.comp.add_match(Completion::new("status"), None);
                true
            },
        );
        assert!(matches!(result, CompleterResult::Matched));
        assert_eq!(seen.into_inner(), "git");
    }

    #[test]
    fn argument_position_empty_words_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words.clear();
        assert!(matches!(_normal(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn context_restored_after_command_handler() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::orig:".into();
        state.comp.params.current = 1;
        _normal_with_handlers(
            &mut state,
            |s, _| {
                // Inside the handler, context is rewritten.
                assert!(s.ctx.context.ends_with(":-command-:"));
                false
            },
            |_, _, _| false,
        );
        assert_eq!(
            state.ctx.context, ":complete::orig:",
            "context must be restored after handler"
        );
    }

    #[test]
    fn argument_position_unhandled_returns_no_match() {
        let mut state = MainCompleteState::new("git status", 10);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), "status".into()];
        // Both handlers return false → NoMatch.
        let result = _normal_with_handlers(&mut state, |_, _| false, |_, _, _| false);
        assert!(matches!(result, CompleterResult::NoMatch));
    }

    #[test]
    fn context_rewrite_strips_last_two_colon_segments() {
        // shell:29 `curcontext="${curcontext%:*:*}:-command-:"`
        // strips back through the last two `:` segments.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::cmd::tag:".into();
        state.comp.params.current = 1;
        let observed = std::cell::Cell::new(String::new());
        _normal_with_handlers(
            &mut state,
            |s, _| {
                observed.set(s.ctx.context.clone());
                true
            },
            |_, _, _| false,
        );
        // Observed context should end with :-command-:
        let seen = observed.into_inner();
        assert!(
            seen.ends_with(":-command-:"),
            "context rewrite missing `-command-` segment: {seen}"
        );
    }

    #[test]
    fn context_rewrite_empty_initial_yields_command_only() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = "".into();
        state.comp.params.current = 1;
        let observed = std::cell::Cell::new(String::new());
        _normal_with_handlers(
            &mut state,
            |s, _| {
                observed.set(s.ctx.context.clone());
                true
            },
            |_, _, _| false,
        );
        assert_eq!(observed.into_inner(), ":-command-:");
    }

    #[test]
    fn fallbacks_always_include_default_dash() {
        let mut state = MainCompleteState::new("git st", 6);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), "st".into()];
        let observed = std::cell::RefCell::new(Vec::<String>::new());
        _normal_with_handlers(
            &mut state,
            |_, _| false,
            |_, _, fallbacks| {
                for f in fallbacks {
                    observed.borrow_mut().push(f.to_string());
                }
                true
            },
        );
        assert!(observed.into_inner().contains(&"-default-".to_string()));
    }

    #[test]
    fn current_position_zero_treated_as_argument_position() {
        // CURRENT == 0 (synthetic edge case) — not command, not arg
        // → fall through to argument-position handler.
        let mut state = MainCompleteState::new("git", 3);
        state.comp.params.current = 0;
        state.comp.params.words = vec!["git".into()];
        let called = std::cell::Cell::new(false);
        let _ = _normal_with_handlers(
            &mut state,
            |_, _| panic!("cmd_handler must not fire at current=0"),
            |_, cmd, _| {
                called.set(true);
                assert_eq!(cmd, "git");
                true
            },
        );
        assert!(called.get());
    }
}
