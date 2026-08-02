//! Port of `_cache_invalid` from
//! `Completion/Base/Utility/_cache_invalid`.
//!
//! Full upstream body (21 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 6  _cache_ident="$1"
//! sh: 8  # If the cache is disabled, we never want to rebuild it, so pretend
//! sh: 9  # it's valid.
//! sh:10  zstyle -t ":completion:${curcontext}:" use-cache || return 1
//! sh:12  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:13  : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:14  _cache_path="$_cache_dir/$_cache_ident"
//! sh:17  zstyle -s ":completion:${curcontext}:" cache-policy _cache_policy
//! sh:18  [[ -n "$_cache_policy" ]] && "$_cache_policy" "$_cache_path" && return 0
//! sh:20  return 1
//! ```
//!
//! Returns 0 when the cache needs rebuilding (per the user-supplied
//! `cache-policy` hook); 1 otherwise (cache disabled or no policy).

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::getsparam;

/// `_cache_invalid` — query the cache-policy hook for tag `$1`.
/// Returns 0 (cache stale) or 1 (cache fresh / disabled / no policy).
pub fn _cache_invalid(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_cache_invalid");
    // sh:6
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:10
    if testforstyle(&ctx, "use-cache") != 0 {
        return 1;
    }

    // sh:12-14
    let cache_dir = lookupstyle(&ctx, "cache-path")
        .first()
        .cloned()
        .unwrap_or_else(|| {
            let home = getsparam("ZDOTDIR")
                .filter(|s| !s.is_empty())
                .or_else(|| getsparam("HOME"))
                .unwrap_or_default();
            format!("{}/.zcompcache", home)
        });
    let cache_path = format!("{}/{}", cache_dir, cache_ident);

    // sh:17-18
    let policy = lookupstyle(&ctx, "cache-policy")
        .first()
        .cloned()
        .unwrap_or_default();
    if !policy.is_empty() && dispatch_function_call(&policy, &[cache_path]).unwrap_or(1) == 0 {
        return 0;
    }

    // sh:20
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_use_cache_disabled() {
        // sh:10 — without use-cache style set, returns 1.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_cache_invalid(&["my-cache".to_string()]), 1);
    }
}
