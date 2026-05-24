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
}
