//! Native (Rust) plugin host — extension; no zsh C counterpart.
//!
//! zsh's `Src/module.c` `dlopen`s C `.so` modules that call `addbuiltin`
//! against the shell's own symbols. zshrs generalises that into a
//! **stable, versioned C ABI** (`znative` crate) so third parties
//! ship a compiled `cdylib` and load it at runtime with
//! `zmodload -R <path>` — no zshrs recompile, no zsh script glue. This
//! is the first JIT-compiled Unix shell hosting native compiled-language
//! plugins.
//!
//! ## Where plugin commands resolve
//!
//! fusevm compiles command names it does not recognise as builtins into
//! *external* execution. A freshly-loaded plugin command is therefore
//! unknown at compile time and arrives at
//! [`crate::vm_helper`]'s `execute_external_bg`, which consults
//! [`dispatch`] BEFORE spawning a process — the same slot zsh uses for
//! `zmodload -ab` autoloaded builtins (`resolvebuiltin`). Plugins thus
//! resolve after real builtins and functions, before PATH lookup.
//!
//! ## ABI safety
//!
//! Everything crossing the boundary is `#[repr(C)]`. The host verifies
//! the plugin's `abi_version` matches [`znative::ABI_VERSION`]
//! before trusting any pointer it returns; a mismatch is refused (a
//! wrong struct layout would be undefined behaviour). The loaded
//! [`libloading::Library`] is kept alive for the process lifetime — its
//! `Drop` is a `dlclose`, which would invalidate the still-registered
//! function pointers, so unload explicitly purges the registry first.

#![allow(unused_imports)]

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::{Mutex, OnceLock};

use znative::{BuiltinFn, CompFn, HostApi, InitFn, PluginInfo, ABI_VERSION, INIT_SYMBOL};

/// One loaded plugin. Dropping `_lib` runs `dlclose`, so this is only
/// ever removed by [`unload`] AFTER its builtins are purged from
/// [`registry`].
struct LoadedPlugin {
    name: String,
    version: String,
    path: String,
    /// Kept alive for the process lifetime; drop = `dlclose`.
    _lib: libloading::Library,
}

/// A registered command → its handler and owning plugin.
#[derive(Clone, Copy)]
struct BuiltinEntry {
    func: BuiltinFn,
    /// Index into the intern table is overkill; store nothing but the
    /// function — ownership is tracked by name-prefix scan at unload.
    _pad: (),
}

fn plugins() -> &'static Mutex<Vec<LoadedPlugin>> {
    static P: OnceLock<Mutex<Vec<LoadedPlugin>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Vec::new()))
}

/// command-name → handler. Consulted by `execute_external_bg`.
fn registry() -> &'static Mutex<HashMap<String, BuiltinEntry>> {
    static R: OnceLock<Mutex<HashMap<String, BuiltinEntry>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Staging area for builtins registered during a single `init` call.
/// `init` runs before it returns the plugin name, so registrations are
/// buffered here and tagged with the owning plugin afterwards. Access is
/// serialised by [`load_lock`].
fn staging() -> &'static Mutex<Vec<(String, BuiltinFn)>> {
    static S: OnceLock<Mutex<Vec<(String, BuiltinFn)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

/// Serialises `load`/`unload` so the [`staging`] buffer is single-writer.
fn load_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// Which plugin currently owns each registered command name — parallel
/// to [`registry`], used only for `unload` bookkeeping.
fn ownership() -> &'static Mutex<HashMap<String, String>> {
    static O: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    O.get_or_init(|| Mutex::new(HashMap::new()))
}

/// compsys `_NAME` → (override handler, owning plugin). Consulted by the
/// completion router (`compsys::router::try_rust_dispatch`) BEFORE the
/// built-in Rust port, so a plugin-provided `_command_names` (etc.) wins.
/// (ABI v4.)
fn compfn_registry() -> &'static Mutex<HashMap<String, (CompFn, String)>> {
    static CR: OnceLock<Mutex<HashMap<String, (CompFn, String)>>> = OnceLock::new();
    CR.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Staging for compfn overrides registered during a single `init`, tagged
/// with the owner after init returns. Serialised by [`load_lock`]. (ABI v4.)
fn compfn_staging() -> &'static Mutex<Vec<(String, CompFn)>> {
    static CS: OnceLock<Mutex<Vec<(String, CompFn)>>> = OnceLock::new();
    CS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Completion wirings a plugin requested via `register_completion` that
/// have not yet been installed into compsys. Each entry is
/// `(cmd, generator_builtin, owning_plugin)`. Flushed by
/// [`flush_pending_completions`] at a safe point in the completion
/// pipeline (NOT during plugin init — evaling compsys glue deep inside
/// the `zmodload` call stack hangs the VM; `do_completion` is a designed
/// re-entry point where compsys itself evals).
fn pending_completions() -> &'static Mutex<Vec<(String, String, String)>> {
    static PC: OnceLock<Mutex<Vec<(String, String, String)>>> = OnceLock::new();
    PC.get_or_init(|| Mutex::new(Vec::new()))
}

/// Completions already wired into compsys, so `unload` can drop their
/// glue functions and `flush` never double-installs. Maps cmd → owner.
fn installed_completions() -> &'static Mutex<HashMap<String, String>> {
    static IC: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    IC.get_or_init(|| Mutex::new(HashMap::new()))
}

// ============================================================
// Host API callbacks — the `extern "C"` functions plugins call back
// through. One shared, leaked `HostApi` table for the whole process.
// ============================================================

extern "C" fn host_register_builtin(
    _host: *const HostApi,
    name: *const c_char,
    handler: BuiltinFn,
) -> c_int {
    if name.is_null() {
        return 1;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    staging().lock().unwrap().push((name, handler));
    0
}

extern "C" fn host_print(_host: *const HostApi, text: *const c_char) {
    if text.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

extern "C" fn host_eval(_host: *const HostApi, code: *const c_char) -> c_int {
    if code.is_null() {
        return 1;
    }
    let code = unsafe { CStr::from_ptr(code) }
        .to_string_lossy()
        .into_owned();
    // A plugin builtin runs inside VM context (dispatch happens in
    // execute_external_bg), so an executor is in scope. Re-entrant
    // with_executor is safe: the borrow is released before `f` runs.
    crate::fusevm_bridge::try_with_executor(|exec| exec.execute_script(&code))
        .map(|r| r.unwrap_or(1))
        .unwrap_or(1)
}

extern "C" fn host_getvar(_host: *const HostApi, name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    match crate::ported::params::getsparam(&name) {
        Some(v) => match CString::new(v) {
            Ok(c) => c.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

extern "C" fn host_setvar(
    _host: *const HostApi,
    name: *const c_char,
    value: *const c_char,
) -> c_int {
    if name.is_null() || value.is_null() {
        return 1;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let value = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    crate::ported::params::setsparam(&name, &value);
    0
}

extern "C" fn host_free_cstring(_host: *const HostApi, s: *mut c_char) {
    if !s.is_null() {
        // Reclaim ownership of a string we handed out via `into_raw`.
        unsafe { drop(CString::from_raw(s)) };
    }
}

extern "C" fn host_register_completion(
    _host: *const HostApi,
    cmd: *const c_char,
    generator: *const c_char,
) -> c_int {
    if cmd.is_null() || generator.is_null() {
        return 1;
    }
    let cmd = unsafe { CStr::from_ptr(cmd) }
        .to_string_lossy()
        .into_owned();
    let generator = unsafe { CStr::from_ptr(generator) }
        .to_string_lossy()
        .into_owned();
    // Owner is tagged after init returns (name unknown yet); stage empty.
    pending_completions()
        .lock()
        .unwrap()
        .push((cmd, generator, String::new()));
    0
}

extern "C" fn host_getfunction(_host: *const HostApi, name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // Route through the exact `${functions[name]}` read: getpmfunction
    // deparses the body into `u_str` and flags PM_UNSET when undefined.
    match crate::ported::modules::parameter::getpmfunction(std::ptr::null_mut(), &name) {
        Some(pm) if (pm.node.flags & crate::ported::zsh_h::PM_UNSET as i32) == 0 => {
            match pm.u_str.and_then(|s| CString::new(s).ok()) {
                Some(c) => c.into_raw(),
                None => std::ptr::null_mut(),
            }
        }
        _ => std::ptr::null_mut(),
    }
}

extern "C" fn host_addfunction(
    _host: *const HostApi,
    name: *const c_char,
    body: *const c_char,
) -> c_int {
    if name.is_null() || body.is_null() {
        return 1;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let body = unsafe { CStr::from_ptr(body) }
        .to_string_lossy()
        .into_owned();
    if name.is_empty() {
        return 1;
    }
    // Same install as `functions[name]=body`: parse `body` and store the
    // shfunc in shfunctab (dis = 0 = enabled).
    crate::ported::modules::parameter::setfunction(&name, body, 0);
    // setfunction installs unconditionally; report success.
    0
}

extern "C" fn host_register_compfn(
    _host: *const HostApi,
    name: *const c_char,
    handler: CompFn,
) -> c_int {
    if name.is_null() {
        return 1;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    if name.is_empty() {
        return 1;
    }
    compfn_staging().lock().unwrap().push((name, handler));
    0
}

extern "C" fn host_comp_dispatch(
    _host: *const HostApi,
    name: *const c_char,
    argc: usize,
    argv: *const *const c_char,
) -> c_int {
    if name.is_null() {
        return 1;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // Decode argv[0..argc] into owned Strings (argv[0] is the _fn name;
    // dispatch_function_call takes the arguments after it).
    let mut args: Vec<String> = Vec::with_capacity(argc);
    if !argv.is_null() {
        for i in 0..argc {
            let p = unsafe { *argv.add(i) };
            if p.is_null() {
                break;
            }
            args.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
        }
    }
    // args[0] is `name` itself (the compfn's argv[0]); pass args[1..] as the
    // dispatched function's argument list, matching how the completer chain
    // invokes `_alternative`/`_path_commands` etc.
    let call_args: &[String] = if args.is_empty() { &[] } else { &args[1..] };
    crate::ported::exec::dispatch_function_call(&name, call_args).unwrap_or(1)
}

extern "C" fn host_empty_command_hash(_host: *const HostApi) {
    crate::ported::hashtable::emptycmdnamtable();
}

/// The single process-wide host table. Leaked so its address is
/// `'static` — plugins may retain the `*const HostApi` and call through
/// it from any builtin at any time.
fn host_api() -> *const HostApi {
    static API: OnceLock<usize> = OnceLock::new();
    let addr = API.get_or_init(|| {
        let boxed = Box::new(HostApi {
            abi_version: ABI_VERSION,
            ctx: std::ptr::null_mut(),
            register_builtin: host_register_builtin,
            print: host_print,
            eval: host_eval,
            getvar: host_getvar,
            setvar: host_setvar,
            free_cstring: host_free_cstring,
            register_completion: host_register_completion,
            getfunction: host_getfunction,
            addfunction: host_addfunction,
            register_compfn: host_register_compfn,
            comp_dispatch: host_comp_dispatch,
            empty_command_hash: host_empty_command_hash,
        });
        Box::into_raw(boxed) as usize
    });
    *addr as *const HostApi
}

// ============================================================
// Public API — driven by `zmodload -R`.
// ============================================================

/// Load a plugin `cdylib` from `path`. Returns the plugin's name on
/// success. Idempotent-ish: loading a plugin whose name is already
/// present is refused (unload first).
pub fn load(path: &str) -> Result<String, String> {
    let _guard = load_lock().lock().unwrap();

    // `dlopen`. libloading resolves relative paths against the loader's
    // search rules; expand `~` for convenience since shells hand raw
    // tokens here.
    let expanded = expand_tilde(path);
    let lib = unsafe { libloading::Library::new(&expanded) }
        .map_err(|e| format!("cannot load `{}`: {}", path, e))?;

    // Resolve the mandatory init symbol.
    let init: libloading::Symbol<InitFn> = unsafe {
        lib.get(INIT_SYMBOL).map_err(|_| {
            format!(
                "`{}`: not a zshrs plugin (no {})",
                path,
                String::from_utf8_lossy(&INIT_SYMBOL[..INIT_SYMBOL.len() - 1])
            )
        })?
    };

    // Clear staging, call init, collect what it registered. Snapshot the
    // pending-completion length so we can tag the entries THIS init adds
    // with the owning plugin (the name isn't known until init returns).
    staging().lock().unwrap().clear();
    compfn_staging().lock().unwrap().clear();
    let pc_start = pending_completions().lock().unwrap().len();
    let info_ptr: *const PluginInfo = init(host_api());
    if info_ptr.is_null() {
        staging().lock().unwrap().clear();
        compfn_staging().lock().unwrap().clear();
        pending_completions().lock().unwrap().truncate(pc_start);
        return Err(format!(
            "`{}`: plugin init failed (ABI mismatch or error)",
            path
        ));
    }
    let info = unsafe { &*info_ptr };
    if info.abi_version != ABI_VERSION {
        staging().lock().unwrap().clear();
        compfn_staging().lock().unwrap().clear();
        pending_completions().lock().unwrap().truncate(pc_start);
        return Err(format!(
            "`{}`: ABI version {} != host {}",
            path, info.abi_version, ABI_VERSION
        ));
    }
    let name = cstr_or(info.name, "unknown");
    let version = cstr_or(info.version, "?");

    // Refuse a duplicate name — the second load's builtins would shadow
    // the first with no clean unload story.
    if plugins().lock().unwrap().iter().any(|p| p.name == name) {
        staging().lock().unwrap().clear();
        compfn_staging().lock().unwrap().clear();
        pending_completions().lock().unwrap().truncate(pc_start);
        return Err(format!("plugin `{}` already loaded", name));
    }

    // Commit staged builtins into the live registry, tagged with owner.
    let staged: Vec<(String, BuiltinFn)> = std::mem::take(&mut *staging().lock().unwrap());
    {
        let mut reg = registry().lock().unwrap();
        let mut own = ownership().lock().unwrap();
        for (cmd, func) in staged {
            reg.insert(cmd.clone(), BuiltinEntry { func, _pad: () });
            own.insert(cmd, name.clone());
        }
    }

    // Commit staged compfn overrides, tagged with owner. (ABI v4.)
    let staged_cf: Vec<(String, CompFn)> = std::mem::take(&mut *compfn_staging().lock().unwrap());
    {
        let mut cr = compfn_registry().lock().unwrap();
        for (fname, func) in staged_cf {
            cr.insert(fname, (func, name.clone()));
        }
    }

    // Tag the completion wirings this init staged with the owning plugin.
    {
        let mut pc = pending_completions().lock().unwrap();
        for entry in pc.iter_mut().skip(pc_start) {
            entry.2 = name.clone();
        }
    }

    plugins().lock().unwrap().push(LoadedPlugin {
        name: name.clone(),
        version: version.clone(),
        path: expanded,
        _lib: lib,
    });

    tracing::info!(plugin = %name, version = %version, path, "loaded native plugin");
    Ok(name)
}

/// Install any completion wirings that plugins requested but that have not
/// yet been bound into compsys. Called at the top of the completion
/// pipeline (`do_completion`) — a safe point where compsys itself evals,
/// unlike plugin-init (deep in the `zmodload` call stack, where evaling
/// hangs the VM). Idempotent: each pending entry is installed once, then
/// moved to `installed_completions`.
///
/// For each `(cmd, generator)` it defines a compsys completion function
/// `_zshrs_plug_<cmd>` that runs the generator with `$CURRENT $words` and
/// `compadd`s its newline-separated output, then binds it with `compdef`.
pub fn flush_pending_completions() {
    let pending: Vec<(String, String, String)> = {
        let mut pc = pending_completions().lock().unwrap();
        if pc.is_empty() {
            return;
        }
        std::mem::take(&mut *pc)
    };
    for (cmd, generator, owner) in pending {
        // `${(@f)...}` splits the generator's stdout on newlines into the
        // match array; guard on compdef so a pre-compinit flush no-ops.
        let glue = format!(
            "_zshrs_plug_{cmd}() {{ \
                local -a _zp_m; \
                _zp_m=(\"${{(@f)$({gen} $CURRENT $words)}}\"); \
                compadd -- $_zp_m; \
             }}; \
             (( ${{+functions[compdef]}} )) && compdef _zshrs_plug_{cmd} {cmd} 2>/dev/null; :",
            cmd = cmd,
            gen = generator,
        );
        // Use the exec.rs free fn (NOT try_with_executor): during
        // `do_completion` the thread-local CURRENT_EXECUTOR is unset —
        // completion runs on SESSION_EXECUTOR. try_with_executor would
        // no-op here and the compdef glue would never eval. This free fn
        // falls back to SESSION_EXECUTOR, so the glue lands on the same
        // executor that then runs the completion.
        let _ = crate::ported::exec::execute_script(&glue);
        installed_completions().lock().unwrap().insert(cmd, owner);
    }
}

/// Unload a plugin by name: purge its command registrations FIRST (so no
/// live function pointer survives), then drop the `Library` (`dlclose`).
pub fn unload(name: &str) -> Result<(), String> {
    let _guard = load_lock().lock().unwrap();

    let present = plugins().lock().unwrap().iter().any(|p| p.name == name);
    if !present {
        return Err(format!("plugin `{}` not loaded", name));
    }

    // Purge registry entries owned by this plugin.
    {
        let mut own = ownership().lock().unwrap();
        let mut reg = registry().lock().unwrap();
        let owned: Vec<String> = own
            .iter()
            .filter(|(_, o)| o.as_str() == name)
            .map(|(c, _)| c.clone())
            .collect();
        for cmd in owned {
            reg.remove(&cmd);
            own.remove(&cmd);
        }
    }

    // Purge compfn overrides owned by this plugin BEFORE dlclose, so no
    // dangling CompFn pointer survives the `dlclose`. (ABI v4.)
    compfn_registry()
        .lock()
        .unwrap()
        .retain(|_, (_, o)| o.as_str() != name);

    // Drop this plugin's completion bookkeeping. We do NOT eval here to
    // tear down the compsys glue function (evaling deep in the `zmodload`
    // call stack hangs the VM); the orphaned `_zshrs_plug_<cmd>` function
    // simply calls a now-removed generator builtin → empty command-subst
    // → no matches. Removing the `installed_completions` record lets a
    // later reload re-install cleanly.
    pending_completions()
        .lock()
        .unwrap()
        .retain(|(_, _, o)| o != name);
    installed_completions()
        .lock()
        .unwrap()
        .retain(|_, o| o != name);

    // Now it is safe to dlclose.
    let mut ps = plugins().lock().unwrap();
    if let Some(pos) = ps.iter().position(|p| p.name == name) {
        let p = ps.remove(pos);
        tracing::info!(plugin = %name, "unloaded native plugin");
        drop(p); // explicit: dlclose here, after registry purge.
    }
    Ok(())
}

/// Completion-router hook. Returns a plugin-registered override handler
/// for the compsys function `name` (a `_NAME`), or `None`. Consulted by
/// `compsys::router::try_rust_dispatch` BEFORE the built-in Rust port, so a
/// plugin's `_command_names` (etc.) supersedes both the port and the shell
/// autoload. (ABI v4.)
pub fn compfn_override(name: &str) -> Option<CompFn> {
    compfn_registry().lock().unwrap().get(name).map(|(f, _)| *f)
}

/// Invoke a plugin's override for compsys `_fn` `name`, if one is
/// registered. `args` are the completion-function arguments (argv[1..]);
/// argv[0] is set to `name`, matching the completer-chain convention.
/// Returns `Some(rc)` if a plugin handled it, else `None` (fall through to
/// the built-in port / shell autoload). (ABI v4.)
pub fn dispatch_compfn(name: &str, args: &[String]) -> Option<i32> {
    let func = compfn_override(name)?;
    // Build argv = [name, args...] as NUL-terminated C strings, valid for
    // the duration of the call (mirrors `dispatch`).
    let mut owned: Vec<CString> = Vec::with_capacity(args.len() + 1);
    owned.push(CString::new(name).ok()?);
    for a in args {
        owned.push(
            CString::new(a.as_str())
                .unwrap_or_else(|_| CString::new(a.replace('\0', "")).unwrap_or_default()),
        );
    }
    let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    let rc = func(host_api(), ptrs.len(), ptrs.as_ptr());
    Some(rc as i32)
}

/// Command-resolution hook. Called from `execute_external_bg` for bare
/// command names. Returns `Some(exit_status)` if a plugin owns `cmd`,
/// else `None` (let PATH lookup proceed).
pub fn dispatch(cmd: &str, args: &[String]) -> Option<i32> {
    let entry = { registry().lock().unwrap().get(cmd).copied() }?;

    // Build argv = [cmd, args...] as NUL-terminated C strings. Interior
    // NULs can't occur in a shell word, but be defensive.
    let mut owned: Vec<CString> = Vec::with_capacity(args.len() + 1);
    owned.push(CString::new(cmd).ok()?);
    for a in args {
        owned.push(
            CString::new(a.as_str())
                .unwrap_or_else(|_| CString::new(a.replace('\0', "")).unwrap_or_default()),
        );
    }
    let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();

    let rc = (entry.func)(host_api(), ptrs.len(), ptrs.as_ptr());
    // `owned`/`ptrs` outlive the call. Done.
    Some(rc as i32)
}

/// `(name, version, path)` for each loaded plugin, for `zmodload -R`
/// listing. Sorted by name for stable output.
pub fn list() -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = plugins()
        .lock()
        .unwrap()
        .iter()
        .map(|p| (p.name.clone(), p.version.clone(), p.path.clone()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// True if `name` is a live plugin command. Used where callers want to
/// know whether a name resolves to a plugin without invoking it.
pub fn is_plugin_command(name: &str) -> bool {
    registry().lock().unwrap().contains_key(name)
}

/// `zmodload -R` native-plugin management, dispatched from
/// `crate::ported::module::bin_zmodload` (kept out of `src/ported/`
/// because it is a zshrs extension, not a C port).
///
///   `zmodload -R  <path>...`  load each cdylib
///   `zmodload -R`             list loaded plugins (`name  version  path`)
///   `zmodload -uR <name>...`  unload each plugin by name
pub fn zmodload_rust_cmd(nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zsh_h::OPT_ISSET;

    // `-u` → unload.
    if OPT_ISSET(ops, b'u') {
        if args.is_empty() {
            zwarnnam(nam, "what do you want to unload?");
            return 1;
        }
        let mut ret = 0;
        for name in args {
            if let Err(e) = unload(name) {
                zwarnnam(nam, &e);
                ret = 1;
            }
        }
        return ret;
    }
    // No args → list.
    if args.is_empty() {
        for (name, version, path) in list() {
            println!("{}  {}  {}", name, version, path);
        }
        return 0;
    }
    // Load each path.
    let mut ret = 0;
    for path in args {
        if let Err(e) = load(path) {
            zwarnnam(nam, &e);
            ret = 1;
        }
    }
    ret
}

fn cstr_or(p: *const c_char, dflt: &str) -> String {
    if p.is_null() {
        dflt.to_string()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}
