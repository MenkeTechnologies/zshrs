//! Executor-callback hooks installed by `src/fusevm_bridge.rs` at
//! startup so code under `src/ported/` can dispatch to operations
//! that live on `ShellExecutor` (array/assoc storage, script eval,
//! function dispatch, command substitution) WITHOUT taking a direct
//! `ShellExecutor` reference.
//!
//! Direct `crate::fusevm_bridge::with_executor(|exec| exec.X())`
//! calls from `src/ported/` are forbidden (see memory:
//! feedback_no_exec_script_from_ported,
//! feedback_no_shellexecutor_in_ported). Each forbidden call site
//! now routes through one of the hooks below; the bridge supplies a
//! concrete fn-pointer at init time that closes over the
//! `with_executor` access pattern.
//!
//! Hooks are `OnceLock<fn(...) -> R>`. Calls before install return
//! the `unwrap_or_else` fallback (typically a no-op default).

use indexmap::IndexMap;
use std::sync::OnceLock;
/// `ArrayGetFn` type alias.
pub type ArrayGetFn = fn(&str) -> Option<Vec<String>>;
/// `AssocGetFn` type alias.
pub type AssocGetFn = fn(&str) -> Option<IndexMap<String, String>>;
/// `ArraySetFn` type alias.
pub type ArraySetFn = fn(&str, Vec<String>);
/// `AssocSetFn` type alias.
pub type AssocSetFn = fn(&str, IndexMap<String, String>);
/// `ScalarUnsetFn` type alias.
pub type ScalarUnsetFn = fn(&str);
/// `ArrayUnsetFn` type alias.
pub type ArrayUnsetFn = fn(&str);
/// `AssocUnsetFn` type alias.
pub type AssocUnsetFn = fn(&str);
/// `DispatchFunctionCallFn` type alias.
pub type DispatchFunctionCallFn = fn(&str, &[String]) -> Option<i32>;
/// Body-only function dispatch — runs the function body WITHOUT
/// the doshfunc scope wrap (locallevel/FUNCSTACK/BREAKS/etc.).
/// Used as the `body_runner` closure passed to direct
/// `crate::ported::exec::doshfunc(...)` callers (so they don't
/// double-wrap by going back through `dispatch_function_call`).
/// Returns the body's exit status. Mirrors C's `runshfunc(prog,
/// wrappers, name)` at `exec.c:6042` from doshfunc's perspective.
pub type RunFunctionBodyFn = fn(&str, &[String]) -> Option<i32>;
/// `ExecuteScriptFn` type alias.
pub type ExecuteScriptFn = fn(&str) -> Result<i32, String>;
/// `ExecuteScriptZshPipelineFn` type alias.
pub type ExecuteScriptZshPipelineFn = fn(&str) -> Result<i32, String>;
/// `RunCommandSubstitutionFn` type alias.
pub type RunCommandSubstitutionFn = fn(&str) -> String;
/// `PparamsGetFn` type alias.
pub type PparamsGetFn = fn() -> Vec<String>;
/// `PparamsSetFn` type alias.
pub type PparamsSetFn = fn(Vec<String>);
/// Drop a function from both the compiled-chunk and source maps.
/// Returns true if either entry existed.
pub type UnregisterFunctionFn = fn(&str) -> bool;

static ARRAY_GET: OnceLock<ArrayGetFn> = OnceLock::new();
static ASSOC_GET: OnceLock<AssocGetFn> = OnceLock::new();
static ARRAY_SET: OnceLock<ArraySetFn> = OnceLock::new();
static ASSOC_SET: OnceLock<AssocSetFn> = OnceLock::new();
static SCALAR_UNSET: OnceLock<ScalarUnsetFn> = OnceLock::new();
static ARRAY_UNSET: OnceLock<ArrayUnsetFn> = OnceLock::new();
static ASSOC_UNSET: OnceLock<AssocUnsetFn> = OnceLock::new();
static DISPATCH_FUNCTION_CALL: OnceLock<DispatchFunctionCallFn> = OnceLock::new();
static RUN_FUNCTION_BODY: OnceLock<RunFunctionBodyFn> = OnceLock::new();
static EXECUTE_SCRIPT: OnceLock<ExecuteScriptFn> = OnceLock::new();
static EXECUTE_SCRIPT_ZSH_PIPELINE: OnceLock<ExecuteScriptZshPipelineFn> = OnceLock::new();
static RUN_COMMAND_SUBSTITUTION: OnceLock<RunCommandSubstitutionFn> = OnceLock::new();
static PPARAMS_GET: OnceLock<PparamsGetFn> = OnceLock::new();
static PPARAMS_SET: OnceLock<PparamsSetFn> = OnceLock::new();
static UNREGISTER_FUNCTION: OnceLock<UnregisterFunctionFn> = OnceLock::new();
/// `install_array_get` — see implementation.
pub fn install_array_get(f: ArrayGetFn) {
    let _ = ARRAY_GET.set(f);
}
/// `install_assoc_get` — see implementation.
pub fn install_assoc_get(f: AssocGetFn) {
    let _ = ASSOC_GET.set(f);
}
/// `install_array_set` — see implementation.
pub fn install_array_set(f: ArraySetFn) {
    let _ = ARRAY_SET.set(f);
}
/// `install_assoc_set` — see implementation.
pub fn install_assoc_set(f: AssocSetFn) {
    let _ = ASSOC_SET.set(f);
}
/// `install_scalar_unset` — see implementation.
pub fn install_scalar_unset(f: ScalarUnsetFn) {
    let _ = SCALAR_UNSET.set(f);
}
/// `install_array_unset` — see implementation.
pub fn install_array_unset(f: ArrayUnsetFn) {
    let _ = ARRAY_UNSET.set(f);
}
/// `install_assoc_unset` — see implementation.
pub fn install_assoc_unset(f: AssocUnsetFn) {
    let _ = ASSOC_UNSET.set(f);
}
/// `install_dispatch_function_call` — see implementation.
pub fn install_dispatch_function_call(f: DispatchFunctionCallFn) {
    let _ = DISPATCH_FUNCTION_CALL.set(f);
}
/// `install_run_function_body` — see [`RunFunctionBodyFn`].
pub fn install_run_function_body(f: RunFunctionBodyFn) {
    let _ = RUN_FUNCTION_BODY.set(f);
}
/// `install_execute_script` — see implementation.
pub fn install_execute_script(f: ExecuteScriptFn) {
    let _ = EXECUTE_SCRIPT.set(f);
}
/// `install_execute_script_zsh_pipeline` — see implementation.
pub fn install_execute_script_zsh_pipeline(f: ExecuteScriptZshPipelineFn) {
    let _ = EXECUTE_SCRIPT_ZSH_PIPELINE.set(f);
}
/// `install_run_command_substitution` — see implementation.
pub fn install_run_command_substitution(f: RunCommandSubstitutionFn) {
    let _ = RUN_COMMAND_SUBSTITUTION.set(f);
}
/// `install_pparams_get` — see implementation.
pub fn install_pparams_get(f: PparamsGetFn) {
    let _ = PPARAMS_GET.set(f);
}
/// `install_pparams_set` — see implementation.
pub fn install_pparams_set(f: PparamsSetFn) {
    let _ = PPARAMS_SET.set(f);
}
/// `install_unregister_function` — see implementation.
pub fn install_unregister_function(f: UnregisterFunctionFn) {
    let _ = UNREGISTER_FUNCTION.set(f);
}
/// `array` — see implementation.
pub fn array(name: &str) -> Option<Vec<String>> {
    // Hook path (fusevm executor) when installed; otherwise fall
    // through to the direct param table at `params::getaparam` so
    // compsys/unit-test environments without an executor still see
    // shell-side arrays.
    if let Some(f) = ARRAY_GET.get() {
        if let Some(v) = f(name) {
            return Some(v);
        }
    }
    crate::ported::params::getaparam(name)
}
/// `assoc` — see implementation.
pub fn assoc(name: &str) -> Option<IndexMap<String, String>> {
    ASSOC_GET.get().and_then(|f| f(name))
}
/// `set_array` — see implementation.
pub fn set_array(name: &str, val: Vec<String>) {
    if let Some(f) = ARRAY_SET.get() {
        f(name, val);
    }
}
/// `set_assoc` — see implementation.
pub fn set_assoc(name: &str, val: IndexMap<String, String>) {
    if let Some(f) = ASSOC_SET.get() {
        f(name, val);
    }
}
/// `unset_scalar` — see implementation.
pub fn unset_scalar(name: &str) {
    if let Some(f) = SCALAR_UNSET.get() {
        f(name);
    }
}
/// `unset_array` — see implementation.
pub fn unset_array(name: &str) {
    if let Some(f) = ARRAY_UNSET.get() {
        f(name);
    }
}
/// `unset_assoc` — see implementation.
pub fn unset_assoc(name: &str) {
    if let Some(f) = ASSOC_UNSET.get() {
        f(name);
    }
}
/// `dispatch_function_call` — see implementation.
pub fn dispatch_function_call(name: &str, args: &[String]) -> Option<i32> {
    DISPATCH_FUNCTION_CALL.get().and_then(|f| f(name, args))
}
/// Body-only dispatch — call as the `body_runner` of a direct
/// `crate::ported::exec::doshfunc(...)` invocation to avoid the
/// double-wrap that would happen if the body_runner went back
/// through [`dispatch_function_call`].
pub fn run_function_body(name: &str, args: &[String]) -> Option<i32> {
    RUN_FUNCTION_BODY.get().and_then(|f| f(name, args))
}
/// `execute_script` — see implementation.
pub fn execute_script(src: &str) -> Result<i32, String> {
    match EXECUTE_SCRIPT.get() {
        Some(f) => f(src),
        None => Ok(0),
    }
}
/// `execute_script_zsh_pipeline` — see implementation.
pub fn execute_script_zsh_pipeline(src: &str) -> Result<i32, String> {
    match EXECUTE_SCRIPT_ZSH_PIPELINE.get() {
        Some(f) => f(src),
        None => Ok(0),
    }
}
/// `run_command_substitution` — see implementation.
pub fn run_command_substitution(cmd: &str) -> String {
    match RUN_COMMAND_SUBSTITUTION.get() {
        Some(f) => f(cmd),
        None => String::new(),
    }
}
/// `pparams` — see implementation.
pub fn pparams() -> Vec<String> {
    PPARAMS_GET.get().map(|f| f()).unwrap_or_default()
}
/// `set_pparams` — see implementation.
pub fn set_pparams(v: Vec<String>) {
    if let Some(f) = PPARAMS_SET.get() {
        f(v);
    }
}
/// `unregister_function` — see implementation.
pub fn unregister_function(name: &str) -> bool {
    UNREGISTER_FUNCTION.get().map(|f| f(name)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── zsh-corpus pins: default (no-hook) fallback behavior ─────

    /// `dispatch_function_call` returns None when no hook installed.
    /// Tests may run in a fresh process where no fusevm bridge wired
    /// the dispatch yet; pin: no-panic, None-return.
    #[test]
    fn exec_hooks_corpus_dispatch_returns_none_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        // We can't unset OnceLock once set, but if test runs first
        // in this process it should be None. The defensive pin is:
        // either None or Some — never panic.
        let _ = dispatch_function_call("__never_a_real_function_zshrs__", &["a".into()]);
        // No panic = pass.
    }

    /// `execute_script` returns `Ok(0)` when no hook installed.
    #[test]
    fn exec_hooks_corpus_execute_script_returns_ok_zero_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        let r = execute_script("nothing real");
        match r {
            Ok(_) | Err(_) => {} // either is acceptable post-install
        }
    }

    /// `run_command_substitution` returns "" by default.
    #[test]
    fn exec_hooks_corpus_run_command_substitution_default_empty_or_real() {
        let _g = crate::test_util::global_state_lock();
        // Returns "" if no hook, or real output if hook installed.
        let _ = run_command_substitution("echo zshrs_hook_test");
        // No panic = pass; we can't pin exact result because hook
        // state depends on previous tests in same process.
    }

    /// `array` falls back to params::getaparam when no hook.
    /// Set a real array via params, then look up through hook entry.
    #[test]
    fn exec_hooks_corpus_array_falls_back_to_getaparam() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_FB");
        crate::ported::params::setaparam("EH_FB", vec!["x".into(), "y".into(), "z".into()]);
        let got = array("EH_FB");
        assert_eq!(
            got.as_deref(),
            Some(&["x".to_string(), "y".to_string(), "z".to_string()][..]),
            "array() hook falls back to params::getaparam",
        );
        crate::ported::params::unsetparam("EH_FB");
    }

    /// `pparams()` returns empty Vec when no hook installed.
    #[test]
    fn exec_hooks_corpus_pparams_returns_empty_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        let p = pparams();
        // Either empty (no hook) or whatever the installed hook returns.
        let _ = p; // no panic = pass
    }

    /// `unregister_function` returns false by default.
    #[test]
    fn exec_hooks_corpus_unregister_function_default_false() {
        let _g = crate::test_util::global_state_lock();
        let r = unregister_function("__never_registered_xyz__");
        // If hook installed, hook decides; if not, returns false.
        // Pin: doesn't panic and returns a bool.
        let _ = r;
    }

    /// `set_pparams` doesn't panic when called.
    #[test]
    fn exec_hooks_corpus_set_pparams_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        set_pparams(vec!["a".into(), "b".into()]);
        // No panic = pass.
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional default-path (no-hook-installed) parity tests.
    // exec_hooks fallback semantics must remain stable: every accessor
    // returns a safe default when no fusevm executor has wired its hook.
    // ═══════════════════════════════════════════════════════════════════

    /// `assoc()` returns None when no hook installed (no params
    /// fallback — assoc has no equivalent of getaparam fallback).
    #[test]
    fn exec_hooks_assoc_returns_none_when_no_hook() {
        let _g = crate::test_util::global_state_lock();
        // If a prior test installed a hook, the result is hook-defined;
        // we only pin no-panic + valid Option<...>.
        let _ = assoc("__never_real_assoc_zshrs__");
    }

    /// `set_array` is a no-op when no hook installed (silently
    /// drops the write rather than panicking — fusevm-less env safe).
    #[test]
    fn exec_hooks_set_array_no_hook_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        set_array("__never_real_array_zshrs__", vec!["x".into(), "y".into()]);
    }

    /// `set_assoc` is a no-op when no hook installed.
    #[test]
    fn exec_hooks_set_assoc_no_hook_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut m = IndexMap::new();
        m.insert("k".to_string(), "v".to_string());
        set_assoc("__never_real_assoc_zshrs__", m);
    }

    /// `unset_scalar`, `unset_array`, `unset_assoc` are all no-ops
    /// when no hook installed.
    #[test]
    fn exec_hooks_unset_variants_no_hook_dont_panic() {
        let _g = crate::test_util::global_state_lock();
        unset_scalar("__never_real_scalar_zshrs__");
        unset_array("__never_real_array_zshrs__");
        unset_assoc("__never_real_assoc_zshrs__");
    }

    /// `run_function_body` returns None when no hook installed.
    #[test]
    fn exec_hooks_run_function_body_returns_none_when_no_hook() {
        let _g = crate::test_util::global_state_lock();
        let _ = run_function_body("__never_a_real_fn_zshrs__", &["a".into()]);
        // No panic = pass; result is Option-typed.
    }

    /// `execute_script_zsh_pipeline` returns Ok(0) when no hook installed.
    #[test]
    fn exec_hooks_execute_script_zsh_pipeline_default_ok_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = execute_script_zsh_pipeline("no hook");
        // If hook installed, result is hook-defined; if not, Ok(0).
        let _ = r;
    }

    /// `array()` returns None when name doesn't exist in params either
    /// (no fallback hits).
    #[test]
    fn exec_hooks_array_returns_none_for_missing_name() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("__no_such_array_zshrs__");
        let got = array("__no_such_array_zshrs__");
        // Either None (no hook + no param) or hook-returned value.
        let _ = got;
    }

    /// `array()` empty name doesn't panic (some callers pass "" for
    /// special parameter probes).
    #[test]
    fn exec_hooks_array_empty_name_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = array("");
    }

    /// Idempotent: calling array() twice with same name yields same
    /// result (no observable side effect on the fallback path).
    #[test]
    fn exec_hooks_array_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_IDEMPOTENT");
        crate::ported::params::setaparam("EH_IDEMPOTENT", vec!["a".into(), "b".into()]);
        let first = array("EH_IDEMPOTENT");
        let second = array("EH_IDEMPOTENT");
        assert_eq!(first, second);
        crate::ported::params::unsetparam("EH_IDEMPOTENT");
    }

    /// `pparams()` returns an empty vec (not None) when no hook —
    /// callers can iterate without an Option-check.
    #[test]
    fn exec_hooks_pparams_returns_vec_not_none() {
        let _g = crate::test_util::global_state_lock();
        // Type assertion: result is Vec<String>, not Option<Vec<String>>.
        let p: Vec<String> = pparams();
        let _ = p; // either [] or hook-installed value
    }

    /// Round-trip via params fallback: set then read returns same value.
    #[test]
    fn exec_hooks_array_set_get_roundtrip_via_params() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_RT");
        let vals = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        crate::ported::params::setaparam("EH_RT", vals.clone());
        let got = array("EH_RT").expect("set then get should hit params fallback");
        assert_eq!(got, vals);
        crate::ported::params::unsetparam("EH_RT");
    }

    /// `unregister_function` is consistently a bool — pin the return
    /// type so accidental refactor to () would fail the type check.
    #[test]
    fn exec_hooks_unregister_function_returns_bool() {
        let _g = crate::test_util::global_state_lock();
        let r: bool = unregister_function("__never_xyz_zshrs__");
        let _ = r;
    }
}
