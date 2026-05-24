//! Port of `_as_if` — complete as if in different context. Moved from
//! `compsys/functions.rs`. Renamed from `as_if` to mirror zsh shell
//! function name `_as_if`.

use crate::base::MainCompleteState;

/// _as_if - Complete as if in different context
pub fn _as_if(
    state: &mut MainCompleteState,
    new_context: &str,
    action: impl FnOnce(&mut MainCompleteState) -> bool,
) -> bool {
    let old_context = state.ctx.context.clone();
    state.ctx.context = new_context.to_string();

    let result = action(state);

    state.ctx.context = old_context;
    result
}
