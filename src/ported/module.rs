//! Module system for zshrs
//!
//! Port from zsh/Src/module.c (3,646 lines)
//!
//! In C, module.c provides dynamic loading of .so modules at runtime
//! via dlopen/dlsym. In Rust, all modules are statically compiled into
//! the binary — there's no dynamic loading. This module provides the
//! registration, lookup, and management API that the rest of the shell
//! uses to interact with module features (builtins, conditions, parameters,
//! hooks, and math functions).

use std::collections::HashMap;

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
}

impl Module {
    pub fn new(name: &str) -> Self {
        Module {
            name: name.to_string(),
            state: ModuleState::Loaded,
            features: Vec::new(),
            deps: Vec::new(),
            autoloads: Vec::new(),
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
    /// Wrappers (functions wrapping builtins)
    wrappers: Vec<Wrapper>,
}

/// Wrapper entry (from module.c addwrapper/deletewrapper)
#[derive(Debug, Clone)]
/// Function-wrapper hook.
/// Port of `struct funcwrap` from Src/zsh.h — `addwrapper()`
/// (Src/module.c:577) registers a module's pre/post hooks for
/// shell function dispatch (`zsh/zprof` uses this).
pub struct Wrapper {
    pub name: String,
    pub flags: u32,
    pub module: String,
}

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

    /// Load a module (from module.c load_module)
    pub fn load_module(&mut self, name: &str) -> bool {
        if self.modules.contains_key(name) {
            if let Some(m) = self.modules.get_mut(name) {
                m.state = ModuleState::Loaded;
            }
            return true;
        }
        // In zshrs, all modules are static — if it's not registered, it doesn't exist
        false
    }

    /// Unload a module (from module.c unload_module)
    pub fn unload_module(&mut self, name: &str) -> bool {
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
    pub fn addbuiltin(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features.push(ModuleFeature {
                name: name.to_string(),
                feature_type: FeatureType::Builtin,
                enabled: true,
            });
        }
    }

    /// Unregister a builtin (from module.c deletebuiltin)
    pub fn deletebuiltin(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Builtin);
        }
    }

    /// Register autoloading builtin (from module.c add_autobin)
    pub fn add_autobin(&mut self, name: &str, module: &str) {
        self.autoload_builtins
            .insert(name.to_string(), module.to_string());
    }

    /// Remove autoloading builtin (from module.c del_autobin)
    pub fn del_autobin(&mut self, name: &str) {
        self.autoload_builtins.remove(name);
    }

    /// Set builtins en masse (from module.c setbuiltins/addbuiltins)
    pub fn setbuiltins(&mut self, module: &str, names: &[&str]) {
        for name in names {
            self.addbuiltin(name, module);
        }
    }

    // ------- Condition management (from module.c addconddef/deleteconddef) -------

    /// Register a condition (from module.c addconddef)
    pub fn addconddef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features.push(ModuleFeature {
                name: name.to_string(),
                feature_type: FeatureType::Condition,
                enabled: true,
            });
        }
    }

    /// Unregister a condition (from module.c deleteconddef)
    pub fn deleteconddef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Condition);
        }
    }

    /// Get condition definition (from module.c getconddef)
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
    pub fn add_autocond(&mut self, name: &str, module: &str) {
        self.autoload_conditions
            .insert(name.to_string(), module.to_string());
    }

    /// Remove autoloading condition (from module.c del_autocond)
    pub fn del_autocond(&mut self, name: &str) {
        self.autoload_conditions.remove(name);
    }

    // ------- Hook management (from module.c addhookdef/deletehookdef) -------

    /// Register a hook (from module.c addhookdef)
    pub fn addhookdef(&mut self, name: &str) {
        self.hooks.entry(name.to_string()).or_default();
    }

    /// Register multiple hooks (from module.c addhookdefs)
    pub fn addhookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.addhookdef(name);
        }
    }

    /// Unregister a hook (from module.c deletehookdef)
    pub fn deletehookdef(&mut self, name: &str) {
        self.hooks.remove(name);
    }

    /// Unregister multiple hooks (from module.c deletehookdefs)
    pub fn deletehookdefs(&mut self, names: &[&str]) {
        for name in names {
            self.deletehookdef(name);
        }
    }

    /// Add function to hook (from module.c addhookdeffunc/addhookfunc)
    pub fn addhookfunc(&mut self, hook: &str, func: &str) {
        self.hooks
            .entry(hook.to_string())
            .or_default()
            .push(func.to_string());
    }

    /// Remove function from hook (from module.c deletehookdeffunc/deletehookfunc)
    pub fn deletehookfunc(&mut self, hook: &str, func: &str) {
        if let Some(funcs) = self.hooks.get_mut(hook) {
            funcs.retain(|f| f != func);
        }
    }

    /// Get hook definition (from module.c gethookdef)
    pub fn gethookdef(&self, name: &str) -> Option<&Vec<String>> {
        self.hooks.get(name)
    }

    /// Run hook functions (from module.c runhookdef)
    pub fn runhookdef(&self, name: &str) -> Vec<String> {
        self.hooks.get(name).cloned().unwrap_or_default()
    }

    // ------- Parameter management (from module.c addparamdef/deleteparamdef) -------

    /// Register a parameter from module (from module.c addparamdef/checkaddparam)
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
    pub fn deleteparamdef(&mut self, name: &str, module: &str) {
        if let Some(m) = self.modules.get_mut(module) {
            m.features
                .retain(|f| f.name != name || f.feature_type != FeatureType::Parameter);
        }
    }

    /// Set parameters en masse (from module.c setparamdefs)
    pub fn setparamdefs(&mut self, module: &str, names: &[&str]) {
        for name in names {
            self.addparamdef(name, module);
        }
    }

    /// Register autoloading parameter (from module.c add_autoparam)
    pub fn add_autoparam(&mut self, name: &str, module: &str) {
        self.autoload_params
            .insert(name.to_string(), module.to_string());
    }

    /// Remove autoloading parameter (from module.c del_autoparam)
    pub fn del_autoparam(&mut self, name: &str) {
        self.autoload_params.remove(name);
    }

    // ------- Wrapper management (from module.c addwrapper/deletewrapper) -------

    /// Add wrapper (from module.c addwrapper)
    pub fn addwrapper(&mut self, name: &str, flags: u32, module: &str) {
        self.wrappers.push(Wrapper {
            name: name.to_string(),
            flags,
            module: module.to_string(),
        });
    }

    /// Remove wrapper (from module.c deletewrapper)
    pub fn deletewrapper(&mut self, module: &str, name: &str) {
        self.wrappers
            .retain(|w| w.module != module || w.name != name);
    }

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

/// Register module (from module.c register_module)
/// Register a module by name.
/// Port of `register_module()` from Src/module.c:359 — wraps
/// a slot in the global `modulestab` and seeds its lifecycle
/// callbacks.
pub fn register_module(table: &mut ModuleTable, name: &str) -> bool {
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

    #[test]
    fn test_wrappers() {
        let mut table = ModuleTable::new();
        table.addwrapper("cd", 0, "zsh/mymod");
        assert_eq!(table.wrappers.len(), 1);

        table.deletewrapper("zsh/mymod", "cd");
        assert!(table.wrappers.is_empty());
    }

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
impl crate::ported::exec::ShellExecutor {
    /// zmodload - load/unload zsh modules (stub)
    pub(crate) fn bin_zmodload(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `zmodload` event per module name (only
        // for load form — listing/query/unload are not state mutations
        // we want recorded as definitions).
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let mut listing_or_query = false;
            let mut flags: Vec<&str> = Vec::new();
            for a in args {
                match a.as_str() {
                    "-l" | "-L" | "-e" | "-u" => listing_or_query = true,
                    s if s.starts_with('-') => flags.push(s),
                    _ => {}
                }
            }
            if !listing_or_query {
                let ctx = self.recorder_ctx();
                let flag_blob = flags.join(" ");
                for a in args {
                    if a.starts_with('-') {
                        continue;
                    }
                    crate::recorder::emit_zmodload(a, &flag_blob, ctx.clone());
                }
            }
        }
        // Direct port of zsh/Src/module.c bin_zmodload control flow.
        // zshrs's modules are compiled-in, so load/unload simply toggle
        // a tracking flag in self.options['_module_<name>']. Listing
        // returns the union of always-loaded compiled-in modules plus
        // any user-zmodload'd entries.
        let mut list_loaded = false;
        let mut list_reusable = false;
        let mut unload = false;
        let mut existence_test = false; // -e: query whether loaded
        let mut modules: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-l" => list_loaded = true,
                "-L" => {
                    list_loaded = true;
                    list_reusable = true;
                }
                "-u" => unload = true,
                "-e" => existence_test = true,
                "-a" | "-b" | "-c" | "-d" | "-f" | "-i" | "-p" | "-s" | "-A" | "-R" | "-F"
                | "-I" | "-P" => {}
                _ if arg.starts_with('-') => {
                    // BUILTIN("zmodload", ..., "AFRILP:abcfdilmpsue")
                    // declares the valid letter set. Unknown flags
                    // silently dropped previously, masking typos.
                    let bad: String = arg[1..].chars().take(1).collect();
                    eprintln!("zshrs:zmodload:1: bad option: -{}", bad);
                    return 1;
                }
                _ => modules.push(arg),
            }
        }

        // Compiled-in modules zshrs supports. Same set the brew zsh
        // ships in /opt/homebrew/lib/zsh/*.bundle, plus a few zshrs-
        // specific entries (`zsh/profiler`, `zsh/main`, `zsh/random_real`,
        // `zsh/param_private`, etc.). `zmodload NAME` accepts any of
        // these; unknown names error like `failed to load module`.
        const ALWAYS_LOADED: &[&str] = &[
            "zsh/attr",
            "zsh/cap",
            "zsh/clone",
            "zsh/compctl",
            "zsh/complete",
            "zsh/complist",
            "zsh/computil",
            "zsh/curses",
            "zsh/datetime",
            "zsh/db/gdbm",
            "zsh/deltochar",
            "zsh/example",
            "zsh/files",
            "zsh/hlgroup",
            "zsh/ksh93",
            "zsh/langinfo",
            "zsh/main",
            "zsh/mapfile",
            "zsh/mathfunc",
            "zsh/nearcolor",
            "zsh/net/socket",
            "zsh/net/tcp",
            "zsh/newuser",
            "zsh/param/private",
            "zsh/parameter",
            "zsh/pcre",
            "zsh/private",
            "zsh/profiler",
            "zsh/random",
            "zsh/random_real",
            "zsh/regex",
            "zsh/rlimits",
            "zsh/sched",
            "zsh/stat",
            "zsh/system",
            "zsh/termcap",
            "zsh/terminfo",
            "zsh/watch",
            "zsh/zftp",
            "zsh/zle",
            "zsh/zleparameter",
            "zsh/zprof",
            "zsh/zpty",
            "zsh/zselect",
            "zsh/zutil",
        ];

        // `is_loaded` answers: has the user explicitly loaded this
        // module via `zmodload NAME` in this shell? Direct port of
        // zsh's `-e` semantics (Src/module.c bin_zmodload case 'e'):
        // "loaded" is observable state, NOT compile-time presence.
        // zshrs links every module statically, but `-e` must reflect
        // user-controlled load actions only; otherwise scripts that
        // do `zmodload -e zsh/datetime || zmodload zsh/datetime` get
        // the wrong answer.
        let is_loaded = |this: &Self, name: &str| -> bool {
            this
                .options
                .get(&format!("_module_{}", name))
                .copied()
                .unwrap_or(false)
        };

        // -e: existence test. Exit 0 if all named modules are loaded,
        // else 1. zsh module.c bin_zmodload case 'e'.
        if existence_test {
            for m in &modules {
                if !is_loaded(self, m) {
                    return 1;
                }
            }
            return 0;
        }

        if list_loaded || modules.is_empty() {
            let mut all: Vec<String> = ALWAYS_LOADED.iter().map(|s| s.to_string()).collect();
            for (k, v) in &self.options {
                if *v {
                    if let Some(name) = k.strip_prefix("_module_") {
                        all.push(name.to_string());
                    }
                }
            }
            all.sort();
            all.dedup();
            for m in &all {
                if list_reusable {
                    println!("zmodload {}", m);
                } else {
                    println!("{}", m);
                }
            }
            return 0;
        }

        // Reject unknown module names. zsh's bin_zmodload (Src/module.c)
        // attempts dlopen on the named bundle and reports
        // "failed to load module" when the lookup misses; zshrs has
        // no dlopen layer (modules are statically linked), so we
        // gate on the `ALWAYS_LOADED` whitelist instead. Without
        // this check, `zmodload zsh/no_such_module` silently
        // succeeded, masking typos in user scripts.
        for module in &modules {
            if !ALWAYS_LOADED.contains(module) {
                eprintln!("zshrs:zmodload:1: failed to load module: {}", module);
                return 1;
            }
        }
        for module in modules {
            if unload {
                self.options.remove(&format!("_module_{}", module));
            } else {
                self.options.insert(format!("_module_{}", module), true);
            }
        }
        0
    }
}
// END moved-from-exec-rs
