//! Port of `_message` — display a message (no completions).
//!
//! Extracted from `compsys/base.rs` (was `pub fn message`, lines
//! ~840-854). Renamed to `_message` to match the upstream zsh shell
//! function name at `Completion/Base/Core/_message`.

use crate::compcore::CompletionState;
use crate::fns::_description::_description;
use crate::zstyle::ZStyleStore;

/// _message - display a message (no completions)
pub fn _message(
    state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    message: &str,
) {
    let formatted = _description(state, styles, context, tag, message);

    if let Some(msg) = formatted {
        state.begin_group(tag, true);
        state.add_explanation(msg, Some(tag));
        state.end_group();
    }
}
