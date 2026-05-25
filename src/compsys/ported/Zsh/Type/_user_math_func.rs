//! Port of `_user_math_func` from `Completion/Zsh/Type/_user_math_func`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl
//! sh: 4  local -a funcs
//! sh: 5
//! sh: 6  funcs=(${${${(f)"$(functions -M)"}##functions -M }%% *})
//! sh: 7
//! sh: 8  _wanted user-math-functions expl 'user math function' \
//! sh: 9      compadd -S '(' -q "$@" -a funcs
//! ```
//!
//! `functions -M` lists user-defined math functions (zsh's
//! `zmathfuncdef`-style functions). Each emitted name gets the `(`
//! suffix + NOSPACE so the user keeps typing the argument list.
//!
//! Strict Rust port: caller injects the function-name list.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
