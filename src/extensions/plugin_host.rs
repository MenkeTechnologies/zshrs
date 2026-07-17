//! Native (Rust) plugin host — extension; no zsh C counterpart.
//!
//! zsh's `Src/module.c` `dlopen`s C `.so` modules that call `addbuiltin`
//! against the shell's own symbols. zshrs generalises that into a
//! **stable, versioned C ABI** (`zshrs-plugin` crate) so third parties
//! ship a compiled `cdylib` and load it at runtime with
//! `zmodload -R <path>` — no zshrs recompile, no zsh script glue. This
//! is the first compiled Unix shell hosting native compiled-language
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
//! the plugin's `abi_version` matches [`zshrs_plugin::ABI_VERSION`]
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

use zshrs_plugin::{BuiltinFn, HostApi, InitFn, PluginInfo, ABI_VERSION, INIT_SYMBOL};

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
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    staging().lock().unwrap().push((name, handler));
    0
}

extern "C" fn host_print(_host: *const HostApi, text: *const c_char) {
    if text.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

extern "C" fn host_eval(_host: *const HostApi, code: *const c_char) -> c_int {
    if code.is_null() {
        return 1;
    }
    let code = unsafe { CStr::from_ptr(code) }.to_string_lossy().into_owned();
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
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
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
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
    crate::ported::params::setsparam(&name, &value);
    0
}

extern "C" fn host_free_cstring(_host: *const HostApi, s: *mut c_char) {
    if !s.is_null() {
        // Reclaim ownership of a string we handed out via `into_raw`.
        unsafe { drop(CString::from_raw(s)) };
    }
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
        lib.get(INIT_SYMBOL)
            .map_err(|_| format!("`{}`: not a zshrs plugin (no {})", path,
                String::from_utf8_lossy(&INIT_SYMBOL[..INIT_SYMBOL.len() - 1])))?
    };

    // Clear staging, call init, collect what it registered.
    staging().lock().unwrap().clear();
    let info_ptr: *const PluginInfo = init(host_api());
    if info_ptr.is_null() {
        staging().lock().unwrap().clear();
        return Err(format!("`{}`: plugin init failed (ABI mismatch or error)", path));
    }
    let info = unsafe { &*info_ptr };
    if info.abi_version != ABI_VERSION {
        staging().lock().unwrap().clear();
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

    plugins().lock().unwrap().push(LoadedPlugin {
        name: name.clone(),
        version: version.clone(),
        path: expanded,
        _lib: lib,
    });

    tracing::info!(plugin = %name, version = %version, path, "loaded native plugin");
    Ok(name)
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

    // Now it is safe to dlclose.
    let mut ps = plugins().lock().unwrap();
    if let Some(pos) = ps.iter().position(|p| p.name == name) {
        let p = ps.remove(pos);
        tracing::info!(plugin = %name, "unloaded native plugin");
        drop(p); // explicit: dlclose here, after registry purge.
    }
    Ok(())
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
        owned.push(CString::new(a.as_str()).unwrap_or_else(|_| {
            CString::new(a.replace('\0', "")).unwrap_or_default()
        }));
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
pub fn zmodload_rust_cmd(
    nam: &str,
    args: &[String],
    ops: &crate::ported::zsh_h::options,
) -> i32 {
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
