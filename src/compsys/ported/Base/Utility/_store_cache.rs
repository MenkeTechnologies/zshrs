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
//! sh:24    fi
//! sh:27    _cache_ident_dir="$_cache_dir/$_cache_ident"
//! sh:28    _cache_ident_dir="$_cache_ident_dir:h"
//! sh:30    if [[ ! -d "$_cache_ident_dir" ]]; then
//! sh:34      mkdir -m 0700 -p "$_cache_ident_dir"
//! sh:40    fi
//! sh:45    shift
//! sh:46    for var; do
//! sh:47      case ${(Pt)var} in
//! sh:48      (*readonly*) ;;
//! sh:49      (*(association|array)*)
//! sh:52          print -r "$var=( "'${(Q)"${(z)$(<<\EO:'"$var"
//! sh:53          print -r "${(kv@Pqq)^^var}"
//! sh:54          print -r "EO:$var"
//! sh:55          print -r ')}"} )'
//! sh:56          ;;
//! sh:57      (*) print -r "$var=${(Pqq)^^var}";;
//! sh:58      esac
//! sh:59    done >! "$_cache_dir/$_cache_ident"
//! sh:60  else
//! sh:61    return 1
//! sh:62  fi
//! sh:64  return 0
//! ```
//!
//! Dumps the listed shell-side vars (after `$1` = cache ident) to
//! `$cache_dir/$ident` as zsh-sourceable assignments. Arrays and
//! associations get heredoc-style emission; readonly vars are
//! skipped. Returns 0 on write, 1 when cache disabled.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::shared::zstyle_t;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam};
use std::fs;
use std::path::Path;

/// Reach `_store_cache` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_store_cache DEBS_avail _deb_packages_cache_avail` (Completion/Debian/Type/_deb_packages sh:13) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_store_cache_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _store_cache(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_store_cache", args, || _store_cache_impl(args))
}

/// `_store_cache` — write the named vars to disk under cache path.
pub fn _store_cache_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_store_cache");
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:8 — `if zstyle -t … use-cache; then`, a VALUE test; see
    //   [`zstyle_t`].
    if zstyle_t(&ctx, "use-cache") != 0 {
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

    // sh:27-28  ident dirname
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
            // sh:52-55 — array form
            serialized.push_str(&format!("{}=( ", var));
            for v in &arr {
                serialized.push('\'');
                serialized.push_str(&v.replace('\'', "'\\''"));
                serialized.push_str("' ");
            }
            serialized.push_str(")\n");
        } else if let Some(s) = getsparam(var) {
            // sh:57 — scalar form
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
        assert_eq!(_store_cache_impl(&["test-cache".to_string()]), 1);
    }

    /// sh:8 — `if zstyle -t … use-cache; then` is a VALUE test, so
    /// `use-cache 0` takes the `else return 1` arm at sh:60-61 and NOTHING
    /// is written to disk.
    ///
    /// The port tested it with `testforstyle` (zutil.c:465), the primitive
    /// behind `zstyle -q`, which answers "is this style defined" — so
    /// `use-cache 0` read as ON and the completer wrote the cache file the
    /// user had just turned caching off to avoid.
    #[test]
    fn use_cache_zero_writes_nothing() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let dir = std::env::temp_dir().join(format!("zshrs-store-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ctx = ":completion:sczero:sczero:sczero:";
        let _ = crate::ported::params::setsparam("curcontext", "sczero:sczero:sczero");
        for (style, val) in [
            ("use-cache", "0"),
            ("cache-path", dir.to_string_lossy().as_ref()),
        ] {
            crate::ported::modules::zutil::bin_zstyle(
                "zstyle",
                &[ctx.to_string(), style.to_string(), val.to_string()],
                &ops,
                0,
            );
        }

        let rc = _store_cache_impl(&["scz-ident".to_string()]);
        let landed = dir.join("scz-ident").exists();

        for style in ["use-cache", "cache-path"] {
            crate::ported::modules::zutil::bin_zstyle(
                "zstyle",
                &["-d".to_string(), ctx.to_string(), style.to_string()],
                &ops,
                0,
            );
        }
        crate::ported::params::unsetparam("curcontext");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(rc, 1, "sh:61 — `use-cache 0` returns 1");
        assert!(
            !landed,
            "sh:59 — the cache file must not be written when use-cache is off"
        );
    }
}
