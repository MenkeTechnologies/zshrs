//! Port of `_tags` from `Completion/Base/Core/_tags`.
//!
//! Full upstream body (67 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local prev
//! sh: 4
//! sh: 5  # A `--' as the first argument says that we should tell comptags to use
//! sh: 6  # the preceding function nesting level. This is only documented here because
//! sh: 7  # if everything goes well, users won't have to worry about it and should
//! sh: 8  # not mess with it.
//! sh: 9
//! sh:10  if [[ "$1" = -- ]]; then
//! sh:11    prev=-
//! sh:12    shift
//! sh:13  fi
//! sh:14
//! sh:15  if (( $# )); then
//! sh:16
//! sh:17    # We have arguments: the tags supported in this context.
//! sh:18
//! sh:19    local curcontext="$curcontext" order tag nodef tmp
//! sh:20
//! sh:21    if [[ "$1" = -C?* ]]; then
//! sh:22      curcontext="${curcontext%:*}:${1[3,-1]}"
//! sh:23      shift
//! sh:24    elif [[ "$1" = -C ]]; then
//! sh:25      curcontext="${curcontext%:*}:${2}"
//! sh:26      shift 2
//! sh:27    fi
//! sh:28
//! sh:29    [[ "$1" = -(|-) ]] && shift
//! sh:30
//! sh:31    zstyle -a ":completion:${curcontext}:" group-order order &&
//! sh:32        compgroups "$order[@]"
//! sh:33
//! sh:34    # Set and remember offered tags.
//! sh:35
//! sh:36    comptags "-i$prev" "$curcontext" "$@"
//! sh:37
//! sh:38    # Sort the tags.
//! sh:39
//! sh:40    if [[ -n "$_sort_tags" ]]; then
//! sh:41      "$_sort_tags" "$@"
//! sh:42    else
//! sh:43      zstyle -a ":completion:${curcontext}:" tag-order order ||
//! sh:44          (( ! ${@[(I)options]} )) ||
//! sh:45          order=('(|*-)argument-* (|*-)option[-+]* values' options)
//! sh:46
//! sh:47      for tag in $order; do
//! sh:48        case $tag in
//! sh:49        -)     nodef=yes;;
//! sh:50        \!*)   comptry "${(@)argv:#(${(j:|:)~${=~tag[2,-1]}})}";;
//! sh:51        ?*)    comptry -m "$tag";;
//! sh:52        esac
//! sh:53      done
//! sh:54
//! sh:55      [[ -z "$nodef" ]] && comptry "$@"
//! sh:56    fi
//! sh:57
//! sh:58    # Return non-zero if at least one set of tags should be used.
//! sh:59
//! sh:60    comptags "-T$prev"
//! sh:61
//! sh:62    return
//! sh:63  fi
//! sh:64
//! sh:65  # The other mode: switch to the next set of tags.
//! sh:66
//! sh:67  comptags "-N$prev"
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
