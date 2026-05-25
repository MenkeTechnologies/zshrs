//! Port of `_cache_invalid` from `Completion/Base/Utility/_cache_invalid`.
//!
//! Full upstream body (21 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  #
//! sh: 3  # Function to decide whether a completions cache needs rebuilding
//! sh: 4
//! sh: 5  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 6  _cache_ident="$1"
//! sh: 7
//! sh: 8  # If the cache is disabled, we never want to rebuild it, so pretend
//! sh: 9  # it's valid.
//! sh:10  zstyle -t ":completion:${curcontext}:" use-cache || return 1
//! sh:11
//! sh:12  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:13  : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:14  _cache_path="$_cache_dir/$_cache_ident"
//! sh:15
//! sh:16  # See whether the caching policy says that the cache needs rebuilding
//! sh:17  # (the policy will return 0 if it does).
//! sh:18  zstyle -s ":completion:${curcontext}:" cache-policy _cache_policy
//! sh:19  [[ -n "$_cache_policy" ]] && "$_cache_policy" "$_cache_path" && return 0
//! sh:20
//! sh:21  return 1
//! ```
//!
//! Upstream gates on the `use-cache` zstyle (no-cache → return 1
//! meaning "NOT invalid, caller skips reload") and otherwise
//! consults the `cache-policy` user-defined function. If no
//! cache-policy → return 1 (treat as NOT invalid).
//!
//! Strict Rust port: matches upstream return semantics exactly.
//! `true` = "cache IS invalid, reload" (shell return 0). `false` =
//! "cache is fine, skip reload" (shell return 1). The cache-policy
//! callout dispatches through `_call_function` so registered Rust
//! callbacks (or shell-fn shims) decide invalidity.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
