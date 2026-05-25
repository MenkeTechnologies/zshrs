//! Port of `_autocd` from `Completion/Zsh/Context/_autocd`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef -command-
//! sh: 2
//! sh: 3  _command_names
//! sh: 4  local ret=$?
//! sh: 5  [[ -o autocd ]] && _cd || return ret
//! ```
//!
//! Strict Rust port: faithful 1:1 — calls our ported
//! [`_command_names`]. When the shell `autocd` option is set
//! (caller-supplied via `autocd_set`), additionally tries `_cd`
//! (per-command completer; not in the engine layer — caller
//! dispatches that themselves).
//!
//! TODO: `_cd` is a Zsh/Command per-builtin completer not in the
//! engine port. Caller passes a closure that runs `_cd` when
//! `autocd_set=true` AND `_command_names` returned false (i.e. the
//! bareword is a directory, not a command).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
