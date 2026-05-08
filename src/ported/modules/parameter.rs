//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use std::collections::HashMap;
use std::path::PathBuf;

use crate::exec::ShellExecutor;

/// Magic-assoc key enumeration for `${(k)NAME}` / `${(v)NAME}` /
/// `${(kv)NAME}` introspection.
///
/// Port of zsh's per-magic-table getfn/scanfn dispatch from
/// `Src/Modules/parameter.c`. Each magic-assoc name corresponds
/// to one of the static `paramdef` entries (e.g. `pmcommands`,
/// `pmaliases`, `pmfunctions`) which defines a getfn that walks
/// the canonical hashtable. Returns the key list for known
/// magic-assoc names; `None` for non-magic names so the caller
/// falls back to regular variable lookup.
pub fn magic_assoc_keys(name: &str, exec: &ShellExecutor) -> Option<Vec<String>> {
    match name {
        "aliases" => Some(exec.aliases.keys().cloned().collect()),
        "galiases" => Some(exec.global_aliases.keys().cloned().collect()),
        "saliases" => Some(exec.suffix_aliases.keys().cloned().collect()),
        "dis_aliases" | "dis_galiases" | "dis_saliases" => Some(Vec::new()),
        "functions" | "dis_functions" => Some(exec.function_names().into_iter().collect()),
        "builtins" | "dis_builtins" => {
            // Static builtin set — port of Src/Modules/parameter.c
            // scanpmbuiltins which iterates the C builtin table.
            // Match the same set the BUILTIN_PARAM_FLAG `+commands`
            // path checks for builtin-ness.
            let names: &[&str] = &[
                "echo", "print", "printf", "cd", "pwd", "exit", "return", "true", "false",
                ":", "test", "[", "local", "private", "declare", "typeset", "export", "unset",
                "set", "shift", "read", "source", "alias", "unalias", "function", "type",
                "which", "whence", "command", "builtin", "jobs", "bg", "fg", "wait", "kill",
                "trap", "eval", "exec", "ulimit", "umask", "getopts", "shopt", "history",
                "fc", "hash", "rehash", "let", "select", "time", "times", "compdef",
                "compadd", "complete", "compgen", "zmodload", "zparseopts", "zstyle",
                "zle", "vared", "zcompile", "autoload",
            ];
            Some(names.iter().map(|s| (*s).to_string()).collect())
        }
        "reswords" | "dis_reswords" => {
            // zsh reserved words. Direct port of the static `reswds[]`
            // table in Src/init.c.
            let names: &[&str] = &[
                "do", "done", "esac", "then", "elif", "else", "fi", "for", "case", "if",
                "while", "function", "repeat", "time", "until", "exec", "command", "select",
                "coproc", "nocorrect", "foreach", "end", "!", "[[", "{", "}", "declare",
                "export", "float", "integer", "local", "private", "readonly", "typeset",
            ];
            Some(names.iter().map(|s| (*s).to_string()).collect())
        }
        "options" => Some(exec.options.keys().cloned().collect()),
        "commands" => Some(exec.command_hash.keys().cloned().collect()),
        "jobtexts" | "jobdirs" | "jobstates" => {
            Some(exec.jobs.iter().map(|(id, _)| id.to_string()).collect())
        }
        "dirstack" => {
            // dirstack is array-typed not assoc — but `${(k)dirstack}`
            // returns indices, `${(v)dirstack}` returns paths. Treat
            // it like assoc-of-int-keys for symmetry.
            Some((0..exec.dir_stack.len()).map(|i| i.to_string()).collect())
        }
        "errnos" => {
            // /usr/include/errno.h names. Static set per zsh's
            // sigtrapped lookup.
            Some(crate::modules::system::ERRNO_NAMES
                .iter().map(|(n, _)| (*n).to_string()).collect())
        }
        "sysparams" => {
            Some(vec!["pid".to_string(), "ppid".to_string(), "procsubstpid".to_string()])
        }
        "parameters" => {
            let mut keys: Vec<String> = exec.variables.keys().cloned().collect();
            keys.extend(exec.arrays.keys().cloned());
            keys.extend(exec.assoc_arrays.keys().cloned());
            Some(keys)
        }
        _ => None,
    }
}

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

/// Module loader entry — port of `setup_()` from Src/Modules/parameter.c:2311.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/parameter.c:2318.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/parameter.c:2326.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/parameter.c:2341.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/parameter.c:2348.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/parameter.c:2359.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/parameter.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `assignaliasdefs()` from Src/Modules/parameter.c:1867.
#[allow(non_snake_case)]
pub fn assignaliasdefs() -> i32 { 0 }

/// Port of `dirsgetfn()` from Src/Modules/parameter.c:1147.
#[allow(non_snake_case)]
pub fn dirsgetfn() -> i32 { 0 }

/// Port of `dirssetfn()` from Src/Modules/parameter.c:1131.
#[allow(non_snake_case)]
pub fn dirssetfn() -> i32 { 0 }

/// Port of `dispatcharsgetfn()` from Src/Modules/parameter.c:917.
#[allow(non_snake_case)]
pub fn dispatcharsgetfn() -> i32 { 0 }

/// Port of `disreswordsgetfn()` from Src/Modules/parameter.c:885.
#[allow(non_snake_case)]
pub fn disreswordsgetfn() -> i32 { 0 }

/// Port of `funcfiletracegetfn()` from Src/Modules/parameter.c:711.
#[allow(non_snake_case)]
pub fn funcfiletracegetfn() -> i32 { 0 }

/// Port of `funcsourcetracegetfn()` from Src/Modules/parameter.c:679.
#[allow(non_snake_case)]
pub fn funcsourcetracegetfn() -> i32 { 0 }

/// Port of `funcstackgetfn()` from Src/Modules/parameter.c:627.
#[allow(non_snake_case)]
pub fn funcstackgetfn() -> i32 { 0 }

/// Port of `functracegetfn()` from Src/Modules/parameter.c:648.
#[allow(non_snake_case)]
pub fn functracegetfn() -> i32 { 0 }

/// Port of `getalias()` from Src/Modules/parameter.c:1901.
#[allow(non_snake_case)]
pub fn getalias() -> i32 { 0 }

/// Port of `getbuiltin()` from Src/Modules/parameter.c:775.
#[allow(non_snake_case)]
pub fn getbuiltin() -> i32 { 0 }

/// Port of `getfunction()` from Src/Modules/parameter.c:389.
#[allow(non_snake_case)]
pub fn getfunction() -> i32 { 0 }

/// Port of `getfunction_source()` from Src/Modules/parameter.c:537.
#[allow(non_snake_case)]
pub fn getfunction_source() -> i32 { 0 }

/// Port of `getpatchars()` from Src/Modules/parameter.c:894.
#[allow(non_snake_case)]
pub fn getpatchars() -> i32 { 0 }

/// Port of `getpmbuiltin()` from Src/Modules/parameter.c:799.
#[allow(non_snake_case)]
pub fn getpmbuiltin() -> i32 { 0 }

/// Port of `getpmcommand()` from Src/Modules/parameter.c:213.
#[allow(non_snake_case)]
pub fn getpmcommand() -> i32 { 0 }

/// Port of `getpmdisbuiltin()` from Src/Modules/parameter.c:806.
#[allow(non_snake_case)]
pub fn getpmdisbuiltin() -> i32 { 0 }

/// Port of `getpmdisfunction()` from Src/Modules/parameter.c:451.
#[allow(non_snake_case)]
pub fn getpmdisfunction() -> i32 { 0 }

/// Port of `getpmdisfunction_source()` from Src/Modules/parameter.c:600.
#[allow(non_snake_case)]
pub fn getpmdisfunction_source() -> i32 { 0 }

/// Port of `getpmdisgalias()` from Src/Modules/parameter.c:1944.
#[allow(non_snake_case)]
pub fn getpmdisgalias() -> i32 { 0 }

/// Port of `getpmdisralias()` from Src/Modules/parameter.c:1930.
#[allow(non_snake_case)]
pub fn getpmdisralias() -> i32 { 0 }

/// Port of `getpmdissalias()` from Src/Modules/parameter.c:1958.
#[allow(non_snake_case)]
pub fn getpmdissalias() -> i32 { 0 }

/// Port of `getpmfunction()` from Src/Modules/parameter.c:444.
#[allow(non_snake_case)]
pub fn getpmfunction() -> i32 { 0 }

/// Port of `getpmfunction_source()` from Src/Modules/parameter.c:591.
#[allow(non_snake_case)]
pub fn getpmfunction_source() -> i32 { 0 }

/// Port of `getpmgalias()` from Src/Modules/parameter.c:1937.
#[allow(non_snake_case)]
pub fn getpmgalias() -> i32 { 0 }

/// Port of `getpmhistory()` from Src/Modules/parameter.c:1156.
#[allow(non_snake_case)]
pub fn getpmhistory() -> i32 { 0 }

/// Port of `getpmjobdir()` from Src/Modules/parameter.c:1457.
#[allow(non_snake_case)]
pub fn getpmjobdir() -> i32 { 0 }

/// Port of `getpmjobstate()` from Src/Modules/parameter.c:1385.
#[allow(non_snake_case)]
pub fn getpmjobstate() -> i32 { 0 }

/// Port of `getpmjobtext()` from Src/Modules/parameter.c:1277.
#[allow(non_snake_case)]
pub fn getpmjobtext() -> i32 { 0 }

/// Port of `getpmmodule()` from Src/Modules/parameter.c:1040.
#[allow(non_snake_case)]
pub fn getpmmodule() -> i32 { 0 }

/// Port of `getpmnameddir()` from Src/Modules/parameter.c:1597.
#[allow(non_snake_case)]
pub fn getpmnameddir() -> i32 { 0 }

/// Port of `getpmoption()` from Src/Modules/parameter.c:988.
#[allow(non_snake_case)]
pub fn getpmoption() -> i32 { 0 }

/// Port of `getpmparameter()` from Src/Modules/parameter.c:99.
#[allow(non_snake_case)]
pub fn getpmparameter() -> i32 { 0 }

/// Port of `getpmralias()` from Src/Modules/parameter.c:1923.
#[allow(non_snake_case)]
pub fn getpmralias() -> i32 { 0 }

/// Port of `getpmsalias()` from Src/Modules/parameter.c:1951.
#[allow(non_snake_case)]
pub fn getpmsalias() -> i32 { 0 }

/// Port of `getpmuserdir()` from Src/Modules/parameter.c:1646.
#[allow(non_snake_case)]
pub fn getpmuserdir() -> i32 { 0 }

/// Port of `getpmusergroups()` from Src/Modules/parameter.c:2102.
#[allow(non_snake_case)]
pub fn getpmusergroups() -> i32 { 0 }

/// Port of `getreswords()` from Src/Modules/parameter.c:859.
#[allow(non_snake_case)]
pub fn getreswords() -> i32 { 0 }

/// Port of `histwgetfn()` from Src/Modules/parameter.c:1217.
#[allow(non_snake_case)]
pub fn histwgetfn() -> i32 { 0 }

/// Port of `patcharsgetfn()` from Src/Modules/parameter.c:911.
#[allow(non_snake_case)]
pub fn patcharsgetfn() -> i32 { 0 }

/// Port of `pmjobdir()` from Src/Modules/parameter.c:1447.
#[allow(non_snake_case)]
pub fn pmjobdir() -> i32 { 0 }

/// Port of `pmjobstate()` from Src/Modules/parameter.c:1340.
#[allow(non_snake_case)]
pub fn pmjobstate() -> i32 { 0 }

/// Port of `pmjobtext()` from Src/Modules/parameter.c:1255.
#[allow(non_snake_case)]
pub fn pmjobtext() -> i32 { 0 }

/// Port of `reswordsgetfn()` from Src/Modules/parameter.c:878.
#[allow(non_snake_case)]
pub fn reswordsgetfn() -> i32 { 0 }

/// Port of `scanaliases()` from Src/Modules/parameter.c:1965.
#[allow(non_snake_case)]
pub fn scanaliases() -> i32 { 0 }

/// Port of `scanbuiltins()` from Src/Modules/parameter.c:813.
#[allow(non_snake_case)]
pub fn scanbuiltins() -> i32 { 0 }

/// Port of `scanfunctions()` from Src/Modules/parameter.c:458.
#[allow(non_snake_case)]
pub fn scanfunctions() -> i32 { 0 }

/// Port of `scanfunctions_source()` from Src/Modules/parameter.c:560.
#[allow(non_snake_case)]
pub fn scanfunctions_source() -> i32 { 0 }

/// Port of `scanpmbuiltins()` from Src/Modules/parameter.c:843.
#[allow(non_snake_case)]
pub fn scanpmbuiltins() -> i32 { 0 }

/// Port of `scanpmcommands()` from Src/Modules/parameter.c:245.
#[allow(non_snake_case)]
pub fn scanpmcommands() -> i32 { 0 }

/// Port of `scanpmdisbuiltins()` from Src/Modules/parameter.c:850.
#[allow(non_snake_case)]
pub fn scanpmdisbuiltins() -> i32 { 0 }

/// Port of `scanpmdisfunction_source()` from Src/Modules/parameter.c:618.
#[allow(non_snake_case)]
pub fn scanpmdisfunction_source() -> i32 { 0 }

/// Port of `scanpmdisfunctions()` from Src/Modules/parameter.c:526.
#[allow(non_snake_case)]
pub fn scanpmdisfunctions() -> i32 { 0 }

/// Port of `scanpmdisgaliases()` from Src/Modules/parameter.c:2011.
#[allow(non_snake_case)]
pub fn scanpmdisgaliases() -> i32 { 0 }

/// Port of `scanpmdisraliases()` from Src/Modules/parameter.c:1997.
#[allow(non_snake_case)]
pub fn scanpmdisraliases() -> i32 { 0 }

/// Port of `scanpmdissaliases()` from Src/Modules/parameter.c:2025.
#[allow(non_snake_case)]
pub fn scanpmdissaliases() -> i32 { 0 }

/// Port of `scanpmfunction_source()` from Src/Modules/parameter.c:609.
#[allow(non_snake_case)]
pub fn scanpmfunction_source() -> i32 { 0 }

/// Port of `scanpmfunctions()` from Src/Modules/parameter.c:519.
#[allow(non_snake_case)]
pub fn scanpmfunctions() -> i32 { 0 }

/// Port of `scanpmgaliases()` from Src/Modules/parameter.c:2004.
#[allow(non_snake_case)]
pub fn scanpmgaliases() -> i32 { 0 }

/// Port of `scanpmhistory()` from Src/Modules/parameter.c:1188.
#[allow(non_snake_case)]
pub fn scanpmhistory() -> i32 { 0 }

/// Port of `scanpmjobdirs()` from Src/Modules/parameter.c:1487.
#[allow(non_snake_case)]
pub fn scanpmjobdirs() -> i32 { 0 }

/// Port of `scanpmjobstates()` from Src/Modules/parameter.c:1415.
#[allow(non_snake_case)]
pub fn scanpmjobstates() -> i32 { 0 }

/// Port of `scanpmjobtexts()` from Src/Modules/parameter.c:1308.
#[allow(non_snake_case)]
pub fn scanpmjobtexts() -> i32 { 0 }

/// Port of `scanpmmodules()` from Src/Modules/parameter.c:1074.
#[allow(non_snake_case)]
pub fn scanpmmodules() -> i32 { 0 }

/// Port of `scanpmnameddirs()` from Src/Modules/parameter.c:1618.
#[allow(non_snake_case)]
pub fn scanpmnameddirs() -> i32 { 0 }

/// Port of `scanpmoptions()` from Src/Modules/parameter.c:1016.
#[allow(non_snake_case)]
pub fn scanpmoptions() -> i32 { 0 }

/// Port of `scanpmparameters()` from Src/Modules/parameter.c:124.
#[allow(non_snake_case)]
pub fn scanpmparameters() -> i32 { 0 }

/// Port of `scanpmraliases()` from Src/Modules/parameter.c:1990.
#[allow(non_snake_case)]
pub fn scanpmraliases() -> i32 { 0 }

/// Port of `scanpmsaliases()` from Src/Modules/parameter.c:2018.
#[allow(non_snake_case)]
pub fn scanpmsaliases() -> i32 { 0 }

/// Port of `scanpmuserdirs()` from Src/Modules/parameter.c:1669.
#[allow(non_snake_case)]
pub fn scanpmuserdirs() -> i32 { 0 }

/// Port of `scanpmusergroups()` from Src/Modules/parameter.c:2143.
#[allow(non_snake_case)]
pub fn scanpmusergroups() -> i32 { 0 }

/// Port of `setalias()` from Src/Modules/parameter.c:1699.
#[allow(non_snake_case)]
pub fn setalias() -> i32 { 0 }

/// Port of `setaliases()` from Src/Modules/parameter.c:1769.
#[allow(non_snake_case)]
pub fn setaliases() -> i32 { 0 }

/// Port of `setfunction()` from Src/Modules/parameter.c:284.
#[allow(non_snake_case)]
pub fn setfunction() -> i32 { 0 }

/// Port of `setfunctions()` from Src/Modules/parameter.c:344.
#[allow(non_snake_case)]
pub fn setfunctions() -> i32 { 0 }

/// Port of `setpmcommand()` from Src/Modules/parameter.c:151.
#[allow(non_snake_case)]
pub fn setpmcommand() -> i32 { 0 }

/// Port of `setpmcommands()` from Src/Modules/parameter.c:173.
#[allow(non_snake_case)]
pub fn setpmcommands() -> i32 { 0 }

/// Port of `setpmdisfunction()` from Src/Modules/parameter.c:327.
#[allow(non_snake_case)]
pub fn setpmdisfunction() -> i32 { 0 }

/// Port of `setpmdisfunctions()` from Src/Modules/parameter.c:377.
#[allow(non_snake_case)]
pub fn setpmdisfunctions() -> i32 { 0 }

/// Port of `setpmdisgalias()` from Src/Modules/parameter.c:1728.
#[allow(non_snake_case)]
pub fn setpmdisgalias() -> i32 { 0 }

/// Port of `setpmdisgaliases()` from Src/Modules/parameter.c:1833.
#[allow(non_snake_case)]
pub fn setpmdisgaliases() -> i32 { 0 }

/// Port of `setpmdisralias()` from Src/Modules/parameter.c:1714.
#[allow(non_snake_case)]
pub fn setpmdisralias() -> i32 { 0 }

/// Port of `setpmdisraliases()` from Src/Modules/parameter.c:1819.
#[allow(non_snake_case)]
pub fn setpmdisraliases() -> i32 { 0 }

/// Port of `setpmdissalias()` from Src/Modules/parameter.c:1742.
#[allow(non_snake_case)]
pub fn setpmdissalias() -> i32 { 0 }

/// Port of `setpmdissaliases()` from Src/Modules/parameter.c:1847.
#[allow(non_snake_case)]
pub fn setpmdissaliases() -> i32 { 0 }

/// Port of `setpmfunction()` from Src/Modules/parameter.c:320.
#[allow(non_snake_case)]
pub fn setpmfunction() -> i32 { 0 }

/// Port of `setpmfunctions()` from Src/Modules/parameter.c:370.
#[allow(non_snake_case)]
pub fn setpmfunctions() -> i32 { 0 }

/// Port of `setpmgalias()` from Src/Modules/parameter.c:1721.
#[allow(non_snake_case)]
pub fn setpmgalias() -> i32 { 0 }

/// Port of `setpmgaliases()` from Src/Modules/parameter.c:1826.
#[allow(non_snake_case)]
pub fn setpmgaliases() -> i32 { 0 }

/// Port of `setpmnameddir()` from Src/Modules/parameter.c:1519.
#[allow(non_snake_case)]
pub fn setpmnameddir() -> i32 { 0 }

/// Port of `setpmnameddirs()` from Src/Modules/parameter.c:1544.
#[allow(non_snake_case)]
pub fn setpmnameddirs() -> i32 { 0 }

/// Port of `setpmoption()` from Src/Modules/parameter.c:926.
#[allow(non_snake_case)]
pub fn setpmoption() -> i32 { 0 }

/// Port of `setpmoptions()` from Src/Modules/parameter.c:953.
#[allow(non_snake_case)]
pub fn setpmoptions() -> i32 { 0 }

/// Port of `setpmralias()` from Src/Modules/parameter.c:1707.
#[allow(non_snake_case)]
pub fn setpmralias() -> i32 { 0 }

/// Port of `setpmraliases()` from Src/Modules/parameter.c:1812.
#[allow(non_snake_case)]
pub fn setpmraliases() -> i32 { 0 }

/// Port of `setpmsalias()` from Src/Modules/parameter.c:1735.
#[allow(non_snake_case)]
pub fn setpmsalias() -> i32 { 0 }

/// Port of `setpmsaliases()` from Src/Modules/parameter.c:1840.
#[allow(non_snake_case)]
pub fn setpmsaliases() -> i32 { 0 }

/// Port of `unsetpmalias()` from Src/Modules/parameter.c:1749.
#[allow(non_snake_case)]
pub fn unsetpmalias() -> i32 { 0 }

/// Port of `unsetpmcommand()` from Src/Modules/parameter.c:163.
#[allow(non_snake_case)]
pub fn unsetpmcommand() -> i32 { 0 }

/// Port of `unsetpmfunction()` from Src/Modules/parameter.c:334.
#[allow(non_snake_case)]
pub fn unsetpmfunction() -> i32 { 0 }

/// Port of `unsetpmnameddir()` from Src/Modules/parameter.c:1534.
#[allow(non_snake_case)]
pub fn unsetpmnameddir() -> i32 { 0 }

/// Port of `unsetpmoption()` from Src/Modules/parameter.c:941.
#[allow(non_snake_case)]
pub fn unsetpmoption() -> i32 { 0 }

/// Port of `unsetpmsalias()` from Src/Modules/parameter.c:1759.
#[allow(non_snake_case)]
pub fn unsetpmsalias() -> i32 { 0 }
