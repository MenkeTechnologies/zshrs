//! Port of `_requested` from `Completion/Base/Core/_requested`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt
//! sh: 4
//! sh: 5  __gopt=()
//! sh: 6  zparseopts -D -a __gopt 1 2 V J x
//! sh: 7
//! sh: 8  if comptags -R "$1"; then
//! sh: 9    if [[ $# -gt 3 ]]; then
//! sh:10      _all_labels - "$__gopt[@]" "$@" || return 1
//! sh:11    elif [[ $# -gt 1 ]]; then
//! sh:12      _description "$__gopt[@]" "$@"
//! sh:13    fi
//! sh:14    return 0
//! sh:15  else
//! sh:16    return 1
//! sh:17  fi
//! ```
//!
//! Three-clause dispatcher:
//! - With 1 positional arg → bare "is this tag wanted?" check (the
//! mode used by callers like `_files` and `_arguments` when they
//! just want the gate without emitting matches).
//! - With 2-3 positional args → call `_description` (gate + describe).
//! - With ≥4 positional args → call `_all_labels` (gate + emit via
//! the all-labels loop, which fans out across `tag-order` labels).
//!
//! Flag passthrough: `-1`, `-2`, `-V`, `-J`, `-x` are zparseopts'd
//! into `__gopt` and forwarded to `_all_labels` / `_description`. This
//! port mirrors that — parsed flags are returned alongside the gate
//! result so callers can forward them.
//!
//! The core "is tag wanted" decision is already implemented by
//! [`crate::compsys::base::TagManager::requested`] (see `compsys/base.rs:147`);
//! this port wraps it with the documented flag-parse + arg-count
//! dispatch semantics.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
