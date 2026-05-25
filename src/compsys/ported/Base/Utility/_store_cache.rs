//! Port of `_store_cache` from `Completion/Base/Utility/_store_cache`.
//!
//! Full upstream body (64 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  #
//! sh: 3  # Storage component of completions caching layer
//! sh: 4
//! sh: 5  local _cache_ident _cache_ident_dir _cache_dir
//! sh: 6  _cache_ident="$1"
//! sh: 7
//! sh: 8  if zstyle -t ":completion:${curcontext}:" use-cache; then
//! sh: 9    # Decide which directory to cache to, and ensure it exists
//! sh:10    zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:11    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:12    if [[ ! -d "$_cache_dir" ]]; then
//! sh:13      if [[ -e "$_cache_dir" ]]; then
//! sh:14        _message "cache-dir style points to a non-directory\!"
//! sh:15      else
//! sh:16        # if module load fails, we *should* be okay using normal mkdir so
//! sh:17        # we load feature b:mkdir instead of b:zf_mkdir; note that modules
//! sh:18        # loaded in a sub-shell don't affect the parent.
//! sh:19        ( zmodload -F zsh/files b:mkdir; mkdir -m 0700 -p "$_cache_dir"
//! sh:20        ) 2>/dev/null
//! sh:21        if [[ ! -d "$_cache_dir" ]]; then
//! sh:22          _message "couldn't create cache-dir $_cache_dir"
//! sh:23          return 1
//! sh:24        fi
//! sh:25      fi
//! sh:26    fi
//! sh:27    _cache_ident_dir="$_cache_dir/$_cache_ident"
//! sh:28    _cache_ident_dir="$_cache_ident_dir:h"
//! sh:29
//! sh:30    if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:31      if [[ -e "$_cache_ident_dir" ]]; then
//! sh:32        _message "cache ident dir points to a non-directory:$_cache_ident_dir"
//! sh:33      else
//! sh:34        # See also rationale in zmodload above
//! sh:35        ( zmodload -F zsh/files b:mkdir; mkdir -m 0700 -p "$_cache_ident_dir"
//! sh:36        ) 2>/dev/null
//! sh:37        if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:38          _message "couldn't create cache-ident_dir $_cache_ident_dir"
//! sh:39          return 1
//! sh:40        fi
//! sh:41      fi
//! sh:42    fi
//! sh:43
//! sh:44
//! sh:45    shift
//! sh:46    for var; do
//! sh:47      case ${(Pt)var} in
//! sh:48      (*readonly*) ;;
//! sh:49      (*(association|array)*)
//! sh:50  	# Dump the array as a here-document to reduce parsing overhead
//! sh:51  	# when reloading the cache with "source" from _retrieve_cache
//! sh:52  	print -r "$var=( "'${(Q)"${(z)$(<<\EO:'"$var"
//! sh:53  	print -r "${(kv@Pqq)^^var}"
//! sh:54  	print -r "EO:$var"
//! sh:55  	print -r ')}"} )'
//! sh:56  	;;
//! sh:57      (*) print -r "$var=${(Pqq)^^var}";;
//! sh:58      esac
//! sh:59    done >! "$_cache_dir/$_cache_ident"
//! sh:60  else
//! sh:61    return 1
//! sh:62  fi
//! sh:63
//! sh:64  return 0
//! ```
//!
//! Upstream writes shell-eval-safe `name=("v1" "v2" …)` assignments
//! into the cache so `_retrieve_cache`'s `builtin .` re-sources
//! them into the calling shell.
//!
//! Simplified Rust port: writes the data slice joined by newlines
//! (one entry per line). Pairs with `_retrieve_cache`'s
//! line-by-line read for round-trip — pinned by the
//! `store_then_retrieve_round_trips` test.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
