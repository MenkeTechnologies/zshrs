//! Port of `_gnu_generic` from `Completion/Unix/Command/_gnu_generic`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is for GNU-like commands which understand the --help option,
//! sh: 4  # but which do not otherwise require special completion handling.
//! sh: 5
//! sh: 6  _arguments '*:arg: _default' --
//! ```
//!
//! Upstream lets `_arguments` handle the `--help` parsing — it
//! recognises `--option` and `-o` from the cmd's own help output
//! and offers them as completions, plus `_default` for positional
//! args.
//!
//! Simplified Rust port: forks `<command> --help`, parses
//! dash-prefixed options from the output (`--option`, `--option=`,
//! `-o`) and emits them. Doesn't yet delegate positional args to
//! `_files` like the shell's `_default` would.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
