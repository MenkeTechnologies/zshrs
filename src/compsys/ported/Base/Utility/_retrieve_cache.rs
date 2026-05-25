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



use crate::compsys::base::MainCompleteState;

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

    #[test]
    fn single_line_cache_returns_one_element() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_one_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("o.cache"), b"only-line").unwrap();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![tmp.to_string_lossy().to_string()],
            false,
        );
        assert_eq!(
            _retrieve_cache(&state, "o.cache"),
            Some(vec!["only-line".to_string()])
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn trailing_newline_preserved_as_no_empty_tail() {
        // `lines()` semantics: trailing `\n` does NOT produce an
        // empty trailing entry. Pin that contract.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_nl_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("nl.cache"), b"a\nb\nc\n").unwrap();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec![tmp.to_string_lossy().to_string()],
            false,
        );
        let r = _retrieve_cache(&state, "nl.cache").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r, vec!["a", "b", "c"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
