//! Port of `_retrieve_cache` from
//! `Completion/Base/Utility/_retrieve_cache`.
//!
//! Full upstream body (31 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 6  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 7  _cache_ident="$1"
//! sh: 9  if zstyle -t ":completion:${curcontext}:" use-cache; then
//! sh:10    zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:11    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:12    if [[ ! -d "$_cache_dir" ]]; then
//! sh:13      [[ -e "$_cache_dir" ]] &&
//! sh:14        _message "cache-dir ($_cache_dir) isn't a directory\!"
//! sh:15      return 1
//! sh:16    fi
//! sh:18    _cache_path="$_cache_dir/$_cache_ident"
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
//! `. "$_cache_path"` — sources the cache file in the current
//! shell. Our port dispatches via the `source` builtin (real
//! `bin_dot`) when wired; without an executor, the load step is a
//! no-op and we return success when the file exists + is fresh.

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::getsparam;
use std::path::Path;

/// `_retrieve_cache` — load `$cache_ident` from disk if the cache
/// is enabled, exists, and isn't invalid per `_cache_invalid`.
/// Returns 0 on successful load, 1 otherwise.
pub fn _retrieve_cache(args: &[String]) -> i32 {
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:9
    if testforstyle(&ctx, "use-cache") != 0 {
        return 1;
    }

    // sh:10-11
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

    // sh:12-16
    let dir_meta = std::fs::metadata(&cache_dir);
    match dir_meta {
        Ok(m) if m.is_dir() => {}
        Ok(_) => {
            let _ = _message(&[format!("cache-dir ({}) isn't a directory!", cache_dir)]);
            return 1;
        }
        Err(_) => return 1,
    }

    // sh:18
    let cache_path = format!("{}/{}", cache_dir, cache_ident);

    // sh:20
    if Path::new(&cache_path).exists() {
        // sh:21
        if _cache_invalid(&[cache_ident]) == 0 {
            return 1;
        }
        // sh:23  source the cache file
        let _ = dispatch_function_call("source", &[cache_path]);
        // sh:24
        0
    } else {
        // sh:26
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_use_cache_disabled() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_retrieve_cache(&["my-cache".to_string()]), 1);
    }
}
