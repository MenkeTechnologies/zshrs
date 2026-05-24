//! Port of `_retrieve_cache` — retrieve completion data from cache.
//! Moved from `compsys/functions.rs`.

use crate::base::MainCompleteState;

/// _retrieve_cache - Retrieve completion data from cache
pub fn _retrieve_cache(state: &MainCompleteState, cache_name: &str) -> Option<Vec<String>> {
    let context = format!(":completion:{}:", state.ctx.context);

    if let Some(cache_path) = state.styles.lookup_values(&context, "cache-path") {
        if let Some(path) = cache_path.first() {
            let cache_file = format!("{}/{}", path, cache_name);
            if let Ok(contents) = std::fs::read_to_string(&cache_file) {
                return Some(contents.lines().map(String::from).collect());
            }
        }
    }

    None
}
