//! Port of `_default` — default completion (files).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_default`
//! (system copy `/opt/homebrew/share/zsh/functions/_default`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  5  if { zstyle -s ":completion:${curcontext}:" use-compctl ctl ||
//!  6       zmodload -e zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]; then
//! 11    compcall "$opt[@]" || return 0
//! 14  _files "$@" && return 0
//! 17  # magicequalsubst allows arguments like <any-old-stuff>=~/foo …
//! ```
//!
//! Upstream first tries `compcall` (legacy compctl fallback) if the
//! `use-compctl` zstyle is set; then falls back to `_files`.
//!
//! Simplified Rust port: skips the compctl fallback (legacy
//! compatibility shim we don't need) and just calls `files_execute`
//! with default opts. The user-visible default-completion behavior
//! IS `_files` for the modern compsys.

use crate::compcore::CompletionState;

use super::_files::{files_execute, FilesOpts};

/// _default - Default completion (files)
pub fn _default(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_files_execute_default() {
        // Pin: _default IS files_execute(FilesOpts::default()).
        // Whether matches return depends on test cwd contents —
        // we only assert the call doesn't panic and exercises the
        // delegation path.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg".into();
        let _ = _default(&mut state);
    }
}
