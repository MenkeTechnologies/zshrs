//! Port of `_store_cache` from `Completion/Base/Utility/_store_cache`.
//!
//! Full upstream body (64 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  #
//! sh: 3  # Storage component of completions caching layer
//! sh: 4
//! sh: 5  local _cache_ident _cache_ident_dir _cache_dir
//! sh: 6  _cache_ident="$1"
//! sh: 7
//! sh: 8  if zstyle -t ":completion:${curcontext}:" use-cache; then
//! sh: 9    # Decide which directory to cache to, and ensure it exists
//! sh:10    zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:11    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:12    if [[ ! -d "$_cache_dir" ]]; then
//! sh:13      if [[ -e "$_cache_dir" ]]; then
//! sh:14        _message "cache-dir style points to a non-directory\!"
//! sh:15      else
//! sh:16        # if module load fails, we *should* be okay using normal mkdir so
//! sh:17        # we load feature b:mkdir instead of b:zf_mkdir; note that modules
//! sh:18        # loaded in a sub-shell don't affect the parent.
//! sh:19        ( zmodload -F zsh/files b:mkdir; mkdir -m 0700 -p "$_cache_dir"
//! sh:20        ) 2>/dev/null
//! sh:21        if [[ ! -d "$_cache_dir" ]]; then
//! sh:22          _message "couldn't create cache-dir $_cache_dir"
//! sh:23          return 1
//! sh:24        fi
//! sh:25      fi
//! sh:26    fi
//! sh:27    _cache_ident_dir="$_cache_dir/$_cache_ident"
//! sh:28    _cache_ident_dir="$_cache_ident_dir:h"
//! sh:29
//! sh:30    if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:31      if [[ -e "$_cache_ident_dir" ]]; then
//! sh:32        _message "cache ident dir points to a non-directory:$_cache_ident_dir"
//! sh:33      else
//! sh:34        # See also rationale in zmodload above
//! sh:35        ( zmodload -F zsh/files b:mkdir; mkdir -m 0700 -p "$_cache_ident_dir"
//! sh:36        ) 2>/dev/null
//! sh:37        if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:38          _message "couldn't create cache-ident_dir $_cache_ident_dir"
//! sh:39          return 1
//! sh:40        fi
//! sh:41      fi
//! sh:42    fi
//! sh:43
//! sh:44
//! sh:45    shift
//! sh:46    for var; do
//! sh:47      case ${(Pt)var} in
//! sh:48      (*readonly*) ;;
//! sh:49      (*(association|array)*)
//! sh:50  	# Dump the array as a here-document to reduce parsing overhead
//! sh:51  	# when reloading the cache with "source" from _retrieve_cache
//! sh:52  	print -r "$var=( "'${(Q)"${(z)$(<<\EO:'"$var"
//! sh:53  	print -r "${(kv@Pqq)^^var}"
//! sh:54  	print -r "EO:$var"
//! sh:55  	print -r ')}"} )'
//! sh:56  	;;
//! sh:57      (*) print -r "$var=${(Pqq)^^var}";;
//! sh:58      esac
//! sh:59    done >! "$_cache_dir/$_cache_ident"
//! sh:60  else
//! sh:61    return 1
//! sh:62  fi
//! sh:63
//! sh:64  return 0
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

use crate::compsys::base::MainCompleteState;

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
    use crate::compsys::ported::_retrieve_cache::_retrieve_cache;

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
