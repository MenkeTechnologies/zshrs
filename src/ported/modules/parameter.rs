//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use std::collections::HashMap;
use std::path::PathBuf;

// (impl ShellExecutor block + magic_assoc_keys() moved to
// src/exec_shims.rs — that method aggregates per-magic-table
// dispatch which the C source splits across separate
// scanpm{aliases,functions,builtins,commands,reswords,options}
// helpers. Each branch in the moved Rust method is FAKE — it
// hard-codes static lists or reaches `&self.<field>` instead of
// walking the canonical hashtable like the C source. The honest
// fix is to replace each branch with a call to the real scanpm*
// port in this file (parameter.rs); until those land, the moved
// method stands as a placeholder.)

/// Parameter type flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Parameter type tag.
/// Mirrors the `PM_*` type bits from Src/zsh.h —
/// `paramtypestr()` from Src/Modules/parameter.c:43 maps these
/// onto the `typeset -p` output letters.
pub enum ParamType {
    Scalar,
    Integer,
    Float,
    Array,
    Associative,
    Nameref,
}

impl ParamType {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn name(&self) -> &'static str {
        match self {
            ParamType::Scalar => "scalar",
            ParamType::Integer => "integer",
            ParamType::Float => "float",
            ParamType::Array => "array",
            ParamType::Associative => "association",
            ParamType::Nameref => "nameref",
        }
    }
}

/// Parameter attributes
#[derive(Debug, Clone, Default)]
/// Per-parameter flag bits.
/// Port of the `PM_*` modifier flags Src/zsh.h declares
/// (`PM_LOCAL` / `PM_READONLY` / `PM_TAGGED` / `PM_EXPORTED` /
/// `PM_HASHED` / `PM_HIDE` / etc.).
pub struct ParamFlags {
    pub local: bool,
    pub left_justify: bool,
    pub right_blanks: bool,
    pub right_zeros: bool,
    pub lower: bool,
    pub upper: bool,
    pub readonly: bool,
    pub tagged: bool,
    pub tied: bool,
    pub exported: bool,
    pub unique: bool,
    pub hide: bool,
    pub hideval: bool,
    pub special: bool,
}

/// Generate parameter type string (like "scalar-local-export")
/// Render a parameter's type as a `typeset -p` flag string.
/// Port of `paramtypestr()` from Src/Modules/parameter.c:43.
pub fn paramtypestr(ptype: ParamType, flags: &ParamFlags) -> String {
    let mut parts = vec![ptype.name().to_string()];

    if flags.local {
        parts.push("local".to_string());
    }
    if flags.left_justify {
        parts.push("left".to_string());
    }
    if flags.right_blanks {
        parts.push("right_blanks".to_string());
    }
    if flags.right_zeros {
        parts.push("right_zeros".to_string());
    }
    if flags.lower {
        parts.push("lower".to_string());
    }
    if flags.upper {
        parts.push("upper".to_string());
    }
    if flags.readonly {
        parts.push("readonly".to_string());
    }
    if flags.tagged {
        parts.push("tag".to_string());
    }
    if flags.tied {
        parts.push("tied".to_string());
    }
    if flags.exported {
        parts.push("export".to_string());
    }
    if flags.unique {
        parts.push("unique".to_string());
    }
    if flags.hide {
        parts.push("hide".to_string());
    }
    if flags.hideval {
        parts.push("hideval".to_string());
    }
    if flags.special {
        parts.push("special".to_string());
    }

    parts.join("-")
}

/// Commands hash table ($commands)
#[derive(Debug, Default)]
/// `${commands}` special-parameter table.
/// Port of the parameter Src/Modules/parameter.c installs via
/// `getpmcommand()` (line 213) / `scanpmcommands()` (line 245)
/// — exposes the `cmdnamtab` HashTable as a hash parameter.
pub struct CommandsTable {
    hashed: HashMap<String, PathBuf>,
}

impl CommandsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<&PathBuf> {
        self.hashed.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, name: &str, path: PathBuf) {
        self.hashed.insert(name.to_string(), path);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn unset(&mut self, name: &str) {
        self.hashed.remove(name);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn clear(&mut self) {
        self.hashed.clear();
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.hashed.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn len(&self) -> usize {
        self.hashed.len()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn is_empty(&self) -> bool {
        self.hashed.is_empty()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn rehash(&mut self, path_dirs: &[PathBuf]) {
        self.hashed.clear();
        for dir in path_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_file() || ft.is_symlink() {
                            if let Some(name) = entry.file_name().to_str() {
                                self.hashed.insert(name.to_string(), entry.path());
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Functions hash table ($functions)
#[derive(Debug, Clone)]
/// Function definition body.
/// Port of the body slot `getfunction()` (Src/Modules/
/// parameter.c:389) returns — for `${functions[name]}` lookups.
pub struct FunctionDef {
    pub body: String,
    pub flags: u32,
    pub autoload: bool,
}

#[derive(Debug, Default)]
/// `${functions}` / `${dis_functions}` special-parameter table.
/// Port of the param Src/Modules/parameter.c installs via
/// `getpmfunction()` (line 444) / `scanpmfunctions()` (line 519)
/// — exposes `shfunctab` as a hash parameter.
pub struct FunctionsTable {
    functions: HashMap<String, FunctionDef>,
    disabled: HashMap<String, FunctionDef>,
}

impl FunctionsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get_disabled(&self, name: &str) -> Option<&FunctionDef> {
        self.disabled.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, name: &str, def: FunctionDef) {
        self.functions.insert(name.to_string(), def);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn unset(&mut self, name: &str) {
        self.functions.remove(name);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn disable(&mut self, name: &str) {
        if let Some(def) = self.functions.remove(name) {
            self.disabled.insert(name.to_string(), def);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn enable(&mut self, name: &str) {
        if let Some(def) = self.disabled.remove(name) {
            self.functions.insert(name.to_string(), def);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &FunctionDef)> {
        self.functions.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter_disabled(&self) -> impl Iterator<Item = (&String, &FunctionDef)> {
        self.disabled.iter()
    }
}

/// Aliases hash table ($aliases)
#[derive(Debug, Clone)]
/// Alias definition record.
/// Port of the per-entry shape `${aliases}` / `${galiases}` /
/// `${saliases}` returns — Src/Modules/parameter.c uses
/// `aliastab` HashTable nodes directly.
pub struct AliasDef {
    pub value: String,
    pub global: bool,
    pub suffix: bool,
}

#[derive(Debug, Default)]
/// `${aliases}` / `${galiases}` / `${saliases}` parameter.
/// Port of the alias-table parameters Src/Modules/parameter.c
/// installs (regular + global + suffix variants).
pub struct AliasesTable {
    aliases: HashMap<String, AliasDef>,
    disabled: HashMap<String, AliasDef>,
    global_aliases: HashMap<String, AliasDef>,
    suffix_aliases: HashMap<String, AliasDef>,
}

impl AliasesTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<&AliasDef> {
        self.aliases.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get_global(&self, name: &str) -> Option<&AliasDef> {
        self.global_aliases.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get_suffix(&self, suffix: &str) -> Option<&AliasDef> {
        self.suffix_aliases.get(suffix)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, name: &str, def: AliasDef) {
        if def.global {
            self.global_aliases.insert(name.to_string(), def);
        } else if def.suffix {
            self.suffix_aliases.insert(name.to_string(), def);
        } else {
            self.aliases.insert(name.to_string(), def);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn unset(&mut self, name: &str) {
        self.aliases.remove(name);
        self.global_aliases.remove(name);
        self.suffix_aliases.remove(name);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn disable(&mut self, name: &str) {
        if let Some(def) = self.aliases.remove(name) {
            self.disabled.insert(name.to_string(), def);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn enable(&mut self, name: &str) {
        if let Some(def) = self.disabled.remove(name) {
            self.aliases.insert(name.to_string(), def);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.aliases.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter_global(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.global_aliases.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter_suffix(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.suffix_aliases.iter()
    }
}

/// Builtins list ($builtins)
#[derive(Debug, Default)]
/// `${builtins}` / `${dis_builtins}` parameter.
/// Port of the builtin-table parameter Src/Modules/parameter.c
/// installs — exposes `builtintab` HashTable.
pub struct BuiltinsTable {
    builtins: HashMap<String, bool>,
    disabled: HashMap<String, bool>,
}

impl BuiltinsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn register(&mut self, name: &str) {
        self.builtins.insert(name.to_string(), true);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains_key(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn disable(&mut self, name: &str) {
        if self.builtins.remove(name).is_some() {
            self.disabled.insert(name.to_string(), true);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn enable(&mut self, name: &str) {
        if self.disabled.remove(name).is_some() {
            self.builtins.insert(name.to_string(), true);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn list(&self) -> Vec<&str> {
        self.builtins.keys().map(|s| s.as_str()).collect()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn list_disabled(&self) -> Vec<&str> {
        self.disabled.keys().map(|s| s.as_str()).collect()
    }
}

/// Directory stack ($dirstack)
#[derive(Debug, Default)]
/// `${dirstack}` parameter.
/// Port of the dirstack accessor in Src/Modules/parameter.c —
/// reads the directory stack `pushd` / `popd` (Src/builtin.c)
/// maintains.
pub struct DirStack {
    stack: Vec<PathBuf>,
}

impl DirStack {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn push(&mut self, dir: PathBuf) {
        self.stack.push(dir);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn pop(&mut self) -> Option<PathBuf> {
        self.stack.pop()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, index: usize) -> Option<&PathBuf> {
        self.stack.get(index)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, stack: Vec<PathBuf>) {
        self.stack = stack;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.stack.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn to_array(&self) -> Vec<String> {
        self.stack
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }
}

/// Options special parameter ($options)
#[derive(Debug, Default)]
/// `${options}` parameter.
/// Port of the parameter-shape exposing `optab[]` (Src/options.c)
/// from Src/Modules/parameter.c.
pub struct OptionsTable {
    options: HashMap<String, bool>,
}

impl OptionsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, name: &str, value: bool) {
        self.options.insert(name.to_lowercase(), value);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<bool> {
        self.options.get(&name.to_lowercase()).copied()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn is_set(&self, name: &str) -> bool {
        self.options
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or(false)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &bool)> {
        self.options.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn to_hash(&self) -> HashMap<String, String> {
        self.options
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    if *v {
                        "on".to_string()
                    } else {
                        "off".to_string()
                    },
                )
            })
            .collect()
    }
}

/// Named directories ($nameddirs, $userdirs)
#[derive(Debug, Default)]
/// `${nameddirs}` parameter.
/// Port of the parameter exposing `nameddirtab`
/// (Src/hashnameddir.c) from Src/Modules/parameter.c.
pub struct NamedDirsTable {
    dirs: HashMap<String, PathBuf>,
}

impl NamedDirsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn set(&mut self, name: &str, path: PathBuf) {
        self.dirs.insert(name.to_string(), path);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<&PathBuf> {
        self.dirs.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn unset(&mut self, name: &str) {
        self.dirs.remove(name);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn find_name(&self, path: &PathBuf) -> Option<&str> {
        self.dirs
            .iter()
            .find(|(_, p)| *p == path)
            .map(|(n, _)| n.as_str())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.dirs.iter()
    }
}

/// Job states ($jobstates)
#[derive(Debug, Clone)]
/// One job state entry.
/// Mirrors `struct job` from Src/zsh.h — Src/Modules/parameter.c
/// reads the `jobtab` for `${jobtexts}` / `${jobdirs}` /
/// `${jobstates}`.
pub struct JobState {
    pub running: bool,
    pub suspended: bool,
    pub done: bool,
}

impl JobState {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn as_str(&self) -> &'static str {
        if self.done {
            "done"
        } else if self.suspended {
            "suspended"
        } else if self.running {
            "running"
        } else {
            "unknown"
        }
    }
}

/// Job texts ($jobtexts)
#[derive(Debug, Default)]
/// `${jobtexts}` / `${jobdirs}` / `${jobstates}` parameter.
/// Port of the job-table parameter shape Src/Modules/parameter.c
/// installs.
pub struct JobsTable {
    jobs: HashMap<i32, (JobState, String)>,
}

impl JobsTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn add(&mut self, id: i32, state: JobState, text: String) {
        self.jobs.insert(id, (state, text));
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn remove(&mut self, id: i32) {
        self.jobs.remove(&id);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get_state(&self, id: i32) -> Option<&JobState> {
        self.jobs.get(&id).map(|(s, _)| s)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get_text(&self, id: i32) -> Option<&str> {
        self.jobs.get(&id).map(|(_, t)| t.as_str())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn states(&self) -> HashMap<String, String> {
        self.jobs
            .iter()
            .map(|(id, (state, _))| (id.to_string(), state.as_str().to_string()))
            .collect()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn texts(&self) -> HashMap<String, String> {
        self.jobs
            .iter()
            .map(|(id, (_, text))| (id.to_string(), text.clone()))
            .collect()
    }
}

/// Modules table ($modules)
#[derive(Debug, Clone)]
/// Per-module info entry for `${modules}`.
/// Mirrors the C source's `Module` struct (Src/zsh.h) the way
/// Src/Modules/parameter.c presents it through the
/// `${modules[NAME]}` parameter.
pub struct ModuleInfo {
    pub loaded: bool,
    pub autoload: bool,
}

#[derive(Debug, Default)]
/// `${modules}` special-parameter.
/// Port of the parameter exposing `modulestab` (Src/module.c)
/// from Src/Modules/parameter.c.
pub struct ModulesTable {
    modules: HashMap<String, ModuleInfo>,
}

impl ModulesTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn register(&mut self, name: &str, info: ModuleInfo) {
        self.modules.insert(name.to_string(), info);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn get(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.modules.get(name).map(|m| m.loaded).unwrap_or(false)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ModuleInfo)> {
        self.modules.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/parameter.c`.
    pub fn to_hash(&self) -> HashMap<String, String> {
        self.modules
            .iter()
            .map(|(k, v)| {
                let status = if v.loaded {
                    "loaded"
                } else if v.autoload {
                    "autoload"
                } else {
                    "unloaded"
                };
                (k.clone(), status.to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_type_str() {
        let flags = ParamFlags::default();
        assert_eq!(paramtypestr(ParamType::Scalar, &flags), "scalar");

        let flags = ParamFlags {
            local: true,
            exported: true,
            ..Default::default()
        };
        assert_eq!(
            paramtypestr(ParamType::Array, &flags),
            "array-local-export"
        );
    }

    #[test]
    fn test_commands_table() {
        let mut table = CommandsTable::new();
        table.set("ls", PathBuf::from("/bin/ls"));

        assert_eq!(table.get("ls"), Some(&PathBuf::from("/bin/ls")));
        assert!(table.get("nonexistent").is_none());

        table.unset("ls");
        assert!(table.get("ls").is_none());
    }

    #[test]
    fn test_functions_table() {
        let mut table = FunctionsTable::new();
        table.set(
            "myfunc",
            FunctionDef {
                body: "echo hello".to_string(),
                flags: 0,
                autoload: false,
            },
        );

        assert!(table.get("myfunc").is_some());

        table.disable("myfunc");
        assert!(table.get("myfunc").is_none());
        assert!(table.get_disabled("myfunc").is_some());

        table.enable("myfunc");
        assert!(table.get("myfunc").is_some());
    }

    #[test]
    fn test_aliases_table() {
        let mut table = AliasesTable::new();
        table.set(
            "ll",
            AliasDef {
                value: "ls -l".to_string(),
                global: false,
                suffix: false,
            },
        );

        assert!(table.get("ll").is_some());
        assert_eq!(table.get("ll").unwrap().value, "ls -l");
    }

    #[test]
    fn test_builtins_table() {
        let mut table = BuiltinsTable::new();
        table.register("echo");
        table.register("cd");

        assert!(table.is_builtin("echo"));
        assert!(!table.is_builtin("nonexistent"));

        table.disable("echo");
        assert!(!table.is_builtin("echo"));
    }

    #[test]
    fn test_dir_stack() {
        let mut stack = DirStack::new();
        stack.push(PathBuf::from("/home"));
        stack.push(PathBuf::from("/tmp"));

        assert_eq!(stack.len(), 2);
        assert_eq!(stack.pop(), Some(PathBuf::from("/tmp")));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn test_options_table() {
        let mut table = OptionsTable::new();
        table.set("autocd", true);
        table.set("EXTENDEDGLOB", true);

        assert!(table.is_set("autocd"));
        assert!(table.is_set("extendedglob")); // case insensitive
    }

    #[test]
    fn test_named_dirs() {
        let mut table = NamedDirsTable::new();
        table.set("proj", PathBuf::from("/home/user/projects"));

        assert_eq!(
            table.get("proj"),
            Some(&PathBuf::from("/home/user/projects"))
        );
        assert_eq!(
            table.find_name(&PathBuf::from("/home/user/projects")),
            Some("proj")
        );
    }

    #[test]
    fn test_jobs_table() {
        let mut table = JobsTable::new();
        table.add(
            1,
            JobState {
                running: true,
                suspended: false,
                done: false,
            },
            "vim file.txt".to_string(),
        );

        assert_eq!(table.get_state(1).unwrap().as_str(), "running");
        assert_eq!(table.get_text(1), Some("vim file.txt"));
    }

    #[test]
    fn test_modules_table() {
        let mut table = ModulesTable::new();
        table.register(
            "zsh/datetime",
            ModuleInfo {
                loaded: true,
                autoload: false,
            },
        );

        assert!(table.is_loaded("zsh/datetime"));
        assert!(!table.is_loaded("nonexistent"));
    }
}

// =====================================================================
// static struct features module_features                            c:2300 (parameter.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 0,
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 14,                                      // partab[14]: parameters/commands/options/aliases/etc
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/parameter.c:2311`.
pub fn setup_(_m: *const module) -> i32 { 0 }

/// Port of `features_()` from `Src/Modules/parameter.c:2318`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/parameter.c:2326`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/parameter.c:2341`.
pub fn boot_(_m: *const module) -> i32 { 0 }

/// Port of `cleanup_()` from `Src/Modules/parameter.c:2348`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/parameter.c:2359`.
pub fn finish_(_m: *const module) -> i32 { 0 }

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["p:parameters".to_string(), "p:commands".to_string(), "p:functions".to_string(),
         "p:dis_functions".to_string(), "p:functions_source".to_string(),
         "p:dis_functions_source".to_string(), "p:builtins".to_string(),
         "p:dis_builtins".to_string(), "p:reswords".to_string(), "p:dis_reswords".to_string(),
         "p:options".to_string(), "p:modules".to_string(), "p:dirstack".to_string(),
         "p:history".to_string(), "p:historywords".to_string(), "p:jobtexts".to_string(),
         "p:jobdirs".to_string(), "p:jobstates".to_string(), "p:nameddirs".to_string(),
         "p:userdirs".to_string(), "p:aliases".to_string(), "p:dis_aliases".to_string(),
         "p:galiases".to_string(), "p:dis_galiases".to_string(), "p:saliases".to_string(),
         "p:dis_saliases".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() { *enables = Some(getfeatureenables(m, f)); }
    else if let Some(e) = enables.as_ref() { return setfeatureenables(m, f, Some(e)); }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

// (`scan_magic_assoc_keys` moved out of src/ported/ to
// src/exec_shims.rs — it has no C counterpart and the
// no-non-C-fns-in-src/ported rule applies. The canonical scanpm*
// ports below ARE the C dispatch; the aggregator is a
// fusevm-bridge convenience that fans the magic-assoc table NAME
// out into the right scanpm* call. See exec_shims.rs.)

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/parameter.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Direct port of `assignaliasdefs()` from Src/Modules/parameter.c:1867.
/// C signature: `static void assignaliasdefs(Param pm, int flags)`.
/// C body sets `pm->node.flags = PM_SCALAR` (c:1869) then dispatches
/// `pm->gsu.s` to one of six static gsu_scalar handler tables based
/// on the alias-flavour bits (raw/global/suffix × normal/disabled).
/// Static-link path: the gsu table machinery is not yet ported; the
/// flag-to-handler mapping is recorded in a side-map keyed by the
/// param's name so future gsu lookups resolve the right handler.
#[allow(non_snake_case)]
pub fn assignaliasdefs(pm: *mut crate::ported::zsh_h::param,                 // c:1867
                       flags: i32) {
    use crate::ported::zsh_h::{PM_SCALAR, ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED};
    if !pm.is_null() {
        unsafe { (*pm).node.flags = PM_SCALAR as i32; }                      // c:1869
    }
    // c:1871-1893 — switch on flag combination to pick the gsu table.
    let handler = match flags {                                              // c:1873
        0                              => "pmralias_gsu",                    // c:1874
        f if f == ALIAS_GLOBAL          => "pmgalias_gsu",                   // c:1877
        f if f == ALIAS_SUFFIX          => "pmsalias_gsu",                   // c:1880
        f if f == DISABLED              => "pmdisralias_gsu",                // c:1883
        f if f == ALIAS_GLOBAL|DISABLED => "pmdisgalias_gsu",                // c:1886
        f if f == ALIAS_SUFFIX|DISABLED => "pmdissalias_gsu",                // c:1889
        _ => return,
    };
    if !pm.is_null() {
        let name = unsafe { (*pm).node.nam.clone() };
        let m = ALIAS_GSU_HANDLER.get_or_init(|| std::sync::Mutex::new(
            std::collections::HashMap::new()));
        if let Ok(mut g) = m.lock() {
            g.insert(name, handler.to_string());
        }
    }
}

// =====================================================================
// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// `ALIAS_GSU_HANDLER` records which `pm*alias_gsu` static dispatch
// table assignaliasdefs() selected for each parameter name. The C
// source stores this directly on `Param->gsu.s` as a function-table
// pointer (Src/Modules/parameter.c:1842-1860). Until the gsu_scalar
// dispatch table machinery is ported in full, this side-map is the
// bridge so future gsu lookups can resolve the right handler.
//
// !!! Remove this side-map once the gsu_scalar dispatch table is
// ported in src/ported/params.rs and assignaliasdefs() can write
// `pm->gsu.s = &pmralias_gsu` directly. !!!
// =====================================================================
static ALIAS_GSU_HANDLER: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

/// Port of `dirsgetfn()` from Src/Modules/parameter.c:1147.
/// C: `static char **dirsgetfn(UNUSED(Param pm))` →
///   `return hlinklist2array(dirstack, 1);`
#[allow(non_snake_case)]
pub fn dirsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {     // c:1147
    // c:1150 — hlinklist2array(dirstack, 1) returns the dirstack as
    // a heap-allocated array. Static-link path reads from the global
    // DIRSTACK list maintained by `dirs`/`pushd`/`popd`.
    DIRSTACK.lock().map(|d| d.clone()).unwrap_or_default()                   // c:1150
}

/// Port of `dirssetfn()` from Src/Modules/parameter.c:1131.
/// C: `static void dirssetfn(UNUSED(Param pm), char **x)` — replaces
/// the dirstack with the provided array (when not in cleanup).
#[allow(non_snake_case)]
pub fn dirssetfn(_pm: *mut crate::ported::zsh_h::param, x: Vec<String>) {    // c:1131
    let incleanup = INCLEANUP.load(std::sync::atomic::Ordering::Relaxed);    // c:1136
    if incleanup == 0 {                                                      // c:1136
        if let Ok(mut d) = DIRSTACK.lock() {                                 // c:1137-1140
            d.clear();                                                       // c:1137
            for entry in &x {                                                // c:1139
                d.push(entry.clone());                                       // c:1140
            }
        }
    }
    // c:1142-1143 — freearray(ox); Rust drops `x` automatically.
    drop(x);
}

/// Port of `dispatcharsgetfn()` from Src/Modules/parameter.c:917.
/// C: `static char **dispatcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(1);`
#[allow(non_snake_case)]
pub fn dispatcharsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:917
    getpatchars(1)                                                           // c:920
}

/// Port of `disreswordsgetfn()` from Src/Modules/parameter.c:885.
/// C: `static char **disreswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(DISABLED);`
#[allow(non_snake_case)]
pub fn disreswordsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:885
    getreswords(crate::ported::zsh_h::DISABLED)                              // c:888
}

/// Port of `funcfiletracegetfn()` from Src/Modules/parameter.c:711.
/// C: `static char **funcfiletracegetfn(UNUSED(Param pm))` — walks
/// `funcstack` building a "<file>:<lineno>" pair per frame.
#[allow(non_snake_case)]
pub fn funcfiletracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:711
    // c:715-740 — walk funcstack, build colonpair "<filename>:<flineno>".
    // Static-link path: FUNCSTACK is the live runtime call stack.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.filename, f.flineno))                    // c:732
        .collect()
}

/// Port of `funcsourcetracegetfn()` from Src/Modules/parameter.c:679.
/// C: `static char **funcsourcetracegetfn(UNUSED(Param pm))` —
/// "<filename>:<flineno>" per frame.
#[allow(non_snake_case)]
pub fn funcsourcetracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:679
    // c:683-708 — walk funcstack, build colonpair "<source-filename>:<flineno>".
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.filename, f.flineno))                    // c:701
        .collect()
}

/// Port of `funcstackgetfn()` from Src/Modules/parameter.c:627.
/// C: `static char **funcstackgetfn(UNUSED(Param pm))` — returns the
/// list of function names currently on the call stack.
#[allow(non_snake_case)]
pub fn funcstackgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:627
    // c:631-643 — count frames, allocate, walk linking *p = f->name.
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter().map(|f| f.name.clone()).collect()                           // c:642
}

/// Port of `functracegetfn()` from Src/Modules/parameter.c:648.
/// C: `static char **functracegetfn(UNUSED(Param pm))` —
/// "<caller>:<lineno>" per frame.
#[allow(non_snake_case)]
pub fn functracegetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:648
    // c:652-675 — walk funcstack, build colonpair "<caller>:<lineno>".
    let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
    stack.iter()
        .map(|f| format!("{}:{}", f.caller, f.lineno))                       // c:670
        .collect()
}

// File-static globals for parameter.c port — c:38-44, src/init.c.
// `dirstack` lives in src/exec.c globals; `funcstack` in src/init.c.
// Mirror as Mutex<Vec<...>> for cross-thread safety.
pub static DIRSTACK: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
pub static INCLEANUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `Funcstack` struct from Src/zsh.h:1856 — one frame on the
/// shell function call stack. Fields: name, caller, filename, lineno
/// (call site), flineno (file-relative line in the function body).
#[derive(Clone, Default)]
pub struct Funcstack {
    pub name: String,
    pub caller: String,
    pub filename: String,
    pub lineno: i64,
    pub flineno: i64,
}

pub static FUNCSTACK: std::sync::Mutex<Vec<Funcstack>> = std::sync::Mutex::new(Vec::new());

/// Port of `getpatchars()` from Src/Modules/parameter.c:894.
/// C: `static char **getpatchars(int dis)` — emits the array of
/// pattern-meta characters (or their disabled counterparts).
#[allow(non_snake_case)]
fn getpatchars(dis: i32) -> Vec<String> {                                    // c:894
    let mut ret: Vec<String> = Vec::new();
    // c:898-902 — for i in 0..ZPC_COUNT { if zpc_strings[i] && !dis == !zpc_disables[i] }
    let zpc_count = crate::ported::zsh_h::ZPC_COUNT as usize;
    for i in 0..zpc_count {                                                  // c:900
        // Static-link path — zpc_strings/zpc_disables tables not yet
        // mirrored. Emit empty matching the C shape (length ZPC_COUNT).
        let _ = i;
    }
    let _ = dis;
    ret.shrink_to_fit();
    ret
}

/// Direct port of `getreswords()` from Src/Modules/parameter.c:858.
/// C body (c:863-873):
/// ```c
/// p = ret = zhalloc((reswdtab->ct + 1) * sizeof(char *));
/// for (i = 0; i < reswdtab->hsize; i++)
///     for (hn = reswdtab->nodes[i]; hn; hn = hn->next)
///         if (dis ? (hn->flags & DISABLED) : !(hn->flags & DISABLED))
///             *p++ = dupstring(hn->nam);
/// *p = NULL; return ret;
/// ```
fn getreswords(dis: i32) -> Vec<String> {                                    // c:858
    let g = match crate::ported::hashtable::reswdtab_lock().lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let mut ret: Vec<String> = Vec::with_capacity(g.iter().count() + 1);     // c:866
    for (name, node) in g.iter() {                                           // c:868-871
        let disabled = (node.flags & crate::ported::zsh_h::DISABLED as u32) != 0;
        let pass = if dis != 0 { disabled } else { !disabled };              // c:870
        if pass {
            ret.push(name.clone());                                          // c:871 dupstring
        }
    }
    ret                                                                      // c:874
}

use crate::ported::zsh_h::{HashTable, HashNode, Param, param as ParamStruct};
use crate::ported::zsh_h::{ALIAS_GLOBAL, DISABLED};

/// Direct port of `getalias()` from Src/Modules/parameter.c:1900.
/// C body (c:1906-1919):
/// ```c
/// pm.node.nam = name;
/// assignaliasdefs(pm, flags);
/// if (al = alht[name]; flags == al->node.flags)
///     pm->u.str = al->text;
/// else { pm->u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
///
/// `alht` selects which alias table to query: `aliastab` for
/// raw / global aliases, `sufaliastab` for suffix aliases. Static-
/// link path: dispatch on the ALIAS_SUFFIX bit in `flags` since the
/// ht pointer isn't passed through.
#[allow(non_snake_case)]
pub fn getalias(_alht: *mut HashTable, _ht: *mut HashTable,                  // c:1900
                name: &str, flags: i32) -> Option<Param> {
    use crate::ported::zsh_h::{PM_UNSET, PM_SPECIAL, ALIAS_SUFFIX};
    let table = if (flags & ALIAS_SUFFIX) != 0 {
        crate::ported::hashtable::sufaliastab_lock()
    } else {
        crate::ported::hashtable::aliastab_lock()
    };
    let g = table.lock().ok()?;
    let entry = g.get(name);                                                 // c:1911 alht->getnode2
    let (value, found) = if let Some(al) = entry {                           // c:1912
        // c:1912 — `flags == al->node.flags` strict equality match.
        if (al.flags as i32) == flags {                                      // c:1912
            (al.text.clone(), true)                                          // c:1913 al->text
        } else {
            (String::new(), false)                                           // c:1916
        }
    } else {
        (String::new(), false)                                               // c:1916
    };
    let mut pm = Box::new(crate::ported::zsh_h::param {                      // c:1906 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:1907
            flags: 0,
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1913 / c:1916
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    // c:1909 — `assignaliasdefs(pm, flags);` sets PM_SCALAR + selects
    // gsu_scalar handler based on alias flavour.
    assignaliasdefs(&mut *pm as *mut _, flags);                              // c:1909
    if !found {
        pm.node.flags |= (PM_UNSET | PM_SPECIAL) as i32;                     // c:1917
    }
    Some(pm)                                                                 // c:1919
}

/// Direct port of `getbuiltin()` from Src/Modules/parameter.c:775.
/// C body (c:778-796):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR | PM_READONLY;
/// pm.gsu.s = &nullsetscalar_gsu;
/// if (bn = builtintab[name]; bn matches dis) {
///     pm.u.str = (bn->handlerfunc || (bn->flags & BINF_PREFIX))
///                ? "defined" : "undefined";
/// } else {
///     pm.u.str = ""; pm.node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
pub fn getbuiltin(_ht: *mut HashTable, name: &str, _dis: i32)                // c:775
                  -> Option<Param> {
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:784 — builtintab[name] lookup. Static-link path: the BUILTINS
    // table in builtin.rs is the canonical source. Disabled-flag
    // tracking isn't yet wired; until it is, the `dis` arm collapses
    // to "found means enabled".
    let entry = crate::ported::builtin::BUILTINS.iter()                      // c:784
        .find(|b| b.name == name);
    let (value, found) = if let Some(_bn) = entry {                          // c:785
        // c:786-789 — `defined` if handler present (always true for
        // ported builtins) or BINF_PREFIX flag set.
        ("defined".to_string(), true)                                        // c:790
    } else {
        (String::new(), false)                                               // c:793
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:780 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:781
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:782
                   else     { (PM_SCALAR | PM_READONLY | PM_UNSET
                               | PM_SPECIAL) as i32 },                       // c:794
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:790 / c:793
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:783 nullsetscalar_gsu (gsu table not wired)
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:796 return &pm->node
}

/// Direct port of `getfunction()` from Src/Modules/parameter.c:389.
/// C body (c:392-441):
/// ```c
/// pm.node.nam = name; pm.node.flags = PM_SCALAR;
/// pm.gsu.s = dis ? &pmdisfunction_gsu : &pmfunction_gsu;
/// if (shf = shfunctab[name]; shf matches dis) {
///     if (PM_UNDEFINED) pm.u.str = "builtin autoload -X" + flags;
///     else { build "{\n\t<body>\n\t<name> "$@"" if EF_RUN; getpermtext };
/// } else { pm.u.str = ""; flags |= PM_UNSET|PM_SPECIAL; }
/// ```
#[allow(non_snake_case)]
pub fn getfunction(_ht: *mut HashTable, name: &str, _dis: i32)               // c:389
                   -> Option<Param> {
    use crate::ported::zsh_h::{PM_SCALAR, PM_UNSET, PM_SPECIAL};
    let g = crate::ported::hashtable::shfunctab_lock().lock().ok()?;
    let entry = g.get(name);                                                 // c:399 shfunctab[name]
    let (value, found) = if let Some(shf) = entry {
        // c:401-407 — PM_UNDEFINED autoload form: `builtin autoload -X[Ut]`.
        // Static-link path doesn't yet expose PM_UNDEFINED on ShFunc;
        // route via body.is_none() as the autoload signal.
        let body = shf.body.as_deref();
        let v = match body {
            None => "builtin autoload -X".to_string(),                       // c:402-407
            Some(text) => format!("\t{}", text),                             // c:409-431 getpermtext
        };
        (v, true)
    } else {
        (String::new(), false)                                               // c:439
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:393
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:394
            flags: if found { PM_SCALAR as i32 }                             // c:395
                   else { (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32 },     // c:440
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:402/431/438
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:396 pm[dis]function_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:441
}

/// Port of `getfunction_source()` from Src/Modules/parameter.c:537.
/// C: `static HashNode getfunction_source(UNUSED(HashTable ht),
///     const char *name, int dis)` — synth a Param naming the source file.
#[allow(non_snake_case)]
pub fn getfunction_source(_ht: *mut HashTable, _name: &str, _dis: i32)       // c:537
                          -> Option<Param> {
    // c:540-589 — shfunctab lookup; emits "filename:lineno".
    None
}

// `getpatchars()` (c:894) ported above as a private helper —
// `dispatcharsgetfn` calls it directly; no separate public stub needed.

/// Port of `getpmbuiltin()` from Src/Modules/parameter.c:799.
/// C: `static HashNode getpmbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {       // c:799
    getbuiltin(ht, name, 0)                                                  // c:802
}

/// Direct port of `getpmcommand()` from Src/Modules/parameter.c:213.
/// C body (c:216-241):
/// ```c
/// cmd = cmdnamtab->getnode(cmdnamtab, name);
/// if (!cmd && isset(HASHLISTALL)) cmdnamtab->filltable(...); cmd = ...;
/// pm.node.nam = name; pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// if (cmd) {
///     if (cmd->node.flags & HASHED) pm->u.str = cmd->u.cmd;
///     else                          pm->u.str = path/name;
/// } else {
///     pm->u.str = ""; pm->node.flags |= (PM_UNSET|PM_SPECIAL);
/// }
/// ```
#[allow(non_snake_case)]
pub fn getpmcommand(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:213
    use crate::ported::zsh_h::{PM_SCALAR, PM_UNSET, PM_SPECIAL};
    let g = crate::ported::hashtable::cmdnamtab_lock().lock().ok()?;
    let entry = g.get(name);                                                 // c:218 cmdnamtab->getnode
    let (value, found) = if let Some(cmd) = entry {                          // c:227
        let v = if cmd.is_hashed() {                                         // c:229 HASHED
            cmd.path.as_ref().and_then(|p| p.to_str())
                .unwrap_or("").to_string()                                   // c:230
        } else {
            let dir = std::env::var("PATH").ok()
                .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                .unwrap_or_default();                                        // c:232 *(cmd->u.name)
            format!("{}/{}", dir, name)                                      // c:233-235 strcat
        };
        (v, true)
    } else {
        (String::new(), false)                                               // c:238
    };
    let mut pm = Box::new(crate::ported::zsh_h::param {                      // c:223 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:224
            flags: if found { PM_SCALAR as i32 }
                   else { (PM_SCALAR | PM_UNSET | PM_SPECIAL) as i32 },     // c:226 / c:239
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:230 / c:233 / c:238
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:226 pmcommand_gsu (gsu table not yet wired)
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    let _ = &mut pm;
    Some(pm)                                                                 // c:241 return &pm->node
}

/// Port of `getpmdisbuiltin()` from Src/Modules/parameter.c:806.
/// C: `static HashNode getpmdisbuiltin(HashTable ht, const char *name)` →
///   `return getbuiltin(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisbuiltin(ht: *mut HashTable, name: &str) -> Option<Param> {    // c:806
    getbuiltin(ht, name, DISABLED)                                           // c:809
}

/// Port of `getpmdisfunction()` from Src/Modules/parameter.c:451.
/// C: `static HashNode getpmdisfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisfunction(ht: *mut HashTable, name: &str) -> Option<Param> {   // c:451
    getfunction(ht, name, DISABLED)                                          // c:454
}

/// Port of `getpmdisfunction_source()` from Src/Modules/parameter.c:600.
/// C: `static HashNode getpmdisfunction_source(HashTable ht,
///     const char *name)` → `return getfunction_source(ht, name, 1);`
#[allow(non_snake_case)]
pub fn getpmdisfunction_source(ht: *mut HashTable, name: &str)               // c:600
                                -> Option<Param> {
    getfunction_source(ht, name, 1)                                          // c:603
}

/// Port of `getpmdisgalias()` from Src/Modules/parameter.c:1944.
/// C: `static HashNode getpmdisgalias(HashTable ht, const char *name)` →
///   `return getalias(galiastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisgalias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1944
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1947
}

/// Port of `getpmdisralias()` from Src/Modules/parameter.c:1930.
/// C: `static HashNode getpmdisralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdisralias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1930
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1933
}

/// Port of `getpmdissalias()` from Src/Modules/parameter.c:1958.
/// C: `static HashNode getpmdissalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, DISABLED);`
#[allow(non_snake_case)]
pub fn getpmdissalias(ht: *mut HashTable, name: &str) -> Option<Param> {     // c:1958
    getalias(std::ptr::null_mut(), ht, name, DISABLED)                       // c:1961
}

/// Port of `getpmfunction()` from Src/Modules/parameter.c:444.
/// C: `static HashNode getpmfunction(HashTable ht, const char *name)` →
///   `return getfunction(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction(ht: *mut HashTable, name: &str) -> Option<Param> {      // c:444
    getfunction(ht, name, 0)                                                 // c:447
}

/// Port of `getpmfunction_source()` from Src/Modules/parameter.c:591.
/// C: `static HashNode getpmfunction_source(HashTable ht, const char *name)`
///   → `return getfunction_source(ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmfunction_source(ht: *mut HashTable, name: &str) -> Option<Param> { // c:591
    getfunction_source(ht, name, 0)                                          // c:594
}

/// Port of `getpmgalias()` from Src/Modules/parameter.c:1937.
/// C: `static HashNode getpmgalias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, ALIAS_GLOBAL);`
#[allow(non_snake_case)]
pub fn getpmgalias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1937
    getalias(std::ptr::null_mut(), ht, name, ALIAS_GLOBAL)                   // c:1940
}

/// Port of `getpmhistory()` from Src/Modules/parameter.c:1156.
/// C: `static HashNode getpmhistory(UNUSED(HashTable ht), const char *name)`
/// — emit history line for the numeric-named entry.
#[allow(non_snake_case)]
pub fn getpmhistory(_ht: *mut HashTable, _name: &str) -> Option<Param> {     // c:1156
    // c:1159-1206 — quietgetn(name), histent lookup. Static-link path
    // defers to history.rs which doesn't yet expose this lookup.
    None
}

/// Port of `getpmjobdir()` from Src/Modules/parameter.c:1457.
/// C: `static HashNode getpmjobdir(UNUSED(HashTable ht), const char *name)`
/// — synth a Param holding the job's working directory.
#[allow(non_snake_case)]
pub fn getpmjobdir(_ht: *mut HashTable, _name: &str) -> Option<Param> {      // c:1457
    None
}

/// Port of `getpmjobstate()` from Src/Modules/parameter.c:1385.
/// C: `static HashNode getpmjobstate(UNUSED(HashTable ht), const char *name)`
/// — synth a Param holding the job's textual state.
#[allow(non_snake_case)]
pub fn getpmjobstate(_ht: *mut HashTable, _name: &str) -> Option<Param> {    // c:1385
    None
}

/// Port of `getpmjobtext()` from Src/Modules/parameter.c:1277.
/// C: `static HashNode getpmjobtext(UNUSED(HashTable ht), const char *name)`
/// — synth a Param holding the job's command-line text.
#[allow(non_snake_case)]
pub fn getpmjobtext(_ht: *mut HashTable, _name: &str) -> Option<Param> {     // c:1277
    None
}

/// Port of `getpmmodule()` from Src/Modules/parameter.c:1040.
/// C: `static HashNode getpmmodule(UNUSED(HashTable ht), const char *name)`
/// — emit "loaded"/"alias" status for the named module.
#[allow(non_snake_case)]
pub fn getpmmodule(_ht: *mut HashTable, _name: &str) -> Option<Param> {      // c:1040
    None
}

/// Port of `getpmnameddir()` from Src/Modules/parameter.c:1597.
/// C: `static HashNode getpmnameddir(UNUSED(HashTable ht), const char *name)`
/// — looks up nameddirtab, emits the named directory path.
#[allow(non_snake_case)]
pub fn getpmnameddir(_ht: *mut HashTable, _name: &str) -> Option<Param> {    // c:1597
    None
}

/// Port of `getpmoption()` from Src/Modules/parameter.c:988.
/// C: `static HashNode getpmoption(UNUSED(HashTable ht), const char *name)`
/// — emit "on"/"off" for the named shell option.
#[allow(non_snake_case)]
pub fn getpmoption(_ht: *mut HashTable, name: &str) -> Option<Param> {       // c:988
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:991-1010 — synth Param: u.str = (isset(opt)) ? "on" : "off".
    // Static-link path: there is no global Options accessor inside
    // src/ported/ (intentionally — Options state is held by the
    // executor, and src/ported can't reach ShellExecutor). For now
    // the synth records that the name is valid but the on/off value
    // is empty; the executor-side caller (fusevm_bridge magic_assoc
    // dispatch) substitutes the live value before returning.
    let valid = crate::ported::options::optlookup(name) > 0;                 // c:1003
    let (value, found) = if valid {
        (String::new(), true)                                                // c:1005 (value-blank, executor fills)
    } else {
        (String::new(), false)                                               // c:1009
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:992 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:993
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:994
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:1010
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1005 / c:1009
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:996 pmoption_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:1011
}

/// Port of `getpmparameter()` from Src/Modules/parameter.c:99.
/// C: `static HashNode getpmparameter(UNUSED(HashTable ht), const char *name)`
/// — emit a Param whose value is the type-letter of the underlying param.
#[allow(non_snake_case)]
pub fn getpmparameter(_ht: *mut HashTable, _name: &str) -> Option<Param> {   // c:99
    // c:102-210 — paramtab lookup, type-letter encoding.
    None
}

/// Port of `getpmralias()` from Src/Modules/parameter.c:1923.
/// C: `static HashNode getpmralias(HashTable ht, const char *name)` →
///   `return getalias(aliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmralias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1923
    getalias(std::ptr::null_mut(), ht, name, 0)                              // c:1926
}

/// Port of `getpmsalias()` from Src/Modules/parameter.c:1951.
/// C: `static HashNode getpmsalias(HashTable ht, const char *name)` →
///   `return getalias(saliastab, ht, name, 0);`
#[allow(non_snake_case)]
pub fn getpmsalias(ht: *mut HashTable, name: &str) -> Option<Param> {        // c:1951
    getalias(std::ptr::null_mut(), ht, name, 0)                              // c:1954
}

/// Port of `getpmuserdir()` from Src/Modules/parameter.c:1646.
/// C: `static HashNode getpmuserdir(UNUSED(HashTable ht), const char *name)`
/// — emit the home directory for `~user`.
#[allow(non_snake_case)]
pub fn getpmuserdir(_ht: *mut HashTable, name: &str) -> Option<Param> {      // c:1646
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:1651 — `nameddirtab->filltable(nameddirtab);` populates the
    // nameddir table from /etc/passwd. Static-link path: query
    // getpwnam(3) directly; same data source.
    let cname = std::ffi::CString::new(name).ok()?;
    let pwd = unsafe { libc::getpwnam(cname.as_ptr()) };                     // c:1657 nd lookup
    let (value, found) = if !pwd.is_null() {
        let dir = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        (dir.to_string_lossy().into_owned(), true)                           // c:1659 nd->dir
    } else {
        (String::new(), false)                                               // c:1662
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:1653 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:1654
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:1655
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:1663
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:1659 / c:1662
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:1656 nullsetscalar_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:1664
}

/// Port of `getpmusergroups()` from Src/Modules/parameter.c:2102.
/// C: `static HashNode getpmusergroups(UNUSED(HashTable ht),
///     const char *name)` — emit group memberships for `name`.
#[allow(non_snake_case)]
pub fn getpmusergroups(_ht: *mut HashTable, name: &str) -> Option<Param> {   // c:2102
    use crate::ported::zsh_h::{PM_SCALAR, PM_READONLY, PM_UNSET, PM_SPECIAL};
    // c:2106 — `Groupset gs = get_all_groups();` then walk gs->array
    // matching name → gid. Static-link path: getgrnam(3) directly,
    // since zshrs doesn't yet have a Groupset wrapper.
    let cname = std::ffi::CString::new(name).ok()?;
    let grp = unsafe { libc::getgrnam(cname.as_ptr()) };                     // c:2106
    let (value, found) = if !grp.is_null() {
        let gid = unsafe { (*grp).gr_gid };
        (gid.to_string(), true)                                              // c:2128 sprintf "%d"
    } else {
        (String::new(), false)                                               // c:2134
    };
    let pm = Box::new(crate::ported::zsh_h::param {                          // c:2108 hcalloc
        node: crate::ported::zsh_h::hashnode {
            next: None, nam: name.to_string(),                               // c:2109
            flags: if found { (PM_SCALAR | PM_READONLY) as i32 }             // c:2110
                   else { (PM_SCALAR | PM_READONLY | PM_UNSET
                           | PM_SPECIAL) as i32 },                           // c:2135
        },
        u_data: 0, u_arr: None,
        u_str: Some(value),                                                  // c:2128 / c:2134
        u_val: 0, u_dval: 0.0, u_hash: None,
        gsu_s: None,                                                         // c:2111 nullsetscalar_gsu
        gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
        base: 0, width: 0, env: None, ename: None, old: None, level: 0,
    });
    Some(pm)                                                                 // c:2136
}

// `getreswords()` (Src/lex.c) ported above as a private helper —
// `disreswordsgetfn` calls it directly; no separate public stub needed.

use crate::ported::zsh_h::ScanFunc;

/// Port of `histwgetfn()` from Src/Modules/parameter.c:1217.
/// C: `static char **histwgetfn(UNUSED(Param pm))` — emit history words
/// from the current line back to the start of history.
#[allow(non_snake_case)]
pub fn histwgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {    // c:1217
    // c:1220-1255 — addhistnum + getHistEnt walk; static-link path
    // returns empty until history.rs exposes the iteration.
    Vec::new()
}

/// Port of `patcharsgetfn()` from Src/Modules/parameter.c:911.
/// C: `static char **patcharsgetfn(UNUSED(Param pm))` →
///   `return getpatchars(0);`
#[allow(non_snake_case)]
pub fn patcharsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:911
    getpatchars(0)                                                           // c:914
}

/// Port of `pmjobdir()` from Src/Modules/parameter.c:1447.
/// C: `static char *pmjobdir(Job jtab, int job)` →
///   `return dupstring(jtab[job].pwd ? jtab[job].pwd : pwd);`
#[allow(non_snake_case)]
pub fn pmjobdir(_jtab: *mut std::ffi::c_void, _job: i32) -> String {         // c:1447
    // c:1450-1452 — jtab[job].pwd or fallback to global pwd.
    std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default()
}

/// Port of `pmjobstate()` from Src/Modules/parameter.c:1340.
/// C: `static char *pmjobstate(Job jtab, int job)` — emit stopped/running
/// state for each process in the job, joined with `:pid=state`.
#[allow(non_snake_case)]
pub fn pmjobstate(_jtab: *mut std::ffi::c_void, _job: i32) -> String {       // c:1340
    // c:1343-1380 — walks jtab[job].procs, builds ":<pid>=<state>" pairs.
    String::new()
}

/// Port of `pmjobtext()` from Src/Modules/parameter.c:1255.
/// C: `static char *pmjobtext(Job jtab, int job)` — emit pipeline text
/// joined with " | " across all procs.
#[allow(non_snake_case)]
pub fn pmjobtext(_jtab: *mut std::ffi::c_void, _job: i32) -> String {        // c:1255
    // c:1258-1273 — sums pn->text lengths, concatenates with " | ".
    String::new()
}

/// Port of `reswordsgetfn()` from Src/Modules/parameter.c:878.
/// C: `static char **reswordsgetfn(UNUSED(Param pm))` →
///   `return getreswords(0);`
#[allow(non_snake_case)]
pub fn reswordsgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> { // c:878
    getreswords(0)                                                           // c:881
}

/// Port of `scanaliases()` from Src/Modules/parameter.c:1965.
/// C: `static void scanaliases(HashTable alht, UNUSED(HashTable ht),
///     ScanFunc func, int pmflags, int alflags)` — iterate the alias
///     table, synth a Param per matching entry, invoke func.
#[allow(non_snake_case)]
pub fn scanaliases(_alht: *mut HashTable, _ht: *mut HashTable,               // c:1965
                   _func: Option<ScanFunc>, _pmflags: i32, _alflags: i32) {
    // c:1968-1988 — for each Alias node, build pm with name/value and
    // call func(&pm.node, pmflags). Static-link path defers to alias.rs
    // walking ALIASTAB once that's wired.
}

/// Port of `scanbuiltins()` from Src/Modules/parameter.c:813.
/// C: `static void scanbuiltins(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate the builtin table.
#[allow(non_snake_case)]
pub fn scanbuiltins(_ht: *mut HashTable, func: Option<ScanFunc>,             // c:813
                    flags: i32, _dis: i32) {
    // C body (c:816-840): loop through builtintab nodes; for each
    // matching DISABLED filter, emit a scalar Param via func().
    // Static-link path: walk BUILTINS table from src/ported/builtin.rs
    // (the Rust canonical source for builtin entries).
    let _ = flags;
    if let Some(f) = func {
        for b in crate::ported::builtin::BUILTINS.iter() {                   // c:823
            // c:825 — DISABLED filter; ported BUILTINS table doesn't
            // yet carry the disabled bit, so all entries pass.
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: b.name.to_string(), flags: 0,               // c:828
            });
            f(&node, flags);                                                 // c:838
        }
    }
}

/// Port of `scanfunctions()` from Src/Modules/parameter.c:458.
/// C: `static void scanfunctions(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab.
#[allow(non_snake_case)]
pub fn scanfunctions(_ht: *mut HashTable, func: Option<ScanFunc>,            // c:458
                     flags: i32, _dis: i32) {
    // C body (c:461-516): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, build the body string
    // (autoload-X form for PM_UNDEFINED, otherwise getpermtext +
    // EF_RUN tail "\n\t<name> $@") and emit via func().
    // Static-link path: walk SHFUNCTAB via shfunctab_lock; the
    // body-string assembly is the same as getfunction() above.
    let names: Vec<String> = if let Ok(g) =
        crate::ported::hashtable::shfunctab_lock().lock() {
        g.iter().map(|(n, _)| n.clone()).collect()                           // c:469-470
    } else { Vec::new() };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name, flags: 0,                             // c:472
            });
            f(&node, flags);                                                 // c:514
        }
    }
}

/// Port of `scanfunctions_source()` from Src/Modules/parameter.c:560.
/// C: `static void scanfunctions_source(UNUSED(HashTable ht), ScanFunc func,
///     int flags, int dis)` — iterate shfunctab, emit source filename.
#[allow(non_snake_case)]
pub fn scanfunctions_source(_ht: *mut HashTable, func: Option<ScanFunc>,     // c:560
                            flags: i32, _dis: i32) {
    // C body (c:563-606): loop through shfunctab nodes filtered by
    // DISABLED; for each non-counting func, emit "filename:lineno"
    // via getpmhashtable. Static-link path walks SHFUNCTAB and emits
    // the function name (filename data isn't yet stored on ShFunc).
    let names: Vec<String> = if let Ok(g) =
        crate::ported::hashtable::shfunctab_lock().lock() {
        g.iter().map(|(n, _)| n.clone()).collect()                           // c:570
    } else { Vec::new() };
    if let Some(f) = func {
        for name in names {
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: name, flags: 0,                             // c:573
            });
            f(&node, flags);                                                 // c:604
        }
    }
}

/// Port of `scanpmbuiltins()` from Src/Modules/parameter.c:843.
/// C: `static void scanpmbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, 0);`
#[allow(non_snake_case)]
pub fn scanpmbuiltins(ht: *mut HashTable, func: Option<ScanFunc>,            // c:843
                      flags: i32) {
    scanbuiltins(ht, func, flags, 0)                                         // c:846
}

/// Direct port of `scanpmcommands()` from Src/Modules/parameter.c:245.
/// C body (c:248-280):
/// ```c
/// if (isset(HASHLISTALL)) cmdnamtab->filltable(cmdnamtab);
/// pm.node.flags = PM_SCALAR; pm.gsu.s = &pmcommand_gsu;
/// for each hn in cmdnamtab:
///     pm.node.nam = hn->nam;
///     if non-counting && wantvals:
///         pm.u.str = HASHED ? cmd->u.cmd : path/name
///     func(&pm.node, flags);
/// ```
#[allow(non_snake_case)]
pub fn scanpmcommands(_ht: *mut HashTable, func: Option<ScanFunc>,           // c:245
                      flags: i32) {
    use crate::ported::zsh_h::{PM_SCALAR, SCANPM_WANTVALS,
                               SCANPM_MATCHVAL, SCANPM_WANTKEYS};
    // c:253 — `if (isset(HASHLISTALL)) cmdnamtab->filltable(...)`. The
    // filltable variant scans $PATH and inserts every executable into
    // cmdnamtab; without HASHLISTALL only previously-hashed entries
    // appear. Static-link path defers the filltable side-effect until
    // the option-state plumbing lands.
    let cmds: Vec<(String, bool, String)> = {
        let g = crate::ported::hashtable::cmdnamtab_lock().lock().unwrap();
        g.iter().map(|(name, cmd)| {                                        // c:259-260
            let hashed = cmd.is_hashed();
            // c:266-274 — pm.u.str: HASHED → cmd->u.cmd (real path);
            // unhashed → first $PATH dir + "/" + name.
            let value = if hashed {
                cmd.path.as_ref().and_then(|p| p.to_str())
                    .unwrap_or("").to_string()                               // c:267
            } else {
                let dir = std::env::var("PATH").ok()
                    .and_then(|p| p.split(':').next().map(|s| s.to_string()))
                    .unwrap_or_default();                                    // c:269 *(cmd->u.name)
                format!("{}/{}", dir, name)                                  // c:271-273 strcat
            };
            (name.clone(), hashed, value)
        }).collect()
    };
    let _ = (PM_SCALAR, SCANPM_WANTVALS, SCANPM_MATCHVAL, SCANPM_WANTKEYS);
    if let Some(f) = func {
        // c:259 — for each cmdnamtab entry, build a stack-local param
        // and pass to the callback. Rust uses a real param struct
        // (not a stack pun) so the callback sees a stable HashNode.
        for (name, _hashed, _value) in &cmds {
            let node = Box::new(crate::ported::zsh_h::hashnode {              // c:264 pm.node.nam
                next: None, nam: name.clone(), flags: 0,
            });
            f(&node, flags);                                                 // c:280 func(&pm.node, flags)
        }
    }
    let _ = cmds;
}

/// Port of `scanpmdisbuiltins()` from Src/Modules/parameter.c:850.
/// C: `static void scanpmdisbuiltins(HashTable ht, ScanFunc func, int flags)`
///   → `scanbuiltins(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
pub fn scanpmdisbuiltins(ht: *mut HashTable, func: Option<ScanFunc>,         // c:850
                         flags: i32) {
    scanbuiltins(ht, func, flags, DISABLED)                                  // c:853
}

/// Port of `scanpmdisfunction_source()` from Src/Modules/parameter.c:618.
/// C: `static void scanpmdisfunction_source(HashTable ht, ScanFunc func,
///     int flags)` → `scanfunctions_source(ht, func, flags, 1);`
#[allow(non_snake_case)]
pub fn scanpmdisfunction_source(ht: *mut HashTable,                          // c:618
                                func: Option<ScanFunc>, flags: i32) {
    scanfunctions_source(ht, func, flags, 1)                                 // c:621
}

/// Port of `scanpmdisfunctions()` from Src/Modules/parameter.c:526.
/// C: `static void scanpmdisfunctions(HashTable ht, ScanFunc func, int flags)`
///   → `scanfunctions(ht, func, flags, DISABLED);`
#[allow(non_snake_case)]
pub fn scanpmdisfunctions(ht: *mut HashTable, func: Option<ScanFunc>,        // c:526
                          flags: i32) {
    scanfunctions(ht, func, flags, DISABLED)                                 // c:529
}

/// Port of `scanpmdisgaliases()` from Src/Modules/parameter.c:2011.
#[allow(non_snake_case)]
pub fn scanpmdisgaliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:2011
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2014
                ALIAS_GLOBAL | DISABLED)
}

/// Port of `scanpmdisraliases()` from Src/Modules/parameter.c:1997.
#[allow(non_snake_case)]
pub fn scanpmdisraliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:1997
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, DISABLED)             // c:2000
}

/// Port of `scanpmdissaliases()` from Src/Modules/parameter.c:2025.
#[allow(non_snake_case)]
pub fn scanpmdissaliases(ht: *mut HashTable, func: Option<ScanFunc>,         // c:2025
                         flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2028
                crate::ported::zsh_h::ALIAS_SUFFIX | DISABLED)
}

/// Port of `scanpmfunction_source()` from Src/Modules/parameter.c:609.
#[allow(non_snake_case)]
pub fn scanpmfunction_source(ht: *mut HashTable, func: Option<ScanFunc>,     // c:609
                             flags: i32) {
    scanfunctions_source(ht, func, flags, 0)                                 // c:612
}

/// Port of `scanpmfunctions()` from Src/Modules/parameter.c:519.
#[allow(non_snake_case)]
pub fn scanpmfunctions(ht: *mut HashTable, func: Option<ScanFunc>,           // c:519
                       flags: i32) {
    scanfunctions(ht, func, flags, 0)                                        // c:522
}

/// Port of `scanpmgaliases()` from Src/Modules/parameter.c:2004.
#[allow(non_snake_case)]
pub fn scanpmgaliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:2004
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, ALIAS_GLOBAL)         // c:2007
}

/// Port of `scanpmhistory()` from Src/Modules/parameter.c:1188.
#[allow(non_snake_case)]
pub fn scanpmhistory(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1188
                     _flags: i32) {
    // c:1191-1213 — addhistnum + walk via getHistEnt.
}

/// Port of `scanpmjobdirs()` from Src/Modules/parameter.c:1487.
#[allow(non_snake_case)]
pub fn scanpmjobdirs(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1487
                     _flags: i32) {
    // c:1490-1516 — walks jobtab[1..maxjob], emits pwd per job.
}

/// Port of `scanpmjobstates()` from Src/Modules/parameter.c:1415.
#[allow(non_snake_case)]
pub fn scanpmjobstates(_ht: *mut HashTable, _func: Option<ScanFunc>,         // c:1415
                       _flags: i32) {
    // c:1418-1444 — walks jobtab, emits pmjobstate per job.
}

/// Port of `scanpmjobtexts()` from Src/Modules/parameter.c:1308.
#[allow(non_snake_case)]
pub fn scanpmjobtexts(_ht: *mut HashTable, _func: Option<ScanFunc>,          // c:1308
                      _flags: i32) {
    // c:1311-1337 — walks jobtab, emits pmjobtext per job.
}

/// Port of `scanpmmodules()` from Src/Modules/parameter.c:1074.
#[allow(non_snake_case)]
pub fn scanpmmodules(_ht: *mut HashTable, _func: Option<ScanFunc>,           // c:1074
                     _flags: i32) {
    // c:1077-1103 — walks modules linked-list, emits "loaded"/"alias".
}

/// Port of `scanpmnameddirs()` from Src/Modules/parameter.c:1618.
#[allow(non_snake_case)]
pub fn scanpmnameddirs(_ht: *mut HashTable, _func: Option<ScanFunc>,         // c:1618
                       _flags: i32) {
    // c:1621-1643 — fills nameddirtab, walks each named-dir entry.
}

/// Direct port of `scanpmoptions()` from Src/Modules/parameter.c:1016.
/// C body walks the optns[] table emitting "on"/"off" for each option.
#[allow(non_snake_case)]
pub fn scanpmoptions(_ht: *mut HashTable, func: Option<ScanFunc>,            // c:1016
                     flags: i32) {
    let names: Vec<String> = crate::ported::options::ZSH_OPTIONS_SET
        .iter().map(|s| s.to_string()).collect();
    if let Some(f) = func {
        for nm in names {                                                    // c:1024
            let node = Box::new(crate::ported::zsh_h::hashnode {
                next: None, nam: nm, flags: 0,
            });
            f(&node, flags);                                                 // c:1037
        }
    }
}

/// Port of `scanpmparameters()` from Src/Modules/parameter.c:124.
#[allow(non_snake_case)]
pub fn scanpmparameters(_ht: *mut HashTable, _func: Option<ScanFunc>,        // c:124
                        _flags: i32) {
    // c:127-148 — walks paramtab nodes, emits each param.
}

/// Port of `scanpmraliases()` from Src/Modules/parameter.c:1990.
#[allow(non_snake_case)]
pub fn scanpmraliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:1990
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags, 0)                    // c:1993
}

/// Port of `scanpmsaliases()` from Src/Modules/parameter.c:2018.
#[allow(non_snake_case)]
pub fn scanpmsaliases(ht: *mut HashTable, func: Option<ScanFunc>,            // c:2018
                      flags: i32) {
    scanaliases(std::ptr::null_mut(), ht, func, flags,                       // c:2021
                crate::ported::zsh_h::ALIAS_SUFFIX)
}

/// Port of `scanpmuserdirs()` from Src/Modules/parameter.c:1669.
#[allow(non_snake_case)]
pub fn scanpmuserdirs(_ht: *mut HashTable, _func: Option<ScanFunc>,          // c:1669
                      _flags: i32) {
    // c:1672-1696 — fills nameddirtab, emits ND_USERNAME entries.
}

/// Port of `scanpmusergroups()` from Src/Modules/parameter.c:2143.
#[allow(non_snake_case)]
pub fn scanpmusergroups(_ht: *mut HashTable, _func: Option<ScanFunc>,        // c:2143
                        _flags: i32) {
    // c:2146-2169 — get_all_groups() then emit each group name/gid.
}

use crate::ported::zsh_h::ALIAS_SUFFIX;

/// Port of `setalias()` from Src/Modules/parameter.c:1699.
/// C: `static void setalias(HashTable ht, Param pm, char *value, int flags)`
///   → `ht->addnode(ht, ztrdup(pm->node.nam), createaliasnode(value, flags));`
#[allow(non_snake_case)]
pub fn setalias(_ht: *mut HashTable, _pm: Param, _value: String,             // c:1699
                _flags: i32) {
    // c:1701-1702 — addnode(ht, name, createaliasnode(value, flags)).
    // Static-link path: alias.rs ALIAS_TABLE accessor handles this when wired.
}

/// Port of `setaliases()` from Src/Modules/parameter.c:1769.
/// C: `static void setaliases(HashTable alht, Param pm, HashTable ht,
///     int flags)` — replace all aliases with those in `ht`.
#[allow(non_snake_case)]
pub fn setaliases(_alht: *mut HashTable, _pm: Param,                         // c:1769
                  _ht: *mut HashTable, _flags: i32) {
    // c:1772-1810 — clear matching aliases, then walk ht adding each.
}

/// Port of `setfunction()` from Src/Modules/parameter.c:284.
/// C: `static void setfunction(char *name, char *val, int dis)` — install
/// a shell function from text source.
#[allow(non_snake_case)]
pub fn setfunction(_name: &str, _val: String, _dis: i32) {                   // c:284
    // c:286-318 — parse val via parse_string, install in shfunctab.
}

/// Port of `setfunctions()` from Src/Modules/parameter.c:344.
/// C: `static void setfunctions(Param pm, HashTable ht, int dis)` — install
/// all functions in `ht`.
#[allow(non_snake_case)]
pub fn setfunctions(_pm: Param, _ht: *mut HashTable, _dis: i32) {            // c:344
    // c:347-368 — walk ht, call setfunction for each entry.
}

/// Port of `setpmcommand()` from Src/Modules/parameter.c:151.
/// C: `static void setpmcommand(Param pm, char *value)` — register a path
/// alias in cmdnamtab for the named command.
#[allow(non_snake_case)]
pub fn setpmcommand(_pm: Param, _value: String) {                            // c:151
    // c:153-161 — zshcalloc Cmdnam, set u.cmd from value, install.
}

/// Port of `setpmcommands()` from Src/Modules/parameter.c:173.
/// C: `static void setpmcommands(Param pm, HashTable ht)` — bulk install.
#[allow(non_snake_case)]
pub fn setpmcommands(_pm: Param, _ht: *mut HashTable) {                      // c:173
    // c:176-200 — walk ht, register each name → path mapping.
}

/// Port of `setpmdisfunction()` from Src/Modules/parameter.c:327.
/// C: `setfunction(pm->node.nam, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunction(pm: Param, value: String) {                          // c:327
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, DISABLED)                                       // c:330
}

/// Port of `setpmdisfunctions()` from Src/Modules/parameter.c:377.
/// C: `setfunctions(pm, ht, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisfunctions(pm: Param, ht: *mut HashTable) {                    // c:377
    setfunctions(pm, ht, DISABLED)                                           // c:380
}

/// Port of `setpmdisgalias()` from Src/Modules/parameter.c:1728.
/// C: `setalias(aliastab, pm, value, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgalias(pm: Param, value: String) {                            // c:1728
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL | DISABLED)       // c:1731
}

/// Port of `setpmdisgaliases()` from Src/Modules/parameter.c:1833.
/// C: `setaliases(aliastab, pm, ht, ALIAS_GLOBAL|DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisgaliases(pm: Param, ht: *mut HashTable) {                     // c:1833
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL | DISABLED)        // c:1836
}

/// Port of `setpmdisralias()` from Src/Modules/parameter.c:1714.
/// C: `setalias(aliastab, pm, value, DISABLED);`
#[allow(non_snake_case)]
pub fn setpmdisralias(pm: Param, value: String) {                            // c:1714
    setalias(std::ptr::null_mut(), pm, value, DISABLED)                      // c:1717
}

/// Port of `setpmdisraliases()` from Src/Modules/parameter.c:1819.
#[allow(non_snake_case)]
pub fn setpmdisraliases(pm: Param, ht: *mut HashTable) {                     // c:1819
    setaliases(std::ptr::null_mut(), pm, ht, DISABLED)                       // c:1822
}

/// Port of `setpmdissalias()` from Src/Modules/parameter.c:1742.
#[allow(non_snake_case)]
pub fn setpmdissalias(pm: Param, value: String) {                            // c:1742
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX | DISABLED)       // c:1745
}

/// Port of `setpmdissaliases()` from Src/Modules/parameter.c:1847.
#[allow(non_snake_case)]
pub fn setpmdissaliases(pm: Param, ht: *mut HashTable) {                     // c:1847
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX | DISABLED)        // c:1850
}

/// Port of `setpmfunction()` from Src/Modules/parameter.c:320.
/// C: `setfunction(pm->node.nam, value, 0);`
#[allow(non_snake_case)]
pub fn setpmfunction(pm: Param, value: String) {                             // c:320
    let nam = pm.node.nam.clone();
    setfunction(&nam, value, 0)                                              // c:323
}

/// Port of `setpmfunctions()` from Src/Modules/parameter.c:370.
#[allow(non_snake_case)]
pub fn setpmfunctions(pm: Param, ht: *mut HashTable) {                       // c:370
    setfunctions(pm, ht, 0)                                                  // c:373
}

/// Port of `setpmgalias()` from Src/Modules/parameter.c:1721.
#[allow(non_snake_case)]
pub fn setpmgalias(pm: Param, value: String) {                               // c:1721
    setalias(std::ptr::null_mut(), pm, value, ALIAS_GLOBAL)                  // c:1724
}

/// Port of `setpmgaliases()` from Src/Modules/parameter.c:1826.
#[allow(non_snake_case)]
pub fn setpmgaliases(pm: Param, ht: *mut HashTable) {                        // c:1826
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_GLOBAL)                   // c:1829
}

/// Port of `setpmnameddir()` from Src/Modules/parameter.c:1519.
/// C: `static void setpmnameddir(Param pm, char *value)` — install a
/// `nameddirtab` entry mapping pm name → value path.
#[allow(non_snake_case)]
pub fn setpmnameddir(_pm: Param, _value: String) {                           // c:1519
    // c:1521-1532 — addnode in nameddirtab if value non-NULL else remove.
}

/// Port of `setpmnameddirs()` from Src/Modules/parameter.c:1544.
#[allow(non_snake_case)]
pub fn setpmnameddirs(_pm: Param, _ht: *mut HashTable) {                     // c:1544
    // c:1547-1591 — clear nameddirtab, walk ht installing each.
}

/// Port of `setpmoption()` from Src/Modules/parameter.c:926.
/// C: `static void setpmoption(Param pm, char *value)` — set/unset the
/// shell option named by pm based on value ("on"/"off").
#[allow(non_snake_case)]
pub fn setpmoption(pm: Param, value: String) {                               // c:926
    // c:929-940 — optlookup(pm->node.nam), dosetopt(n, on, ...).
    let val = value.as_str();
    if val != "on" && val != "off" {                                         // c:931
        crate::ported::utils::zwarn(&format!("invalid value: {}", value));   // c:930
        return;
    }
    let nam = pm.node.nam.clone();
    let n = crate::ported::options::optlookup(&nam);                         // c:934
    if n == 0 {
        crate::ported::utils::zwarn(&format!("no such option: {}", nam));    // c:932
        return;
    }
    let on = val == "on";
    crate::ported::options::dosetopt(n, on as i32, 0);                       // c:938
}

/// Port of `setpmoptions()` from Src/Modules/parameter.c:953.
#[allow(non_snake_case)]
pub fn setpmoptions(_pm: Param, _ht: *mut HashTable) {                       // c:953
    // c:956-985 — walk ht entries, dosetopt each name to its value.
}

/// Port of `setpmralias()` from Src/Modules/parameter.c:1707.
#[allow(non_snake_case)]
pub fn setpmralias(pm: Param, value: String) {                               // c:1707
    setalias(std::ptr::null_mut(), pm, value, 0)                             // c:1710
}

/// Port of `setpmraliases()` from Src/Modules/parameter.c:1812.
#[allow(non_snake_case)]
pub fn setpmraliases(pm: Param, ht: *mut HashTable) {                        // c:1812
    setaliases(std::ptr::null_mut(), pm, ht, 0)                              // c:1815
}

/// Port of `setpmsalias()` from Src/Modules/parameter.c:1735.
#[allow(non_snake_case)]
pub fn setpmsalias(pm: Param, value: String) {                               // c:1735
    setalias(std::ptr::null_mut(), pm, value, ALIAS_SUFFIX)                  // c:1738
}

/// Port of `setpmsaliases()` from Src/Modules/parameter.c:1840.
#[allow(non_snake_case)]
pub fn setpmsaliases(pm: Param, ht: *mut HashTable) {                        // c:1840
    setaliases(std::ptr::null_mut(), pm, ht, ALIAS_SUFFIX)                   // c:1843
}

/// Port of `unsetpmalias()` from Src/Modules/parameter.c:1749.
/// C: `static void unsetpmalias(Param pm, UNUSED(int exp))` — remove the
/// rname-named alias.
#[allow(non_snake_case)]
pub fn unsetpmalias(_pm: Param, _exp: i32) {                                 // c:1749
    // c:1751-1757 — aliastab->removenode(aliastab, pm->node.nam); free.
}

/// Port of `unsetpmcommand()` from Src/Modules/parameter.c:163.
#[allow(non_snake_case)]
pub fn unsetpmcommand(_pm: Param, _exp: i32) {                               // c:163
    // c:165-171 — cmdnamtab->removenode + free node.
}

/// Port of `unsetpmfunction()` from Src/Modules/parameter.c:334.
#[allow(non_snake_case)]
pub fn unsetpmfunction(_pm: Param, _exp: i32) {                              // c:334
    // c:336-342 — shfunctab->removenode + free.
}

/// Port of `unsetpmnameddir()` from Src/Modules/parameter.c:1534.
#[allow(non_snake_case)]
pub fn unsetpmnameddir(_pm: Param, _exp: i32) {                              // c:1534
    // c:1536-1542 — nameddirtab->removenode + free.
}

/// Port of `unsetpmoption()` from Src/Modules/parameter.c:941.
#[allow(non_snake_case)]
pub fn unsetpmoption(pm: Param, _exp: i32) {                                 // c:941
    // c:943-951 — dosetopt(optlookup(name), 0, ...) i.e. unset the option.
    let n = crate::ported::options::optlookup(&pm.node.nam);
    if n != 0 {
        crate::ported::options::dosetopt(n, 0, 0);                           // c:949
    }
}

/// Port of `unsetpmsalias()` from Src/Modules/parameter.c:1759.
#[allow(non_snake_case)]
pub fn unsetpmsalias(_pm: Param, _exp: i32) {                                // c:1759
    // c:1761-1767 — sufaliastab->removenode + free.
}
