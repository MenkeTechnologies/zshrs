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

use crate::compsys::ported::shared::zstyle_t;
use crate::ported::exec::{dispatch_function_call, execute_script_zsh_pipeline};
use crate::ported::modules::zutil::lookupstyle;
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

/// Reach `_cache_invalid` the way every upstream caller writes it — as a BARE
/// COMMAND WORD (`_cache_invalid "$_cache_ident"`, `_retrieve_cache` sh:21;
/// `_cache_invalid $cache_id`, `_python_modules` sh:25) — so the normal
/// function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (a user's own `_cache_invalid` earlier on `$fpath` stays live
/// instead of being inert) and the `doshfunc` frame. Without that frame
/// `$funcstack` inside the `cache-policy` hook read `zpwrDailyCachingPolicy
/// _retrieve_cache __fasd_files_comp …` where zsh reads `… _cache_invalid
/// _retrieve_cache …`.
///
/// [`_cache_invalid_impl`] is the raw body, reserved for the two callers that
/// must not re-enter dispatch: this wrapper's own fallback (it runs only when
/// neither a shell function nor a registered port claims the name — i.e. unit
/// tests with no executor installed), and the `compsys::router` arm, which
/// has to target the body or dispatch would re-enter this wrapper forever.
pub fn _cache_invalid(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_cache_invalid", args, || {
        _cache_invalid_impl(args)
    })
}

/// `_cache_invalid` — query the cache-policy hook for tag `$1`.
/// Returns 0 (cache stale) or 1 (cache fresh / disabled / no policy).
pub fn _cache_invalid_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_cache_invalid");
    // sh:6
    let cache_ident = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:10 — `zstyle -t … use-cache || return 1`, a VALUE test; see
    //   [`zstyle_t`]. Both the "set but not boolean-true" exit (1) and the
    //   "no pattern matched" exit (2) take the `|| return 1` arm.
    if zstyle_t(&ctx, "use-cache") != 0 {
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
        assert_eq!(_cache_invalid_impl(&["my-cache".to_string()]), 1);
    }

    /// sh:10 — `zstyle -t … use-cache || return 1` is a VALUE test. The
    /// comment above it says why the answer matters: "If the cache is
    /// disabled, we never want to rebuild it, so pretend it's valid."
    /// `use-cache 0` must therefore return 1 (valid) WITHOUT consulting the
    /// `cache-policy` hook at sh:19.
    ///
    /// The port tested it with `testforstyle` (zutil.c:465), the primitive
    /// behind `zstyle -q`, which answers "is this style defined" — so with
    /// caching switched off the policy still ran, and a policy that says
    /// "stale" turned this into a 0 and forced the rebuild.
    #[test]
    fn use_cache_zero_skips_the_policy_hook() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let ctx = ":completion:cizero:cizero:cizero:";
        let _ = crate::ported::params::setsparam("curcontext", "cizero:cizero:cizero");
        // `true` always succeeds, i.e. "the cache IS stale" at sh:19.
        for (style, val) in [("use-cache", "0"), ("cache-policy", "true")] {
            crate::ported::modules::zutil::bin_zstyle(
                "zstyle",
                &[ctx.to_string(), style.to_string(), val.to_string()],
                &ops,
                0,
            );
        }

        let rc = _cache_invalid_impl(&["ciz-ident".to_string()]);

        for style in ["use-cache", "cache-policy"] {
            crate::ported::modules::zutil::bin_zstyle(
                "zstyle",
                &["-d".to_string(), ctx.to_string(), style.to_string()],
                &ops,
                0,
            );
        }
        crate::ported::params::unsetparam("curcontext");

        assert_eq!(
            rc, 1,
            "sh:10 — `use-cache 0` returns 1 before the always-stale policy runs"
        );
    }
}
