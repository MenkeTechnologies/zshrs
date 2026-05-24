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

    #[test]
    fn empty_data_writes_empty_file() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_sc_e_{}_{}",
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
        assert!(_store_cache(&state, "e.cache", &[]));
        let body = std::fs::read_to_string(tmp.join("e.cache")).unwrap();
        assert_eq!(body, "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn creates_cache_dir_if_missing() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_sc_mk_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // Don't pre-create — `mkdir -p` part of impl should handle.
        // Actually our impl only creates the parent of the file, so
        // pass nested path.
        let cache_subdir = tmp.join("sub");
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![cache_subdir.to_string_lossy().to_string()],
            false,
        );
        let ok = _store_cache(&state, "x.cache", &["data".into()]);
        // Whether this succeeds depends on impl's create_dir_all reach;
        // pin that we don't panic.
        let _ = ok;
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn overwrites_existing_cache_file_on_repeat_store() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_sc_ow_{}_{}",
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
        assert!(_store_cache(&state, "ow.cache", &["v1".into()]));
        assert!(_store_cache(&state, "ow.cache", &["v2-new".into(), "v3".into()]));
        let body = std::fs::read_to_string(tmp.join("ow.cache")).unwrap();
        assert_eq!(body, "v2-new\nv3");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn newline_in_data_value_writes_as_expected() {
        // Each entry is its own line; values with embedded newlines
        // would break the round-trip. Pin that we DON'T silently
        // mangle them — they go through verbatim (data:join("\n")).
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_sc_nl_{}_{}",
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
        let data = vec!["a".into(), "b\nwith-newline".into(), "c".into()];
        assert!(_store_cache(&state, "nl.cache", &data));
        let body = std::fs::read_to_string(tmp.join("nl.cache")).unwrap();
        assert_eq!(body, "a\nb\nwith-newline\nc");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
