//! Port of `_next_label` from `Completion/Base/Core/_next_label`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __descr __spec
//! sh: 4
//! sh: 5  __gopt=()
//! sh: 6  zparseopts -D -a __gopt 1 2 V J x
//! sh: 7
//! sh: 8  if comptags -A "$1" curtag __spec; then
//! sh: 9    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:10    _tags_level=$#funcstack
//! sh:11    _comp_tags="$_comp_tags $__spec "
//! sh:12    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:13      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:14      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:15      curtag="${curtag%:*}"
//! sh:16      set -A $2 "${(P@)2}" "${(@)argv[4,-1]}"
//! sh:17    else
//! sh:18      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:19      set -A $2 "${(@)argv[4,-1]}" "${(P@)2}"
//! sh:20    fi
//! sh:21
//! sh:22    return 0
//! sh:23  fi
//! sh:24
//! sh:25  return 1
//! ```
//!
//! Upstream uses `comptags -A` to advance the internal tag-set
//! iterator + extract the next label, then `_description` wraps
//! the result.
//!
//! Faithful Rust port: queries `TagManager::wanted(tag)` and emits
//! the tag name as the label. The `comptags` builtin's internal
//! iteration is the same shape — caller drives the loop with
//! repeated calls until None is returned.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
