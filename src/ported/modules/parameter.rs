//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use std::collections::HashMap;
use std::path::PathBuf;

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
pub fn param_type_str(ptype: ParamType, flags: &ParamFlags) -> String {
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
        assert_eq!(param_type_str(ParamType::Scalar, &flags), "scalar");

        let flags = ParamFlags {
            local: true,
            exported: true,
            ..Default::default()
        };
        assert_eq!(
            param_type_str(ParamType::Array, &flags),
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
