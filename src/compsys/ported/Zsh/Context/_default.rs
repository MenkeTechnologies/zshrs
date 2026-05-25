//! Port of `_default` from `Completion/Zsh/Context/_default`.
//!
//! Full upstream body (27 lines verbatim):
//! ```text
//! sh: 1  #compdef -default-
//! sh: 2
//! sh: 3  local ctl
//! sh: 4
//! sh: 5  if { zstyle -s ":completion:${curcontext}:" use-compctl ctl ||
//! sh: 6       zmodload -e zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]; then
//! sh: 7    local opt
//! sh: 8
//! sh: 9    opt=()
//! sh:10    [[ "$ctl" = *first* ]] && opt=(-T)
//! sh:11    [[ "$ctl" = *default* ]] && opt=("$opt[@]" -D)
//! sh:12    compcall "$opt[@]" || return 0
//! sh:13  fi
//! sh:14
//! sh:15  _files "$@" && return 0
//! sh:16
//! sh:17  # magicequalsubst allows arguments like <any-old-stuff>=~/foo to do
//! sh:18  # file name expansion after the =.  In that case, it's natural to
//! sh:19  # allow completion to handle file names after any equals sign.
//! sh:20
//! sh:21  if [[ -o magicequalsubst && "$PREFIX" = *\=* ]]; then
//! sh:22    compstate[parameter]="${PREFIX%%\=*}"
//! sh:23    compset -P 1 '*='
//! sh:24    _value "$@"
//! sh:25  else
//! sh:26    return 1
//! sh:27  fi
//! ```
//!
//! Faithful re-port: structure mirrors shell's three-branch shape —
//! compctl shim (sh:5-13), `_files` fallback (sh:15), `magicequalsubst`
//! special case (sh:21-26).
//!
//! Skipped branches (documented as `// rust:` divergences):
//! - sh:5-13 (compctl): `zsh/compctl` is a deprecated compatibility
//!   module not present in zshrs. The whole `use-compctl` zstyle is
//!   moot — we always fall through to `_files`.
//! - sh:24 `_value "$@"`: `_value` is currently a stub in our port
//!   (see [`crate::compsys::ported::_value::_value`]). The magicequalsubst
//!   branch enters but the inner dispatch is a no-op until `_value`
//!   is implemented.
//!
//! Shell-local parity:
//! - `ctl` (sh:3): captures the `use-compctl` zstyle value; unused in
//!   the Rust port because the compctl branch is skipped. Documented
//!   inline at sh:3 for traceability.
//! - `opt` (sh:7): inner-scope `compcall` arg accumulator; also unused
//!   because the compctl branch is skipped.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
