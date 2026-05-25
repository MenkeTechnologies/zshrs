//! Port of `_all_labels` from `Completion/Base/Core/_all_labels`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
//! sh: 4
//! sh: 5  if [[ "$1" = - ]]; then
//! sh: 6    __prev=-
//! sh: 7    shift
//! sh: 8  fi
//! sh: 9
//! sh:10  __gopt=()
//! sh:11  zparseopts -D -a __gopt 1 2 V J x
//! sh:12
//! sh:13  __tmp=${argv[(ib:4:)-]}
//! sh:14  __len=$#
//! sh:15  if [[ __tmp -lt __len ]]; then
//! sh:16    __pre=$(( __tmp-1 ))
//! sh:17    __suf=$__tmp
//! sh:18  elif [[ __tmp -eq $# ]]; then
//! sh:19    __pre=-2
//! sh:20    __suf=$(( __len+1 ))
//! sh:21  else
//! sh:22    __pre=4
//! sh:23    __suf=5
//! sh:24  fi
//! sh:25
//! sh:26  while comptags "-A$__prev" "$1" curtag __spec; do
//! sh:27    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:28    _tags_level=$#funcstack
//! sh:29    _comp_tags="$_comp_tags $__spec "
//! sh:30    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:31      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:32      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:33      curtag="${curtag%:*}"
//! sh:34
//! sh:35      "$4" "${(P@)2}" "${(@)argv[5,-1]}" && __ret=0
//! sh:36    else
//! sh:37      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:38
//! sh:39      "${(@)argv[4,__pre]}" "${(P@)2}" "${(@)argv[__suf,-1]}" && __ret=0
//! sh:40    fi
//! sh:41  done
//! sh:42
//! sh:43  return __ret
//! ```
//!
//! Upstream loops over `_next_label` until no more labels remain,
//! eval'ing the caller-supplied command after each label-substitution.
//!
//! Faithful Rust port: convenience wrapper around `_next_label` that
//! runs the supplied closure for each label of the given tag,
//! emitting the description as a group explanation. Same loop shape
//! as shell's `while _next_label …; do eval "$command"; done`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
