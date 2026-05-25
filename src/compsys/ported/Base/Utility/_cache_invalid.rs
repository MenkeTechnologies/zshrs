//! Port of `_cache_invalid` from `Completion/Base/Utility/_cache_invalid`.
//!
//! Full upstream body (21 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  #
//! sh: 3  # Function to decide whether a completions cache needs rebuilding
//! sh: 4
//! sh: 5  local _cache_ident _cache_dir _cache_path _cache_policy
//! sh: 6  _cache_ident="$1"
//! sh: 7
//! sh: 8  # If the cache is disabled, we never want to rebuild it, so pretend
//! sh: 9  # it's valid.
//! sh:10  zstyle -t ":completion:${curcontext}:" use-cache || return 1
//! sh:11
//! sh:12  zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
//! sh:13  : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
//! sh:14  _cache_path="$_cache_dir/$_cache_ident"
//! sh:15
//! sh:16  # See whether the caching policy says that the cache needs rebuilding
//! sh:17  # (the policy will return 0 if it does).
//! sh:18  zstyle -s ":completion:${curcontext}:" cache-policy _cache_policy
//! sh:19  [[ -n "$_cache_policy" ]] && "$_cache_policy" "$_cache_path" && return 0
//! sh:20
//! sh:21  return 1
//! ```
//!
//! Upstream gates on the `use-cache` zstyle (no-cache → return 1
//! meaning "NOT invalid, caller skips reload") and otherwise
//! consults the `cache-policy` user-defined function. If no
//! cache-policy → return 1 (treat as NOT invalid).
//!
//! Strict Rust port: matches upstream return semantics exactly.
//! `true` = "cache IS invalid, reload" (shell return 0). `false` =
//! "cache is fine, skip reload" (shell return 1). The cache-policy
//! callout dispatches through `_call_function` so registered Rust
//! callbacks (or shell-fn shims) decide invalidity.



use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_call_function::_call_function;

/// _cache_invalid - Check if completion cache is invalid.
///
/// Returns `true` when the cache must be regenerated; `false` when
/// the cache is fine OR when no cache is in use (matches upstream
/// `_cache_invalid` `return 1` semantics).
pub fn _cache_invalid(state: &mut MainCompleteState, cache_name: &str) -> bool {
    let context = format!(":completion:{}:", state.ctx.context);

    // shell:9 — `zstyle -t use-cache || return 1`. -t returns 0
    // ONLY when the style is set to a recognized true value
    // (true/yes/on/1). Anything else → return 1 (NOT invalid).
    let use_cache_true = state
        .styles
        .lookup_values(&context, "use-cache")
        .and_then(|v| v.first().cloned())
        .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
        .unwrap_or(false);
    if !use_cache_true {
        return false;
    }

    // shell:11-12 — resolve cache-path (defaults to $ZDOTDIR/.zcompcache).
    let cache_dir = state
        .styles
        .lookup_values(&context, "cache-path")
        .and_then(|v| v.first().cloned())
        .unwrap_or_else(|| {
            let base = std::env::var("ZDOTDIR")
                .ok()
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            format!("{}/.zcompcache", base)
        });
    let cache_path = format!("{}/{}", cache_dir, cache_name);

    // shell:16-17 — call cache-policy fn if set. Its exit status IS
    // our return value (true result → invalid).
    if let Some(policy) = state
        .styles
        .lookup_values(&context, "cache-policy")
        .and_then(|v| v.first().cloned())
    {
        // Expose the cache_path to the registered policy fn by
        // pushing it onto state.comp.params.words as $1 (matches the
        // shell calling convention `"$_cache_policy" "$_cache_path"`).
        state.comp.params.words.push(cache_path);
        let r = _call_function(state, &policy);
        state.comp.params.words.pop();
        return r;
    }

    // shell:19 — `return 1`: no policy → cache fine.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::compsys::ported::_call_function::{register, unregister};

    #[test]
    fn use_cache_not_set_returns_not_invalid() {
        // shell:9 `zstyle -t ... use-cache || return 1` →
        // unset style means NOT invalid (caller skips reload).
        let mut state = MainCompleteState::new("", 0);
        assert!(!_cache_invalid(&mut state, "x"));
    }

    #[test]
    fn use_cache_no_treated_as_not_invalid() {
        // `no` is NOT one of true/yes/on/1, so -t fails, so return 1.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["no".into()],
            false,
        );
        assert!(!_cache_invalid(&mut state, "any-name"));
    }

    #[test]
    fn use_cache_true_no_policy_returns_not_invalid() {
        // shell:19 — fall through to `return 1`.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["true".into()],
            false,
        );
        assert!(!_cache_invalid(&mut state, "any-name"));
    }

    #[test]
    fn use_cache_true_with_policy_uses_policy_result() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["yes".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-policy",
            vec!["my-stale-checker".into()],
            false,
        );
        // Register a policy that says "invalid" (true).
        register("my-stale-checker", Box::new(|_| true));
        let result = _cache_invalid(&mut state, "x.cache");
        unregister("my-stale-checker");
        assert!(result, "policy returning true → cache invalid");
    }

    #[test]
    fn use_cache_true_with_policy_returning_false_keeps_cache() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["on".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-policy",
            vec!["my-keeper".into()],
            false,
        );
        register("my-keeper", Box::new(|_| false));
        let result = _cache_invalid(&mut state, "x.cache");
        unregister("my-keeper");
        assert!(!result, "policy returning false → cache fine");
    }

    #[test]
    fn policy_sees_cache_path_as_argv1() {
        // The cache_path is pushed onto words so the policy fn can
        // see it as $1 (shell calling convention `"$_cache_policy"
        // "$_cache_path"`). Pin that contract.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["1".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-path",
            vec!["/tmp/zshrs-test-cache".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-policy",
            vec!["argv-spy".into()],
            false,
        );
        register(
            "argv-spy",
            Box::new(|s: &mut MainCompleteState| -> bool {
                let last = s.comp.params.words.last().cloned().unwrap_or_default();
                // Returning true marks cache invalid; we encode the
                // observation: cache path ends in the expected name.
                last.ends_with("/spy.cache")
            }),
        );
        let result = _cache_invalid(&mut state, "spy.cache");
        unregister("argv-spy");
        assert!(
            result,
            "policy should have seen cache path with the requested name"
        );
    }

    #[test]
    fn words_restored_after_policy_call() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["true".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-policy",
            vec!["restore-spy".into()],
            false,
        );
        // Seed pre-existing words.
        let original = vec!["pre".to_string(), "existing".to_string()];
        state.comp.params.words = original.clone();
        register("restore-spy", Box::new(|_| true));
        let _ = _cache_invalid(&mut state, "x.cache");
        unregister("restore-spy");
        assert_eq!(
            state.comp.params.words, original,
            "_cache_invalid must restore words after policy returns"
        );
    }

    #[test]
    fn default_cache_dir_uses_zdotdir_or_home() {
        // If neither cache-path style is set nor ZDOTDIR/HOME, the
        // dir fallback should still work without panic.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["true".into()],
            false,
        );
        // No cache-policy → return false (cache fine).
        assert!(!_cache_invalid(&mut state, "x.cache"));
    }

    #[test]
    fn unknown_policy_name_silently_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "use-cache",
            vec!["true".into()],
            false,
        );
        state.styles.set(
            ":completion::complete::test::",
            "cache-policy",
            vec!["definitely-not-registered-fn".into()],
            false,
        );
        // _call_function returns false for unknown names; we
        // propagate as "cache fine" (false).
        assert!(!_cache_invalid(&mut state, "x.cache"));
    }
}
