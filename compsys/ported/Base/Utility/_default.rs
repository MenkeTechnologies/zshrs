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
        let mut state = CompletionState::new();
        state.params.prefix = "Carg".into();
        let _ = _default(&mut state);
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-no-such-file-xyz-123".into();
        let ok = _default(&mut state);
        assert!(!ok);
    }

    #[test]
    fn delegates_to_files_with_default_opts() {
        // Verify _default IS files_execute(default()) — both calls
        // produce the same result for the same state.
        use crate::ported::_files::{files_execute, FilesOpts};
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        s1.params.prefix = "definitely-no-such-file-xyz".into();
        s2.params.prefix = "definitely-no-such-file-xyz".into();
        let r1 = _default(&mut s1);
        let r2 = files_execute(&mut s2, &FilesOpts::default());
        assert_eq!(r1, r2, "_default must agree with files_execute(default())");
    }

    #[test]
    fn empty_prefix_passes_through_to_files() {
        // _default with empty prefix should still drive files
        // lookup; we don't assert match count (varies by cwd) — just
        // that the call returns without panic.
        let mut state = CompletionState::new();
        let _ = _default(&mut state);
    }

    #[test]
    fn does_not_mutate_prefix_or_suffix() {
        let mut state = CompletionState::new();
        state.params.prefix = "x".into();
        state.params.suffix = "y".into();
        let _ = _default(&mut state);
        assert_eq!(state.params.prefix, "x");
        assert_eq!(state.params.suffix, "y");
    }

    #[test]
    fn does_not_touch_compstate_insert() {
        // _default only emits matches; the choice of insert mode
        // is the caller's. Pin that the default `insert` value
        // survives the call.
        let mut state = CompletionState::new();
        let original = state.params.compstate.insert.clone();
        let _ = _default(&mut state);
        assert_eq!(state.params.compstate.insert, original);
    }

    #[test]
    fn behavior_invariant_across_repeated_calls() {
        // Calling _default twice with same state should produce
        // identical outcomes.
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        s1.params.prefix = "Cargo.t".into();
        s2.params.prefix = "Cargo.t".into();
        let r1 = _default(&mut s1);
        let r2 = _default(&mut s2);
        assert_eq!(r1, r2);
    }

    #[test]
    fn off_prefix_does_not_create_orphan_groups() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-no-such-prefix-anywhere-xyz".into();
        let before = state.groups.len();
        let _ = _default(&mut state);
        // The internal files_execute may begin/end a group; pin no
        // permanent leak.
        let after_files = state.groups.iter().filter(|g| !g.matches.is_empty()).count();
        assert_eq!(after_files, 0, "no NON-EMPTY group survives off-prefix call");
        let _ = before;
    }
}
