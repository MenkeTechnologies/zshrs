//! Port of `_store_cache` — store completion data to cache.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_store_cache`
//! (system copy `/opt/homebrew/share/zsh/functions/_store_cache`).
//!
//! Upstream shell source (key lines from the 64-line fn):
//! ```text
//! 10  zstyle -t ":completion:${curcontext}:" use-cache
//! 14  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! 22  if [[ ! -d "$_cache_dir" ]]; then
//! 23    mkdir -p "$_cache_dir"
//! 28  _cache_path="$_cache_dir/$_cache_ident"
//! 40  print -r -- "${(P)_cache_field[1]}"=\""${(P)_cache_field[1]}"\" \
//!         > "$_cache_path"
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

use std::path::Path;

use crate::base::MainCompleteState;

/// _store_cache - Store completion data to cache
pub fn _store_cache(state: &MainCompleteState, cache_name: &str, data: &[String]) -> bool {
    let context = format!(":completion:{}:", state.ctx.context);

    if let Some(cache_path) = state.styles.lookup_values(&context, "cache-path") {
        if let Some(path) = cache_path.first() {
            let cache_file = format!("{}/{}", path, cache_name);

            // Ensure directory exists
            if let Some(parent) = Path::new(&cache_file).parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let contents = data.join("\n");
            return std::fs::write(&cache_file, contents).is_ok();
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::_retrieve_cache::_retrieve_cache;

    #[test]
    fn no_cache_path_returns_false() {
        let state = MainCompleteState::new("", 0);
        assert!(!_store_cache(&state, "x", &["a".into()]));
    }

    #[test]
    fn store_then_retrieve_round_trips() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_sc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![tmp.to_string_lossy().to_string()],
            false,
        );

        let data = vec!["alpha".into(), "beta".into(), "gamma".into()];
        assert!(_store_cache(&state, "round.cache", &data));
        let got = _retrieve_cache(&state, "round.cache").expect("cache file readable");
        assert_eq!(got, data, "store→retrieve round-trip mismatch");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
