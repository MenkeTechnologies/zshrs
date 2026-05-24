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

pub type ArrayGetFn = fn(&str) -> Option<Vec<String>>;
pub type AssocGetFn = fn(&str) -> Option<IndexMap<String, String>>;
pub type ArraySetFn = fn(&str, Vec<String>);
pub type AssocSetFn = fn(&str, IndexMap<String, String>);
pub type ScalarUnsetFn = fn(&str);
pub type ArrayUnsetFn = fn(&str);
pub type AssocUnsetFn = fn(&str);
pub type DispatchFunctionCallFn = fn(&str, &[String]) -> Option<i32>;
pub type ExecuteScriptFn = fn(&str) -> Result<i32, String>;
pub type ExecuteScriptZshPipelineFn = fn(&str) -> Result<i32, String>;
pub type RunCommandSubstitutionFn = fn(&str) -> String;
pub type PparamsGetFn = fn() -> Vec<String>;
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
static EXECUTE_SCRIPT: OnceLock<ExecuteScriptFn> = OnceLock::new();
static EXECUTE_SCRIPT_ZSH_PIPELINE: OnceLock<ExecuteScriptZshPipelineFn> = OnceLock::new();
static RUN_COMMAND_SUBSTITUTION: OnceLock<RunCommandSubstitutionFn> = OnceLock::new();
static PPARAMS_GET: OnceLock<PparamsGetFn> = OnceLock::new();
static PPARAMS_SET: OnceLock<PparamsSetFn> = OnceLock::new();
static UNREGISTER_FUNCTION: OnceLock<UnregisterFunctionFn> = OnceLock::new();

pub fn install_array_get(f: ArrayGetFn) {
    let _ = ARRAY_GET.set(f);
}
pub fn install_assoc_get(f: AssocGetFn) {
    let _ = ASSOC_GET.set(f);
}
pub fn install_array_set(f: ArraySetFn) {
    let _ = ARRAY_SET.set(f);
}
pub fn install_assoc_set(f: AssocSetFn) {
    let _ = ASSOC_SET.set(f);
}
pub fn install_scalar_unset(f: ScalarUnsetFn) {
    let _ = SCALAR_UNSET.set(f);
}
pub fn install_array_unset(f: ArrayUnsetFn) {
    let _ = ARRAY_UNSET.set(f);
}
pub fn install_assoc_unset(f: AssocUnsetFn) {
    let _ = ASSOC_UNSET.set(f);
}
pub fn install_dispatch_function_call(f: DispatchFunctionCallFn) {
    let _ = DISPATCH_FUNCTION_CALL.set(f);
}
pub fn install_execute_script(f: ExecuteScriptFn) {
    let _ = EXECUTE_SCRIPT.set(f);
}
pub fn install_execute_script_zsh_pipeline(f: ExecuteScriptZshPipelineFn) {
    let _ = EXECUTE_SCRIPT_ZSH_PIPELINE.set(f);
}
pub fn install_run_command_substitution(f: RunCommandSubstitutionFn) {
    let _ = RUN_COMMAND_SUBSTITUTION.set(f);
}
pub fn install_pparams_get(f: PparamsGetFn) {
    let _ = PPARAMS_GET.set(f);
}
pub fn install_pparams_set(f: PparamsSetFn) {
    let _ = PPARAMS_SET.set(f);
}
pub fn install_unregister_function(f: UnregisterFunctionFn) {
    let _ = UNREGISTER_FUNCTION.set(f);
}

pub fn array(name: &str) -> Option<Vec<String>> {
    ARRAY_GET.get().and_then(|f| f(name))
}
pub fn assoc(name: &str) -> Option<IndexMap<String, String>> {
    ASSOC_GET.get().and_then(|f| f(name))
}
pub fn set_array(name: &str, val: Vec<String>) {
    if let Some(f) = ARRAY_SET.get() {
        f(name, val);
    }
}
pub fn set_assoc(name: &str, val: IndexMap<String, String>) {
    if let Some(f) = ASSOC_SET.get() {
        f(name, val);
    }
}
pub fn unset_scalar(name: &str) {
    if let Some(f) = SCALAR_UNSET.get() {
        f(name);
    }
}
pub fn unset_array(name: &str) {
    if let Some(f) = ARRAY_UNSET.get() {
        f(name);
    }
}
pub fn unset_assoc(name: &str) {
    if let Some(f) = ASSOC_UNSET.get() {
        f(name);
    }
}
pub fn dispatch_function_call(name: &str, args: &[String]) -> Option<i32> {
    DISPATCH_FUNCTION_CALL.get().and_then(|f| f(name, args))
}
pub fn execute_script(src: &str) -> Result<i32, String> {
    match EXECUTE_SCRIPT.get() {
        Some(f) => f(src),
        None => Ok(0),
    }
}
pub fn execute_script_zsh_pipeline(src: &str) -> Result<i32, String> {
    match EXECUTE_SCRIPT_ZSH_PIPELINE.get() {
        Some(f) => f(src),
        None => Ok(0),
    }
}
pub fn run_command_substitution(cmd: &str) -> String {
    match RUN_COMMAND_SUBSTITUTION.get() {
        Some(f) => f(cmd),
        None => String::new(),
    }
}
pub fn pparams() -> Vec<String> {
    PPARAMS_GET.get().map(|f| f()).unwrap_or_default()
}
pub fn set_pparams(v: Vec<String>) {
    if let Some(f) = PPARAMS_SET.get() {
        f(v);
    }
}
pub fn unregister_function(name: &str) -> bool {
    UNREGISTER_FUNCTION.get().map(|f| f(name)).unwrap_or(false)
}
