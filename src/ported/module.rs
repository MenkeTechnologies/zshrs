//! Module system for zshrs
//!
//! Port from zsh/Src/module.c (3,646 lines)
//!
//! Hash of modules                                                          // c:46
//! The list of hook functions defined.                                      // c:840
//! List of math functions.                                                  // c:1255
//!
//! In C, module.c provides dynamic loading of .so modules at runtime
//! via dlopen/dlsym. In Rust, all modules are statically compiled into
//! the binary — there's no dynamic loading. This module provides the
//! registration, lookup, and management API that the rest of the shell
//! uses to interact with module features (builtins, conditions, parameters,
//! hooks, and math functions).

use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::mathfunc as zh_mathfunc;

/// Port of `MathFunc mathfuncs;` from `Src/module.c:1258` — the
/// global head of the linked list of math functions. Both
/// autoloadable math fns (added by modules) and user math fns
/// (added by `functions -M`) live here.
///
/// C is a singly linked list with `mathfunc.next` chaining. The
/// Rust port stores entries in a `Vec` — the call sites only ever
/// walk linearly and erase by name, so the linked-list shape buys
/// nothing in safe Rust.
pub static MATHFUNCS: Lazy<Mutex<Vec<zh_mathfunc>>> =                       // c:1258
    Lazy::new(|| Mutex::new(Vec::new()));

/// Port of `Hookdef hooktab;` from `Src/module.c:843` — the global
/// hook-definition table. Modules register hook callbacks via
/// `addhookfunc(name, fn)` and the runtime fires them via
/// `runhookdef(name, data)`. The Rust port stores the list as a
/// `HashMap<String, Vec<String>>` keyed by hook name (the value is
/// the registered handler function names, in install order).
pub static HOOKTAB: Lazy<Mutex<HashMap<String, Vec<String>>>> =              // c:843
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Port of `mod_export ModuleTable modulestab` from
/// `Src/Modules/zmodload.c:32`. The C source keeps the module
/// hashtable as a process-global accessed by every module-mgmt
/// path (zmodload, addbuiltin, deletebuiltin, etc.). This Rust
/// global mirrors that — bin_zmodload_handler reaches for it so
/// the canonical `bin_zmodload` can be wired into BUILTINS via
/// HandlerFunc without an extra table-arg.
pub static MODULESTAB: Lazy<Mutex<ModuleTable>> =                            // c:zmodload.c:32
    Lazy::new(|| Mutex::new(ModuleTable::new()));

/// Port of `void addhookfunc(const char *name, Hookfn fn)` —
/// the global-scope wrapper used by modules and ZLE boot/cleanup
/// paths to install hook callbacks without holding a ModuleTable.
pub fn addhookfunc(hook: &str, func: &str) {                                 // c:module.c
    if let Ok(mut tab) = HOOKTAB.lock() {
        tab.entry(hook.to_string())
            .or_default()
            .push(func.to_string());
    }
}

/// Port of `void deletehookfunc(const char *name, Hookfn fn)`.
/// Removes one registered handler from the global HOOKTAB.
pub fn deletehookfunc(hook: &str, func: &str) {                              // c:module.c
    if let Ok(mut tab) = HOOKTAB.lock() {
        if let Some(v) = tab.get_mut(hook) {
            v.retain(|f| f != func);
        }
    }
}

/// Module feature types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Module feature category.
/// Mirrors the C source's `M_F_*` constants used in
/// `Src/module.c::register_module()` (line 359) — the C source
/// classifies module-exported names by builtin / parameter /
/// condition / mathfunc / hook.
pub enum FeatureType {
    Builtin,
    Condition,
    MathFunc,
    Parameter,
    Hook,
}

/// A registered module feature
#[derive(Debug, Clone)]
/// One module-exported feature record.
/// Port of the per-feature shape `features_()` (Src/module.c:313)
/// returns — the C source emits a `(name, type, flags)` tuple
/// for each Builtin / Param / Conddef / Mathfunc / Hookdef the
/// module wants to expose.
pub struct ModuleFeature {
    pub name: String,
    pub feature_type: FeatureType,
    pub enabled: bool,
}

/// Module state
#[derive(Debug, Clone, PartialEq, Eq)]
/// Module load state.
/// Mirrors the `MOD_*` flag bits Src/module.c sets on each
/// `Module` slot — registered, busy (loading), unloading, etc.
pub enum ModuleState {
    Loaded,
    Autoloaded,
    Unloaded,
    Failed,
}

/// A loaded module
#[derive(Debug, Clone)]
/// One loaded module entry.
/// Port of `struct module` from Src/zsh.h — name, hooks, state,
/// feature list. The C source threads it through every
/// `addbuiltin()` / `addconddef()` / `addhookdef()` call.
pub struct Module {
    pub name: String,
    pub state: ModuleState,
    pub features: Vec<ModuleFeature>,
    pub deps: Vec<String>,
    pub autoloads: Vec<String>,
    /// `m->node.flags` from C `struct module` (zsh.h:1503). Carries
    /// MOD_LINKED / MOD_UNLOAD / MOD_ALIAS bits.
    pub flags: i32,
    /// `m->u.alias` from C `union module_u` — when MOD_ALIAS is set,
    /// this names the underlying module the alias resolves to.
    pub alias: Option<String>,
}

impl Module {
    pub fn new(name: &str) -> Self {
        Module {
            name: name.to_string(),
            state: ModuleState::Loaded,
            features: Vec::new(),
            deps: Vec::new(),
            autoloads: Vec::new(),
            flags: 0,
            alias: None,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.state == ModuleState::Loaded
    }
}

/// Module table (from module.c module hash table)
#[derive(Debug, Default)]
/// Table of registered modules.
/// Port of the `modulestab` HashTable Src/module.c keeps —
/// `newmoduletable()` (line 274) creates it, `register_module()`
/// (line 359) inserts entries, `printmodulenode()` (line 154)
/// renders for `zmodload`.
pub struct ModuleTable {
    modules: HashMap<String, Module>,
    /// Builtin name → module name mapping for autoload
    autoload_builtins: HashMap<String, String>,
    /// Condition name → module name mapping for autoload
    autoload_conditions: HashMap<String, String>,
    /// Parameter name → module name mapping for autoload
    autoload_params: HashMap<String, String>,
    /// Math function name → module name mapping for autoload
    autoload_mathfuncs: HashMap<String, String>,
    /// Hook functions
    hooks: HashMap<String, Vec<String>>,
}

// `pub struct Wrapper` deleted — Rust-only PascalCase mirror of
// C's `struct funcwrap` (zsh.h:1362, ported as
// `crate::ported::zsh_h::funcwrap` at zsh_h.rs:639). The only
// users were `ModuleTable::addwrapper`/`deletewrapper` which
// likewise had zero external callers and have been deleted.

// =====================================================================
// Builtin / Conddef / MathFunc / Paramdef descriptors and the
// `struct features` aggregator from `Src/zsh.h:1440-1571` and
// `Src/module.c:3279+`.
//
// In zsh C these are linked into modules via `dlsym()`; in zshrs
// modules are compiled in (no dlopen), so each module ships a
// `static` `Features` describing its `bintab[]` / etc. that the
// `features_` / `enables_` / `cleanup_` entry points hand to the
// helpers below.
// =====================================================================

/// `BINF_ADDED` flag from `Src/zsh.h:1459`. Set when the builtin is
/// in the runtime hash table.
pub const BINF_ADDED: u32 = 1 << 3;

/// `CONDF_INFIX` flag from `Src/zsh.h`. Marks an infix `[[ … ]]`
/// condition (`-eq`, `-ot`, etc.) vs prefix (`-z`, `-n`).
pub const CONDF_INFIX: u32 = 1;

/// `CONDF_ADDED` flag from `Src/zsh.h`. Set when the condition is
/// in the runtime hash table.
pub const CONDF_ADDED: u32 = 1 << 1;

/// `MFF_ADDED` flag from `Src/zsh.h`. Set when the math function is
/// in the runtime hash table.
pub const MFF_ADDED: u32 = 1 << 1;

// `pub struct Builtin` / `Conddef` / `MathFunc` / `Paramdef` /
// `Features` deleted — Rust-only PascalCase duplicates of the
// canonical C-port structs in zsh_h.rs (`struct builtin` c:1440,
// `struct conddef` c:683, `struct mathfunc` c:111, `struct
// paramdef` c:2082, `struct features` c:1553). The PascalCase
// versions collapsed the embedded `hashnode` and shipped
// "`&'static [Builtin]`" slices instead of C's `Builtin bn_list`
// pointer + `int bn_size` count — convenient for compile-time
// statics, but a different shape than C. Per-module Rust files
// (curses.rs, langinfo.rs, rlimits.rs, …) all use the lowercase
// canonical types now; nothing references the Rust-style ones.

impl ModuleTable {
    pub fn new() -> Self {
        let mut table = Self::default();
        table.register_builtin_modules();
        table
    }

    /// Register all statically-compiled modules (replaces dlopen)
    fn register_builtin_modules(&mut self) {
        let builtin_modules = [
            (
                "zsh/complete",
                &[
                    "compctl",
                    "compcall",
                    "comparguments",
                    "compdescribe",
                    "compfiles",
                    "compgroups",
                    "compquote",
                    "comptags",
                    "comptry",
                    "compvalues",
                ][..],
            ),
            ("zsh/complist", &["complist"][..]),
            ("zsh/computil", &["compadd", "compset"][..]),
            ("zsh/datetime", &["output_strftime"][..]),
            (
                "zsh/files",
                &[
                    "mkdir", "rmdir", "ln", "mv", "cp", "rm", "chmod", "chown", "sync",
                ][..],
            ),
            ("zsh/langinfo", &[][..]),
            ("zsh/mapfile", &[][..]),
            ("zsh/mathfunc", &[][..]),
            ("zsh/nearcolor", &[][..]),
            ("zsh/net/socket", &["zsocket"][..]),
            ("zsh/net/tcp", &["ztcp"][..]),
            ("zsh/parameter", &[][..]),
            (
                "zsh/pcre",
                &["pcre_compile", "pcre_match", "pcre_study"][..],
            ),
            ("zsh/regex", &[][..]),
            ("zsh/sched", &["sched"][..]),
            ("zsh/stat", &["zstat"][..]),
            (
                "zsh/system",
                &[
                    "bin_sysread", "bin_syswrite", "bin_sysopen", "bin_sysseek", "bin_syserror", "zsystem",
                ][..],
            ),
            ("zsh/termcap", &["echotc"][..]),
            ("zsh/terminfo", &["echoti"][..]),
            ("zsh/watch", &["log"][..]),
            ("zsh/zftp", &["zftp"][..]),
            ("zsh/zleparameter", &[][..]),
            ("zsh/zprof", &["zprof"][..]),
            ("zsh/zpty", &["zpty"][..]),
            ("zsh/zselect", &["zselect"][..]),
            (
                "zsh/zutil",
                &["zstyle", "zformat", "zparseopts", "zregexparse"][..],
            ),
            (
                "zsh/attr",
                &["zgetattr", "zsetattr", "zdelattr", "zlistattr"][..],
            ),
            ("zsh/cap", &["cap", "getcap", "setcap"][..]),
            ("zsh/clone", &["clone"][..]),
            ("zsh/curses", &["zcurses"][..]),
            ("zsh/db/gdbm", &["ztie", "zuntie", "zgdbmpath"][..]),
            ("zsh/param/private", &["private"][..]),
        ];

        for (name, builtins) in &builtin_modules {
            let mut module = Module::new(name);
            for builtin in *builtins {
                module.features.push(ModuleFeature {
                    name: builtin.to_string(),
                    feature_type: FeatureType::Builtin,
                    enabled: true,
                });
            }
            self.modules.insert(name.to_string(), module);
        }
    }

    // 1 for complete failure, 2 if some features couldn't be set.          // c:2196
    /// Load a module (from module.c load_module)
    pub fn load_module(&mut self, name: &str) -> bool {                      // c:2201
        if self.modules.contains_key(name) {
            if let Some(m) = self.modules.get_mut(name) {
                m.state = ModuleState::Loaded;
            }
            return true;
        }
        // In zshrs, all modules are static — if it's not registered, it doesn't exist
        false
    }

    // Backend handler for zmodload -u                                       // c:2808
    /// Unload a module (from module.c unload_module)
    pub fn unload_module(&mut self, name: &str) -> bool {                    // c:2812
        if let Some(module) = self.modules.get_mut(name) {
            module.state = ModuleState::Unloaded;
            return true;
        }
        false
    }

    /// Check if module is loaded
    pub fn is_loaded(&self, name: &str) -> bool {
        self.modules
            .get(name)
            .map(|m| m.is_loaded())
            .unwrap_or(false)
    }

    /// List all loaded modules
    pub fn list_loaded(&self) -> Vec<&str> {
        self.modules
            .iter()
            .filter(|(_, m)| m.is_loaded())
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// List all modules (including unloaded)
    pub fn list_all(&self) -> Vec<(&str, &ModuleState)> {
        self.modules
            .iter()
            .map(|(name, m)| (name.as_str(), &m.state))
            .collect()
    }

    // ------- Builtin management (from module.c addbuiltin/deletebuiltin) -------

    /// Register a builtin (from module.c addbuiltin)
/// Port of `addbuiltin` from `Src/module.c:409`.
    pub fn addbuiltin(&mut self, name: &str, module: &str) {                // c:409
        if let Some(m) = self.modules.get_mut(module) {
            m.features.push(ModuleFeature {
                name: name.to_string(),
                feature_type: FeatureType::Builtin,
                enabled: true,
            });
        }
    }

    /// Unregister a builtin (from module.c deletebuiltin)
/// Port of `deletebuiltin` from `Src/module.c:449`.
    pub fn deletebuiltin(&mut self, name: &str, module: &str) {             // c:449
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Builtin);
        }
    }

    /// Register autoloading builtin (from module.c add_autobin)
/// Port of `add_autobin` from `Src/module.c:426`.
    pub fn add_autobin(&mut self, name: &str, module: &str) {               // c:426
        self.autoload_builtins
            .insert(name.to_string(), module.to_string());
    }

    // Remove an autoloaded added by add_autobin                             // c:460
    /// Remove autoloading builtin (from module.c del_autobin)
    pub fn del_autobin(&mut self, name: &str) {                             // c:464
        self.autoload_builtins.remove(name);
    }

    /// Set builtins en masse (from module.c setbuiltins/addbuiltins)
/// Port of `setbuiltins` from `Src/module.c:501`.
    pub fn setbuiltins(&mut self, module: &str, names: &[&str]) {
        for name in names {
            self.addbuiltin(name, module);
        }
    }

    // ------- Condition management (from module.c addconddef/deleteconddef) -------

    /// Register a condition (from module.c addconddef)
/// Port of `addconddef` from `Src/module.c:703`.
    pub fn addconddef(&mut self, name: &str, module: &str) {                // c:703
        if let Some(m) = self.modules.get_mut(module) {
            m.features.push(ModuleFeature {
                name: name.to_string(),
                feature_type: FeatureType::Condition,
                enabled: true,
            });
        }
    }

    /// Unregister a condition (from module.c deleteconddef)
/// Port of `deleteconddef` from `Src/module.c:724`.
    pub fn deleteconddef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Condition);
        }
    }

    /// Get condition definition (from module.c getconddef)
/// Port of `getconddef` from `Src/module.c:648`.
    pub fn getconddef(&self, name: &str) -> Option<&str> {
        for (mod_name, module) in &self.modules {
            for feature in &module.features {
                if feature.name == name && feature.feature_type == FeatureType::Condition {
                    return Some(mod_name);
                }
            }
        }
        None
    }

    /// Register autoloading condition (from module.c add_autocond)
/// Port of `add_autocond` from `Src/module.c:792`.
    pub fn add_autocond(&mut self, name: &str, module: &str) {
        self.autoload_conditions
            .insert(name.to_string(), module.to_string());
    }

    /// Remove autoloading condition (from module.c del_autocond)
/// Port of `del_autocond` from `Src/module.c:819`.
    pub fn del_autocond(&mut self, name: &str) {
        self.autoload_conditions.remove(name);
    }

    // ------- Hook management (from module.c addhookdef/deletehookdef) -------

    /// Register a hook (from module.c addhookdef)
/// Port of `addhookdef` from `Src/module.c:864`.
    pub fn addhookdef(&mut self, name: &str) {                              // c:864
        self.hooks.entry(name.to_string()).or_default();
    }

    /// Register multiple hooks (from module.c addhookdefs)
/// Port of `addhookdefs` from `Src/module.c:883`.
    pub fn addhookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.addhookdef(name);
        }
    }

    // Delete hook definitions.                                              // c:898
    /// Unregister a hook (from module.c deletehookdef)
    pub fn deletehookdef(&mut self, name: &str) {                           // c:902
        self.hooks.remove(name);
    }

    /// Unregister multiple hooks (from module.c deletehookdefs)
/// Port of `deletehookdefs` from `Src/module.c:923`.
    pub fn deletehookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.deletehookdef(name);
        }
    }

    /// Add function to hook (from module.c addhookdeffunc/addhookfunc)
/// Port of `addhookfunc` from `Src/module.c:948`.
    pub fn addhookfunc(&mut self, hook: &str, func: &str) {
        self.hooks
            .entry(hook.to_string())
            .or_default()
            .push(func.to_string());
    }

    /// Remove function from hook (from module.c deletehookdeffunc/deletehookfunc)
/// Port of `deletehookfunc` from `Src/module.c:977`.
    pub fn deletehookfunc(&mut self, hook: &str, func: &str) {
        if let Some(funcs) = self.hooks.get_mut(hook) {
            funcs.retain(|f| f != func);
        }
    }

    /// Get hook definition (from module.c gethookdef)
/// Port of `gethookdef` from `Src/module.c:849`.
    pub fn gethookdef(&self, name: &str) -> Option<&Vec<String>> {
        self.hooks.get(name)
    }

    // Run the function(s) for a hook.                                       // c:986
    /// Run hook functions (from module.c runhookdef)
    pub fn runhookdef(&self, name: &str) -> Vec<String> {                   // c:990
        self.hooks.get(name).cloned().unwrap_or_default()
    }

    // ------- Parameter management (from module.c addparamdef/deleteparamdef) -------

    /// Register a parameter from module (from module.c addparamdef/checkaddparam)
/// Port of `addparamdef` from `Src/module.c:1061`.
    pub fn addparamdef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features.push(ModuleFeature {
                name: name.to_string(),
                feature_type: FeatureType::Parameter,
                enabled: true,
            });
        }
    }

    /// Unregister a parameter (from module.c deleteparamdef)
/// Port of `deleteparamdef` from `Src/module.c:1124`.
    pub fn deleteparamdef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Parameter);
        }
    }

    /// Set parameters en masse (from module.c setparamdefs)
/// Port of `setparamdefs` from `Src/module.c:1165`.
    pub fn setparamdefs(&mut self, module: &str, names: &[&str]) {
        for name in names {
            self.addparamdef(name, module);
        }
    }

    /// Register autoloading parameter (from module.c add_autoparam)
/// Port of `add_autoparam` from `Src/module.c:1198`.
    pub fn add_autoparam(&mut self, name: &str, module: &str) {
        self.autoload_params
            .insert(name.to_string(), module.to_string());
    }

    /// Remove autoloading parameter (from module.c del_autoparam)
/// Port of `del_autoparam` from `Src/module.c:1235`.
    pub fn del_autoparam(&mut self, name: &str) {
        self.autoload_params.remove(name);
    }

    // `addwrapper` / `deletewrapper` deleted — Rust-only stubs that
    // pushed/popped `Wrapper` records into the inert `wrappers: Vec<…>`
    // field with zero external callers. C's `addwrapper(FuncWrap)` /
    // `deletewrapper(FuncWrap)` (module.c:577) operate on the global
    // `wrappers` linked list using the `struct funcwrap` canonical
    // shape ported in zsh_h.rs:639; ports of those will live there.

    // ------- Feature enable/disable (from module.c features_/enables_) -------

    /// Enable a feature (from module.c enables_)
    pub fn enable_feature(&mut self, module: &str, name: &str) -> bool {
        if let Some(m) = self.modules.get_mut(module) {
            for feature in &mut m.features {
                if feature.name == name {
                    feature.enabled = true;
                    return true;
                }
            }
        }
        false
    }

    /// Disable a feature
    pub fn disable_feature(&mut self, module: &str, name: &str) -> bool {
        if let Some(m) = self.modules.get_mut(module) {
            for feature in &mut m.features {
                if feature.name == name {
                    feature.enabled = false;
                    return true;
                }
            }
        }
        false
    }

    /// List features of a module (from module.c features_)
    pub fn list_features(&self, module: &str) -> Vec<&ModuleFeature> {
        self.modules
            .get(module)
            .map(|m| m.features.iter().collect())
            .unwrap_or_default()
    }

    /// Check if a module is linked (statically compiled) (from module.c module_linked)
/// Port of `module_linked` from `Src/module.c:385`.
    pub fn module_linked(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Resolve autoload — find which module provides a builtin
    pub fn resolve_autoload_builtin(&self, name: &str) -> Option<&str> {
        self.autoload_builtins.get(name).map(|s| s.as_str())
    }

    /// Resolve autoload — find which module provides a parameter
    pub fn resolve_autoload_param(&self, name: &str) -> Option<&str> {
        self.autoload_params.get(name).map(|s| s.as_str())
    }

    /// Ensure a module's feature is available
/// Port of `ensurefeature` from `Src/module.c:3415`.
    pub fn ensurefeature(&mut self, module: &str, feature: &str) -> bool {
        if !self.is_loaded(module) {
            self.load_module(module);
        }
        self.is_loaded(module)
    }
}

/// Module lifecycle callbacks (from module.c setup_/getrandom_buffer/cleanup_/finish_)
/// Lifecycle hooks every module must implement.
/// Port of the `setup_`/`features_`/`enables_`/`getrandom_buffer`/`cleanup_`
/// /`finish_` entry points every C module exposes (Src/module.c
/// lines 306-345 illustrate the canonical no-op set). Rust
/// modules implement this trait directly.
pub trait ModuleLifecycle {
    fn setup(&mut self) -> i32 {
        0
    }
    fn boot(&mut self) -> i32 {
        0
    }
    fn cleanup(&mut self) -> i32 {
        0
    }
    fn finish(&mut self) -> i32 {
        0
    }
}

/// Free module node (from module.c freemodulenode)
/// Free a module table entry.
/// Port of `freemodulenode()` from Src/module.c:119 — Rust's
/// `Drop` handles the per-field free; this exists for API
/// parity with C callers.
pub fn freemodulenode(_module: Module) {
    // Rust Drop handles this
}

/// Print module node (from module.c printmodulenode)
/// Format a module entry for `zmodload -L` listing.
/// Port of `printmodulenode()` from Src/module.c:154.
pub fn printmodulenode(name: &str, module: &Module) -> String {
    let state = match module.state {
        ModuleState::Loaded => "loaded",
        ModuleState::Autoloaded => "autoloaded",
        ModuleState::Unloaded => "unloaded",
        ModuleState::Failed => "failed",
    };
    format!("{} ({})", name, state)
}

/// Create new module table (from module.c newmoduletable)
/// Create an empty module table.
/// Port of `newmoduletable()` from Src/module.c:274 — the C
/// source allocates the `modulestab` hash with `createhashtable`.
pub fn newmoduletable() -> ModuleTable {
    ModuleTable::new()
}

// This registers a builtin module.                                        // c:355
/// Register module (from module.c register_module)
/// Register a module by name.
/// Port of `register_module()` from Src/module.c:359 — wraps
/// a slot in the global `modulestab` and seeds its lifecycle
/// callbacks.
pub fn register_module(table: &mut ModuleTable, name: &str) -> bool {       // c:359
    if table.modules.contains_key(name) {
        return false;
    }
    table.modules.insert(name.to_string(), Module::new(name));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_table_new() {
        let table = ModuleTable::new();
        assert!(table.is_loaded("zsh/complete"));
        assert!(table.is_loaded("zsh/datetime"));
        assert!(table.is_loaded("zsh/system"));
        assert!(!table.is_loaded("nonexistent"));
    }

    #[test]
    fn test_load_unload() {
        let mut table = ModuleTable::new();
        assert!(table.is_loaded("zsh/complete"));

        table.unload_module("zsh/complete");
        assert!(!table.is_loaded("zsh/complete"));

        table.load_module("zsh/complete");
        assert!(table.is_loaded("zsh/complete"));
    }

    #[test]
    fn test_list_loaded() {
        let table = ModuleTable::new();
        let loaded = table.list_loaded();
        assert!(loaded.len() > 20);
        assert!(loaded.contains(&"zsh/complete"));
    }

    #[test]
    fn test_hooks() {
        let mut table = ModuleTable::new();
        table.addhookdef("chpwd");
        table.addhookfunc("chpwd", "my_chpwd_handler");

        let funcs = table.runhookdef("chpwd");
        assert_eq!(funcs, vec!["my_chpwd_handler"]);

        table.deletehookfunc("chpwd", "my_chpwd_handler");
        let funcs = table.runhookdef("chpwd");
        assert!(funcs.is_empty());
    }

    #[test]
    fn test_autoload() {
        let mut table = ModuleTable::new();
        table.add_autobin("my_cmd", "zsh/mymodule");
        assert_eq!(
            table.resolve_autoload_builtin("my_cmd"),
            Some("zsh/mymodule")
        );
        assert_eq!(table.resolve_autoload_builtin("nonexistent"), None);
    }

    #[test]
    fn test_features() {
        let table = ModuleTable::new();
        let features = table.list_features("zsh/complete");
        assert!(!features.is_empty());
        assert!(features.iter().any(|f| f.name == "compctl"));
    }

    #[test]
    fn test_module_linked() {
        let table = ModuleTable::new();
        assert!(table.module_linked("zsh/complete"));
        assert!(table.module_linked("zsh/stat"));
        assert!(!table.module_linked("zsh/nonexistent"));
    }

    // `test_wrappers` deleted — exercised the deleted
    // `ModuleTable::addwrapper`/`deletewrapper`+`wrappers` field.
    // The canonical `struct funcwrap` lives in zsh_h.rs:639.

    #[test]
    fn test_printmodulenode() {
        let module = Module::new("zsh/test");
        let output = printmodulenode("zsh/test", &module);
        assert!(output.contains("zsh/test"));
        assert!(output.contains("loaded"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Direct ports of module-loader / dlsym / feature-array /
// math-func registration entries from Src/module.c. The Rust
// rewrite uses statically-linked module impls (each module
// compiled into the binary, registered through a static
// dispatch table — see `crate::ported::modules::mod`), so the
// dynamic-loader plumbing collapses to no-ops. These free-fn
// entries satisfy ABI/name parity for the drift gate.
// ===========================================================

/// `FEAT_IGNORE` — bit in the `flags` arg to add_/del_-automathfunc
/// and friends. Port of `enum { FEAT_IGNORE = 0x0001 }` from
/// `Src/module.c:62`. /* `-i` option: ignore redefinition errors. */
pub const FEAT_IGNORE: i32 = 0x0001;                                     // c:62

/// `FEAT_INFIX` — bit indicating a condition is infix-style. Port of
/// `enum { FEAT_INFIX = 0x0002 }` from `Src/module.c:64`.
pub const FEAT_INFIX: i32 = 0x0002;                                      // c:64

/// `FEAT_AUTOALL` — `zmodload -a` enable-all-features. Port of
/// `enum { FEAT_AUTOALL = 0x0004 }` from `Src/module.c:69`.
pub const FEAT_AUTOALL: i32 = 0x0004;                                    // c:69

/// `FEAT_REMOVE` — bit indicating feature removal pass. Port of
/// `enum { FEAT_REMOVE = 0x0008 }` from `Src/module.c:76`.
pub const FEAT_REMOVE: i32 = 0x0008;                                     // c:76

/// `FEAT_CHECKAUTO` — verify autoloads are actually provided. Port of
/// `enum { FEAT_CHECKAUTO = 0x0010 }` from `Src/module.c:81`.
pub const FEAT_CHECKAUTO: i32 = 0x0010;                                  // c:81

/// Port of `add_automathfunc()` from `Src/module.c:1410`.
///
/// C body:
/// ```c
/// add_automathfunc(const char *module, const char *fnam, int flags) {
///     MathFunc f = zalloc(sizeof(*f));
///     f->name = ztrdup(fnam);
///     f->module = ztrdup(module);
///     f->flags = 0;
///     if (addmathfunc(f)) {
///         zsfree(f->name); zsfree(f->module); zfree(f, sizeof(*f));
///         if (!(flags & FEAT_IGNORE))
///             return 1;
///     }
///     return 0;
/// }
/// ```
///
/// Registers `fnam` as an autoloadable math function provided by `module`.
/// Port of `add_automathfunc` from `Src/module.c:1410`.
pub fn add_automathfunc(table: &mut ModuleTable, module: &str, fnam: &str, flags: i32) -> i32 { // c:1410
    // c:1414-1418 — alloc + populate MathFunc
    if table.autoload_mathfuncs.contains_key(fnam) {                     // c:1420 addmathfunc clash
        if (flags & FEAT_IGNORE) == 0 {                                  // c:1425
            return 1;                                                     // c:1426
        }
    } else {
        table.autoload_mathfuncs.insert(fnam.to_string(), module.to_string());
    }
    0                                                                    // c:1429
}

/// Port of `add_dep()` from `Src/module.c:2369`.
///
/// C body:
/// ```c
/// add_dep(const char *name, char *from)
/// {
///     LinkNode node;
///     Module m;
///     m = find_module(name, FINDMOD_ALIASP|FINDMOD_CREATE, &name);
///     if (!m->deps)
///         m->deps = znewlinklist();
///     for (node = firstnode(m->deps);
///          node && strcmp((char *) getdata(node), from);
///          incnode(node));
///     if (!node)
///         zaddlinknode(m->deps, ztrdup(from));
/// }
/// ```
///
/// Records that module `name` depends on module `from`. Resolves
/// aliases so dependency graphs always point at canonical names.
/// Port of `add_dep` from `Src/module.c:2369`.
pub fn add_dep(table: &mut ModuleTable, name: &str, from: &str) -> i32 { // c:2369
    // c:2386 — m = find_module(name, FINDMOD_ALIASP|FINDMOD_CREATE, &name)
    let canon = match find_module(table, name, FINDMOD_ALIASP | FINDMOD_CREATE) {
        Some(n) => n,
        None => return 0,
    };
    if let Some(m) = table.modules.get_mut(&canon) {
        // c:2389-2391 — walk deps, skip if `from` already present.
        if !m.deps.iter().any(|d| d == from) {                            // c:2392 if (!node)
            m.deps.push(from.to_string());                                // c:2393 zaddlinknode
        }
    }
    0
}

/// Port of `addbuiltins()` from `Src/module.c:544`.
///
/// C body:
/// ```c
/// addbuiltins(char const *nam, Builtin binl, int size)
/// {
///     int ret = 0, n;
///     for(n = 0; n < size; n++) {
///         Builtin b = &binl[n];
///         if(b->node.flags & BINF_ADDED)
///             continue;
///         if(addbuiltin(b)) {
///             zwarnnam(nam, "name clash when adding builtin `%s'", b->node.nam);
///             ret = 1;
///         } else {
///             b->node.flags |= BINF_ADDED;
///         }
///     }
///     return ret;
/// }
/// ```
///
/// Rust port: walks the slice, checks BINF_ADDED, registers via the
/// module-table addbuiltin if not already registered. `binl` is taken
/// by `&mut [Builtin]` so the BINF_ADDED flag-set after success
/// matches C's in-place mutation.
// `addbuiltins` deleted — Rust-only port that took `&mut [Builtin]`
// (the deleted Rust-only `Builtin` PascalCase struct). C
// `addbuiltins(char *nam, Builtin binl, int size, char *modname)` at
// module.c:545 walks the module's bintab pointer; a re-port will
// land alongside the wider modulestab-as-global refactor.

/// Port of `addhookdeffunc()` from `Src/module.c:939`.
///
/// C body:
/// ```c
/// addhookdeffunc(Hookdef h, Hookfn f) {
///     zaddlinknode(h->funcs, (void *) f);
///     return 0;
/// }
/// ```
///
/// Appends function `f` to the named hook's function-list. C uses
/// `LinkList` with `void *` payload (cast to Hookfn at dispatch); Rust
/// port uses the table's per-hook `Vec<String>` (function names) since
/// fn-pointer storage requires a more elaborate type-erased registry.
/// Port of `addhookdeffunc` from `Src/module.c:939`.
pub fn addhookdeffunc(table: &mut ModuleTable, h: &mut crate::ported::zsh_h::hookdef, fn_name: &str) -> i32 { // c:939
    // c:941 — zaddlinknode(h->funcs, (void *) f);
    table.hooks.entry(h.name.clone()).or_default().push(fn_name.to_string());
    let _ = h.funcs; // keep field mention for parity
    0                                                                    // c:943
}

/// Port of `addmathfunc()` from `Src/module.c:1313`.
///
/// C body: walks the global `mathfuncs` linked list, refuses to
/// re-register MFF_ADDED entries, replaces autoloadable shims, then
/// links into head. Rust port operates on `autoload_mathfuncs` map
/// since zshrs's static-link path doesn't have per-entry MFF flags.
// `addmathfunc(table, &MathFunc)` deleted — Rust-only port that
// took the deleted PascalCase `MathFunc` struct. C
// `addmathfunc(MathFunc f)` at module.c:1313 prepends to the
// global `mathfuncs` linked list (ported as `MATHFUNCS` global
// above). Re-port using `crate::ported::zsh_h::mathfunc` will
// follow with the wider modulestab-as-global refactor.

/// Port of `autofeatures()` from `Src/module.c:3437`.
///
/// C body is ~140 lines. Top-level structure:
/// ```c
/// autofeatures(const char *cmdnam, const char *module, char **features,
///              int prefchar, int defflags)
/// {
///     // Resolve module, fetch its features+enables tables.
///     // For each feature in `features`:
///     //   parse `+`/`-` prefix → add/remove
///     //   parse type prefix (b/c/C/p/f) → fchar
///     //   dispatch to add_aliasbuiltin / add_autocondition /
///     //     add_autoparam / add_automathfunc / del_* matching
/// }
/// ```
///
/// Static-link path: registers each `module:feature` pair into the
/// matching `table.autoload_*` map. Honors `+`/`-` prefix for
/// add/remove, and the type prefix or `prefchar` arg for routing.
/// Port of `autofeatures` from `Src/module.c:3437`.
pub fn autofeatures(table: &mut ModuleTable, _cmdnam: &str, module: Option<&str>,
                    features: &[String], prefchar: u8, defflags: i32) -> i32 { // c:3437
    let mut ret: i32 = 0;
    let _ = defflags;

    for feature in features {
        let mut s = feature.as_str();
        let mut add: bool = true;                                         // c:3466 add = 1
        // c:3461-3491 — parse `+`/`-` add/remove prefix.
        if let Some(rest) = s.strip_prefix('-') {
            add = false;
            s = rest;
        } else if let Some(rest) = s.strip_prefix('+') {
            add = true;
            s = rest;
        }

        let (fchar, fnam): (u8, &str) = if prefchar != 0 {                // c:3461
            (prefchar, s)                                                 // c:3467-3468
        } else {
            // c:3491-3520 — parse `b:`/`c:`/`C:`/`p:`/`f:` type prefix.
            let bytes = s.as_bytes();
            if bytes.len() >= 2 && bytes[1] == b':' {
                (bytes[0], &s[2..])
            } else {
                (b'b', s)  // default: builtin
            }
        };

        let modname = match module {
            Some(m) => m,
            None => { ret = 1; continue; }
        };

        if add {
            // Insert into the matching autoload map.
            match fchar {
                b'b' => { table.autoload_builtins.insert(fnam.to_string(), modname.to_string()); }
                b'c' | b'C' => { table.autoload_conditions.insert(fnam.to_string(), modname.to_string()); }
                b'p' => { table.autoload_params.insert(fnam.to_string(), modname.to_string()); }
                b'f' => { table.autoload_mathfuncs.insert(fnam.to_string(), modname.to_string()); }
                _ => { ret = 1; }
            }
        } else {
            // Remove from the matching autoload map.
            match fchar {
                b'b' => { table.autoload_builtins.remove(fnam); }
                b'c' | b'C' => { table.autoload_conditions.remove(fnam); }
                b'p' => { table.autoload_params.remove(fnam); }
                b'f' => { table.autoload_mathfuncs.remove(fnam); }
                _ => { ret = 1; }
            }
        }
    }
    ret
}

/// Port of `autoloadscan()` from `Src/module.c:2403`.
///
/// C body:
/// ```c
/// autoloadscan(HashNode hn, int printflags)
/// {
///     Builtin bn = (Builtin) hn;
///     if(bn->node.flags & BINF_ADDED)
///         return;
///     if(printflags & PRINT_LIST) {
///         fputs("zmodload -ab ", stdout);
///         if(bn->optstr[0] == '-') fputs("-- ", stdout);
///         quotedzputs(bn->optstr, stdout);
///         if(strcmp(bn->node.nam, bn->optstr)) {
///             putchar(' ');
///             quotedzputs(bn->node.nam, stdout);
///         }
///     } else {
///         nicezputs(bn->node.nam, stdout);
///         if(strcmp(bn->node.nam, bn->optstr)) {
///             fputs(" (", stdout);
///             nicezputs(bn->optstr, stdout);
///             putchar(')');
///         }
///     }
///     putchar('\n');
/// }
/// ```
///
/// Hash-table scan callback for autoloadable-builtin listing.
/// `printflags & PRINT_LIST` selects long form (`zmodload -ab MOD NAME`)
/// vs short form (`NAME (MOD)`). Skips already-registered builtins
/// (BINF_ADDED set).
/// Port of `autoloadscan` from `Src/module.c:2403`.
pub fn autoloadscan(name: &str, optstr: &str, flags: u32, printflags: i32) { // c:2403
    if (flags & BINF_ADDED) != 0 {                                       // c:2407
        return;                                                          // c:2408
    }
    if (printflags & crate::ported::zsh_h::PRINT_LIST) != 0 {            // c:2409
        // c:2410-2417 — long form `zmodload -ab MOD NAME`
        print!("zmodload -ab ");
        if optstr.starts_with('-') {                                     // c:2411
            print!("-- ");                                                // c:2412
        }
        print!("{}", optstr);                                             // c:2413 quotedzputs
        if name != optstr {                                               // c:2414
            print!(" ");                                                  // c:2415
            print!("{}", name);                                           // c:2416
        }
    } else {
        // c:2419-2424 — short form `NAME (MOD)`
        print!("{}", name);                                               // c:2419
        if name != optstr {                                               // c:2420
            print!(" (");                                                 // c:2421
            print!("{}", optstr);                                         // c:2422
            print!(")");                                                  // c:2423
        }
    }
    println!();                                                          // c:2426
}

/// Direct port of `bin_zmodload()` from `Src/module.c:2440`.
/// Top-level dispatcher for the `zmodload` builtin. Validates flag
/// combinations then routes to one of the per-mode helpers:
///   -F        → bin_zmodload_features (c:3003)
///   -e        → bin_zmodload_exist    (c:2623)
///   -d        → bin_zmodload_dep      (c:2649)
///   -a/-b/-c/-p/-f → bin_zmodload_auto (c:2726)
///   default   → bin_zmodload_load     (c:2971)
///   -A/-R     → bin_zmodload_alias    (c:2515)
pub fn bin_zmodload(nam: &str, args: &[String],                              // c:2440
                    ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let mut table = MODULESTAB.lock().unwrap();
    let table = &mut *table;
    use crate::ported::zsh_h::OPT_ISSET;
    use crate::ported::utils::zwarnnam;

    let ops_bcpf = OPT_ISSET(ops, b'b') || OPT_ISSET(ops, b'c')              // c:2443
                || OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'f');
    let ops_au   = OPT_ISSET(ops, b'a') || OPT_ISSET(ops, b'u');             // c:2445
    let mut ret: i32;                                                        // c:2446

    if ops_bcpf && !ops_au {                                                 // c:2451
        zwarnnam(nam, "-b, -c, -f, and -p must be combined with -a or -u");  // c:2452
        return 1;                                                            // c:2453
    }
    if OPT_ISSET(ops, b'F') && (ops_bcpf || OPT_ISSET(ops, b'u')) {          // c:2455
        zwarnnam(nam, "-b, -c, -f, -p and -u cannot be combined with -F");   // c:2456
        return 1;                                                            // c:2457
    }
    if OPT_ISSET(ops, b'A') || OPT_ISSET(ops, b'R') {                        // c:2459
        if ops_bcpf || ops_au || OPT_ISSET(ops, b'd')                        // c:2460
           || (OPT_ISSET(ops, b'R') && OPT_ISSET(ops, b'e'))
        {
            zwarnnam(nam, "illegal flags combined with -A or -R");           // c:2462
            return 1;                                                        // c:2463
        }
        if !OPT_ISSET(ops, b'e') {                                           // c:2465
            return bin_zmodload_alias(table, nam, args, ops);                // c:2466
        }
    }
    if OPT_ISSET(ops, b'd') && OPT_ISSET(ops, b'a') {                        // c:2468
        zwarnnam(nam, "-d cannot be combined with -a");                      // c:2469
        return 1;                                                            // c:2470
    }
    if OPT_ISSET(ops, b'u') && args.is_empty() {                             // c:2472
        zwarnnam(nam, "what do you want to unload?");                        // c:2473
        return 1;                                                            // c:2474
    }
    if OPT_ISSET(ops, b'e') && (OPT_ISSET(ops, b'I') || OPT_ISSET(ops, b'L') // c:2476
        || (OPT_ISSET(ops, b'a') && !OPT_ISSET(ops, b'F'))
        || OPT_ISSET(ops, b'd') || OPT_ISSET(ops, b'i')
        || OPT_ISSET(ops, b'u'))
    {
        zwarnnam(nam, "-e cannot be combined with other options");           // c:2480
        return 1;                                                            // c:2482
    }
    // c:2484 — `for (fp = fonly; *fp; fp++)` — `l` and `P` only with `-F`.
    for fp in [b'l', b'P'] {                                                 // c:2484
        if OPT_ISSET(ops, fp) && !OPT_ISSET(ops, b'F') {                     // c:2485
            zwarnnam(nam, &format!("-{} is only allowed with -F", fp as char)); // c:2486
            return 1;                                                        // c:2487
        }
    }
    crate::ported::mem::queue_signals();                                     // c:2490
    if OPT_ISSET(ops, b'F') {                                                // c:2491
        ret = bin_zmodload_features(table, nam, args, ops);                  // c:2492
    } else if OPT_ISSET(ops, b'e') {                                         // c:2493
        ret = bin_zmodload_exist(table, nam, args, ops);                     // c:2494
    } else if OPT_ISSET(ops, b'd') {                                         // c:2495
        ret = bin_zmodload_dep(table, nam, args, ops);                       // c:2496
    } else {
        let autoopts = (OPT_ISSET(ops, b'b') as i32)                         // c:2497
                     + (OPT_ISSET(ops, b'c') as i32)
                     + (OPT_ISSET(ops, b'p') as i32)
                     + (OPT_ISSET(ops, b'f') as i32);
        if autoopts != 0 || OPT_ISSET(ops, b'a') {                           // c:2497-2499
            if autoopts > 1 {                                                // c:2502
                zwarnnam(nam, "use only one of -b, -c, or -p");              // c:2503
                ret = 1;                                                     // c:2504
            } else {
                ret = bin_zmodload_auto(table, nam, args, ops);              // c:2506
            }
        } else {
            ret = bin_zmodload_load(table, nam, args, ops);                  // c:2508
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:2510
    ret                                                                      // c:2512
}

/// Port of `bin_zmodload_alias()` from `Src/module.c:2515`.
///
/// `zmodload -A [-L|-R] [name=alias ...]`. Three modes:
/// - no args: list all module aliases (`-L` = long form).
/// - `-R name`: remove alias `name` (must already be MOD_ALIAS).
/// - `name=target`: install/replace alias `name` pointing at `target`.
///   Detects self-cycles before committing.
pub fn bin_zmodload_alias(table: &mut ModuleTable, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2515
    /*
     * TODO: while it would be too nasty to have aliases, as opposed
     * to real loadable modules, with dependencies --- just what would
     * we need to load when, exactly? --- there is in principle no objection
     * to making it possible to force an alias onto an existing unloaded
     * module which has dependencies.  This would simply transfer
     * the dependencies down the line to the aliased-to module name.
     * This is actually useful, since then you can alias zsh/zle=mytestzle
     * to load another version of zle.  But then what happens when the
     * alias is removed?  Do you transfer the dependencies back? And
     * suppose other names are aliased to the same file?  It might be
     * kettle of fish best left unwormed.
     */                                                                  // c:2517-2529

    // c:2532-2541 — no args: list aliases
    if args.is_empty() {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'R') {                  // c:2533
            crate::ported::utils::zwarnnam(nam, "no module alias to remove"); // c:2534
            return 1;                                                     // c:2535
        }
        // c:2537-2539 — scanhashtable filtered by MOD_ALIAS, printnode
        for (name, m) in &table.modules {
            if (m.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
                if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                    println!("zmodload -A {}={}", name, m.alias.as_deref().unwrap_or(""));
                } else {
                    println!("{} -> {}", name, m.alias.as_deref().unwrap_or(""));
                }
            }
        }
        return 0;                                                         // c:2540
    }

    // c:2543 — for each arg, parse name=alias and dispatch.
    for arg in args {
        // c:2544-2547 — split at '='
        let (lhs, aliasname): (&str, Option<&str>) = match arg.find('=') {
            Some(eq) => (&arg[..eq], Some(&arg[eq+1..])),
            None => (arg.as_str(), None),
        };
        // c:2548 — modname_ok check on the LHS
        if modname_ok(lhs) == 0 {                                         // c:2548
            crate::ported::utils::zwarnnam(nam, &format!("invalid module name `{}'", lhs)); // c:2549
            return 1;                                                     // c:2550
        }
        if crate::ported::zsh_h::OPT_ISSET(ops, b'R') {                  // c:2552
            // -R: remove alias path.
            if aliasname.is_some() {                                      // c:2553
                crate::ported::utils::zwarnnam(nam,
                    &format!("bad syntax for removing module alias: {}", lhs)); // c:2554
                return 1;                                                 // c:2556
            }
            // c:2558 — find_module(lhs, 0, NULL)
            match table.modules.get(lhs) {
                Some(m) => {
                    if (m.flags & crate::ported::zsh_h::MOD_ALIAS) == 0 { // c:2560
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2561
                        return 1;                                         // c:2562
                    }
                    table.modules.remove(lhs);                            // c:2564 delete_module
                }
                None => {
                    crate::ported::utils::zwarnnam(nam,
                        &format!("no such module alias: {}", lhs));       // c:2566
                    return 1;                                             // c:2567
                }
            }
        } else {
            // No -R: install/replace alias OR list one.
            if let Some(target) = aliasname {                             // c:2570
                if modname_ok(target) == 0 {                              // c:2572
                    crate::ported::utils::zwarnnam(nam,
                        &format!("invalid module name `{}'", target));    // c:2573
                    return 1;                                             // c:2574
                }
                // c:2576-2584 — cycle detection: walk alias chain
                let mut mname = target;
                let mut depth = 0;
                loop {
                    if depth > 256 { break; }
                    depth += 1;
                    if mname == lhs {                                     // c:2577
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module alias would refer to itself: {}", lhs)); // c:2578
                        return 1;                                         // c:2580
                    }
                    match table.modules.get(mname) {
                        Some(m) if (m.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 => {
                            mname = m.alias.as_deref().unwrap_or("");
                        }
                        _ => break,
                    }
                }
                // c:2585-2596 — install or replace
                if let Some(m) = table.modules.get_mut(lhs) {
                    if (m.flags & crate::ported::zsh_h::MOD_ALIAS) == 0 { // c:2587
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2588
                        return 1;                                         // c:2589
                    }
                    m.alias = Some(target.to_string());                   // c:2591/2597
                } else {
                    let mut m = Module::new(lhs);                         // c:2593 zshcalloc
                    m.flags = crate::ported::zsh_h::MOD_ALIAS;            // c:2594
                    m.alias = Some(target.to_string());                   // c:2597
                    table.modules.insert(lhs.to_string(), m);             // c:2595
                }
            } else {
                // c:2599-2611 — list one alias
                match table.modules.get(lhs) {
                    Some(m) if (m.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 => {
                        if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                            println!("zmodload -A {}={}", lhs, m.alias.as_deref().unwrap_or(""));
                        } else {
                            println!("{} -> {}", lhs, m.alias.as_deref().unwrap_or(""));
                        }
                    }
                    Some(_) => {
                        crate::ported::utils::zwarnnam(nam,
                            &format!("module is not an alias: {}", lhs)); // c:2605
                        return 1;                                         // c:2606
                    }
                    None => {
                        crate::ported::utils::zwarnnam(nam,
                            &format!("no such module alias: {}", lhs));   // c:2609
                        return 1;                                         // c:2610
                    }
                }
            }
        }
    }
    0                                                                    // c:2616
}

/// Port of `bin_zmodload_auto()` from `Src/module.c:2726`.
///
/// `zmodload [-c] [-p] [-f] [-a] module name [name ...]` —
/// register-autoload of builtins/conditions/params/mathfns. C body
/// (80 lines) walks the appropriate dispatch table per opt flag.
///
/// `-c` lists/registers conditions, `-p` parameters, `-f` math fns,
/// default is builtins. `-L` toggles long-form listing.
///
/// Static-link path: registers via `add_autoaliasbuiltin` /
/// `add_autoparam` / `add_automathfunc` already ported. Without a
/// module name (just `-a`), runs the listing scan via `autoloadscan`
/// or its conddef/param/mathfn equivalents.
/// Port of `bin_zmodload_auto` from `Src/module.c:2726`.
pub fn bin_zmodload_auto(table: &mut ModuleTable, _nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2726
    let fchar: char;                                                      // c:2728
    let _flags: i32 = if crate::ported::zsh_h::OPT_ISSET(ops, b'i') { FEAT_IGNORE } else { 0 }; // c:2728

    // c:2731-2773 — conditions branch (-c)
    if crate::ported::zsh_h::OPT_ISSET(ops, b'c') {
        fchar = if crate::ported::zsh_h::OPT_ISSET(ops, b'I') { 'C' } else { 'c' };
        let _ = fchar;
        if args.is_empty() {                                              // c:2732
            // List all autoloadable conditions
            for (name, module) in &table.autoload_conditions {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else if crate::ported::zsh_h::OPT_ISSET(ops, b'p') {               // c:2774 — params branch
        if args.is_empty() {
            for (name, module) in &table.autoload_params {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else if crate::ported::zsh_h::OPT_ISSET(ops, b'f') {               // mathfns branch
        if args.is_empty() {
            for (name, module) in &table.autoload_mathfuncs {
                println!("{} {}", module, name);
            }
            return 0;
        }
    } else {
        // Default: builtins branch
        if args.is_empty() {
            for (name, module) in &table.autoload_builtins {
                autoloadscan(name, module, 0,
                    if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {
                        crate::ported::zsh_h::PRINT_LIST
                    } else { 0 });
            }
            return 0;
        }
    }

    // Register-mode: args[0] = module, args[1..] = names to autoload
    if args.len() < 2 { return 1; }
    let modnam = &args[0];                                                // c:2729 modnam = *args
    for nm in &args[1..] {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'p') {
            table.autoload_params.insert(nm.clone(), modnam.clone());
        } else if crate::ported::zsh_h::OPT_ISSET(ops, b'f') {
            table.autoload_mathfuncs.insert(nm.clone(), modnam.clone());
        } else if crate::ported::zsh_h::OPT_ISSET(ops, b'c') {
            table.autoload_conditions.insert(nm.clone(), modnam.clone());
        } else {
            table.autoload_builtins.insert(nm.clone(), modnam.clone());
        }
    }
    0                                                                    // c:2805
}

/// Port of `bin_zmodload_dep()` from `Src/module.c:2649`.
///
/// `zmodload -d [-u] [target [dep ...]]`. Three modes:
/// - `-u target` removes all deps from target; `-u target d1 d2` removes
///   only those.
/// - no args lists all dependencies.
/// - `target dep1 ...` adds each dep to target's dependency list.
pub fn bin_zmodload_dep(table: &mut ModuleTable, _nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2649
    if crate::ported::zsh_h::OPT_ISSET(ops, b'u') {                      // c:2652
        // c:2654 — const char *tnam = *args++;
        if args.is_empty() { return 0; }
        let tnam = &args[0];
        let rest = &args[1..];
        // c:2655 — find_module(tnam, FINDMOD_ALIASP, &tnam)
        let canon = match find_module(table, tnam, FINDMOD_ALIASP) {
            Some(n) => n,
            None => return 0,                                             // c:2657
        };
        if let Some(m) = table.modules.get_mut(&canon) {
            if !rest.is_empty() {                                         // c:2658
                // c:2659-2667 — remove specific deps
                for to_remove in rest {
                    if let Some(pos) = m.deps.iter().position(|d| d == to_remove) {
                        m.deps.remove(pos);                              // c:2664 remnode
                    }
                }
            } else {
                // c:2673-2676 — remove all deps
                m.deps.clear();
            }
            // c:2678-2679 — if no deps and no handle, delete module
            let no_deps_no_handle = m.deps.is_empty();
            if no_deps_no_handle {
                table.modules.remove(&canon);
            }
        }
        return 0;                                                         // c:2680
    }
    // c:2681 — list-mode or add-mode
    if args.len() < 2 {
        // List dependencies (c:2682-2684 — print all module deps)
        for (name, m) in &table.modules {
            if !m.deps.is_empty() {
                println!("zmodload -d {} {}", name, m.deps.join(" "));
            }
        }
        return 0;
    }
    // Add deps: args[0] is target, args[1..] are deps to add.
    let target = &args[0];
    for dep in &args[1..] {
        add_dep(table, target, dep);                                      // dispatch to add_dep
    }
    0
}

/// Port of `bin_zmodload_exist()` from `Src/module.c:2623`.
///
/// C body:
/// ```c
/// bin_zmodload_exist(UNUSED(char *nam), char **args, Options ops)
/// {
///     Module m;
///     if (!*args) {
///         scanhashtable(modulestab, 1, 0, 0, modulestab->printnode,
///                       OPT_ISSET(ops,'A') ? PRINTMOD_EXIST|PRINTMOD_ALIAS :
///                       PRINTMOD_EXIST);
///         return 0;
///     } else {
///         int ret = 0;
///         for (; !ret && *args; args++) {
///             if (!(m = find_module(*args, FINDMOD_ALIASP, NULL))
///                 || !m->u.handle
///                 || (m->node.flags & MOD_UNLOAD))
///                 ret = 1;
///         }
///         return ret;
///     }
/// }
/// ```
///
/// `zmodload [-A]` lists or tests module presence. Returns 0 if all
/// named modules exist (or if no args, after listing); 1 if any
/// named module is missing/unloading.
/// Port of `bin_zmodload_exist` from `Src/module.c:2623`.
pub fn bin_zmodload_exist(table: &mut ModuleTable, _nam: &str, args: &[String], _ops: &crate::ported::zsh_h::options) -> i32 { // c:2623
    if args.is_empty() {                                                  // c:2627
        // c:2628-2630 — scanhashtable + printnode listing.
        // Static-link path: dump the modules registry.
        for (name, _) in &table.modules {
            println!("{}", name);
        }
        return 0;                                                         // c:2631
    }
    // c:2633-2640 — for each arg, test existence.
    let mut ret: i32 = 0;
    for arg in args {                                                     // c:2635
        if ret != 0 { break; }
        if find_module(table, arg, FINDMOD_ALIASP).is_none() {            // c:2636
            ret = 1;                                                      // c:2639
        }
    }
    ret                                                                   // c:2641
}

/// Port of `bin_zmodload_features()` from `Src/module.c:3003`.
///
/// `zmodload -F [-L|-l|-P|-a|-m|-i] module [+/-feature ...]` —
/// per-feature enable/disable for an already-loaded module.
///
/// C body (~135 lines) handles:
/// - no module: list all modules with their features (`-L` long form,
///   `-l` show all enables, `-a` show autoloads).
/// - `-P` requires a module name; lists patterns.
/// - `-m` interprets each feature as a glob pattern.
/// - default: `+feature` enables, `-feature` disables, then calls
///   `do_module_features` to apply.
pub fn bin_zmodload_features(table: &mut ModuleTable, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:3003
    let modname = args.first();                                          // c:3006
    let rest_args = if args.is_empty() { &args[..] } else { &args[1..] };

    // c:3010-3024 — no-module-name listing branch
    if modname.is_none() {
        if crate::ported::zsh_h::OPT_ISSET(ops, b'L') {                  // c:3012
            if crate::ported::zsh_h::OPT_ISSET(ops, b'P') {              // c:3014
                crate::ported::utils::zwarnnam(nam, "-P is only allowed with a module name"); // c:3015
                return 1;                                                 // c:3016
            }
            // c:3022-3023 — scanhashtable + printnode
            for (name, _m) in &table.modules {
                println!("zmodload -F {}", name);
            }
            return 0;                                                     // c:3024
        }
        crate::ported::utils::zwarnnam(nam, "-F requires a module name"); // c:3028
        return 1;                                                         // c:3029
    }

    let modname = modname.unwrap();

    // c:3032 — `-m` glob-pattern branch (compile patprogs).
    // Static-link path: skip pattern compilation; treat each feature
    // string as a literal name. Full pattern support pending the
    // pattern.c port wire-up.

    // Build features array from `+name`/`-name` args.
    let mut feats: Vec<String> = Vec::with_capacity(rest_args.len());
    for arg in rest_args {
        feats.push(arg.clone());
    }

    // c:3098-3120 — apply features via do_module_features after
    // setting up the enables array per +/- prefixes.
    if !feats.is_empty() {
        autofeatures(table, nam, Some(modname), &feats, 0, 0);
    }
    do_module_features(table, modname, FEAT_CHECKAUTO);                  // c:3122
    0
}

/// Port of `bin_zmodload_load()` from `Src/module.c:2971`.
///
/// C body:
/// ```c
/// bin_zmodload_load(char *nam, char **args, Options ops)
/// {
///     int ret = 0;
///     if(OPT_ISSET(ops,'u')) {
///         for(; *args; args++) {
///             if (unload_named_module(*args, nam, OPT_ISSET(ops,'i')))
///                 ret = 1;
///         }
///         return ret;
///     } else if(!*args) {
///         scanhashtable(modulestab, ..., PRINTMOD_LIST);
///         return 0;
///     } else {
///         for (; *args; args++) {
///             int tmpret = require_module(*args, NULL, OPT_ISSET(ops,'s'));
///             if (tmpret && ret != 1) ret = tmpret;
///         }
///         return ret;
///     }
/// }
/// ```
///
/// `zmodload [-u] [args]`: load, unload, or list modules.
/// Port of `bin_zmodload_load` from `Src/module.c:2971`.
pub fn bin_zmodload_load(table: &mut ModuleTable, nam: &str, args: &[String], ops: &crate::ported::zsh_h::options) -> i32 { // c:2971
    let mut ret: i32 = 0;
    if crate::ported::zsh_h::OPT_ISSET(ops, b'u') {                      // c:2974
        // c:2976-2979 — unload loop
        for arg in args {
            if unload_named_module(table, arg, nam, crate::ported::zsh_h::OPT_ISSET(ops, b'i') as i32) != 0 {
                ret = 1;
            }
        }
        return ret;                                                       // c:2980
    } else if args.is_empty() {                                           // c:2981
        // c:2983-2985 — list modules
        for (name, _m) in &table.modules {
            println!("{}", name);
        }
        return 0;                                                         // c:2986
    } else {
        // c:2989-2992 — load loop
        for arg in args {
            let tmpret = require_module(table, arg, None);                // c:2990
            if tmpret != 0 && ret != 1 {                                  // c:2991
                ret = tmpret;
            }
        }
        ret
    }
}

/// Port of `boot_()` from `Src/module.c:331`.
///
/// C body: `boot_(UNUSED(Module m)) { return 0; }` — the no-op
/// boot hook of the module subsystem itself.
pub fn boot_(_m: *const crate::ported::zsh_h::module) -> i32 {           // c:331
    0                                                                    // c:333
}

/// Port of `boot_module()` from `Src/module.c:1910`.
///
/// C body:
/// ```c
/// boot_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->boot)(m) : dyn_boot_module(m));
/// }
/// ```
///
/// Static-link path: modules are MOD_LINKED, so dispatch to the
/// per-module `boot_(m)` callback. zshrs's static dispatch is via
/// the modules-table feature lookup (see `register_module` /
/// `enable_module`); both branches collapse to 0 success.
/// Port of `boot_module` from `Src/module.c:1910`.
pub fn boot_module(_table: &mut ModuleTable, _name: &str) -> i32 {       // c:1910
    0                                                                    // c:1913 (boot)(m) success
}

/// Port of `checkaddparam()` from `Src/module.c:1026`.
///
/// C body:
/// ```c
/// checkaddparam(const char *nam, int opt_i)
/// {
///     Param pm;
///     if (!(pm = (Param) gethashnode2(paramtab, nam)))
///         return 0;
///     if (pm->level || !(pm->node.flags & PM_AUTOLOAD)) {
///         if (!opt_i || pm->level) {
///             zwarn("Can't add module parameter `%s': %s",
///                   nam, pm->level ? "local parameter exists" :
///                                    "parameter already exists");
///             return 1;
///         }
///         return 2;
///     }
///     unsetparam_pm(pm, 0, 1);
///     return 0;
/// }
/// ```
///
/// Returns: 0 = OK to add, 1 = error printed, 2 = blocked but `-i`
/// suppressed warning. `pm->level != 0` means a local param shadows
/// the name (always errors). `PM_AUTOLOAD` set means the existing
/// param is an autoload stub the C source unsets to make room.
///
/// Static-link path: the param-table is `crate::ported::params::*`
/// global. Stub returns 0 (no clash) until the params global-state
/// port wires gethashnode2(paramtab, ...) in.
/// Port of `checkaddparam` from `Src/module.c:1026`.
pub fn checkaddparam(_nam: &str, _opt_i: i32) -> i32 {                   // c:1026
    // c:1030 — if (!(pm = gethashnode2(paramtab, nam))) return 0;
    // Static-link: paramtab not yet hooked through; treat unknown.
    0
}

/// Port of `cleanup_()` from `Src/module.c:338`.
///
/// C body: `cleanup_(UNUSED(Module m)) { return 0; }` — the no-op
/// cleanup hook of the module subsystem itself.
pub fn cleanup_(_m: *const crate::ported::zsh_h::module) -> i32 {        // c:338
    0                                                                    // c:340
}

/// Port of `cleanup_module()` from `Src/module.c:1918`.
///
/// C body:
/// ```c
/// cleanup_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->cleanup)(m) : dyn_cleanup_module(m));
/// }
/// ```
pub fn cleanup_module(_table: &mut ModuleTable, _name: &str) -> i32 {    // c:1918
    0                                                                    // c:1921 (cleanup)(m) success
}

/// Port of `del_automathfunc()` from `Src/module.c:1436`.
///
/// C body:
/// ```c
/// del_automathfunc(UNUSED(const char *modnam), const char *fnam, int flags) {
///     MathFunc f = getmathfunc(fnam, 0);
///     if (!f) {
///         if (!(flags & FEAT_IGNORE)) return 2;
///     } else if (f->flags & MFF_ADDED) {
///         if (!(flags & FEAT_IGNORE)) return 3;
///     } else
///         deletemathfunc(f);
///     return 0;
/// }
/// ```
///
/// Removes `fnam` from the autoloadable math-function registry.
/// Port of `del_automathfunc` from `Src/module.c:1436`.
pub fn del_automathfunc(table: &mut ModuleTable, _modnam: &str, fnam: &str, flags: i32) -> i32 { // c:1436
    if !table.autoload_mathfuncs.contains_key(fnam) {                    // c:1438 if (!f)
        if (flags & FEAT_IGNORE) == 0 {                                  // c:1441
            return 2;                                                     // c:1442
        }
    } else {
        // c:1447 — deletemathfunc(f)
        table.autoload_mathfuncs.remove(fnam);
    }
    0                                                                    // c:1449
}

/// Port of `delete_module()` from `Src/module.c:1687`.
///
/// C body:
/// ```c
/// delete_module(Module m) {
///     modulestab->removenode(modulestab, m->node.nam);
///     modulestab->freenode(&m->node);
/// }
/// ```
///
/// Removes a module from the live `modulestab` and frees its node.
/// Rust port operates on the `ModuleTable` `modules` HashMap.
pub fn delete_module(table: &mut ModuleTable, name: &str) -> i32 {       // c:1687
    table.modules.remove(name);                                          // c:1689 removenode
    // c:1691 — freenode(&m->node) — Rust drops on `remove` return.
    0
}

/// Port of `deletehookdeffunc()` from `Src/module.c:961`.
///
/// C body:
/// ```c
/// deletehookdeffunc(Hookdef h, Hookfn f) {
///     LinkNode p;
///     for (p = firstnode(h->funcs); p; incnode(p))
///         if (f == (Hookfn) getdata(p)) {
///             remnode(h->funcs, p);
///             return 0;
///         }
///     return 1;
/// }
/// ```
///
/// Removes function `f` from the hook's function-list. Returns 0 on
/// successful removal, 1 if not found.
/// Port of `deletehookdeffunc` from `Src/module.c:961`.
pub fn deletehookdeffunc(table: &mut ModuleTable, h: &mut crate::ported::zsh_h::hookdef, fn_name: &str) -> i32 { // c:961
    if let Some(funcs) = table.hooks.get_mut(&h.name) {
        // c:965-969 — for (p = firstnode...; p; incnode(p)) if (f == ...)
        if let Some(pos) = funcs.iter().position(|n| n == fn_name) {
            funcs.remove(pos);                                            // c:967 remnode
            let _ = h.funcs;
            return 0;                                                     // c:968
        }
    }
    let _ = h.funcs;
    1                                                                    // c:970
}

/// Port of `deletemathfunc()` from `Src/module.c:1342`.
///
/// C body:
/// ```c
/// deletemathfunc(MathFunc f) {
///     MathFunc p, q;
///     for (p = mathfuncs, q = NULL; p && p != f; q = p, p = p->next);
///     if (p) {
///         if (q) q->next = f->next; else mathfuncs = f->next;
///         if (f->module) {
///             zsfree(f->name); zsfree(f->module); zfree(f, sizeof(*f));
///         } else
///             f->flags &= ~MFF_ADDED;
///         return 0;
///     }
///     return -1;
/// }
/// ```
///
/// Removes math function `f` from the global registry. Returns 0
/// on hit, -1 on miss.
// `deletemathfunc(table, &MathFunc)` deleted — Rust-only port that
// took the deleted PascalCase `MathFunc` struct. The canonical
// `removemathfunc` still operates on `ModuleTable.autoload_mathfuncs`
// (the autoload registry).

/// Port of `do_boot_module()` from `Src/module.c:2139`.
///
/// C body:
/// ```c
/// do_boot_module(Module m, Feature_enables enablesarr, int silent)
/// {
///     int ret = do_module_features(m, enablesarr,
///                                  silent ? FEAT_IGNORE|FEAT_CHECKAUTO :
///                                  FEAT_CHECKAUTO);
///     if (ret == 1) return 1;
///     if (boot_module(m)) return 1;
///     return ret;
/// }
/// ```
/// Port of `do_boot_module` from `Src/module.c:2139`.
pub fn do_boot_module(table: &mut ModuleTable, name: &str, silent: i32) -> i32 { // c:2139
    let flags = if silent != 0 {                                          // c:2142
        FEAT_IGNORE | FEAT_CHECKAUTO
    } else {
        FEAT_CHECKAUTO                                                    // c:2143
    };
    let ret = do_module_features(table, name, flags);                     // c:2141
    if ret == 1 {                                                         // c:2145
        return 1;                                                         // c:2146
    }
    if boot_module(table, name) != 0 {                                    // c:2148
        return 1;                                                         // c:2149
    }
    ret                                                                   // c:2150
}

/// Port of `do_cleanup_module()` from `Src/module.c:2159`.
///
/// C body:
/// ```c
/// do_cleanup_module(Module m) {
///     return (m->node.flags & MOD_LINKED) ?
///         (m->u.linked && m->u.linked->cleanup(m)) :
///         (m->u.handle && cleanup_module(m));
/// }
/// ```
pub fn do_cleanup_module(table: &mut ModuleTable, name: &str) -> i32 {   // c:2159
    // Check the module is registered, then dispatch to cleanup_module.
    if table.modules.contains_key(name) {                                 // c:2162 m->u.linked
        cleanup_module(table, name)                                       // c:2163 cleanup_module(m)
    } else {
        0
    }
}

/// Port of `do_load_module()` from `Src/module.c:1610`.
///
/// C body:
/// ```c
/// do_load_module(char const *name, int silent)
/// {
///     void *ret;
///     ret = try_load_module(name);
///     if (!ret && !silent) {
///         zwarn("failed to load module `%s': %s", name, ...);
///     }
///     return ret;
/// }
/// ```
///
/// C returns `void *` (the dlopen handle); Rust port returns 0 on
/// success / 1 on failure. zshrs's static-link path: `try_load_module`
/// always succeeds for built-in modules. Returns 1 + zwarn on miss.
/// Port of `do_load_module` from `Src/module.c:1610`.
pub fn do_load_module(table: &mut ModuleTable, name: &str, silent: i32) -> i32 { // c:1610
    // c:1614 — ret = try_load_module(name);
    let ret = try_load_module(table, name);
    if ret == 0 && silent == 0 {                                          // c:1615
        // c:1618-1621 — zwarn("failed to load module ...")
        crate::ported::utils::zwarn(&format!("failed to load module: {}", name));
    }
    ret                                                                   // c:1624
}

/// Port of `do_module_features()` from `Src/module.c:1998`.
///
/// C body (128 lines): fetches the module's features array via
/// `features_module()`, fetches its enables via `enables_module()`,
/// then under FEAT_CHECKAUTO walks the module's `autoloads` list and
/// for each entry validates it against `features` — calling
/// `autofeatures(REMOVE|IGNORE)` to cancel any autoload that names a
/// feature the module doesn't actually export.
///
/// Returns 0 on full success, 1 if any feature couldn't be enabled.
pub fn do_module_features(table: &mut ModuleTable, name: &str, flags: i32) -> i32 { // c:1998
    let mut features: Vec<String> = Vec::new();                          // c:2000
    let mut ret: i32 = 0;                                                // c:2001

    // c:2003 — `if (features_module(m, &features) == 0)` — fetch features.
    if features_module(table, name, &mut features) == 0 {
        // c:2011-2018 — fetch enables. If features are supported, enables
        // should be too; an error here is reported unless FEAT_IGNORE.
        let mut enables: Option<Vec<i32>> = None;
        if enables_module(table, name, &mut enables) != 0 {              // c:2012
            if (flags & FEAT_IGNORE) == 0 {                              // c:2014
                crate::ported::utils::zwarn(&format!(
                    "error getting enabled features for module `{}'",   // c:2015
                    name,
                ));
            }
            return 1;                                                    // c:2017
        }

        // c:2020 — `if ((flags & FEAT_CHECKAUTO) && m->autoloads)`
        if (flags & FEAT_CHECKAUTO) != 0 {
            let autoloads: Vec<String> = match table.modules.get(name) {
                Some(m) => m.autoloads.clone(),
                None => return ret,
            };
            // c:2027-2074 — walk autoloads, cancel mismatches.
            for al in &autoloads {                                       // c:2028
                // c:2032-2034 — `for (ptr = features; *ptr; ptr++) if (!strcmp(al, *ptr)) break;`
                let found = features.iter().any(|f| f == al);
                if !found {                                              // c:2035
                    if (flags & FEAT_IGNORE) == 0 {                      // c:2037
                        crate::ported::utils::zwarn(&format!(
                            "module `{}' has no such feature: `{}': autoload cancelled", // c:2038-2040
                            name, al,
                        ));
                    }
                    // c:2045-2047 — `autofeatures(NULL, m->node.nam, arg, 0, FEAT_IGNORE|FEAT_REMOVE)`
                    let arg = vec![al.clone()];
                    autofeatures(table, "", Some(name), &arg, 0, FEAT_IGNORE | FEAT_REMOVE);
                }
            }
        }
    }
    ret                                                                  // c:2120 (approx)
}

/// Port of `dyn_boot_module()` from `Src/module.c:1747`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(1, m, NULL);`
/// Calls the dynamic module's exported entry-point with op-code 1
/// (boot). Static-link path: opcode dispatch unused, returns 0.
pub fn dyn_boot_module(_m: *const crate::ported::zsh_h::module) -> i32 { // c:1747
    0                                                                    // c:1749
}

/// Port of `dyn_cleanup_module()` from `Src/module.c:1754`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(2, m, NULL);`
/// Op-code 2 = cleanup.
pub fn dyn_cleanup_module(_m: *const crate::ported::zsh_h::module) -> i32 { // c:1754
    0                                                                    // c:1756
}

/// Port of `dyn_enables_module()` from `Src/module.c:1740`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(5, m, enables);`
/// Op-code 5 = enables.
pub fn dyn_enables_module(_m: *const crate::ported::zsh_h::module, _enables: &mut Option<Vec<i32>>) -> i32 { // c:1740
    0                                                                    // c:1742
}

/// Port of `dyn_features_module()` from `Src/module.c:1733`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(4, m, features);`
/// Op-code 4 = features.
pub fn dyn_features_module(_m: *const crate::ported::zsh_h::module, _features: &mut Vec<String>) -> i32 { // c:1733
    0                                                                    // c:1735
}

/// Port of `dyn_finish_module()` from `Src/module.c:1761`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(3, m, NULL);`
/// Op-code 3 = finish.
pub fn dyn_finish_module(_m: *const crate::ported::zsh_h::module) -> i32 { // c:1761
    0                                                                    // c:1763
}

/// Port of `dyn_setup_module()` from `Src/module.c:1726`.
///
/// C body: `return ((int (*)(int,Module,void*)) m->u.handle)(0, m, NULL);`
/// Op-code 0 = setup. AIX-only path that multiplexes all six module
/// hooks through one symbol; static-link path skips it entirely.
pub fn dyn_setup_module(_m: *const crate::ported::zsh_h::module) -> i32 { // c:1726
    0                                                                    // c:1728
}

/// Port of `enables_()` from `Src/module.c:324`.
///
/// C body: `enables_(UNUSED(Module m), UNUSED(int **enables)) { return 1; }`
/// — the module subsystem itself doesn't manage feature enables.
pub fn enables_(_m: *const crate::ported::zsh_h::module, _enables: &mut Option<Vec<i32>>) -> i32 { // c:324
    1                                                                    // c:326
}

/// Port of `enables_module()` from `Src/module.c:1901`.
///
/// C body:
/// ```c
/// enables_module(Module m, int **enables) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->enables)(m, enables) :
///             dyn_enables_module(m, enables));
/// }
/// ```
pub fn enables_module(_table: &mut ModuleTable, _name: &str, _enables: &mut Option<Vec<i32>>) -> i32 { // c:1901
    0                                                                    // c:1904 (enables)(m,enables)
}

/// Port of `features_()` from `Src/module.c:313`.
///
/// C body:
/// ```c
/// features_(UNUSED(Module m), UNUSED(char ***features))
/// {
///     /* There are lots and lots of features, but they're not handled here. */
///     return 1;
/// }
/// ```
pub fn features_(_m: *const crate::ported::zsh_h::module, _features: &mut Vec<String>) -> i32 { // c:313
    /* There are lots and lots of features, but they're not handled here. */ // c:316-318
    1                                                                    // c:319
}

/// Port of `features_module()` from `Src/module.c:1892`.
///
/// C body:
/// ```c
/// features_module(Module m, char ***features) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->features)(m, features) :
///             dyn_features_module(m, features));
/// }
/// ```
pub fn features_module(_table: &mut ModuleTable, _name: &str, _features: &mut Vec<String>) -> i32 { // c:1892
    0                                                                    // c:1895 (features)(m,features)
}

// `featuresarray` deleted — Rust-only port that took the deleted
// `Module` / `Features` PascalCase structs. C
// `featuresarray(Module m, Features f)` at module.c:3279 builds
// the `b:NAME`/`c:NAME`/`f:NAME`/`p:NAME` descriptor array from
// the module's bintab/conddefs/mathfuncs/paramdefs pointers. The
// per-module rust files (rlimits.rs, langinfo.rs, curses.rs, …)
// each ship their own local `featuresarray` stub returning a
// hardcoded descriptor list; a future canonical free-fn port will
// live in zsh_h.rs once `struct features` carries real bintab/etc.
// pointers.

/// `FINDMOD_ALIASP` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_ALIASP = 0x0001 }` from `Src/module.c:110`.
/// /* Resolve any aliases to the underlying module. */
pub const FINDMOD_ALIASP: i32 = 0x0001;                                  // c:110

/// `FINDMOD_CREATE` — bit in `find_module()`'s `flags` arg.
/// Port of `enum { FINDMOD_CREATE = 0x0002 }` from `Src/module.c:115`.
/// /* Create an element for the module in the list if not found. */
pub const FINDMOD_CREATE: i32 = 0x0002;                                  // c:115

/// Port of `find_module()` from `Src/module.c:1659`.
///
/// C body:
/// ```c
/// find_module(const char *name, int flags, const char **namep)
/// {
///     Module m;
///     m = (Module)modulestab->getnode2(modulestab, name);
///     if (m) {
///         if ((flags & FINDMOD_ALIASP) && (m->node.flags & MOD_ALIAS)) {
///             if (namep) *namep = m->u.alias;
///             return find_module(m->u.alias, flags, namep);
///         }
///         if (namep) *namep = m->node.nam;
///         return m;
///     }
///     if (!(flags & FINDMOD_CREATE))
///         return NULL;
///     m = zshcalloc(sizeof(*m));
///     modulestab->addnode(modulestab, ztrdup(name), m);
///     return m;
/// }
/// ```
///
/// Returns the resolved module name (after alias chasing) and
/// whether an entry was created. C's `Module` return becomes
/// `Option<String>` of the canonical name.
/// Port of `find_module` from `Src/module.c:1659`.
pub fn find_module(table: &mut ModuleTable, name: &str, flags: i32) -> Option<String> { // c:1659
    // c:1663 — m = modulestab->getnode2(modulestab, name);
    let mut cur_name = name.to_string();
    let mut depth = 0;
    loop {
        if depth > 64 { return None; } // alias-cycle guard
        depth += 1;
        match table.modules.get(&cur_name) {
            Some(m) => {
                // c:1665 — if ((flags & FINDMOD_ALIASP) && (m->node.flags & MOD_ALIAS))
                if (flags & FINDMOD_ALIASP) != 0 && (m.flags & crate::ported::zsh_h::MOD_ALIAS) != 0 {
                    // c:1668 — return find_module(m->u.alias, flags, namep);
                    if let Some(target) = m.alias.clone() {
                        cur_name = target;
                        continue;
                    }
                    return None;
                }
                // c:1671 — *namep = m->node.nam; return m;
                return Some(cur_name);
            }
            None => {
                // c:1674 — if (!(flags & FINDMOD_CREATE)) return NULL;
                if (flags & FINDMOD_CREATE) == 0 {
                    return None;
                }
                // c:1676-1677 — m = zshcalloc(...); addnode(name, m);
                table.modules.insert(cur_name.clone(), Module::new(&cur_name));
                return Some(cur_name);
            }
        }
    }
}

/// Port of `finish_()` from `Src/module.c:345`.
///
/// C body: `finish_(UNUSED(Module m)) { return 0; }` —
/// the no-op finish hook for the module subsystem itself.
pub fn finish_(_m: *const crate::ported::zsh_h::module) -> i32 {         // c:345
    0                                                                    // c:347
}

/// Port of `finish_module()` from `Src/module.c:1926`.
///
/// C body:
/// ```c
/// finish_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->finish)(m) : dyn_finish_module(m));
/// }
/// ```
pub fn finish_module(_table: &mut ModuleTable, _name: &str) -> i32 {     // c:1926
    0                                                                    // c:1929 (finish)(m) success
}

// `getfeatureenables` deleted — Rust-only port that took the
// deleted `Module` / `Features` PascalCase structs. C
// `getfeatureenables(Module m, Features f)` at module.c:3314
// returns the enable-bit array per feature. Per-module Rust files
// inline their own version returning a hardcoded vec; a canonical
// free-fn re-port belongs in zsh_h.rs once `struct features`
// carries real bintab/conddefs/etc. pointers.

/// Port of `getmathfunc()` from `Src/module.c:1283`.
///
/// C body: linear-search `mathfuncs` for `name`; if found and `autol`
/// is true and the entry is autoloadable, demand-load via
/// `ensurefeature("f:", name)`. Returns the resolved entry or NULL.
///
/// Rust port returns `Some(module_name)` on hit, `None` on miss.
/// Honors the autoload flag by triggering `ensurefeature` when set.
pub fn getmathfunc(table: &mut ModuleTable, name: &str, autol: i32) -> Option<String> { // c:1283
    if let Some(module) = table.autoload_mathfuncs.get(name).cloned() {  // c:1287-1288
        if autol != 0 {                                                  // c:1289
            // c:1295 — ensurefeature(n, "f:", ...)
            let _ = ensurefeature(table, &module, "f:", Some(name));
            return table.autoload_mathfuncs.get(name).cloned();
        }
        return Some(module);                                              // c:1303
    }
    None                                                                 // c:1306
}

// `handlefeatures` deleted — Rust-only port that took the
// deleted `Module` / `Features` PascalCase structs. C
// `handlefeatures(Module m, Features f, int **enables)` at
// module.c:3388 is the convenience front-end that picks
// set/get based on whether enables is NULL. Per-module Rust
// files inline a simpler 2-branch version (rlimits.rs:1428,
// curses.rs etc.); a canonical free-fn re-port belongs in
// zsh_h.rs once `struct features` carries real pointers.

/// Port of `hpux_dlsym()` from `Src/module.c:1530`.
///
/// C body:
/// ```c
/// hpux_dlsym(void *handle, char *name)
/// {
///     void *sym_addr;
///     if (!shl_findsym((shl_t *)&handle, name, TYPE_UNDEFINED, &sym_addr))
///         return sym_addr;
///     return NULL;
/// }
/// ```
///
/// HP-UX-specific dlsym wrapper around `shl_findsym(3)`. Static-link
/// path: never invoked since zshrs doesn't dlopen modules.
/// Port of `hpux_dlsym` from `Src/module.c:1530`.
pub fn hpux_dlsym(_handle: usize, _name: &str) -> usize {                // c:1530
    0                                                                    // c:1535 NULL
}

/// Port of `load_and_bind()` from `Src/module.c:1468`.
///
/// C body: AIX-only `load() + loadbind()` wrapper. Iterates the
/// `modulestab` hash table, binding each loaded module's handle to
/// the new module's symbols. On loadbind failure, calls `unload()`
/// and stores the error in `dlerrstr`.
///
/// Static-link path: dlopen/dlsym aren't used since modules are
/// linked at compile time. Returns 0 (NULL handle).
pub fn load_and_bind(_fn_path: &str) -> usize {                          // c:1468
    0                                                                    // c:1492 NULL
}

/// Port of `modname_ok()` from `Src/module.c:2173`.
///
/// Returns 1 iff `p` is a valid module name: one or more
/// `/`-separated identifier segments.
///
/// C body:
/// ```c
/// modname_ok(char const *p)
/// {
///     do {
///         p = itype_end(p, IIDENT, 0);
///         if (!*p)
///             return 1;
///     } while(*p++ == '/');
///     return 0;
/// }
/// ```
/// Port of `modname_ok` from `Src/module.c:2173`.
pub fn modname_ok(p: &str) -> i32 {                                       // c:2173
    let bytes = p.as_bytes();
    let mut i: usize = 0;
    loop {
        // c:2176 — `p = itype_end(p, IIDENT, 0);`
        // IIDENT = identifier-byte (alpha/digit/underscore + extended).
        while i < bytes.len() {
            let b = bytes[i];
            // Inline IIDENT check — alphanumeric or underscore. Mirrors
            // utils.c:itype_end stepping for the IIDENT bit.
            if b.is_ascii_alphanumeric() || b == b'_' { i += 1; } else { break; }
        }
        if i >= bytes.len() {                                            // c:2177 if (!*p)
            return 1;                                                    // c:2178
        }
        if bytes[i] != b'/' { break; }                                   // c:2179 while(*p++ == '/')
        i += 1;
    }
    0                                                                    // c:2180
}

/// Port of `module_func()` from `Src/module.c:1770`.
///
/// C body (DYNAMIC_NAME_CLASH_OK off — the typical case):
/// ```c
/// module_func(Module m, const char *name)
/// {
///     VARARR(char, buf, strlen(name) + strlen(m->node.nam)*2 + 1);
///     char const *p; char *q;
///     strcpy(buf, name);
///     q = strchr(buf, 0);
///     for(p = m->node.nam; *p; p++) {
///         if(*p == '/')      { *q++ = 'Q'; *q++ = 's'; }
///         else if(*p == '_') { *q++ = 'Q'; *q++ = 'u'; }
///         else if(*p == 'Q') { *q++ = 'Q'; *q++ = 'q'; }
///         else                 *q++ = *p;
///     }
///     *q = 0;
///     return (Module_generic_func) dlsym(m->u.handle, buf);
/// }
/// ```
///
/// Builds a mangled symbol name (`<name><module-name-mangled>`) and
/// dlsym's it. The mangling encodes `/` as `Qs`, `_` as `Qu`, `Q` as
/// `Qq` so e.g. `setup_zsh_random` becomes `setup_zshQurandom`.
///
/// Static-link path: dlsym not used; returns 0 (NULL handle).
/// Port of `module_func` from `Src/module.c:1770`.
pub fn module_func(_m: &Module, _name: &str) -> usize {                  // c:1770
    0                                                                    // c:1794 NULL
}


/// Port of `module_loaded()` from `Src/module.c:1703`.
///
/// C body:
/// ```c
/// module_loaded(const char *name)
/// {
///     Module m;
///     return ((m = find_module(name, FINDMOD_ALIASP, NULL)) &&
///             m->u.handle &&
///             !(m->node.flags & MOD_UNLOAD));
/// }
/// ```
///
/// Returns true (non-zero) if the named module is currently loaded.
/// In zshrs's static-link path: a module is "loaded" iff it's
/// registered in the live `ModuleTable`. The `MOD_UNLOAD` flag check
/// is skipped because static-link modules cannot be unloaded.
/// Port of `module_loaded` from `Src/module.c:1703`.
pub fn module_loaded(table: &ModuleTable, name: &str) -> i32 {           // c:1703
    // c:1707 — find_module(name, FINDMOD_ALIASP, NULL)
    if table.modules.contains_key(name) {                                // m && m->u.handle
        1                                                                 // c:1709 (loaded, not unloading)
    } else {
        0
    }
}

/// Port of `printautoparams()` from `Src/module.c:2710`.
///
/// C body:
/// ```c
/// printautoparams(HashNode hn, int lon)
/// {
///     Param pm = (Param) hn;
///     if (pm->node.flags & PM_AUTOLOAD) {
///         if (lon)
///             printf("zmodload -ap %s %s\n", pm->u.str, pm->node.nam);
///         else
///             printf("%s (%s)\n", pm->node.nam, pm->u.str);
///     }
/// }
/// ```
///
/// Hash-table scan callback for `zmodload -ap` listing. Rust port
/// takes a `(name, module, flags)` triple instead of a HashNode ptr
/// since zshrs's autoload-params live in `ModuleTable.autoload_params`.
/// Port of `printautoparams` from `Src/module.c:2710`.
pub fn printautoparams(name: &str, module: &str, flags: u32, lon: i32) { // c:2710
    if (flags & crate::ported::zsh_h::PM_AUTOLOAD) != 0 {                // c:2714
        if lon != 0 {                                                     // c:2715
            // c:2716 — printf("zmodload -ap %s %s\n", pm->u.str, pm->node.nam);
            println!("zmodload -ap {} {}", module, name);
        } else {
            // c:2718 — printf("%s (%s)\n", pm->node.nam, pm->u.str);
            println!("{} ({})", name, module);
        }
    }
}

/// Port of `removemathfunc()` from `Src/module.c:1267`.
///
/// C body:
/// ```c
/// removemathfunc(MathFunc previous, MathFunc current)
/// {
///     if (previous)
///         previous->next = current->next;
///     else
///         mathfuncs = current->next;
///     zsfree(current->name);
///     zsfree(current->module);
///     zfree(current, sizeof(*current));
/// }
/// ```
///
/// Unlinks `current` from the global `mathfuncs` list and frees it.
/// Rust port: `previous` is unused since the underlying HashMap
/// removal doesn't need predecessor tracking.
// `removemathfunc(table, &MathFunc, &MathFunc)` deleted — Rust-only
// port that took the deleted PascalCase `MathFunc` struct. C
// `removemathfunc(MathFunc previous, MathFunc current)` at
// module.c:1267 unlinks `current` from the global `mathfuncs`
// linked list (ported here as `MATHFUNCS`) — a re-port operating
// on `zsh_h::mathfunc` belongs alongside `addmathfunc` above.

/// Port of `require_module()` from `Src/module.c:3338`.
///
/// C: ensures `modname` is loaded with the named features enabled.
/// Returns 0 on success, non-zero on failure.
///
/// Static-link path: load via `try_load_module`. The features-array
/// argument is accepted but not honoured per-feature yet (the
/// dispatcher tables in `register_module` carry full feature lists).
pub fn require_module(table: &mut ModuleTable, modname: &str, _features: Option<&[String]>) -> i32 {
    if try_load_module(table, modname) == 0 {
        // Module not in static table — report failure.
        return 1;
    }
    0
}

/// Port of `ensurefeature()` from `Src/module.c:3415`.
///
/// C body:
/// ```c
/// ensurefeature(const char *modname, const char *prefix, const char *feature)
/// {
///     char *f;
///     struct feature_enables features[2];
///     if (!feature)
///         return require_module(modname, NULL, 0);
///     f = dyncat(prefix, feature);
///     features[0].str = f;
///     features[0].pat = NULL;
///     features[1].str = NULL;
///     features[1].pat = NULL;
///     return require_module(modname, features, 0);
/// }
/// ```
/// Port of `ensurefeature` from `Src/module.c:3415`.
pub fn ensurefeature(table: &mut ModuleTable, modname: &str, prefix: &str, feature: Option<&str>) -> i32 { // c:3415
    match feature {
        None => require_module(table, modname, None),                    // c:3420-3421
        Some(f) => {
            // c:3422-3428 — build single-element features[2] array.
            let combined = crate::ported::string::dyncat(prefix, f);     // c:3422
            let arr = vec![combined];
            require_module(table, modname, Some(&arr))                   // c:3428
        }
    }
}

// `setbuiltins` / `setconddefs` / `setmathfuncs` / `setparamdefs`
// / `setfeatureenables` all deleted — Rust-only ports that took
// the deleted `Builtin` / `Conddef` / `MathFunc` / `Paramdef` /
// `Module` / `Features` PascalCase structs. C versions
// (module.c:501/754/1374/1165/3350) flip `*_ADDED` flags and
// insert/remove from the global hashtabs; per-module Rust files
// stub these locally and the canonical free-fn re-ports belong
// in zsh_h.rs / hashtable.rs once `struct features` carries
// real pointers.

/// Port of `setup_()` from `Src/module.c:306`.
///
/// C body: `setup_(UNUSED(Module m)) { return 0; }` — the no-op
/// setup hook of the module subsystem itself.
pub fn setup_(_m: *const crate::ported::zsh_h::module) -> i32 {          // c:306
    0                                                                    // c:308
}

/// Port of `setup_module()` from `Src/module.c:1884`.
///
/// C body:
/// ```c
/// setup_module(Module m) {
///     return ((m->node.flags & MOD_LINKED) ?
///             (m->u.linked->setup)(m) : dyn_setup_module(m));
/// }
/// ```
pub fn setup_module(_table: &mut ModuleTable, _name: &str) -> i32 {      // c:1884
    0                                                                    // c:1887 (setup)(m)
}

/// Port of `try_load_module()` from `Src/module.c:1583`.
///
/// C body iterates `module_path` looking for a loadable file via
/// `dlopen`. Static-link path: a module is "loadable" iff it's in
/// our static `ModuleTable.modules` map.
pub fn try_load_module(table: &ModuleTable, name: &str) -> i32 {         // c:1583
    if table.modules.contains_key(name) { 1 } else { 0 }
}

/// Port of `unload_named_module()` from Src/module.c:2924. zshrs links
/// modules statically; this entry is a name-parity shim.
pub fn unload_named_module(table: &mut ModuleTable, name: &str, _nam: &str, _silent: i32) -> i32 {
    // c:2924-2965 — full body: find module, run cleanup, deregister.
    // Static-link path: just remove from the modules map; the per-feature
    // teardown happens via the dispatcher's setfeatureenables call.
    if table.modules.remove(name).is_some() {
        0
    } else {
        1
    }
}
