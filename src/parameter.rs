//! Parameter interface to shell internals - port of Modules/parameter.c
//!
//! Provides special parameters: $commands, $functions, $aliases, $builtins,
//! $modules, $dirstack, $history, $historywords, $options, $nameddirs, $userdirs

use std::collections::HashMap;
use std::path::PathBuf;

/// Parameter type flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Scalar,
    Integer,
    Float,
    Array,
    Associative,
    Nameref,
}

impl ParamType {
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
pub struct CommandsTable {
    hashed: HashMap<String, PathBuf>,
}

impl CommandsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&PathBuf> {
        self.hashed.get(name)
    }

    pub fn set(&mut self, name: &str, path: PathBuf) {
        self.hashed.insert(name.to_string(), path);
    }

    pub fn unset(&mut self, name: &str) {
        self.hashed.remove(name);
    }

    pub fn clear(&mut self) {
        self.hashed.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.hashed.iter()
    }

    pub fn len(&self) -> usize {
        self.hashed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashed.is_empty()
    }

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
pub struct FunctionDef {
    pub body: String,
    pub flags: u32,
    pub autoload: bool,
}

#[derive(Debug, Default)]
pub struct FunctionsTable {
    functions: HashMap<String, FunctionDef>,
    disabled: HashMap<String, FunctionDef>,
}

impl FunctionsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.get(name)
    }

    pub fn get_disabled(&self, name: &str) -> Option<&FunctionDef> {
        self.disabled.get(name)
    }

    pub fn set(&mut self, name: &str, def: FunctionDef) {
        self.functions.insert(name.to_string(), def);
    }

    pub fn unset(&mut self, name: &str) {
        self.functions.remove(name);
    }

    pub fn disable(&mut self, name: &str) {
        if let Some(def) = self.functions.remove(name) {
            self.disabled.insert(name.to_string(), def);
        }
    }

    pub fn enable(&mut self, name: &str) {
        if let Some(def) = self.disabled.remove(name) {
            self.functions.insert(name.to_string(), def);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &FunctionDef)> {
        self.functions.iter()
    }

    pub fn iter_disabled(&self) -> impl Iterator<Item = (&String, &FunctionDef)> {
        self.disabled.iter()
    }
}

/// Aliases hash table ($aliases)
#[derive(Debug, Clone)]
pub struct AliasDef {
    pub value: String,
    pub global: bool,
    pub suffix: bool,
}

#[derive(Debug, Default)]
pub struct AliasesTable {
    aliases: HashMap<String, AliasDef>,
    disabled: HashMap<String, AliasDef>,
    global_aliases: HashMap<String, AliasDef>,
    suffix_aliases: HashMap<String, AliasDef>,
}

impl AliasesTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&AliasDef> {
        self.aliases.get(name)
    }

    pub fn get_global(&self, name: &str) -> Option<&AliasDef> {
        self.global_aliases.get(name)
    }

    pub fn get_suffix(&self, suffix: &str) -> Option<&AliasDef> {
        self.suffix_aliases.get(suffix)
    }

    pub fn set(&mut self, name: &str, def: AliasDef) {
        if def.global {
            self.global_aliases.insert(name.to_string(), def);
        } else if def.suffix {
            self.suffix_aliases.insert(name.to_string(), def);
        } else {
            self.aliases.insert(name.to_string(), def);
        }
    }

    pub fn unset(&mut self, name: &str) {
        self.aliases.remove(name);
        self.global_aliases.remove(name);
        self.suffix_aliases.remove(name);
    }

    pub fn disable(&mut self, name: &str) {
        if let Some(def) = self.aliases.remove(name) {
            self.disabled.insert(name.to_string(), def);
        }
    }

    pub fn enable(&mut self, name: &str) {
        if let Some(def) = self.disabled.remove(name) {
            self.aliases.insert(name.to_string(), def);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.aliases.iter()
    }

    pub fn iter_global(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.global_aliases.iter()
    }

    pub fn iter_suffix(&self) -> impl Iterator<Item = (&String, &AliasDef)> {
        self.suffix_aliases.iter()
    }
}

/// Builtins list ($builtins)
#[derive(Debug, Default)]
pub struct BuiltinsTable {
    builtins: HashMap<String, bool>,
    disabled: HashMap<String, bool>,
}

impl BuiltinsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str) {
        self.builtins.insert(name.to_string(), true);
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains_key(name)
    }

    pub fn disable(&mut self, name: &str) {
        if self.builtins.remove(name).is_some() {
            self.disabled.insert(name.to_string(), true);
        }
    }

    pub fn enable(&mut self, name: &str) {
        if self.disabled.remove(name).is_some() {
            self.builtins.insert(name.to_string(), true);
        }
    }

    pub fn list(&self) -> Vec<&str> {
        self.builtins.keys().map(|s| s.as_str()).collect()
    }

    pub fn list_disabled(&self) -> Vec<&str> {
        self.disabled.keys().map(|s| s.as_str()).collect()
    }
}

/// Directory stack ($dirstack)
#[derive(Debug, Default)]
pub struct DirStack {
    stack: Vec<PathBuf>,
}

impl DirStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, dir: PathBuf) {
        self.stack.push(dir);
    }

    pub fn pop(&mut self) -> Option<PathBuf> {
        self.stack.pop()
    }

    pub fn get(&self, index: usize) -> Option<&PathBuf> {
        self.stack.get(index)
    }

    pub fn set(&mut self, stack: Vec<PathBuf>) {
        self.stack = stack;
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.stack.iter()
    }

    pub fn to_array(&self) -> Vec<String> {
        self.stack
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }
}

/// Options special parameter ($options)
#[derive(Debug, Default)]
pub struct OptionsTable {
    options: HashMap<String, bool>,
}

impl OptionsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, value: bool) {
        self.options.insert(name.to_lowercase(), value);
    }

    pub fn get(&self, name: &str) -> Option<bool> {
        self.options.get(&name.to_lowercase()).copied()
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.options
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or(false)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &bool)> {
        self.options.iter()
    }

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
pub struct NamedDirsTable {
    dirs: HashMap<String, PathBuf>,
}

impl NamedDirsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, path: PathBuf) {
        self.dirs.insert(name.to_string(), path);
    }

    pub fn get(&self, name: &str) -> Option<&PathBuf> {
        self.dirs.get(name)
    }

    pub fn unset(&mut self, name: &str) {
        self.dirs.remove(name);
    }

    pub fn find_name(&self, path: &PathBuf) -> Option<&str> {
        self.dirs
            .iter()
            .find(|(_, p)| *p == path)
            .map(|(n, _)| n.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.dirs.iter()
    }
}

/// Job states ($jobstates)
#[derive(Debug, Clone)]
pub struct JobState {
    pub running: bool,
    pub suspended: bool,
    pub done: bool,
}

impl JobState {
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
pub struct JobsTable {
    jobs: HashMap<i32, (JobState, String)>,
}

impl JobsTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, id: i32, state: JobState, text: String) {
        self.jobs.insert(id, (state, text));
    }

    pub fn remove(&mut self, id: i32) {
        self.jobs.remove(&id);
    }

    pub fn get_state(&self, id: i32) -> Option<&JobState> {
        self.jobs.get(&id).map(|(s, _)| s)
    }

    pub fn get_text(&self, id: i32) -> Option<&str> {
        self.jobs.get(&id).map(|(_, t)| t.as_str())
    }

    pub fn states(&self) -> HashMap<String, String> {
        self.jobs
            .iter()
            .map(|(id, (state, _))| (id.to_string(), state.as_str().to_string()))
            .collect()
    }

    pub fn texts(&self) -> HashMap<String, String> {
        self.jobs
            .iter()
            .map(|(id, (_, text))| (id.to_string(), text.clone()))
            .collect()
    }
}

/// Modules table ($modules)
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub loaded: bool,
    pub autoload: bool,
}

#[derive(Debug, Default)]
pub struct ModulesTable {
    modules: HashMap<String, ModuleInfo>,
}

impl ModulesTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str, info: ModuleInfo) {
        self.modules.insert(name.to_string(), info);
    }

    pub fn get(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.get(name)
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.modules.get(name).map(|m| m.loaded).unwrap_or(false)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ModuleInfo)> {
        self.modules.iter()
    }

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
