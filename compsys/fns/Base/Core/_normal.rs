//! Port of `_normal` — normal command completion.
//!
//! Extracted from `compsys/base.rs` (was lines ~298-316). Mirrors zsh
//! upstream `Completion/Base/Core/_normal`. Determines whether the
//! current word is the command name (position 1) or an argument, and
//! dispatches accordingly.
//!
//! Note: the original base.rs implementation invoked a 2-arg
//! `_dispatch(state, cmd)` stub that always returned `NoMatch`. The
//! current `fns/Base/Core/_dispatch.rs` port has a richer 3-arg
//! signature (state, comps, commands). Until the shell-side comps
//! table is wired through `_normal`, this port preserves the original
//! no-dispatch behavior — returns `NoMatch` for unrecognized commands.

use crate::base::{CompleterResult, MainCompleteState};

/// _normal - normal command completion
pub fn _normal(state: &mut MainCompleteState) -> CompleterResult {
    let current = state.comp.params.current as usize;

    // Completing command name (position 1)?
    if current == 1 {
        // Would dispatch to -command- completion
        return CompleterResult::NoMatch;
    }

    // Get the command name
    let cmd = if !state.comp.params.words.is_empty() {
        state.comp.params.words[0].clone()
    } else {
        return CompleterResult::NoMatch;
    };

    // Would dispatch to command-specific completion via `_dispatch`
    // — see module doc comment.
    let _ = cmd;
    CompleterResult::NoMatch
}
