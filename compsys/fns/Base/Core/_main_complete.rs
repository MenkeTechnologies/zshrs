//! Port of `_main_complete` — top-level completion entry point.
//!
//! Extracted from `compsys/base.rs` (was lines ~210-295). Mirrors zsh
//! upstream `Completion/Base/Core/_main_complete`. Walks the configured
//! completer list (`_complete` / `_approximate` / `_match` / …),
//! invoking each via the caller-supplied `dispatch` closure until one
//! returns matches.

use crate::base::{CompleterResult, MainCompleteState};

/// Main completion entry point (_main_complete)
///
/// This is THE function that gets called when the user presses TAB.
pub fn _main_complete(
    state: &mut MainCompleteState,
    dispatch: impl Fn(&mut MainCompleteState, &str) -> CompleterResult,
) -> i32 {
    // Get completers from style or use defaults
    if let Some(completers) = state
        .styles
        .lookup_values(&state.context_string(), "completer")
    {
        state.completers = completers.to_vec();
    }

    state.ctx.completer_num = 1;

    // Call pre-functions
    let prefuncs = state.prefuncs.clone();
    for func in &prefuncs {
        // Would call the function here
        let _ = func;
    }

    // Try each completer
    for completer_name in state.completers.clone() {
        // Extract completer name (handle _complete:foo syntax)
        let (completer, name) = if let Some(pos) = completer_name.find(':') {
            (&completer_name[..pos], &completer_name[pos + 1..])
        } else {
            (completer_name.as_str(), &completer_name[1..]) // strip leading _
        };

        state.ctx.completer = name.replace('_', "-");

        // Get matcher list
        let matchers = state
            .styles
            .lookup_values(&state.context_string(), "matcher-list")
            .map(|v| v.to_vec())
            .unwrap_or_else(|| vec![String::new()]);

        state.ctx.matcher_num = 1;

        for matcher in &matchers {
            state.ctx.matcher = matcher.clone();

            // Call the completer
            match dispatch(state, completer) {
                CompleterResult::Matched => {
                    state.ret = 0;
                    break;
                }
                CompleterResult::Skip => break,
                CompleterResult::NoMatch => {}
            }

            state.ctx.matcher_num += 1;
        }

        if state.ret == 0 {
            break;
        }

        state.ctx.completer_num += 1;
    }

    // Call post-functions
    let postfuncs = state.postfuncs.clone();
    for func in &postfuncs {
        let _ = func;
    }

    // Store lastcomp info
    state
        .lastcomp
        .insert("nmatches".to_string(), state.comp.nmatches.to_string());
    state
        .lastcomp
        .insert("completer".to_string(), state.ctx.completer.clone());
    state
        .lastcomp
        .insert("prefix".to_string(), state.comp.params.prefix.clone());
    state
        .lastcomp
        .insert("suffix".to_string(), state.comp.params.suffix.clone());

    state.ret
}
