//! Port of `_retrieve_cache` — retrieve completion data from cache.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_retrieve_cache`
//! (system copy `/opt/homebrew/share/zsh/functions/_retrieve_cache`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  7  if zstyle -t ":completion:${curcontext}:" use-cache; then
//!  9    zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! 10    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! 11    if [[ ! -d "$_cache_dir" ]]; then
//! 16    _cache_path="$_cache_dir/$_cache_ident"
//! 18    if [[ -e "$_cache_path" ]]; then
//! 19      builtin . "$_cache_path"
//! ```
//!
//! Upstream `builtin . "$_cache_path"` SOURCES the cache file
//! (which is a shell script that sets shell-side parameters).
//!
//! Simplified Rust port: reads the cache file as plain text and
//! returns one entry per line. Loses the "sourced parameter
//! assignments" semantic but covers the common case where the
//! cache file is a wordlist (one candidate per line).

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn no_cache_path_returns_none() {
        let state = MainCompleteState::new("", 0);
        assert!(_retrieve_cache(&state, "any").is_none());
    }

    #[test]
    fn existing_cache_file_loaded_line_by_line() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let cache_file = tmp.join("test.cache");
        std::fs::File::create(&cache_file)
            .unwrap()
            .write_all(b"foo\nbar\nbaz")
            .unwrap();

        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![tmp.to_string_lossy().to_string()],
            false,
        );
        let result = _retrieve_cache(&state, "test.cache");
        assert_eq!(
            result,
            Some(vec!["foo".into(), "bar".into(), "baz".into()])
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_cache_file_returns_none() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_miss_{}_{}",
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
        assert!(_retrieve_cache(&state, "nonexistent.cache").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn empty_cache_file_returns_empty_vec() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_e_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("e.cache"), b"").unwrap();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![tmp.to_string_lossy().to_string()],
            false,
        );
        let result = _retrieve_cache(&state, "e.cache");
        assert_eq!(result, Some(vec![]));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
