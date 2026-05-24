//! Port of `_default` — `-default-` context handler.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_default`.
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
//! Strict Rust port: faithful 1:1 — skips the compctl legacy-shim
//! (zsh/compctl module is a deprecated compatibility layer we
//! don't model) and dispatches to our ported [`crate::ported::_default`]
//! at `Base/Utility/_default.rs` which already calls `_files`.
//!
//! NOTE: this file is the `-default-` CONTEXT dispatcher (per
//! `#compdef -default-`); the `Base/Utility/_default` is the
//! lower-level wrapper. Both exist upstream at different paths and
//! we mirror that — see `Base/Utility/_default.rs` for the inner
//! implementation.

use crate::compcore::CompletionState;

/// `_default` (context) — fall through to `_files` via
/// [`crate::ported::_default::_default`].
pub fn _default(state: &mut CompletionState) -> bool {
    crate::ported::_default::_default(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_base_utility_default() {
        let mut s1 = CompletionState::new();
        let mut s2 = CompletionState::new();
        s1.params.prefix = "Cargo.t".into();
        s2.params.prefix = "Cargo.t".into();
        assert_eq!(_default(&mut s1), crate::ported::_default::_default(&mut s2));
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-no-such-prefix-anywhere".into();
        assert!(!_default(&mut state));
    }

    #[test]
    fn no_panic_with_empty_prefix() {
        let mut state = CompletionState::new();
        let _ = _default(&mut state);
    }
}
