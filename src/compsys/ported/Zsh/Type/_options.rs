//! Port of `_options` from `Completion/Zsh/Type/_options`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This should be used to complete all option names.
//! sh: 4
//! sh: 5  local expl
//! sh: 6
//! sh: 7  _wanted zsh-options expl 'zsh option' \
//! sh: 8      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -k - options
//! ```
//!
//! Upstream uses the matchspec:
//! `B:[nN][oO]=`    — strip leading `no` from PREFIX (so
//! `EXTENDED_GLOB` matches `noEXTENDED_GLOB`)
//! `M:_=`           — underscore in input matches any char
//! `M:{A-Z}={a-z}`  — case-fold (input upper matches table lower)
//!
//! Then `compadd -k options` pulls names from `$options` (zsh's
//! built-in associative array).
//!
//! Faithful Rust port: takes the option list `&[(name, is_set)]`
//! from the caller (since we don't have direct access to the shell
//! `$options` array at the leaf) AND implements the upstream
//! matchspec by normalising both sides (drop leading `no`, case-fold,
//! treat `_` as wildcard). Emits each matching name with
//! `name (set)` / `name (unset)` disp formatting so the user can
//! see the current state.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
