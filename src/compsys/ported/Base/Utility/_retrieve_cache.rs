//! Port of `_retrieve_cache` from `Completion/Base/Utility/_retrieve_cache`.
//!
//! Full upstream body (30 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  #
//! sh: 3  # Retrieval component of completions caching layer
//! sh: 4
//! sh: 5  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 6  _cache_ident="$1"
//! sh: 7
//! sh: 8  if zstyle -t ":completion:${curcontext}:" use-cache; then
//! sh: 9    # Decide which directory to retrieve cache from, and ensure it exists
//! sh:10    zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:11    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:12    if [[ ! -d "$_cache_dir" ]]; then
//! sh:13      [[ -e "$_cache_dir" ]] &&
//! sh:14        _message "cache-dir ($_cache_dir) isn't a directory\!"
//! sh:15      return 1
//! sh:16    fi
//! sh:17
//! sh:18    _cache_path="$_cache_dir/$_cache_ident"
//! sh:19
//! sh:20    if [[ -e "$_cache_path" ]]; then
//! sh:21      _cache_invalid "$_cache_ident" && return 1
//! sh:22
//! sh:23      . "$_cache_path"
//! sh:24      return 0
//! sh:25    else
//! sh:26      return 1
//! sh:27    fi
//! sh:28  else
//! sh:29    return 1
//! sh:30  fi
//! ```
//!
//! Upstream `builtin . "$_cache_path"` SOURCES the cache file
//! (which is a shell script that sets shell-side parameters).
//!
//! Simplified Rust port: reads the cache file as plain text and
//! returns one entry per line. Loses the "sourced parameter
//! assignments" semantic but covers the common case where the
//! cache file is a wordlist (one candidate per line).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
