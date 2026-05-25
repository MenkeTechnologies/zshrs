//! Port of `_math` from `Completion/Zsh/Context/_math`.
//!
//! Full upstream body (14 lines verbatim):
//! ```text
//! sh: 1  #compdef -math- let
//! sh: 2
//! sh: 3  if [[ "$PREFIX" = *[^a-zA-Z0-9_]* ]]; then
//! sh: 4    IPREFIX="$IPREFIX${PREFIX%%[a-zA-Z0-9_]#}"
//! sh: 5    PREFIX="${PREFIX##*[^a-zA-Z0-9_]}"
//! sh: 6  fi
//! sh: 7  if [[ "$SUFFIX" = *[^a-zA-Z0-9_]* ]]; then
//! sh: 8    ISUFFIX="${SUFFIX##[a-zA-Z0-9_]#}$ISUFFIX"
//! sh: 9    SUFFIX="${SUFFIX%%[^a-zA-Z0-9_]*}"
//! sh:10  fi
//! sh:11
//! sh:12  _alternative 'math-parameters:math parameter: _math_params' \
//! sh:13      'user-math-functions:user math function: _user_math_func' \
//! sh:14      'module-math-functions:math function from zsh/mathfunc: _module_math_func'
//! ```
//!
//! Strict Rust port: faithful 1:1 — strips non-identifier chars
//! from PREFIX/SUFFIX (rewriting into IPREFIX/ISUFFIX), then
//! dispatches via [`_alternative`] with the exact three specs
//! upstream uses. Caller injects the data the three Zsh/Type
//! helpers need (params, user math fns, module math fns).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
