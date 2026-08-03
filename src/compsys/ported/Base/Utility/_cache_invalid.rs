//! Port of `_cache_invalid` from
//! `Completion/Base/Utility/_cache_invalid`.
//!
//! Full upstream body (21 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 6  _cache_ident="$1"
//! sh: 8  # If the cache is disabled, we never want to rebuild it, so pretend
//! sh: 9  # it's valid.
//! sh:10  zstyle -t ":completion:${curcontext}:" use-cache || return 1
//! sh:12  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:13  : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:14  _cache_path="$_cache_dir/$_cache_ident"
//! sh:18  zstyle -s ":completion:${curcontext}:" cache-policy _cache_policy
//! sh:19  [[ -n "$_cache_policy" ]] && "$_cache_policy" "$_cache_path" && return 0
//! sh:21  return 1
//! ```
//!
//! Returns 0 when the cache needs rebuilding (per the user-supplied
//! `cache-policy` hook); 1 otherwise (cache disabled or no policy).

use crate::ported::exec::{dispatch_function_call, execute_script_zsh_pipeline};
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::getsparam;
use crate::ported::utils::quotestring;
use crate::ported::zsh_h::QT_SINGLE;

/// sh:19 — the policy hook is a COMMAND WORD (`"$_cache_policy" "$_cache_path"`),
/// so zsh resolves it function → builtin → `$PATH` and, when nothing matches,
/// prints `command not found` and yields status 127.
///
/// `dispatch_function_call` only covers the first of those three steps, so a
/// policy naming a builtin or an external program silently "failed" and an
/// undefined policy produced no diagnostic at all (reference zsh prints
/// `_cache_invalid:19: command not found: <policy>`).
///
/// Functions keep the direct dispatch — it is the same resolution zsh performs
/// first, and it avoids re-entering the parser on the completion hot path.
/// Everything else is handed to the canonical string-execution entry
/// (`execute_script_zsh_pipeline`, the same one `execstring` and `zstyle -e`'s
/// `evalstyle` use), which performs the remaining builtin/`$PATH` steps and
/// emits the diagnostic.
///
/// Both words are single-quoted, reproducing sh:19's `"…"` expansions: the
/// policy name and cache path are passed through verbatim, never split or
/// globbed.
fn call_cache_policy(policy: &str, cache_path: &str) -> i32 {
    if let Some(rc) = dispatch_function_call(policy, &[cache_path.to_string()]) {
        return rc;
    }
    // The `command not found` diagnostic carries the line the command word sits
    // on in `_cache_invalid`'s source. A freshly parsed string always starts at
    // line 1, so pad the source with the 18 lines that precede sh:19 upstream —
    // that makes the parsed line number 19 and the whole diagnostic
    // byte-identical to zsh's. `scriptname` is already `_cache_invalid` — the
    // `FnScope` guard in `_cache_invalid` sets it — which supplies the other
    // half of the prefix.
    let src = format!(
        "{}{} {}",
        "\n".repeat(18),
        quotestring(policy, QT_SINGLE),
        quotestring(cache_path, QT_SINGLE)
    );
    execute_script_zsh_pipeline(&src).unwrap_or(1)
}

/// `_cache_invalid` — query the cache-policy hook for tag `$1`.
/// Returns 0 (cache stale) or 1 (cache fresh / disabled / no policy).
pub fn _cache_invalid(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_cache_invalid");
    // sh:6
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:10
    if testforstyle(&ctx, "use-cache") != 0 {
        return 1;
    }

    // sh:12-14
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
    let cache_path = format!("{}/{}", cache_dir, cache_ident);

    // sh:18-19
    let policy = lookupstyle(&ctx, "cache-policy")
        .first()
        .cloned()
        .unwrap_or_default();
    if !policy.is_empty() && call_cache_policy(&policy, &cache_path) == 0 {
        return 0;
    }

    // sh:21
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_use_cache_disabled() {
        // sh:10 — without use-cache style set, returns 1.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_cache_invalid(&["my-cache".to_string()]), 1);
    }
}
