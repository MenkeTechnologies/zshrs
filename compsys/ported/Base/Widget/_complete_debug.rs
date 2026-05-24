//! Port of `_complete_debug` — debug completion.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_complete_debug`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete_debug`).
//!
//! Upstream shell source is a `complete-word` widget that runs
//! completion under `xtrace` (`set -x`) and writes the trace to a
//! tmpfile + opens it in a pager so the user can see what compsys
//! actually did. The first ~30 lines:
//! ```text
//!  3  _complete_debug () {
//!  4    eval "$_comp_setup"
//!  6    local tmpf
//!  7    tmpf=$(mktemp ${TMPDIR:-/tmp}/zsh-compdebug-XXXXXXXX)
//! 12    {
//! 13      set -x
//! 14      _main_complete "$@"
//! 15      set +x
//! 16    } 2> $tmpf
//! ```
//!
//! Simplified Rust port: emits a one-shot diagnostic dump (context,
//! completer, prefix, suffix, words, current) to stderr and returns
//! `NoMatch`. The user gets the same "show me what compsys thinks
//! the state is" view without paging through an `xtrace` log.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_no_match_no_panic() {
        let mut state = MainCompleteState::new("hello world", 11);
        assert!(matches!(
            _complete_debug(&mut state),
            CompleterResult::NoMatch
        ));
    }
}
