//! Port of `_all_matches` — show all possible matches.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_all_matches`
//! (system copy `/opt/homebrew/share/zsh/functions/_all_matches`).
//!
//! Upstream shell source (key lines from the system file):
//! ```text
//!  6  zstyle -s ":completion:${curcontext}:" old-matches old
//!  8  if [[ "$old" = (only|true|yes|1|on) ]]; then
//! 10    if [[ -n "$compstate[old_list]" ]]; then
//! 11      compstate[insert]=all
//! 12      compstate[old_list]=keep
//! 13      return 0
//! 16    [[ "$old" = *only* ]] && return 1
//! 19  (( $comppostfuncs[(I)_all_matches_end] )) ||
//! 20      comppostfuncs=( "$comppostfuncs[@]" _all_matches_end )
//! 22  _all_matches_context=":completion:${curcontext}:"
//! 24  return 1
//! ```
//!
//! Strict Rust port:
//!   - shell:6 — resolve `old-matches` style.
//!   - shell:8-17 — when truthy AND have_old_list → insert=all,
//!     old_list=keep, return TRUE (success). When `only` AND no
//!     old list → return false (shell `return 1`).
//!   - shell:19-22 — arm `_all_matches_end` as a postfunc by
//!     appending to `state.postfuncs` if not already present, AND
//!     record the context in process-global state for the postfunc
//!     to read.
//!   - shell:24 — default `return 1` → Rust false.
//!
//! `_all_matches_end` (the postfunc, 16 lines) is also ported below
//! and registered on first use.

use std::sync::{Mutex, OnceLock};

use crate::base::MainCompleteState;

/// Process-global `_all_matches_context` — recorded by `_all_matches`
/// so the `_all_matches_end` postfunc knows which context to
/// `zstyle` against.
fn context_cell() -> &'static Mutex<Option<String>> {
    static CELL: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// _all_matches - Show all possible matches.
pub fn _all_matches(state: &mut MainCompleteState) -> bool {
    let ctx = format!(":completion:{}:", state.ctx.context);
    let old = state
        .styles
        .lookup_values(&ctx, "old-matches")
        .and_then(|v| v.first().cloned())
        .unwrap_or_default();

    let old_truthy = matches!(old.as_str(), "only" | "true" | "yes" | "1" | "on");
    if old_truthy {
        let have_old_list = !state.comp.params.compstate.old_list.is_empty();
        if have_old_list {
            // shell:11-13
            state.comp.params.compstate.insert = "all".into();
            state.comp.params.compstate.old_list = "keep".into();
            return true;
        }
        // shell:16 — `only` AND no old list → return 1 (Rust false).
        if old.contains("only") {
            return false;
        }
    }

    // shell:19-20 — append _all_matches_end to postfuncs if absent.
    if !state
        .postfuncs
        .iter()
        .any(|f| f == "_all_matches_end")
    {
        state.postfuncs.push("_all_matches_end".into());
    }
    // shell:22 — record context for the postfunc.
    if let Ok(mut g) = context_cell().lock() {
        *g = Some(ctx);
    }

    // shell:24 — return 1 (Rust false).
    false
}

/// `_all_matches_end` - postfunc that runs after the completer chain.
///
/// Upstream (16 lines):
/// ```text
///   zstyle -s "$_all_matches_context" avoid-completer not ||
///       not=( _expand _old_list _correct _approximate )
///   if [[ "$compstate[nmatches]" -gt 1 && $not[(I)(|_)$_completer] -eq 0 ]]; then
///     if zstyle -t "$_all_matches_context" insert; then
///       compstate[insert]=all
///     else
///       _description all-matches expl 'all matches'
///       compadd "$expl[@]" -C
///     fi
///   fi
///   unset _all_matches_context
/// ```
///
/// On the Rust side this is registered as a postfunc that the
/// caller (`_main_complete`) dispatches via `_call_function`.
pub fn _all_matches_end(state: &mut MainCompleteState) -> bool {
    let ctx = match context_cell().lock() {
        Ok(g) => g.clone().unwrap_or_default(),
        Err(_) => return false,
    };
    // Always clear; matches shell `unset _all_matches_context`.
    if let Ok(mut g) = context_cell().lock() {
        *g = None;
    }
    if ctx.is_empty() {
        return false;
    }

    // shell — `zstyle -s avoid-completer not || not=(default-list)`
    let avoid: Vec<String> = state
        .styles
        .lookup_values(&ctx, "avoid-completer")
        .map(|v| v.to_vec())
        .unwrap_or_else(|| {
            vec![
                "_expand".into(),
                "_old_list".into(),
                "_correct".into(),
                "_approximate".into(),
            ]
        });

    let current_completer = format!("_{}", state.ctx.completer.replace('-', "_"));
    let avoided = avoid.iter().any(|a| {
        // shell `$not[(I)(|_)$_completer]` matches `name` OR `_name`.
        let a = a.as_str();
        let trimmed = a.strip_prefix('_').unwrap_or(a);
        a == current_completer
            || a == state.ctx.completer
            || trimmed == state.ctx.completer.trim_start_matches('_')
    });

    if state.comp.nmatches > 1 && !avoided {
        let insert_style = state
            .styles
            .lookup_values(&ctx, "insert")
            .and_then(|v| v.first().cloned())
            .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
            .unwrap_or(false);
        if insert_style {
            state.comp.params.compstate.insert = "all".into();
        } else {
            // shell: `_description all-matches expl 'all matches';
            //          compadd "$expl[@]" -C`
            // The `-C` adds the explanation as a "completion" with
            // no actual candidate string — at our layer we emit an
            // explanation under the `all-matches` group.
            state
                .comp
                .add_explanation("all matches".into(), Some("all-matches"));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as SyncMutex;

    // Tests touch the process-global `context_cell`; serialize them.
    static SERIAL: SyncMutex<()> = SyncMutex::new(());

    fn reset() {
        if let Ok(mut g) = context_cell().lock() {
            *g = None;
        }
    }

    #[test]
    fn old_truthy_with_old_list_returns_true_and_arms_insert_all() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::amb:".into();
        state.comp.params.compstate.old_list = "shown".into();
        state.styles.set(
            ":completion::t::amb::",
            "old-matches",
            vec!["yes".into()],
            false,
        );
        let r = _all_matches(&mut state);
        assert!(r, "shell `return 0` on old-matches truthy + have_old_list");
        assert_eq!(state.comp.params.compstate.insert, "all");
        assert_eq!(state.comp.params.compstate.old_list, "keep");
    }

    #[test]
    fn old_only_without_old_list_returns_false() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::amb:".into();
        state.styles.set(
            ":completion::t::amb::",
            "old-matches",
            vec!["only".into()],
            false,
        );
        // No old_list set.
        assert!(!_all_matches(&mut state));
        // And postfunc NOT armed (we bailed early).
        assert!(!state.postfuncs.iter().any(|f| f == "_all_matches_end"));
    }

    #[test]
    fn default_path_arms_postfunc_and_records_context() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::default:".into();
        // No old-matches style at all → default path.
        let r = _all_matches(&mut state);
        assert!(!r, "shell `return 1` → false");
        assert!(state.postfuncs.iter().any(|f| f == "_all_matches_end"));
        let recorded = context_cell().lock().unwrap().clone();
        assert_eq!(recorded.as_deref(), Some(":completion::t::default::"));
    }

    #[test]
    fn postfunc_not_double_appended() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t:".into();
        let _ = _all_matches(&mut state);
        let _ = _all_matches(&mut state);
        let _ = _all_matches(&mut state);
        let count = state
            .postfuncs
            .iter()
            .filter(|f| *f == "_all_matches_end")
            .count();
        assert_eq!(count, 1, "postfunc should appear at most once");
    }

    #[test]
    fn end_sets_insert_all_when_insert_style_true() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::e:".into();
        state.ctx.completer = "complete".into();
        // Seed context via _all_matches.
        let _ = _all_matches(&mut state);
        // Mark there are >1 matches.
        state.comp.nmatches = 5;
        state.styles.set(
            ":completion::t::e::",
            "insert",
            vec!["true".into()],
            false,
        );
        _all_matches_end(&mut state);
        assert_eq!(state.comp.params.compstate.insert, "all");
    }

    #[test]
    fn end_emits_all_matches_explanation_when_insert_style_false() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::e2:".into();
        state.ctx.completer = "complete".into();
        let _ = _all_matches(&mut state);
        state.comp.nmatches = 5;
        // No `insert` style → take the explanation branch.
        // First, add_explanation needs an existing group → seed one.
        state.comp.begin_group("all-matches", true);
        state.comp.end_group();
        _all_matches_end(&mut state);
        let exps: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(exps.iter().any(|s| *s == "all matches"));
    }

    #[test]
    fn end_avoids_default_avoid_list_completers() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::av:".into();
        state.ctx.completer = "_expand".into();
        let _ = _all_matches(&mut state);
        state.comp.nmatches = 5;
        _all_matches_end(&mut state);
        // `_expand` is in the default avoid-completer list → nothing
        // emitted.
        assert_ne!(state.comp.params.compstate.insert, "all");
    }

    #[test]
    fn end_context_cleared_after_call() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::clear:".into();
        let _ = _all_matches(&mut state);
        assert!(context_cell().lock().unwrap().is_some());
        _all_matches_end(&mut state);
        assert!(
            context_cell().lock().unwrap().is_none(),
            "_all_matches_end must `unset _all_matches_context`"
        );
    }

    #[test]
    fn end_does_nothing_when_only_one_match() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t::one:".into();
        let _ = _all_matches(&mut state);
        state.comp.nmatches = 1;
        let before = state.comp.params.compstate.insert.clone();
        _all_matches_end(&mut state);
        assert_eq!(state.comp.params.compstate.insert, before);
    }

    #[test]
    fn end_returns_false_when_no_context_recorded() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut state = MainCompleteState::new("", 0);
        // Skip _all_matches → no context recorded.
        assert!(!_all_matches_end(&mut state));
    }
}
