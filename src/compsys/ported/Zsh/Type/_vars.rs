//! Port of `_vars` from `Completion/Zsh/Type/_vars`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #compdef getopts unset
//! sh: 2
//! sh: 3  # This will handle completion of keys of associative arrays, e.g. at
//! sh: 4  # `vared foo[<TAB>' could complete to `vared foo[key]'.
//! sh: 5
//! sh: 6  local ret=1
//! sh: 7
//! sh: 8  if [[ $PREFIX = *\[* ]]; then
//! sh: 9    compstate[parameter]=${PREFIX%%(|\\)\[*}
//! sh:10
//! sh:11    IPREFIX=${PREFIX%%\[*}\[
//! sh:12    PREFIX=${PREFIX#*\[}
//! sh:13
//! sh:14    _subscript -q
//! sh:15  else
//! sh:16    _parameters -g '^a*' "$@" && ret=0
//! sh:17
//! sh:18    if compset -S '\[*'; then
//! sh:19      set - -S "" "$@"
//! sh:20    else
//! sh:21      set - -qS"${${QIPREFIX:+[}:-\[}" "$@"
//! sh:22    fi
//! sh:23    _parameters -g 'a*' "$@" && ret=0
//! sh:24    return ret
//! sh:25  fi
//! ```
//!
//! Two faithful `_parameters` calls upstream — first for non-array
//! types (`-g '^a*'`), then for array types (`-g 'a*'`) with `[`
//! as the auto-suffix (unless SUFFIX already starts with `[`, in
//! which case the auto-suffix is suppressed).
//!
//! Strict Rust port: routes both calls through
//! [`_parameters_with_opts`] with the exact `-g`/`-S` flags shell
//! uses.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
