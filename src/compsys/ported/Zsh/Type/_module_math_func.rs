//! Port of `_module_math_func` from `Completion/Zsh/Type/_module_math_func`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local mod
//! sh: 4  local -a funcs alts
//! sh: 5  local -a modules=( example mathfunc system random )
//! sh: 6
//! sh: 7  for mod in $modules; do
//! sh: 8    funcs=( ${${${(f)"$(zmodload -Fl zsh/$mod 2>/dev/null)"}:#^+f:*}##+f:} )
//! sh: 9    alts+=( "module-math-functions.${mod}:math function from zsh/${mod}:compadd -S '(' $funcs" )
//! sh:10  done
//! sh:11
//! sh:12  _alternative $alts
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the alternatives list
//! per upstream verbatim, then dispatches via [`_alternative`].
//! The action string carries the literal `compadd -S '(' fn1 fn2
//! …` invocation; the handler closure parses it and emits each
//! `fn` with `(` suffix + NOSPACE (the `-S '('` semantic).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
