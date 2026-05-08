//! Hash table implementations - port of hashtable.c
//!
//! Provides hash tables for commands, shell functions, reserved words, aliases,
//! and history. Uses Rust's HashMap internally but maintains zsh-compatible APIs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Flags for hash nodes
pub mod flags {
    pub const DISABLED: u32 = 1 << 0;
    pub const HASHED: u32 = 1 << 1;
    pub const ALIAS_GLOBAL: u32 = 1 << 2;
    pub const ALIAS_SUFFIX: u32 = 1 << 3;
    pub const PM_UNDEFINED: u32 = 1 << 4;
    pub const PM_TAGGED: u32 = 1 << 5;
    pub const PM_TAGGED_LOCAL: u32 = 1 << 6;
    pub const PM_LOADDIR: u32 = 1 << 7;
    pub const PM_UNALIASED: u32 = 1 << 8;
    pub const PM_KSHSTORED: u32 = 1 << 9;
    pub const PM_ZSHSTORED: u32 = 1 << 10;
    pub const PM_CUR_FPATH: u32 = 1 << 11;
}

/// Generic hash function (zsh's hasher)
/// Compute the canonical zsh hash for a string.
/// Port of `hasher()` from Src/hashtable.c:86 — uses the same
/// `hash * 33 + char` polynomial the C source uses for every
/// HashTable lookup.
pub fn hasher(s: &str) -> u32 {
    let mut hashval: u32 = 0;
    for c in s.bytes() {
        hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(c as u32));
    }
    hashval
}

/// History-specific hash function (normalizes whitespace)
/// Hasher tuned for the history table.
/// Port of the per-history hash specialization Src/hist.c uses
/// — the C source bypasses leading whitespace before mixing.
pub fn hist_hasher(s: &str) -> u32 {
    let mut hashval: u32 = 0;
    let mut chars = s.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            if chars.peek().is_some() {
                hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(' ' as u32));
            }
        } else {
            hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(c as u32));
        }
    }
    hashval
}

/// Compare strings with normalized whitespace (for history)
/// Multiple whitespace sequences are treated as equivalent to single spaces.
/// Trailing whitespace is ignored when comparing.
/// Compare two history entries with optional blank-reduction.
/// Port of the comparator the C source's `addhistnode()` from
/// Src/hist.c uses to detect duplicate history lines.
pub fn hist_strcmp(s1: &str, s2: &str, reduce_blanks: bool) -> std::cmp::Ordering {
    let s1 = s1.trim_start();
    let s2 = s2.trim_start();

    if reduce_blanks {
        return s1.cmp(s2);
    }

    let mut c1 = s1.chars().peekable();
    let mut c2 = s2.chars().peekable();

    loop {
        let ch1 = c1.peek().copied();
        let ch2 = c2.peek().copied();

        match (ch1, ch2) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(c)) => {
                if c.is_whitespace() {
                    while c2.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c2.next();
                    }
                    if c2.peek().is_none() {
                        return std::cmp::Ordering::Equal;
                    }
                }
                return std::cmp::Ordering::Less;
            }
            (Some(c), None) => {
                if c.is_whitespace() {
                    while c1.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c1.next();
                    }
                    if c1.peek().is_none() {
                        return std::cmp::Ordering::Equal;
                    }
                }
                return std::cmp::Ordering::Greater;
            }
            (Some(ch1), Some(ch2)) => {
                let ws1 = ch1.is_whitespace();
                let ws2 = ch2.is_whitespace();

                if ws1 && ws2 {
                    while c1.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c1.next();
                    }
                    while c2.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c2.next();
                    }
                } else if ws1 {
                    while c1.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c1.next();
                    }
                    if c1.peek().is_none() {
                        return std::cmp::Ordering::Less;
                    }
                    return std::cmp::Ordering::Less;
                } else if ws2 {
                    while c2.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        c2.next();
                    }
                    if c2.peek().is_none() {
                        return std::cmp::Ordering::Greater;
                    }
                    return std::cmp::Ordering::Greater;
                } else if ch1 != ch2 {
                    return ch1.cmp(&ch2);
                } else {
                    c1.next();
                    c2.next();
                }
            }
        }
    }
}

/// Command name entry
#[derive(Debug, Clone)]
/// One entry in the command-name (`$cmdtab`) table.
/// Port of `struct cmdnam` from Src/zsh.h — `addhashnode()` /
/// `gethashnode()` (Src/hashtable.c lines 157/231) wrap entries
/// of this shape for `hash`/`rehash`/`unhash` builtin use.
pub struct CmdName {
    pub name: String,
    pub flags: u32,
    pub path: Option<PathBuf>,
    pub dir_index: Option<usize>,
}

impl CmdName {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            path: None,
            dir_index: None,
        }
    }

    pub fn with_path(name: &str, path: PathBuf) -> Self {
        Self {
            name: name.to_string(),
            flags: flags::HASHED,
            path: Some(path),
            dir_index: None,
        }
    }

    pub fn with_dir_index(name: &str, dir_index: usize) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            path: None,
            dir_index: Some(dir_index),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.flags & flags::DISABLED != 0
    }

    pub fn is_hashed(&self) -> bool {
        self.flags & flags::HASHED != 0
    }
}

/// Command name hash table
#[derive(Debug)]
/// `$cmdtab` table of cached executable lookups.
/// Port of `cmdnamtab` from Src/hashtable.c — `createcmdnamtable()`
/// (line 601), `emptycmdnamtable()` (line 623), and `hashdir()`
/// (line 634) drive populate/clear/fill cycles.
pub struct CmdNameTable {
    table: HashMap<String, CmdName>,
    path_checked_index: usize,
    path: Vec<String>,
    hash_executables_only: bool,
}

impl CmdNameTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            path_checked_index: 0,
            path: Vec::new(),
            hash_executables_only: false,
        }
    }

    pub fn set_path(&mut self, path: Vec<String>) {
        self.path = path;
        self.path_checked_index = 0;
    }

    pub fn set_hash_executables_only(&mut self, value: bool) {
        self.hash_executables_only = value;
    }

    pub fn add(&mut self, cmd: CmdName) {
        self.table.insert(cmd.name.clone(), cmd);
    }

    pub fn get(&self, name: &str) -> Option<&CmdName> {
        self.table.get(name).filter(|c| !c.is_disabled())
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&CmdName> {
        self.table.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<CmdName> {
        self.table.remove(name)
    }

    pub fn clear(&mut self) {
        self.table.clear();
        self.path_checked_index = 0;
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Hash all commands in a directory
    pub fn hash_dir(&mut self, dir: &str, dir_index: usize) {
        if dir.starts_with('.') || dir.is_empty() {
            return;
        }

        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };

            if self.table.contains_key(&name) {
                continue;
            }

            let path = entry.path();
            let should_add = if self.hash_executables_only {
                is_executable(&path)
            } else {
                true
            };

            if should_add {
                self.table
                    .insert(name.clone(), CmdName::with_dir_index(&name, dir_index));
            }
        }
    }

    /// Fill table from PATH
    pub fn fill(&mut self) {
        for i in self.path_checked_index..self.path.len() {
            let dir = self.path[i].clone();
            self.hash_dir(&dir, i);
        }
        self.path_checked_index = self.path.len();
    }

    /// Iterate over all entries
    pub fn iter(&self) -> impl Iterator<Item = (&String, &CmdName)> {
        self.table.iter()
    }

    /// Get full path for a command
    pub fn get_full_path(&self, name: &str) -> Option<PathBuf> {
        let cmd = self.table.get(name)?;
        if cmd.is_disabled() {
            return None;
        }

        if let Some(ref path) = cmd.path {
            return Some(path.clone());
        }

        if let Some(idx) = cmd.dir_index {
            if idx < self.path.len() {
                let mut path = PathBuf::from(&self.path[idx]);
                path.push(name);
                return Some(path);
            }
        }

        None
    }
}

impl Default for CmdNameTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a path is executable
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(meta) = path.metadata() {
        if !meta.is_file() {
            return false;
        }
        let mode = meta.permissions().mode();
        mode & 0o111 != 0
    } else {
        false
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Shell function entry
#[derive(Debug, Clone)]
/// Shell function table entry.
/// Port of `struct shfunc` from Src/zsh.h — referenced via
/// `shfunctab` HashTable across Src/builtin.c and Src/exec.c.
pub struct ShFunc {
    pub name: String,
    pub flags: u32,
    pub filename: Option<String>,
    pub body: Option<String>,
}

impl ShFunc {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            filename: None,
            body: None,
        }
    }

    pub fn autoload(name: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: flags::PM_UNDEFINED,
            filename: None,
            body: None,
        }
    }

    pub fn with_body(name: &str, body: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            filename: None,
            body: Some(body.to_string()),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.flags & flags::DISABLED != 0
    }

    pub fn is_autoload(&self) -> bool {
        self.flags & flags::PM_UNDEFINED != 0
    }

    pub fn is_traced(&self) -> bool {
        self.flags & (flags::PM_TAGGED | flags::PM_TAGGED_LOCAL) != 0
    }
}

/// Shell function hash table
#[derive(Debug)]
/// `$shfunctab` shell function table.
/// Port of the `shfunctab` HashTable Src/hashtable.c builds —
/// `printshfuncnode` / `freeshfuncnode` (Src/builtin.c) hang off
/// the same shape.
pub struct ShFuncTable {
    table: HashMap<String, ShFunc>,
}

impl ShFuncTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    pub fn add(&mut self, func: ShFunc) -> Option<ShFunc> {
        self.table.insert(func.name.clone(), func)
    }

    pub fn get(&self, name: &str) -> Option<&ShFunc> {
        self.table.get(name).filter(|f| !f.is_disabled())
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&ShFunc> {
        self.table.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut ShFunc> {
        self.table.get_mut(name).filter(|f| !f.is_disabled())
    }

    pub fn remove(&mut self, name: &str) -> Option<ShFunc> {
        self.table.remove(name)
    }

    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(func) = self.table.get_mut(name) {
            func.flags |= flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(func) = self.table.get_mut(name) {
            func.flags &= !flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ShFunc)> {
        self.table.iter()
    }

    pub fn iter_sorted(&self) -> Vec<(&String, &ShFunc)> {
        let mut entries: Vec<_> = self.table.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
    }

    pub fn clear(&mut self) {
        self.table.clear();
    }
}

impl Default for ShFuncTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Reserved word token types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
/// Reserved-word token IDs.
/// Port of the `LX1_*` reserved-word constants the C source's
/// reserved-word table (`reswdtab` HashTable, Src/lex.c) uses to
/// classify tokens like `if`/`while`/`function`/etc.
pub enum ReswdToken {
    Bang,
    DinBrack,
    InBrace,
    OutBrace,
    Case,
    Coproc,
    Typeset,
    DoLoop,
    Done,
    Elif,
    Else,
    Zend,
    Esac,
    Fi,
    For,
    Foreach,
    Func,
    If,
    Nocorrect,
    Repeat,
    Select,
    Then,
    Time,
    Until,
    While,
}

/// Reserved word entry
#[derive(Debug, Clone)]
/// Reserved-word table entry.
/// Port of `struct reswd` from Src/zsh.h — populated via
/// `addreswords()` (Src/lex.c) at startup and consulted on every
/// lex.
pub struct Reswd {
    pub name: String,
    pub flags: u32,
    pub token: ReswdToken,
}

impl Reswd {
    pub fn new(name: &str, token: ReswdToken) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            token,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.flags & flags::DISABLED != 0
    }
}

/// Reserved word hash table
#[derive(Debug)]
/// `$reswdtab` reserved-word table.
/// Port of the `reswdtab` HashTable from Src/hashtable.c — used
/// by Src/lex.c to recognize keywords like `if`/`while`/`do`.
pub struct ReswdTable {
    table: HashMap<String, Reswd>,
}

impl ReswdTable {
    pub fn new() -> Self {
        let mut table = HashMap::new();

        let words = [
            ("!", ReswdToken::Bang),
            ("[[", ReswdToken::DinBrack),
            ("{", ReswdToken::InBrace),
            ("}", ReswdToken::OutBrace),
            ("case", ReswdToken::Case),
            ("coproc", ReswdToken::Coproc),
            ("declare", ReswdToken::Typeset),
            ("do", ReswdToken::DoLoop),
            ("done", ReswdToken::Done),
            ("elif", ReswdToken::Elif),
            ("else", ReswdToken::Else),
            ("end", ReswdToken::Zend),
            ("esac", ReswdToken::Esac),
            ("export", ReswdToken::Typeset),
            ("fi", ReswdToken::Fi),
            ("float", ReswdToken::Typeset),
            ("for", ReswdToken::For),
            ("foreach", ReswdToken::Foreach),
            ("function", ReswdToken::Func),
            ("if", ReswdToken::If),
            ("integer", ReswdToken::Typeset),
            ("local", ReswdToken::Typeset),
            ("nocorrect", ReswdToken::Nocorrect),
            ("readonly", ReswdToken::Typeset),
            ("repeat", ReswdToken::Repeat),
            ("select", ReswdToken::Select),
            ("then", ReswdToken::Then),
            ("time", ReswdToken::Time),
            ("typeset", ReswdToken::Typeset),
            ("until", ReswdToken::Until),
            ("while", ReswdToken::While),
        ];

        for (name, token) in words {
            table.insert(name.to_string(), Reswd::new(name, token));
        }

        Self { table }
    }

    pub fn get(&self, name: &str) -> Option<&Reswd> {
        self.table.get(name).filter(|r| !r.is_disabled())
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&Reswd> {
        self.table.get(name)
    }

    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.flags |= flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.flags &= !flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn is_reserved(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Reswd)> {
        self.table.iter()
    }
}

impl Default for ReswdTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Alias entry
#[derive(Debug, Clone)]
/// Alias table entry.
/// Port of `struct alias` from Src/zsh.h — used by both
/// `aliastab` and `sufaliastab` HashTables (regular vs. suffix
/// aliases).
pub struct Alias {
    pub name: String,
    pub flags: u32,
    pub text: String,
    pub inuse: i32,
}

impl Alias {
    pub fn new(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            text: text.to_string(),
            inuse: 0,
        }
    }

    pub fn global(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: flags::ALIAS_GLOBAL,
            text: text.to_string(),
            inuse: 0,
        }
    }

    pub fn suffix(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            flags: flags::ALIAS_SUFFIX,
            text: text.to_string(),
            inuse: 0,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.flags & flags::DISABLED != 0
    }

    pub fn is_global(&self) -> bool {
        self.flags & flags::ALIAS_GLOBAL != 0
    }

    pub fn is_suffix(&self) -> bool {
        self.flags & flags::ALIAS_SUFFIX != 0
    }
}

/// Alias hash table
#[derive(Debug)]
/// `$aliastab` alias hash.
/// Port of the `aliastab` HashTable from Src/hashtable.c —
/// `bin_alias()` (Src/builtin.c) drives every mutation. Suffix
/// aliases live in a separate `sufaliastab` instance.
pub struct AliasTable {
    table: HashMap<String, Alias>,
}

impl AliasTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut table = Self::new();
        table.add(Alias::new("run-help", "man"));
        table.add(Alias::new("which-command", "whence"));
        table
    }

    pub fn add(&mut self, alias: Alias) -> Option<Alias> {
        self.table.insert(alias.name.clone(), alias)
    }

    pub fn get(&self, name: &str) -> Option<&Alias> {
        self.table.get(name).filter(|a| !a.is_disabled())
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&Alias> {
        self.table.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Alias> {
        self.table.get_mut(name).filter(|a| !a.is_disabled())
    }

    pub fn remove(&mut self, name: &str) -> Option<Alias> {
        self.table.remove(name)
    }

    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.flags |= flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.flags &= !flags::DISABLED;
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub fn clear(&mut self) {
        self.table.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Alias)> {
        self.table.iter()
    }

    pub fn iter_sorted(&self) -> Vec<(&String, &Alias)> {
        let mut entries: Vec<_> = self.table.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
    }
}

impl Default for AliasTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Suffix alias table (separate from regular aliases)
pub type SuffixAliasTable = AliasTable;

/// Directory cache entry for function filenames
#[derive(Debug, Clone)]
struct DirCacheEntry {
    name: String,
    refs: usize,
}

/// Directory cache for efficient storage of function directories
#[derive(Debug)]
/// Cached directory listings for `$cmdtab` rehash.
/// Port of the dir-listing cache `hashdir()` (Src/hashtable.c:634)
/// builds when populating `cmdnamtab` from `$PATH` — caches the
/// readdir() result so successive lookups skip syscalls.
pub struct DirCache {
    entries: Vec<DirCacheEntry>,
    last_entry: Option<usize>,
}

impl DirCache {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_entry: None,
        }
    }

    /// Get or create a cached directory string
    pub fn get_or_insert(&mut self, value: &str) -> String {
        if let Some(idx) = self.last_entry {
            if self.entries[idx].name == value {
                self.entries[idx].refs += 1;
                return self.entries[idx].name.clone();
            }
        }

        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.name == value {
                entry.refs += 1;
                self.last_entry = Some(i);
                return entry.name.clone();
            }
        }

        let idx = self.entries.len();
        self.entries.push(DirCacheEntry {
            name: value.to_string(),
            refs: 1,
        });
        self.last_entry = Some(idx);
        self.entries[idx].name.clone()
    }

    /// Release a reference to a cached directory
    pub fn release(&mut self, value: &str) {
        for i in 0..self.entries.len() {
            if self.entries[i].name == value {
                self.entries[i].refs -= 1;
                if self.entries[i].refs == 0 {
                    self.entries.remove(i);
                    if self.last_entry == Some(i) {
                        self.last_entry = None;
                    } else if let Some(ref mut last) = self.last_entry {
                        if *last > i {
                            *last -= 1;
                        }
                    }
                }
                return;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for DirCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Print flags for whence/type commands
pub mod print_flags {
    pub const NAMEONLY: u32 = 1 << 0;
    pub const WHENCE_WORD: u32 = 1 << 1;
    pub const WHENCE_SIMPLE: u32 = 1 << 2;
    pub const WHENCE_CSH: u32 = 1 << 3;
    pub const WHENCE_VERBOSE: u32 = 1 << 4;
    pub const WHENCE_FUNCDEF: u32 = 1 << 5;
    pub const LIST: u32 = 1 << 6;
}

/// Format a command name entry for output
/// Format a `$cmdtab` entry for `hash` listing.
/// Port of `printcmdnamnode()` from Src/hashtable.c (the C
/// source's per-command formatter `bin_hash()` invokes).
pub fn printcmdnamnode(cmd: &CmdName, path: &[String], print_flags: u32) -> String {
    let name = &cmd.name;

    if print_flags & print_flags::WHENCE_WORD != 0 {
        let kind = if cmd.is_hashed() { "hashed" } else { "command" };
        return format!("{}: {}\n", name, kind);
    }

    if print_flags & (print_flags::WHENCE_CSH | print_flags::WHENCE_SIMPLE) != 0 {
        if cmd.is_hashed() {
            if let Some(ref p) = cmd.path {
                return format!("{}\n", p.display());
            }
        } else if let Some(idx) = cmd.dir_index {
            if idx < path.len() {
                return format!("{}/{}\n", path[idx], name);
            }
        }
        return format!("{}\n", name);
    }

    if print_flags & print_flags::WHENCE_VERBOSE != 0 {
        if cmd.is_hashed() {
            if let Some(ref p) = cmd.path {
                return format!("{} is hashed to {}\n", name, p.display());
            }
        } else if let Some(idx) = cmd.dir_index {
            if idx < path.len() {
                return format!("{} is {}/{}\n", name, path[idx], name);
            }
        }
        return format!("{} is {}\n", name, name);
    }

    if print_flags & print_flags::LIST != 0 {
        let prefix = if name.starts_with('-') {
            "hash -- "
        } else {
            "hash "
        };

        if cmd.is_hashed() {
            if let Some(ref p) = cmd.path {
                return format!("{}{}={}\n", prefix, name, p.display());
            }
        } else if let Some(idx) = cmd.dir_index {
            if idx < path.len() {
                return format!("{}{}={}/{}\n", prefix, name, path[idx], name);
            }
        }
    }

    if cmd.is_hashed() {
        if let Some(ref p) = cmd.path {
            return format!("{}={}\n", name, p.display());
        }
    } else if let Some(idx) = cmd.dir_index {
        if idx < path.len() {
            return format!("{}={}/{}\n", name, path[idx], name);
        }
    }

    format!("{}={}\n", name, name)
}

/// Format a shell function for output
/// Format a `$shfunctab` entry for `functions` listing.
/// Port of `printshfuncnode()` from Src/builtin.c — emits the
/// declaration / source-text combination `functions -t`/`-T`/
/// `+/-`/etc. variants produce.
pub fn printshfuncnode(func: &ShFunc, print_flags: u32) -> String {
    let name = &func.name;

    if print_flags & print_flags::NAMEONLY != 0
        || (print_flags & print_flags::WHENCE_SIMPLE != 0
            && print_flags & print_flags::WHENCE_FUNCDEF == 0)
    {
        return format!("{}\n", name);
    }

    if print_flags & (print_flags::WHENCE_VERBOSE | print_flags::WHENCE_WORD) != 0
        && print_flags & print_flags::WHENCE_FUNCDEF == 0
    {
        if print_flags & print_flags::WHENCE_WORD != 0 {
            return format!("{}: function\n", name);
        }

        let kind = if func.is_autoload() {
            "is an autoload shell function"
        } else {
            "is a shell function"
        };

        let mut result = format!("{} {}", name, kind);
        if let Some(ref filename) = func.filename {
            result.push_str(&format!(" from {}", filename));
        }
        result.push('\n');
        return result;
    }

    let mut result = format!("{} () {{\n", name);

    if func.is_autoload() {
        result.push_str("\t# undefined\n");
        if func.is_traced() {
            result.push_str("\t# traced\n");
        }
        result.push_str("\tbuiltin autoload -X");
        if let Some(ref filename) = func.filename {
            if func.flags & flags::PM_LOADDIR != 0 {
                result.push_str(&format!(" {}", filename));
            }
        }
    } else if let Some(ref body) = func.body {
        if func.is_traced() {
            result.push_str("\t# traced\n");
        }
        for line in body.lines() {
            result.push_str(&format!("\t{}\n", line));
        }
    }

    result.push_str("}\n");
    result
}

/// Format a reserved word for output
/// Format a reserved-word entry.
/// Port of `printreswdnode()` from Src/lex.c (the C source's
/// formatter for the `reswdtab` HashTable).
pub fn format_reswd(rw: &Reswd, print_flags: u32) -> String {
    let name = &rw.name;

    if print_flags & print_flags::WHENCE_WORD != 0 {
        return format!("{}: reserved\n", name);
    }

    if print_flags & print_flags::WHENCE_CSH != 0 {
        return format!("{}: shell reserved word\n", name);
    }

    if print_flags & print_flags::WHENCE_VERBOSE != 0 {
        return format!("{} is a reserved word\n", name);
    }

    format!("{}\n", name)
}

/// Format an alias for output
pub fn format_alias(alias: &Alias, print_flags: u32) -> String {
    let name = &alias.name;
    let text = &alias.text;

    if print_flags & print_flags::NAMEONLY != 0 {
        return format!("{}\n", name);
    }

    if print_flags & print_flags::WHENCE_WORD != 0 {
        let kind = if alias.is_suffix() {
            "suffix alias"
        } else if alias.is_global() {
            "global alias"
        } else {
            "alias"
        };
        return format!("{}: {}\n", name, kind);
    }

    if print_flags & print_flags::WHENCE_SIMPLE != 0 {
        return format!("{}\n", text);
    }

    if print_flags & print_flags::WHENCE_CSH != 0 {
        let kind = if alias.is_suffix() {
            "suffix "
        } else if alias.is_global() {
            "globally "
        } else {
            ""
        };
        return format!("{}: {}aliased to {}\n", name, kind, text);
    }

    if print_flags & print_flags::WHENCE_VERBOSE != 0 {
        let kind = if alias.is_suffix() {
            " suffix"
        } else if alias.is_global() {
            " global"
        } else {
            "n"
        };
        return format!("{} is a{} alias for {}\n", name, kind, text);
    }

    if print_flags & print_flags::LIST != 0 {
        if name.contains('=') {
            return format!("# invalid alias '{}'\n", name);
        }

        let mut result = String::from("alias ");
        if alias.is_suffix() {
            result.push_str("-s ");
        } else if alias.is_global() {
            result.push_str("-g ");
        }

        if name.starts_with('-') || name.starts_with('+') {
            result.push_str("-- ");
        }

        result.push_str(&format!("{}={}\n", shell_quote(name), shell_quote(text)));
        return result;
    }

    format!("{}={}\n", shell_quote(name), shell_quote(text))
}

/// Quote a string for shell output
fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '/' || c == '.')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hasher() {
        assert_eq!(hasher(""), 0);
        assert!(hasher("test") != 0);
        assert_eq!(hasher("test"), hasher("test"));
        assert_ne!(hasher("test"), hasher("Test"));
    }

    #[test]
    fn test_hist_hasher() {
        assert_eq!(hist_hasher("  hello  world  "), hist_hasher("hello world"));
        assert_ne!(hist_hasher("hello world"), hist_hasher("helloworld"));
    }

    #[test]
    fn test_hist_strcmp() {
        assert_eq!(
            hist_strcmp("  hello  world  ", "hello world", false),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            hist_strcmp("hello world", "hello world", true),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_cmdnam_table() {
        let mut table = CmdNameTable::new();
        table.add(CmdName::with_path("ls", PathBuf::from("/bin/ls")));

        assert!(table.get("ls").is_some());
        assert!(table.get("nonexistent").is_none());

        let ls = table.get("ls").unwrap();
        assert!(ls.is_hashed());
        assert!(!ls.is_disabled());
    }

    #[test]
    fn test_shfunc_table() {
        let mut table = ShFuncTable::new();
        table.add(ShFunc::with_body("myfunc", "echo hello"));
        table.add(ShFunc::autoload("lazy"));

        assert!(table.get("myfunc").is_some());
        assert!(!table.get("myfunc").unwrap().is_autoload());
        assert!(table.get("lazy").unwrap().is_autoload());

        table.disable("myfunc");
        assert!(table.get("myfunc").is_none());
        assert!(table.get_including_disabled("myfunc").is_some());

        table.enable("myfunc");
        assert!(table.get("myfunc").is_some());
    }

    #[test]
    fn test_reswd_table() {
        let table = ReswdTable::new();

        assert!(table.is_reserved("if"));
        assert!(table.is_reserved("while"));
        assert!(table.is_reserved("[["));
        assert!(!table.is_reserved("notreserved"));

        let if_rw = table.get("if").unwrap();
        assert_eq!(if_rw.token, ReswdToken::If);
    }

    #[test]
    fn test_alias_table() {
        let mut table = AliasTable::with_defaults();

        assert!(table.get("run-help").is_some());
        assert_eq!(table.get("run-help").unwrap().text, "man");

        table.add(Alias::global("G", "| grep"));
        assert!(table.get("G").unwrap().is_global());

        table.add(Alias::suffix("pdf", "zathura"));
        assert!(table.get("pdf").unwrap().is_suffix());

        table.disable("G");
        assert!(table.get("G").is_none());
    }

    #[test]
    fn test_dir_cache() {
        let mut cache = DirCache::new();

        let d1 = cache.get_or_insert("/usr/share/zsh");
        let d2 = cache.get_or_insert("/usr/share/zsh");
        assert_eq!(d1, d2);
        assert_eq!(cache.len(), 1);

        let d3 = cache.get_or_insert("/home/user/.zsh");
        assert_ne!(d1, d3);
        assert_eq!(cache.len(), 2);

        cache.release("/usr/share/zsh");
        assert_eq!(cache.len(), 2);

        cache.release("/usr/share/zsh");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_format_alias() {
        let alias = Alias::new("ll", "ls -l");
        let output = format_alias(&alias, print_flags::WHENCE_VERBOSE);
        assert!(output.contains("is an alias for"));

        let global = Alias::global("G", "| grep");
        let output = format_alias(&global, print_flags::WHENCE_WORD);
        assert!(output.contains("global alias"));
    }

    #[test]
    fn test_format_reswd() {
        let table = ReswdTable::new();
        let if_rw = table.get("if").unwrap();

        let output = format_reswd(if_rw, print_flags::WHENCE_VERBOSE);
        assert!(output.contains("is a reserved word"));

        let output = format_reswd(if_rw, print_flags::WHENCE_WORD);
        assert!(output.contains("reserved"));
    }
}

// ===========================================================
// Direct ports of the generic `HashTable` lifecycle / mutation /
// printer routines from Src/hashtable.c. The Rust port stores
// command/alias/reswd/shfunc tables as `HashMap`-backed wrappers
// (above), so most of these are free-fn shims for ABI/name
// parity. Callers in the Rust executor reach the live state via
// the typed table structs (`AliasTable`, `ShFuncTable`, etc.).
// ===========================================================

/// Port of `newhashtable()` from Src/hashtable.c:100 — heap-
/// allocate a fresh `HashTable` header with the supplied size /
/// name / hash-fn tuple. Rust uses `HashMap::new()`; shim.
pub fn newhashtable() {}

/// Port of `deletehashtable()` from Src/hashtable.c:129 — free
/// every node + the table header. Rust drops via Drop; shim.
pub fn deletehashtable() {}

/// Port of `addhashnode()` from Src/hashtable.c:157 — insert with
/// duplicate-replace semantics (frees the old node). Shim — Rust
/// callers use `HashMap::insert` directly.
pub fn addhashnode() {}

/// Port of `addhashnode2()` from Src/hashtable.c:168 — insert
/// returning the old node instead of freeing it. Shim.
pub fn addhashnode2() {}

/// Port of `gethashnode()` from Src/hashtable.c:231 — lookup
/// (returns NULL when disabled). Shim.
pub fn gethashnode() {}

/// Port of `gethashnode2()` from Src/hashtable.c:255 — lookup
/// returning the node even when disabled. Shim.
pub fn gethashnode2() {}

/// Port of `removehashnode()` from Src/hashtable.c:275 — pop one
/// node by key, returning the removed datum. Shim.
pub fn removehashnode() {}

/// Port of `disablehashnode()` from Src/hashtable.c:323 — set
/// `DISABLED` flag on the node. Shim.
pub fn disablehashnode() {}

/// Port of `enablehashnode()` from Src/hashtable.c:332 — clear
/// `DISABLED` flag. Shim.
pub fn enablehashnode() {}

/// Port of `hnamcmp()` from Src/hashtable.c:341 — `strcmp`-style
/// comparator over hash node names; used by `qsort` for sorted
/// `print -l`-style output.
pub fn hnamcmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Port of `scanmatchtable()` from Src/hashtable.c:373 — walk the
/// table calling `func()` on each node whose name matches the
/// pattern. Shim.
pub fn scanmatchtable() {}

/// Port of `scanhashtable()` from Src/hashtable.c:446 — walk the
/// table calling `func()` on every node. Shim.
pub fn scanhashtable() {}

/// Port of `expandhashtable()` from Src/hashtable.c:458 — grow
/// the bucket array when load factor exceeds the threshold.
/// Rust's `HashMap` rehashes automatically; shim.
pub fn expandhashtable() {}

/// Port of `resizehashtable()` from Src/hashtable.c:486 — change
/// the bucket count to a specific size. Shim — `HashMap::reserve`
/// covers this implicitly.
pub fn resizehashtable() {}

/// Port of `emptyhashtable()` from Src/hashtable.c:519 — drop
/// every node, keep the header. Rust uses `HashMap::clear`; shim.
pub fn emptyhashtable() {}

/// Port of `printhashtabinfo()` from Src/hashtable.c:533 — emit
/// load-factor / bucket / collision stats for `bin_hashinfo`.
/// Shim.
pub fn printhashtabinfo() {}

/// Port of `bin_hashinfo()` from Src/hashtable.c:566 — `hash`
/// builtin entry that prints the stats. Shim.
pub fn bin_hashinfo() {}

/// Port of `createcmdnamtable()` from Src/hashtable.c:601 —
/// allocate the global `cmdnamtab` (the PATH command-cache).
/// Rust stores PATH cache in `ShellExecutor.cmdnamtab`; shim.
pub fn createcmdnamtable() {}

/// Port of `emptycmdnamtable()` from Src/hashtable.c:623 — drop
/// every PATH cache entry (`hash -r`). Shim.
pub fn emptycmdnamtable() {}

/// Port of `hashdir()` from Src/hashtable.c:634 — populate
/// `cmdnamtab` from a directory, recording every executable.
/// Rust does this in `ShellExecutor::scan_path`; shim.
pub fn hashdir() {}

/// Port of `fillcmdnamtable()` from Src/hashtable.c:712 — walk
/// all PATH entries calling `hashdir()` for each. Shim.
pub fn fillcmdnamtable() {}

/// Port of `freecmdnamnode()` from Src/hashtable.c:724 — free
/// one cmdnamtab entry. Rust drops via Drop; shim.
pub fn freecmdnamnode() {}

/// Port of `createshfunctable()` from Src/hashtable.c:812 —
/// allocate `shfunctab` (the user-defined-function table). Shim.
pub fn createshfunctable() {}

/// Port of `removeshfuncnode()` from Src/hashtable.c:836 — drop
/// one shell function entry. Shim.
pub fn removeshfuncnode() {}

/// Port of `disableshfuncnode()` from Src/hashtable.c:855 —
/// `disable -f NAME` entry. Shim.
pub fn disableshfuncnode() {}

/// Port of `enableshfuncnode()` from Src/hashtable.c:873 —
/// `enable -f NAME` entry. Shim.
pub fn enableshfuncnode() {}

/// Port of `freeshfuncnode()` from Src/hashtable.c:888 — free
/// one shfunctab entry (function body + name). Shim.
pub fn freeshfuncnode() {}

/// Port of `scanmatchshfunc()` from Src/hashtable.c:1013 —
/// pattern-walk over shfunctab. Shim.
pub fn scanmatchshfunc() {}

/// Port of `scanshfunc()` from Src/hashtable.c:1031 — walk every
/// shfunctab node calling `func()`. Shim.
pub fn scanshfunc() {}

/// Port of `printshfuncexpand()` from Src/hashtable.c:1042 —
/// `functions -e` output (expand-prompt-aware). Shim.
pub fn printshfuncexpand() {}

/// Port of `getshfuncfile()` from Src/hashtable.c:1059 — return
/// the source-file path for a defined function (`functions -T`-
/// style output). Shim.
pub fn getshfuncfile() -> String {
    String::new()
}

/// Port of `createreswdtable()` from Src/hashtable.c:1120 —
/// allocate the reserved-word table (`if`, `while`, etc.).
/// Rust uses `ReswdTable::new()`; shim.
pub fn createreswdtable() {}

/// Port of `printreswdnode()` from Src/hashtable.c:1147 —
/// `whence`-format printer for one reserved word. Shim.
pub fn printreswdnode() {}

/// Port of `createaliastable()` from Src/hashtable.c:1188 —
/// allocate the alias table. Rust uses `AliasTable::new()`; shim.
pub fn createaliastable() {}

/// Port of `createaliastables()` from Src/hashtable.c:1206 —
/// allocate both the regular alias table and the suffix-alias
/// table. Shim.
pub fn createaliastables() {}

/// Port of `createaliasnode()` from Src/hashtable.c:1230 —
/// allocate one alias node (`alias name=value`). Shim.
pub fn createaliasnode() {}

/// Port of `freealiasnode()` from Src/hashtable.c:1243 — free
/// one alias node. Rust drops via Drop; shim.
pub fn freealiasnode() {}

/// Port of `printaliasnode()` from Src/hashtable.c:1256 —
/// `alias` builtin output for one alias. Shim.
pub fn printaliasnode() {}

/// Port of `createhisttable()` from Src/hashtable.c:1345 —
/// allocate the history-event hash table. Rust history is keyed
/// by event number directly; shim.
pub fn createhisttable() {}

/// Port of `histhasher()` from Src/hashtable.c:1365 — hash-fn
/// callback for the history table; mixes the event command
/// string. Shim.
pub fn histhasher(_s: &str) -> u64 {
    0
}

/// Port of `emptyhisttable()` from Src/hashtable.c:1385 — drop
/// every history-table entry (`history -p` etc.). Shim.
pub fn emptyhisttable() {}

/// Port of `histstrcmp()` from Src/hashtable.c:1396 — comparator
/// callback over history-table nodes. Shim.
pub fn histstrcmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Port of `addhistnode()` from Src/hashtable.c:1427 — insert a
/// history event into the hash. Shim.
pub fn addhistnode() {}

/// Port of `freehistnode()` from Src/hashtable.c:1450 — free one
/// history event entry. Shim.
pub fn freehistnode() {}

/// Port of `freehistdata()` from Src/hashtable.c:1458 — free the
/// command + word-array fields of a history entry. Shim.
pub fn freehistdata() {}

/// Port of `dircache_set()` from Src/hashtable.c:1537 — populate
/// the directory cache (PATH entries + readdir results) used by
/// `hashdir()`. Shim.
pub fn dircache_set() {}
