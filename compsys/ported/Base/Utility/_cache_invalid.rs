//! Port of `_cache_invalid` — check if completion cache is invalid.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_cache_invalid`
//! (system copy `/opt/homebrew/share/zsh/functions/_cache_invalid`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  9  zstyle -t ":completion:${curcontext}:" use-cache || return 1
//! 11  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! 12  : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! 13  _cache_path="$_cache_dir/$_cache_ident"
//! 16  zstyle -s ":completion:${curcontext}:" cache-policy _cache_policy
//! 17  [[ -n "$_cache_policy" ]] && "$_cache_policy" "$_cache_path" && return 0
//! 19  return 1
//! ```
//!
//! Upstream gates on the `use-cache` zstyle (no-cache → always
//! treat as VALID so we don't bother re-querying), and otherwise
//! consults the `cache-policy` user-defined function.
//!
//! Simplified Rust port: returns true (invalid) when
//! `use-cache=no/false/off/0` is set, OR when no `cache-path`
//! style is set, OR when the cache file doesn't exist. Drops the
//! `cache-policy` user-fn callout (no shell-fn-invoke at the leaf).

use std::path::Path;

use crate::base::MainCompleteState;

/// _cache_invalid - Check if completion cache is invalid
pub fn _cache_invalid(state: &MainCompleteState, cache_name: &str) -> bool {
    let context = format!(":completion:{}:", state.ctx.context);

    // Check cache-policy style
    if let Some(policy) = state.styles.lookup_values(&context, "cache-policy") {
        // Would evaluate the policy function
        let _ = policy;
    }

    // Check use-cache style
    if let Some(use_cache) = state.styles.lookup_values(&context, "use-cache") {
        if let Some(v) = use_cache.first() {
            if v == "no" || v == "false" || v == "off" || v == "0" {
                return true;
            }
        }
    }

    // Check cache-path
    if let Some(cache_path) = state.styles.lookup_values(&context, "cache-path") {
        if let Some(path) = cache_path.first() {
            let cache_file = format!("{}/{}", path, cache_name);
            return !Path::new(&cache_file).exists();
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_cache_no_returns_invalid() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["no".into()],
            false,
        );
        assert!(_cache_invalid(&state, "any-name"));
    }

    #[test]
    fn no_cache_path_returns_invalid_by_default() {
        let state = MainCompleteState::new("", 0);
        assert!(_cache_invalid(&state, "x"));
    }

    #[test]
    fn nonexistent_cache_file_returns_invalid() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec!["/tmp/zshrs-nonexistent-cache-dir".into()],
            false,
        );
        assert!(_cache_invalid(&state, "nope.cache"));
    }
}
