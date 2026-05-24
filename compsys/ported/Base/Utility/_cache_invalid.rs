//! Port of `_cache_invalid` — check if completion cache is invalid.
//! Moved from `compsys/functions.rs`.

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
