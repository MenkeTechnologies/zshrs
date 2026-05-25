//! Port of `_default` — `-default-` context handler.
//!
//! Local shell reference: `Completion/Zsh/Context/_default` (the only
//! upstream `_default` — there is no `Base/Utility/_default`).
//!
//! Upstream shell source (~27 lines, key dispatch):
//! ```text
//! #compdef -default-
//!
//! local ctl
//!
//! if { zstyle -s ":completion:${curcontext}:" use-compctl ctl ||
//!      zmodload -e zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]; then
//!   local opt
//!   opt=()
//!   [[ "$ctl" = *first* ]] && opt=(-T)
//!   [[ "$ctl" = *default* ]] && opt=("$opt[@]" -D)
//!   compcall "$opt[@]" || return 0
//! fi
//!
//! _files "$@" && return 0
//! ```
//!
//! Strict Rust port: skips the compctl legacy-shim (`zsh/compctl`
//! module is a deprecated compatibility layer we don't model) and
//! dispatches to `_files` with default opts — the modern compsys
//! default-completion behavior IS `_files`.

use crate::compcore::CompletionState;

use crate::ported::_files::{files_execute, FilesOpts};

/// _default - `-default-` context handler, falls through to `_files`.
pub fn _default(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_files_execute_default() {
        let mut state = CompletionState::new();
        state.params.prefix = "Carg".into();
        let _ = _default(&mut state);
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-no-such-file-xyz-123".into();
        assert!(!_default(&mut state));
    }

    #[test]
    fn no_panic_with_empty_prefix() {
        let mut state = CompletionState::new();
        let _ = _default(&mut state);
    }
}
