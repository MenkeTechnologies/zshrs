//! Port of `_store_cache` from
//! `Completion/Base/Utility/_store_cache`.
//!
//! Full upstream body (64 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local _cache_ident _cache_ident_dir _cache_dir
//! sh: 6  _cache_ident="$1"
//! sh: 8  if zstyle -t … use-cache; then
//! sh:10    zstyle -s … cache-path _cache_dir
//! sh:11    : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:12    if [[ ! -d "$_cache_dir" ]]; then
//! sh:18      mkdir -m 0700 -p "$_cache_dir"
//! sh:22    fi
//! sh:24    _cache_ident_dir="$_cache_dir/$_cache_ident"
//! sh:25    _cache_ident_dir="$_cache_ident_dir:h"
//! sh:27    if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:34      mkdir -m 0700 -p "$_cache_ident_dir"
//! sh:38    fi
//! sh:45    shift
//! sh:46    for var; do
//! sh:47      case ${(Pt)var} in
//! sh:48      (*readonly*) ;;
//! sh:49      (*(association|array)*)
//! sh:50          print -r "$var=( "'${(Q)"${(z)$(<<\EO:'"$var"
//! sh:51          print -r "${(kv@Pqq)^^var}"
//! sh:52          print -r "EO:$var"
//! sh:53          print -r ')}"} )'
//! sh:54          ;;
//! sh:55      (*) print -r "$var=${(Pqq)^^var}";;
//! sh:56      esac
//! sh:57    done >! "$_cache_dir/$_cache_ident"
//! sh:58  else
//! sh:59    return 1
//! sh:60  fi
//! sh:62  return 0
//! ```
//!
//! Dumps the listed shell-side vars (after `$1` = cache ident) to
//! `$cache_dir/$ident` as zsh-sourceable assignments. Arrays and
//! associations get heredoc-style emission; readonly vars are
//! skipped. Returns 0 on write, 1 when cache disabled.

use crate::compsys::ported::_message::_message;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam};
use std::fs;
use std::path::Path;

/// `_store_cache` — write the named vars to disk under cache path.
pub fn _store_cache(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_store_cache");
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:8
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

    // sh:12-22  ensure cache_dir exists
    let dir_path = Path::new(&cache_dir);
    if !dir_path.is_dir() {
        if dir_path.exists() {
            let _ = _message(&["cache-dir style points to a non-directory!".to_string()]);
            return 1;
        }
        if fs::create_dir_all(dir_path).is_err() {
            let _ = _message(&[format!("couldn't create cache-dir {}", cache_dir)]);
            return 1;
        }
    }

    // sh:24-25  ident dirname
    let cache_path = format!("{}/{}", cache_dir, cache_ident);
    let ident_dir = Path::new(&cache_path).parent().map(|p| p.to_path_buf());

    // sh:27-38
    if let Some(p) = ident_dir.as_ref() {
        if !p.exists() {
            if fs::create_dir_all(p).is_err() {
                let _ = _message(&[format!("couldn't create cache-ident_dir {}", p.display())]);
                return 1;
            }
        }
    }

    // sh:45-57  serialize the remaining args (var names)
    let var_names: &[String] = if args.is_empty() { &[] } else { &args[1..] };
    let mut serialized = String::new();
    for var in var_names {
        if let Some(arr) = getaparam(var) {
            // sh:50-53 — array form
            serialized.push_str(&format!("{}=( ", var));
            for v in &arr {
                serialized.push('\'');
                serialized.push_str(&v.replace('\'', "'\\''"));
                serialized.push_str("' ");
            }
            serialized.push_str(")\n");
        } else if let Some(s) = getsparam(var) {
            // sh:55 — scalar form
            serialized.push_str(&format!("{}='{}'\n", var, s.replace('\'', "'\\''")));
        }
    }

    if fs::write(&cache_path, serialized).is_err() {
        return 1;
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_use_cache_disabled() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_store_cache(&["test-cache".to_string()]), 1);
    }
}
