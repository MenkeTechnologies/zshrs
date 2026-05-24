//! Port of `_complete_debug` — debug completion. Moved from
//! `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};

/// _complete_debug - Debug completion
pub fn _complete_debug(state: &mut MainCompleteState) -> CompleterResult {
    // Print debug info
    eprintln!("Context: {}", state.ctx.context);
    eprintln!("Completer: {}", state.ctx.completer);
    eprintln!("Prefix: {}", state.comp.params.prefix);
    eprintln!("Suffix: {}", state.comp.params.suffix);
    eprintln!("Words: {:?}", state.comp.params.words);
    eprintln!("Current: {}", state.comp.params.current);
    CompleterResult::NoMatch
}
