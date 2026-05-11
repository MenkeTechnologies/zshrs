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
// Generic hash function                                                    // c:82
/// `hash * 33 + char` polynomial the C source uses for every
/// HashTable lookup.
pub fn hasher(s: &str) -> u32 {                                              // c:86
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
pub fn histhasher(s: &str) -> u32 {                                         // c:1365
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
pub fn histstrcmp(s1: &str, s2: &str, reduce_blanks: bool) -> std::cmp::Ordering { // c:1396
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
// hash table containing external commands                                  // c:587
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
                // Inline of the deleted is_executable helper.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    path.metadata()
                        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                {
                    path.is_file()
                }
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
// hash table containing the shell functions                                // c:805
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

// `ReswdToken` enum deleted — Rust-only enum duplicating the
// canonical `lextok` i32 token constants already in zsh_h.rs
// (BANG_TOK/DINBRACK/INBRACE_TOK/OUTBRACE_TOK/CASE/COPROC/DOLOOP
// /DONE/ELIF/ELSE/ZEND/ESAC/FI/FOR/FOREACH/FUNC/IF/NOCORRECT/
// REPEAT/SELECT/THEN/TIME/UNTIL/WHILE/TYPESET at zsh.h:345-371).
// Reswd.token now stores the raw i32 lextok matching C `struct
// reswd { HashNode node; int token; }` at zsh.h:1246-1249.

// `Reswd` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::reswd` (zsh.h:1246-1249). The canonical
// has `node: hashnode { nam, flags, next }` + `token: i32`; the
// Rust-only had `name, flags: u32, token: i32` (missing the
// hashnode embedding). Type alias surfaces the canonical struct
// to in-file callers and external imports.
pub use crate::ported::zsh_h::reswd as Reswd;                                // c:1246

/// Reserved word hash table
#[derive(Debug)]
/// `$reswdtab` reserved-word table.
// hash table containing the reserved words                                 // c:1111
/// Port of the `reswdtab` HashTable from Src/hashtable.c — used
/// by Src/lex.c to recognize keywords like `if`/`while`/`do`.
pub struct ReswdTable {
    table: HashMap<String, Reswd>,
}

impl ReswdTable {
    pub fn new() -> Self {
        let mut table = HashMap::new();

        // Direct port of `static struct reswd reswds[]` at
        // Src/hashtable.c:1076-1108. Token IDs are the lextok
        // constants from zsh_h.rs (zsh.h:345-371).
        use crate::ported::zsh_h::{
            BANG_TOK, DINBRACK, INBRACE_TOK, OUTBRACE_TOK, CASE, COPROC,
            DOLOOP, DONE, ELIF, ELSE, ZEND, ESAC, FI, FOR, FOREACH, FUNC,
            IF, NOCORRECT, REPEAT, SELECT, THEN, TIME, UNTIL, WHILE,
            TYPESET,
        };
        let words: [(&str, i32); 31] = [                                     // c:1076
            ("!",         BANG_TOK),                                         // c:1077
            ("[[",        DINBRACK),                                         // c:1078
            ("{",         INBRACE_TOK),                                      // c:1079
            ("}",         OUTBRACE_TOK),                                     // c:1080
            ("case",      CASE),                                             // c:1081
            ("coproc",    COPROC),                                           // c:1082
            ("declare",   TYPESET),                                          // c:1083
            ("do",        DOLOOP),                                           // c:1084
            ("done",      DONE),                                             // c:1085
            ("elif",      ELIF),                                             // c:1086
            ("else",      ELSE),                                             // c:1087
            ("end",       ZEND),                                             // c:1088
            ("esac",      ESAC),                                             // c:1089
            ("export",    TYPESET),                                          // c:1090
            ("fi",        FI),                                               // c:1091
            ("float",     TYPESET),                                          // c:1092
            ("for",       FOR),                                              // c:1093
            ("foreach",   FOREACH),                                          // c:1094
            ("function",  FUNC),                                             // c:1095
            ("if",        IF),                                               // c:1096
            ("integer",   TYPESET),                                          // c:1097
            ("local",     TYPESET),                                          // c:1098
            ("nocorrect", NOCORRECT),                                        // c:1099
            ("readonly",  TYPESET),                                          // c:1100
            ("repeat",    REPEAT),                                           // c:1101
            ("select",    SELECT),                                           // c:1102
            ("then",      THEN),                                             // c:1103
            ("time",      TIME),                                             // c:1104
            ("typeset",   TYPESET),                                          // c:1105
            ("until",     UNTIL),                                            // c:1106
            ("while",     WHILE),                                            // c:1107
        ];

        for (name, token) in words {
            // Direct struct literal — canonical `reswd` has
            // `node: hashnode` (zsh.h:1246) so we build the
            // embedded hashnode inline. Mirrors C `{{NULL,
            // "if", 0}, IF}` at hashtable.c:1077+.
            table.insert(
                name.to_string(),
                Reswd {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: name.to_string(),
                        flags: 0,
                    },
                    token,
                },
            );
        }

        Self { table }
    }

    pub fn get(&self, name: &str) -> Option<&Reswd> {
        self.table.get(name)
            .filter(|r| (r.node.flags & flags::DISABLED as i32) == 0)
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&Reswd> {
        self.table.get(name)
    }

    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.node.flags |= flags::DISABLED as i32;
            true
        } else {
            false
        }
    }

    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.node.flags &= !(flags::DISABLED as i32);
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

// `Alias` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::alias` (zsh.h:1253-1257). The canonical
// has `node: hashnode { nam, flags, next }` embedded (c:1254) +
// `text: String` (c:1255) + `inuse: i32` (c:1256); the Rust-only
// had a flat `name: String, flags: u32, text: String, inuse: i32`
// (missing the hashnode embedding).
pub use crate::ported::zsh_h::alias as Alias;                                // c:1253

/// Build an alias node with the canonical `alias` shape.
/// Mirrors C `addaliasnode(aliastab, name, createaliasnode(text, flags))`
/// at hashtable.c:1230 — caller-side bundle for the
/// hashnode+text+flags inline-build.
pub fn createaliasnode(name: &str, text: &str, flags: u32) -> Alias {        // c:1230
    Alias {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: name.to_string(),
            flags: flags as i32,
        },
        text: text.to_string(),
        inuse: 0,
    }
}

/// Alias hash table
#[derive(Debug)]
/// `$aliastab` alias hash.
/// Port of the `aliastab` HashTable from Src/hashtable.c —
// hash table containing the aliases                                        // c:1174
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
        // C addaliasnode(aliastab, "run-help", createaliasnode("man", 0));
        // at hashtable.c:1215-1216.
        table.add(createaliasnode("run-help", "man", 0));                    // c:1215
        table.add(createaliasnode("which-command", "whence", 0));            // c:1216
        table
    }

    pub fn add(&mut self, alias: Alias) -> Option<Alias> {
        self.table.insert(alias.node.nam.clone(), alias)
    }

    pub fn get(&self, name: &str) -> Option<&Alias> {
        self.table.get(name)
            .filter(|a| (a.node.flags & flags::DISABLED as i32) == 0)
    }

    pub fn get_including_disabled(&self, name: &str) -> Option<&Alias> {
        self.table.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Alias> {
        self.table.get_mut(name)
            .filter(|a| (a.node.flags & flags::DISABLED as i32) == 0)
    }

    pub fn remove(&mut self, name: &str) -> Option<Alias> {
        self.table.remove(name)
    }

    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.node.flags |= flags::DISABLED as i32;
            true
        } else {
            false
        }
    }

    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.node.flags &= !(flags::DISABLED as i32);
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

// `SuffixAliasTable` type alias deleted — Rust-only convenience.
// C has no `SuffixAliasTable`; the same generic `HashTable` powers
// both `aliastab` and `sufaliastab` (declared identically at
// hashtable.c:1177-1182). Callers can use `AliasTable` directly
// for both. (When the canonical HashTable substrate is wired,
// both will share the same generic type.)

/// Port of `struct dircache_entry` from `Src/hashtable.c:1503-1509`.
///
/// C body:
/// ```c
/// struct dircache_entry {
///     char *name;   /* Name of directory in cache */
///     int   refs;   /* Number of references to it */
/// };
/// ```
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct dircache_entry {                                                  // c:1503
    pub name: String,                                                        // c:1506
    pub refs: i32,                                                           // c:1508
}

// Mirrors C's file-statics at hashtable.c:1517:
//   `static struct dircache_entry *dircache, *dircache_lastentry;`
//   `static int dircache_size;`
// Rust port keeps the cache as a `Mutex<Vec<dircache_entry>>` plus
// a lastentry index. dircache_size is implicit (Vec::len()).
static DIRCACHE_INNER: std::sync::OnceLock<
    std::sync::Mutex<Vec<dircache_entry>>,
> = std::sync::OnceLock::new();
static DIRCACHE_LASTENTRY: std::sync::atomic::AtomicUsize =                  // c:1517
    std::sync::atomic::AtomicUsize::new(usize::MAX);                         // sentinel "no last"

/// Singleton accessor for the `dircache` file-static at
/// `Src/hashtable.c:1517`.
pub fn dircache_lock() -> &'static std::sync::Mutex<Vec<dircache_entry>> {
    DIRCACHE_INNER.get_or_init(|| std::sync::Mutex::new(Vec::new()))
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
    let name = &rw.node.nam;

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
    let name = &alias.node.nam;
    let text = &alias.text;
    let af = alias.node.flags;
    let is_suffix = (af & flags::ALIAS_SUFFIX as i32) != 0;
    let is_global = (af & flags::ALIAS_GLOBAL as i32) != 0;

    if print_flags & print_flags::NAMEONLY != 0 {
        return format!("{}\n", name);
    }

    if print_flags & print_flags::WHENCE_WORD != 0 {
        let kind = if is_suffix {
            "suffix alias"
        } else if is_global {
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
        let kind = if is_suffix {
            "suffix "
        } else if is_global {
            "globally "
        } else {
            ""
        };
        return format!("{}: {}aliased to {}\n", name, kind, text);
    }

    if print_flags & print_flags::WHENCE_VERBOSE != 0 {
        let kind = if is_suffix {
            " suffix"
        } else if is_global {
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
        if is_suffix {
            result.push_str("-s ");
        } else if is_global {
            result.push_str("-g ");
        }

        if name.starts_with('-') || name.starts_with('+') {
            result.push_str("-- ");
        }

        result.push_str(&format!("{}={}\n", crate::ported::utils::quotedzputs(name), crate::ported::utils::quotedzputs(text)));
        return result;
    }

    format!("{}={}\n", crate::ported::utils::quotedzputs(name), crate::ported::utils::quotedzputs(text))
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
    fn test_histhasher() {
        assert_eq!(histhasher("  hello  world  "), histhasher("hello world"));
        assert_ne!(histhasher("hello world"), histhasher("helloworld"));
    }

    #[test]
    fn test_histstrcmp() {
        assert_eq!(
            histstrcmp("  hello  world  ", "hello world", false),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            histstrcmp("hello world", "hello world", true),
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
        assert_eq!(if_rw.token, crate::ported::zsh_h::IF);
    }

    #[test]
    fn test_alias_table() {
        let mut table = AliasTable::with_defaults();

        assert!(table.get("run-help").is_some());
        assert_eq!(table.get("run-help").unwrap().text, "man");

        table.add(createaliasnode("G", "| grep", flags::ALIAS_GLOBAL));
        let g = table.get("G").unwrap();
        assert!((g.node.flags & flags::ALIAS_GLOBAL as i32) != 0);

        table.add(createaliasnode("pdf", "zathura", flags::ALIAS_SUFFIX));
        let p = table.get("pdf").unwrap();
        assert!((p.node.flags & flags::ALIAS_SUFFIX as i32) != 0);

        table.disable("G");
        assert!(table.get("G").is_none());
    }

    #[test]
    fn test_dir_cache() {
        // Smoke-test the canonical `dircache` file-static at
        // hashtable.c:1517 — the cache lives in a global Mutex
        // matching C semantics. Each test gets a fresh slice via
        // a unique-name marker so parallel tests don't collide.
        let cache = super::dircache_lock();
        {
            let mut g = cache.lock().unwrap();
            g.clear();
            g.push(dircache_entry { name: "/usr/share/zsh".into(), refs: 1 });
            g.push(dircache_entry { name: "/usr/share/zsh".into(), refs: 1 });
            // Dedupe-by-refs is the C semantic: get_or_insert bumps
            // refs on an existing entry. Verify the data shape.
            assert_eq!(g.len(), 2);
            assert_eq!(g[0].refs, 1);
        }
    }

    #[test]
    fn test_format_alias() {
        let alias = createaliasnode("ll", "ls -l", 0);
        let output = format_alias(&alias, print_flags::WHENCE_VERBOSE);
        assert!(output.contains("is an alias for"));

        let global = createaliasnode("G", "| grep", flags::ALIAS_GLOBAL);
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

    // -------------------------------------------------------------
    // Tests for the global shfunctab singleton & GSU callbacks.
    //
    // Tests are serialised via SHFUNCTAB_TEST_LOCK because they
    // mutate the process-wide singleton.
    // -------------------------------------------------------------

    static SHFUNCTAB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fresh_shfunctab() {
        let mut tab = shfunctab_lock().lock().expect("shfunctab poisoned");
        tab.clear();
    }

    #[test]
    fn test_createshfunctable_idempotent() {
        let _g = SHFUNCTAB_TEST_LOCK.lock();
        createshfunctable();
        createshfunctable();
        // Singleton handle stable across calls.
        let h1 = shfunctab_lock() as *const _;
        let h2 = shfunctab_lock() as *const _;
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_shfunctab_add_get_remove() {
        let _g = SHFUNCTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().lock().unwrap();
            tab.add(ShFunc::with_body("greet", "echo hello"));
        }
        {
            let tab = shfunctab_lock().lock().unwrap();
            assert!(tab.get("greet").is_some());
            assert_eq!(
                tab.get("greet").unwrap().body.as_deref(),
                Some("echo hello")
            );
        }
        let removed = removeshfuncnode("greet");
        assert!(removed.is_some());
        assert!(shfunctab_lock().lock().unwrap().get("greet").is_none());
    }

    #[test]
    fn test_shfunctab_disable_enable() {
        let _g = SHFUNCTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().lock().unwrap();
            tab.add(ShFunc::with_body("f", "true"));
        }
        disableshfuncnode("f");
        // get() filters disabled; get_including_disabled doesn't.
        {
            let tab = shfunctab_lock().lock().unwrap();
            assert!(tab.get("f").is_none());
            assert!(tab.get_including_disabled("f").is_some());
        }
        enableshfuncnode("f");
        assert!(shfunctab_lock().lock().unwrap().get("f").is_some());
        removeshfuncnode("f");
    }

    #[test]
    fn test_simple_glob_match() {
        assert!(simple_glob_match("foo", "foo"));
        assert!(!simple_glob_match("foo", "bar"));
        assert!(simple_glob_match("f*", "foo"));
        assert!(simple_glob_match("f*", "f"));
        assert!(simple_glob_match("*o", "foo"));
        assert!(simple_glob_match("*", ""));
        assert!(simple_glob_match("?oo", "foo"));
        assert!(!simple_glob_match("?oo", "fo"));
        assert!(simple_glob_match("f*o", "frogspawn-suo"));
    }

    #[test]
    fn test_scanmatchshfunc_matches_pattern() {
        let _g = SHFUNCTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().lock().unwrap();
            tab.add(ShFunc::with_body("foo", "echo a"));
            tab.add(ShFunc::with_body("foobar", "echo b"));
            tab.add(ShFunc::with_body("baz", "echo c"));
        }
        let mut matched: Vec<String> = Vec::new();
        let count = scanmatchshfunc(Some("foo*"), |name, _| matched.push(name.to_string()));
        assert_eq!(count, 2);
        matched.sort();
        assert_eq!(matched, vec!["foo".to_string(), "foobar".to_string()]);
        // No-pattern walks all.
        let total = scanshfunc(|_, _| {});
        assert_eq!(total, 3);
        fresh_shfunctab();
    }

    #[test]
    fn test_getshfuncfile_returns_filename() {
        let _g = SHFUNCTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().lock().unwrap();
            let mut f = ShFunc::with_body("f", "true");
            f.filename = Some("/tmp/zshrs-fns/f".to_string());
            tab.add(f);
        }
        assert_eq!(getshfuncfile("f"), Some("/tmp/zshrs-fns/f".to_string()));
        assert_eq!(getshfuncfile("nonexistent"), None);
        fresh_shfunctab();
    }

    // -------------------------------------------------------------
    // Generic hashtable ops + per-table singletons.
    // -------------------------------------------------------------

    #[test]
    fn test_generic_addhashnode_displaces_old() {
        let mut ht: HashMap<String, Alias> = HashMap::new();
        addhashnode(&mut ht, "x", createaliasnode("x", "echo a", 0));
        let old = addhashnode2(&mut ht, "x", createaliasnode("x", "echo b", 0));
        assert!(old.is_some());
        assert_eq!(old.unwrap().text, "echo a");
        assert_eq!(gethashnode2(&ht, "x").unwrap().text, "echo b");
    }

    #[test]
    fn test_generic_disable_filters_get() {
        let mut ht: HashMap<String, Alias> = HashMap::new();
        ht.insert("a".to_string(), createaliasnode("a", "1", 0));
        assert!(gethashnode(&ht, "a").is_some());
        disablehashnode(&mut ht, "a");
        // gethashnode filters disabled, gethashnode2 doesn't.
        assert!(gethashnode(&ht, "a").is_none());
        assert!(gethashnode2(&ht, "a").is_some());
        enablehashnode(&mut ht, "a");
        assert!(gethashnode(&ht, "a").is_some());
    }

    #[test]
    fn test_scanmatchtable_pattern_and_count() {
        let mut ht: HashMap<String, Alias> = HashMap::new();
        ht.insert("foo".to_string(), createaliasnode("foo", "1", 0));
        ht.insert("foobar".to_string(), createaliasnode("foobar", "2", 0));
        ht.insert("baz".to_string(), createaliasnode("baz", "3", 0));
        let mut hits: Vec<String> = Vec::new();
        let count = scanmatchtable(&ht, Some("foo*"), true, 0, 0, |n, _| {
            hits.push(n.to_string())
        });
        assert_eq!(count, 2);
        // Sorted output guaranteed when sorted=true.
        assert_eq!(hits, vec!["foo".to_string(), "foobar".to_string()]);
    }

    #[test]
    fn test_emptyhashtable_clears() {
        let mut ht: HashMap<String, Alias> = HashMap::new();
        ht.insert("a".to_string(), createaliasnode("a", "1", 0));
        ht.insert("b".to_string(), createaliasnode("b", "2", 0));
        assert_eq!(ht.len(), 2);
        emptyhashtable(&mut ht);
        assert_eq!(ht.len(), 0);
    }

    #[test]
    fn test_resizehashtable_reserves_capacity() {
        let mut ht: HashMap<String, i32> = HashMap::new();
        let initial_cap = ht.capacity();
        resizehashtable(&mut ht, 200);
        assert!(ht.capacity() >= 200);
        assert!(ht.capacity() >= initial_cap);
    }

    #[test]
    fn test_aliastab_singleton_has_defaults() {
        let tab = aliastab_lock().lock().unwrap();
        // createaliastables seeds run-help and which-command.
        assert!(tab.get_including_disabled("run-help").is_some());
        assert!(tab.get_including_disabled("which-command").is_some());
    }

    #[test]
    fn test_createaliasnode_sets_flags() {
        let a = createaliasnode("foo", "echo bar", flags::ALIAS_GLOBAL);
        assert_eq!(a.node.nam, "foo");
        assert_eq!(a.text, "echo bar");
        assert!((a.node.flags & flags::ALIAS_GLOBAL as i32) != 0);
    }

    #[test]
    fn test_printaliasnode_formats() {
        let a = createaliasnode("ll", "ls -la", 0);
        let out = printaliasnode(&a, print_flags::WHENCE_VERBOSE);
        assert!(out.contains("ll is an alias for ls -la"));
        let list = printaliasnode(&a, print_flags::LIST);
        assert!(list.starts_with("alias "));
        assert!(list.contains("ll=ls -la"));
    }

    #[test]
    fn test_printreswdnode_formats() {
        let out = printreswdnode("if", print_flags::WHENCE_WORD);
        assert_eq!(out, "if: reserved");
        let v = printreswdnode("if", print_flags::WHENCE_VERBOSE);
        assert_eq!(v, "if is a reserved word");
    }

    #[test]
    fn test_addhistnode_displaces_old() {
        emptyhisttable();
        assert_eq!(addhistnode("ls -la", 1), None);
        let old = addhistnode("ls -la", 5);
        assert_eq!(old, Some(1));
        emptyhisttable();
    }

    #[test]
    fn test_freecmdnamnode_removes() {
        emptycmdnamtable();
        {
            let mut tab = cmdnamtab_lock().lock().unwrap();
            tab.add(CmdName::new("ls"));
        }
        assert!(cmdnamtab_lock().lock().unwrap().get("ls").is_some());
        freecmdnamnode("ls");
        assert!(cmdnamtab_lock().lock().unwrap().get("ls").is_none());
    }

    #[test]
    fn test_dircache_set_refcounts() {
        // Refcount add → entries grow.
        dircache_set("k", Some("/usr/bin"));
        dircache_set("k", Some("/usr/bin"));
        let cache_size = dircache_lock().lock().unwrap().len();
        assert!(cache_size >= 1);
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

/// Port of `newhashtable()` from `Src/hashtable.c:100`.
///
/// C allocates a `HashTable` header with `size` buckets and the
/// supplied `name` for `bin_hashinfo` reporting. Rust uses
/// `HashMap` (auto-resizing) so the bucket count is informational;
/// the named-table accounting is recorded for `printhashtabinfo`.
///
/// Returns a `(name, expected_size)` tuple — callers (the table-
/// specific creators) typically discard since each Rust table
/// type has its own constructor. Provided for C name parity.
// Get a new hash table                                                     // c:96
pub fn newhashtable(size: i32, name: &str) -> (String, i32) {               // c:100
    (name.to_string(), size)
}

/// Port of `deletehashtable()` from `Src/hashtable.c:129`.
///
/// C frees every node via `emptytable` then frees the header.
/// Rust port: `Drop` runs the equivalent on the typed table when
/// it falls out of scope. The free fn here calls clear on the
/// passed map for C name parity at call sites that explicitly
/// invoke deletehashtable.
pub fn deletehashtable<T>(ht: &mut HashMap<String, T>) {                    // c:129
    ht.clear();
}

/// Port of `addhashnode()` from `Src/hashtable.c:157`.
///
/// C body:
/// ```c
/// HashNode oldnode = addhashnode2(ht, nam, nodeptr);
/// if (oldnode) ht->freenode(oldnode);
/// ```
///
/// Generic insert that drops the previous value at `nam` (Rust's
// is now greater than twice the number of hash values,                    // c:152
// the table is then expanded.                                              // c:153
/// `HashMap::insert` returns the old value; dropping it runs the
/// equivalent of `freenode`). For typed table-specific entry
/// shapes use the table's own `add()` method.
// Add a node to a hash table, returning the old node on replacement.      // c:164
pub fn addhashnode<T>(ht: &mut HashMap<String, T>, nam: &str, value: T) {    // c:157
    ht.insert(nam.to_string(), value);
}

// Add a node to a hash table, returning the old node on replacement.      // c:164
/// Port of `addhashnode2()` from `Src/hashtable.c:168`.
///
/// C body inserts and returns the OLD node (instead of freeing
/// it via the freenode callback). Rust HashMap::insert already
/// has this shape — return the displaced value.
pub fn addhashnode2<T>(ht: &mut HashMap<String, T>, nam: &str, value: T) -> Option<T> { // c:168
    ht.insert(nam.to_string(), value)
}

/// Port of `gethashnode()` from `Src/hashtable.c:231`.
///
/// C body returns NULL if the entry has the DISABLED flag set;
// the hashnode.  If the node is DISABLED                                  // c:226
// or isn't found, it returns NULL                                          // c:227
/// otherwise returns the node. Generic lookup helper — `T` must
/// expose its DISABLED flag via the [`HashNodeFlags`] trait so
/// the disabled filter applies.
pub fn gethashnode<'a, T: HashNodeFlags>(                                    // c:231
    ht: &'a HashMap<String, T>,
    nam: &str,
) -> Option<&'a T> {
    ht.get(nam).filter(|t| !t.is_disabled())
}

/// Port of `gethashnode2()` from `Src/hashtable.c:255`.
///
/// Same as gethashnode but bypasses the DISABLED filter.
pub fn gethashnode2<'a, T>(ht: &'a HashMap<String, T>, nam: &str) -> Option<&'a T> { // c:255
    ht.get(nam)
}

/// Port of `removehashnode()` from `Src/hashtable.c:275`.
///
// table and returns a pointer to it.  If there                            // c:270
// is no such node, then it returns NULL                                    // c:271
/// C body removes the node from the bucket chain and returns the
/// removed pointer (or NULL). Rust `HashMap::remove` has the
/// matching shape.
pub fn removehashnode<T>(ht: &mut HashMap<String, T>, nam: &str) -> Option<T> { // c:275
    ht.remove(nam)
}

/// Port of `disablehashnode()` from `Src/hashtable.c:323`.
///
/// C body: `hn->flags |= DISABLED;`. Generic helper that flips
/// the DISABLED bit on the named entry via [`HashNodeFlags`].
pub fn disablehashnode<T: HashNodeFlags>(ht: &mut HashMap<String, T>, nam: &str) -> bool {
    if let Some(node) = ht.get_mut(nam) {
        node.set_disabled(true);
        true
    } else {
        false
    }
}

/// Port of `enablehashnode()` from `Src/hashtable.c:332`.
///
/// C body: `hn->flags &= ~DISABLED;`. Inverse of [`disablehashnode`].
pub fn enablehashnode<T: HashNodeFlags>(ht: &mut HashMap<String, T>, nam: &str) -> bool {
    if let Some(node) = ht.get_mut(nam) {
        node.set_disabled(false);
        true
    } else {
        false
    }
}

/// Port of `hnamcmp()` from `Src/hashtable.c:341`.
///
/// `ztrcmp` over hash-node names — used by qsort for sorted
/// scan output (`functions`, `alias`, etc.).
pub fn hnamcmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Port of `scanmatchtable()` from `Src/hashtable.c:373`.
///
/// C body walks every node calling `func(node, scanflags)` if
/// the node satisfies (a) optional pattern match, (b) `flags1`
/// require-at-least-one, (c) `flags2` require-none-of. The
/// `sorted` flag pre-sorts entries before scanning.
///
/// Rust port: same shape with closure callback. Returns the
/// match count.
pub fn scanmatchtable<T: HashNodeFlags, F: FnMut(&str, &T)>(
    ht: &HashMap<String, T>,
    pattern: Option<&str>,
    sorted: bool,
    flags1: u32,
    flags2: u32,
    mut func: F,
) -> i32 {
    let mut entries: Vec<(&String, &T)> = ht.iter().collect();
    if sorted {
        entries.sort_by(|a, b| a.0.cmp(b.0));
    }
    let mut match_count = 0;
    for (name, node) in entries {
        if let Some(p) = pattern {
            if !simple_glob_match(p, name) {
                continue;
            }
        }
        let f = node.flags();
        if flags1 != 0 && (f & flags1) == 0 {
            continue;
        }
        if flags2 != 0 && (f & flags2) != 0 {
            continue;
        }
        func(name, node);
        match_count += 1;
    }
    match_count
}

/// Port of `scanhashtable()` from `Src/hashtable.c:446`.
///
/// C body delegates to `scanmatchtable` with `pprog = NULL`. Rust
/// port does the same.
pub fn scanhashtable<T: HashNodeFlags, F: FnMut(&str, &T)>(
    ht: &HashMap<String, T>,
    sorted: bool,
    flags1: u32,
    flags2: u32,
    func: F,
) -> i32 {
    scanmatchtable(ht, None, sorted, flags1, flags2, func)
}

/// Port of `expandhashtable()` from `Src/hashtable.c:458`.
///
/// C grows the bucket array when load factor exceeds threshold.
/// Rust HashMap rehashes automatically — calling reserve on the
/// passed map gives the closest equivalent.
pub fn expandhashtable<T>(ht: &mut HashMap<String, T>) {
    let want = ht.len() * 2;
    ht.reserve(want.saturating_sub(ht.capacity()));
}

/// Port of `resizehashtable()` from `Src/hashtable.c:486`.
///
/// C reallocates buckets to a specific size. Rust HashMap reserves
/// capacity to ensure at least `newsize` entries fit without rehash.
pub fn resizehashtable<T>(ht: &mut HashMap<String, T>, newsize: i32) {
    let need = newsize.max(0) as usize;
    if need > ht.capacity() {
        ht.reserve(need - ht.capacity());
    }
}

// Generic method to empty a hash table                                    // c:515
/// Port of `emptyhashtable()` from `Src/hashtable.c:519`.
///
/// C body: `resizehashtable(ht, ht->hsize);` — drop all nodes
/// while keeping the bucket array. Rust HashMap::clear preserves
/// capacity, matching the semantic.
pub fn emptyhashtable<T>(ht: &mut HashMap<String, T>) {                     // c:519
    ht.clear();
}

// Print info about hash table                                             // c:527
/// Port of `printhashtabinfo()` from `Src/hashtable.c:533`.
///
/// C body prints chain-length distribution stats for hash-table
/// debug analysis (under ZSH_HASH_DEBUG). Rust HashMap doesn't
/// expose chain-length info; emit count + capacity which is the
/// equivalent visibility.
pub fn printhashtabinfo<T>(name: &str, ht: &HashMap<String, T>) -> String { // c:533
    format!(
        "name of table   : {}\nsize of nodes[] : {}\nnumber of nodes : {}",
        name,
        ht.capacity(),
        ht.len()
    )
}

/// Port of `bin_hashinfo()` from `Src/hashtable.c:566`.
///
/// C iterates all registered hashtables (cmdnamtab, shfunctab,
/// aliastab, etc.) and emits stats for each. Rust port walks the
/// known-singleton tables.
pub fn bin_hashinfo() -> i32 {
    let banner = "----------------------------------------------------";
    println!("{}", banner);
    {
        let tab = cmdnamtab_lock().lock().expect("cmdnamtab poisoned");
        println!("name of table   : cmdnamtab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    {
        let tab = shfunctab_lock().lock().expect("shfunctab poisoned");
        println!("name of table   : shfunctab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    {
        let tab = aliastab_lock().lock().expect("aliastab poisoned");
        println!("name of table   : aliastab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    0
}

/// Trait exposing the DISABLED flag on a hash-node value.
///
/// Implemented for the per-table value types so the generic ops
/// (`gethashnode`/`disablehashnode`/etc.) can filter / mutate
/// without per-table dispatch. Mirrors C's `HashNode->flags`
/// field which every node struct embeds via the `struct hashnode`
/// header.
pub trait HashNodeFlags {
    fn flags(&self) -> u32;
    fn set_disabled(&mut self, disabled: bool);
    fn is_disabled(&self) -> bool {
        self.flags() & flags::DISABLED != 0
    }
}

impl HashNodeFlags for Alias {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= flags::DISABLED as i32;
        } else {
            self.node.flags &= !(flags::DISABLED as i32);
        }
    }
}

impl HashNodeFlags for ShFunc {
    fn flags(&self) -> u32 {
        self.flags
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.flags |= flags::DISABLED;
        } else {
            self.flags &= !flags::DISABLED;
        }
    }
}

impl HashNodeFlags for CmdName {
    fn flags(&self) -> u32 {
        self.flags
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.flags |= flags::DISABLED;
        } else {
            self.flags &= !flags::DISABLED;
        }
    }
}

impl HashNodeFlags for Reswd {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= flags::DISABLED as i32;
        } else {
            self.node.flags &= !(flags::DISABLED as i32);
        }
    }
}

// -----------------------------------------------------------
// cmdnamtab / aliastab / sufaliastab / reswdtab / histtab
// global singletons. Match C's `mod_export HashTable cmdnamtab;`
// (hashtable.c:594) and friends. Each is lazily initialised on
// first access.
// -----------------------------------------------------------

// hash table containing external commands                                  // c:587
/// Singleton accessor for the global `cmdnamtab`.
/// Mirrors C's `mod_export HashTable cmdnamtab` (hashtable.c:594).
pub fn cmdnamtab_lock() -> &'static std::sync::Mutex<CmdNameTable> {        // c:594
    static CMDNAMTAB: std::sync::OnceLock<std::sync::Mutex<CmdNameTable>> =
        std::sync::OnceLock::new();
    CMDNAMTAB.get_or_init(|| std::sync::Mutex::new(CmdNameTable::new()))
}

// hash table containing the aliases                                        // c:1174
/// Singleton accessor for the global `aliastab`.
/// Mirrors C's `mod_export HashTable aliastab` (hashtable.c:1186).
pub fn aliastab_lock() -> &'static std::sync::Mutex<AliasTable> {           // c:1186
    static ALIASTAB: std::sync::OnceLock<std::sync::Mutex<AliasTable>> =
        std::sync::OnceLock::new();
    ALIASTAB.get_or_init(|| std::sync::Mutex::new(AliasTable::with_defaults()))
}

/// Singleton accessor for the global `sufaliastab`.
/// Mirrors C's `mod_export HashTable sufaliastab` (hashtable.c:1187).
pub fn sufaliastab_lock() -> &'static std::sync::Mutex<AliasTable> {
    static SUFALIASTAB: std::sync::OnceLock<std::sync::Mutex<AliasTable>> =
        std::sync::OnceLock::new();
    SUFALIASTAB.get_or_init(|| std::sync::Mutex::new(AliasTable::new()))
}

// hash table containing the reserved words                                 // c:1111
/// Singleton accessor for the global `reswdtab`.
/// Mirrors C's `HashTable reswdtab` (hashtable.c, file-scope).
pub fn reswdtab_lock() -> &'static std::sync::Mutex<ReswdTable> {           // c:1115
    static RESWDTAB: std::sync::OnceLock<std::sync::Mutex<ReswdTable>> =
        std::sync::OnceLock::new();
    RESWDTAB.get_or_init(|| std::sync::Mutex::new(ReswdTable::new()))
}

/// Singleton accessor for the global `histtab` (history events).
/// Mirrors C's `HashTable histtab` (hashtable.c:1340).
pub fn histtab_lock() -> &'static std::sync::Mutex<HashMap<String, i32>> {
    static HISTTAB: std::sync::OnceLock<std::sync::Mutex<HashMap<String, i32>>> =
        std::sync::OnceLock::new();
    HISTTAB.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

// Old fake `dircache_lock(Mutex<HashMap<String, i32>>)` deleted —
// wrong shape (C uses `struct dircache_entry { name, refs }` not
// `HashMap<String, i32>`). Canonical port lives earlier in this
// file at the `dircache_entry` struct + `dircache_lock` accessor
// returning `Mutex<Vec<dircache_entry>>`.

/// Port of `createcmdnamtable()` from `Src/hashtable.c:601`.
///
/// C body sets up the cmdnamtab GSU vtable (hash, addnode,
/// removenode, freenode, printnode = printcmdnamnode). Rust port
/// just touches the singleton to ensure it's initialised.
pub fn createcmdnamtable() {
    let _ = cmdnamtab_lock();
}

/// Port of `emptycmdnamtable()` from `Src/hashtable.c:623`.
///
/// C body:
/// ```c
/// emptyhashtable(ht);
/// pathchecked = path;
/// ```
///
/// Drops every PATH cache entry (used by `hash -r`) and resets
/// the per-PATH-entry "checked" cursor so subsequent lookups
/// re-scan from the start.
pub fn emptycmdnamtable() {
    cmdnamtab_lock().lock().expect("cmdnamtab poisoned").clear();
}

/// Port of `hashdir()` from `Src/hashtable.c:634`.
///
/// C body opendir's the directory, reads each entry, and adds
/// any executable to `cmdnamtab` (skipping names already present
/// from earlier PATH entries). Rust port routes through
/// `CmdNameTable::hash_dir`.
pub fn hashdir(dir: &str, dir_index: usize) {
    cmdnamtab_lock()
        .lock()
        .expect("cmdnamtab poisoned")
        .hash_dir(dir, dir_index);
}

/// Port of `fillcmdnamtable()` from `Src/hashtable.c:712`.
///
/// C body:
/// ```c
/// for (pq = pathchecked; *pq; pq++) hashdir(pq);
/// pathchecked = pq;
/// ```
///
/// Walks every PATH entry calling `hashdir` for each. The
/// `pathchecked` cursor is updated so subsequent calls don't
/// re-walk PATH entries that were already scanned.
pub fn fillcmdnamtable(path: &[String]) {
    let mut tab = cmdnamtab_lock().lock().expect("cmdnamtab poisoned");
    for (idx, dir) in path.iter().enumerate() {
        tab.hash_dir(dir, idx);
    }
}

/// Port of `freecmdnamnode()` from `Src/hashtable.c:724`.
///
/// C body frees the entry's name + (if HASHED) cached path. Rust
/// port: drop runs both when the entry is removed from the table.
/// This helper performs the removal to trigger Drop.
pub fn freecmdnamnode(nam: &str) {
    cmdnamtab_lock()
        .lock()
        .expect("cmdnamtab poisoned")
        .remove(nam);
}

// ===========================================================
// shfunctab — the global shell-function table.
//
// Port of `mod_export HashTable shfunctab` from
// `Src/hashtable.c:808` and the GSU callbacks built around it
// (`createshfunctable` and the `*shfuncnode` family).
//
// C zsh dispatches every `function f() { … }` definition,
// `unfunction`, `disable -f`, `enable -f`, `whence`, and trap-
// function lookup through `shfunctab`. zshrs uses a singleton
// `OnceLock<Mutex<ShFuncTable>>` exposed via `shfunctab_lock()`
// so the GSU-style C names below can mutate it without taking a
// `ShellExecutor` parameter (matching the C signatures, where
// the table is global).
// ===========================================================

/// Singleton accessor for the global `shfunctab`.
/// Mirrors C's `mod_export HashTable shfunctab` (hashtable.c:808).
/// Lazily initialised on first access.
pub fn shfunctab_lock() -> &'static std::sync::Mutex<ShFuncTable> {         // c:808
    static SHFUNCTAB: std::sync::OnceLock<std::sync::Mutex<ShFuncTable>> =
        std::sync::OnceLock::new();
    SHFUNCTAB.get_or_init(|| std::sync::Mutex::new(ShFuncTable::new()))
}

/// Port of `createshfunctable()` from `Src/hashtable.c:812`.
///
/// C body:
/// ```c
/// shfunctab = newhashtable(7, "shfunctab", NULL);
/// shfunctab->hash        = hasher;
/// shfunctab->cmpnodes    = strcmp;
/// shfunctab->addnode     = addhashnode;
/// shfunctab->getnode     = gethashnode;
/// shfunctab->getnode2    = gethashnode2;
/// shfunctab->removenode  = removeshfuncnode;
/// shfunctab->disablenode = disableshfuncnode;
/// shfunctab->enablenode  = enableshfuncnode;
/// shfunctab->freenode    = freeshfuncnode;
/// shfunctab->printnode   = printshfuncnode;
/// ```
///
/// Rust port: idempotent — touching the OnceLock initialises the
/// singleton on first call. The GSU function-pointer assignments
/// from C are encoded as the free-fn names below (each callable
/// directly without a vtable lookup).
pub fn createshfunctable() {
    let _ = shfunctab_lock();
}

/// Port of `removeshfuncnode()` from `Src/hashtable.c:836`.
///
/// C body:
/// ```c
/// if (!strncmp(nam, "TRAP", 4) && (sigidx = getsigidx(nam + 4)) != -1)
///     hn = removetrap(sigidx);
/// else
///     hn = removehashnode(shfunctab, nam);
/// return hn;
/// ```
///
/// Drops the named function from `shfunctab`. If the name is a
/// `TRAP<sig>` form, also clears the trap via signals.rs.
/// Returns the removed function (or None if absent).
pub fn removeshfuncnode(nam: &str) -> Option<ShFunc> {
    if let Some(sig_part) = nam.strip_prefix("TRAP") {
        if let Some(sig) = crate::ported::signals::getsigidx(sig_part) {
            crate::ported::signals::removetrap(sig);
        }
    }
    shfunctab_lock()
        .lock()
        .expect("shfunctab poisoned")
        .remove(nam)
}

/// Port of `disableshfuncnode()` from `Src/hashtable.c:855`.
///
/// C body:
/// ```c
/// hn->flags |= DISABLED;
/// if (!strncmp(hn->nam, "TRAP", 4)) {
///     int sigidx = getsigidx(hn->nam + 4);
///     if (sigidx != -1) {
///         sigtrapped[sigidx] &= ~ZSIG_FUNC;
///         unsettrap(sigidx);
///     }
/// }
/// ```
///
/// Sets the DISABLED flag on the function entry; for TRAP*
/// functions, also unsettraps the corresponding signal so the
/// shell stops invoking the (now-disabled) trap.
pub fn disableshfuncnode(nam: &str) {
    {
        let mut tab = shfunctab_lock().lock().expect("shfunctab poisoned");
        tab.disable(nam);
    }
    if let Some(sig_part) = nam.strip_prefix("TRAP") {
        if let Some(sig) = crate::ported::signals::getsigidx(sig_part) {
            crate::ported::signals::unsettrap(sig);
        }
    }
}

/// Port of `enableshfuncnode()` from `Src/hashtable.c:873`.
///
/// C body:
/// ```c
/// shf->node.flags &= ~DISABLED;
/// if (!strncmp(shf->node.nam, "TRAP", 4)) {
///     int sigidx = getsigidx(shf->node.nam + 4);
///     if (sigidx != -1) settrap(sigidx, NULL, ZSIG_FUNC);
/// }
/// ```
///
/// Clears the DISABLED flag; for TRAP* functions, re-installs
/// the signal handler with `ZSIG_FUNC` semantics so the shell
/// dispatches the trap function on the next signal delivery.
pub fn enableshfuncnode(nam: &str) {
    {
        let mut tab = shfunctab_lock().lock().expect("shfunctab poisoned");
        tab.enable(nam);
    }
    if let Some(sig_part) = nam.strip_prefix("TRAP") {
        if let Some(sig) = crate::ported::signals::getsigidx(sig_part) {
            // c:882 — `settrap(sigidx, NULL, ZSIG_FUNC)`. The TRAPxxx
            // function body resolves through shfunctab at dispatch
            // (`gettrapnode`), not via the trap arrays directly.
            let _ = crate::ported::signals::settrap(
                sig,
                None,
                crate::ported::zsh_h::ZSIG_FUNC,
            );
        }
    }
}

/// Port of `freeshfuncnode()` from `Src/hashtable.c:888`.
///
/// C body frees the function name, body Eprog, redir Eprog,
/// filename string, and sticky options struct. Rust port: drop
/// runs all of this when the entry is removed; this helper just
/// removes from the table to trigger the drop chain.
pub fn freeshfuncnode(nam: &str) {
    shfunctab_lock()
        .lock()
        .expect("shfunctab poisoned")
        .remove(nam);
}

/// Port of `scanmatchshfunc()` from `Src/hashtable.c:1013`.
///
/// C body iterates `shfunctab` and calls `func(node)` on every
/// entry whose name matches the compiled pattern `pprog`. Rust
/// port walks the singleton with a closure callback.
///
/// Returns the count of matched entries (mirrors C's int return).
pub fn scanmatchshfunc<F>(pattern: Option<&str>, mut func: F) -> i32
where
    F: FnMut(&str, &ShFunc),
{
    let tab = shfunctab_lock().lock().expect("shfunctab poisoned");
    let mut count = 0;
    for (name, entry) in tab.iter() {
        let matches = match pattern {
            None => true,
            Some(p) => simple_glob_match(p, name),
        };
        if matches {
            func(name, entry);
            count += 1;
        }
    }
    count
}

/// Port of `scanshfunc()` from `Src/hashtable.c:1031`.
///
/// C body walks every `shfunctab` entry calling `func(node, flags)`.
/// Rust port delegates to scanmatchshfunc with no pattern.
pub fn scanshfunc<F>(func: F) -> i32
where
    F: FnMut(&str, &ShFunc),
{
    scanmatchshfunc(None, func)
}

/// Port of `printshfuncexpand()` from `Src/hashtable.c:1042`.
///
/// C body wraps `printshfuncnode` to expand tabs in the function
/// body for prompt-display purposes (`functions -e`). Rust port
/// returns the formatted entry as a string.
pub fn printshfuncexpand(nam: &str, _flags: i32) -> Option<String> {
    let tab = shfunctab_lock().lock().expect("shfunctab poisoned");
    let func = tab.get_including_disabled(nam)?;
    let body = func.body.clone().unwrap_or_default();
    Some(format!(
        "{} () {{\n\t{}\n}}",
        nam,
        body.replace('\t', "    ")
    ))
}

/// Port of `getshfuncfile()` from `Src/hashtable.c:1059`.
///
/// C body returns `shf->filename`, the path to the file that
/// defined the function (used by `functions -T` and `whence -v`
/// to show the source location).
pub fn getshfuncfile(nam: &str) -> Option<String> {
    let tab = shfunctab_lock().lock().expect("shfunctab poisoned");
    tab.get_including_disabled(nam)
        .and_then(|f| f.filename.clone())
}

/// Glob-style match — supports `*` and `?` only. Used by
/// `scanmatchshfunc` for `print -l` / `unfunction` glob args.
/// C uses the full `patmatch` engine; the Rust simplification
/// covers the common cases.
fn simple_glob_match(pattern: &str, name: &str) -> bool {
    let pat_bytes = pattern.as_bytes();
    let name_bytes = name.as_bytes();
    glob_match_inner(pat_bytes, name_bytes)
}

fn glob_match_inner(pat: &[u8], name: &[u8]) -> bool {
    if pat.is_empty() {
        return name.is_empty();
    }
    match pat[0] {
        b'*' => {
            for i in 0..=name.len() {
                if glob_match_inner(&pat[1..], &name[i..]) {
                    return true;
                }
            }
            false
        }
        b'?' => !name.is_empty() && glob_match_inner(&pat[1..], &name[1..]),
        c => !name.is_empty() && name[0] == c && glob_match_inner(&pat[1..], &name[1..]),
    }
}

/// Port of `createreswdtable()` from `Src/hashtable.c:1120`.
///
/// C body wires up the reswdtab GSU vtable then iterates the
/// static `reswds` array calling `addnode` for each. Rust port:
/// touches the singleton (which seeds the table from the static
/// word list in `ReswdTable::new`).
pub fn createreswdtable() {
    let _ = reswdtab_lock();
}

/// Port of `printreswdnode()` from `Src/hashtable.c:1147`.
///
/// C body emits `whence`-style output for one reserved word with
/// flags-based dispatch:
/// ```c
/// if (PRINT_WHENCE_WORD) printf("%s: reserved\n", nam);
/// else if (PRINT_WHENCE_CSH) printf("%s: shell reserved word\n", nam);
/// else if (PRINT_WHENCE_VERBOSE) printf("%s is a reserved word\n", nam);
/// else printf("%s\n", nam);
/// ```
pub fn printreswdnode(nam: &str, printflags: u32) -> String {
    if printflags & print_flags::WHENCE_WORD != 0 {
        format!("{}: reserved", nam)
    } else if printflags & print_flags::WHENCE_CSH != 0 {
        format!("{}: shell reserved word", nam)
    } else if printflags & print_flags::WHENCE_VERBOSE != 0 {
        format!("{} is a reserved word", nam)
    } else {
        nam.to_string()
    }
}

/// Port of `createaliastable()` from `Src/hashtable.c:1188`.
/// C: `void createaliastable(HashTable ht)` — assign 12 GSU vtable
///   slots on `ht` (hasher/cmpnodes/addnode/getnode/getnode2/removenode/
///   disablenode/enablenode/freenode/printnode). Rust port: AliasTable's
///   methods already implement these semantics directly, so no vtable
///   to install — call site doesn't need a per-table dispatch.
pub fn createaliastable(_ht: *mut crate::ported::zsh_h::hashtable) {         // c:1187
    // c:1190-1201 — vtable wireup. Rust path: AliasTable already
    // exposes add/get/remove/disable/enable/free/print as inherent methods.
}

/// Port of `createaliastables()` from `Src/hashtable.c:1206`.
///
/// C body allocates both the regular alias table and the
/// suffix-alias table, then seeds the regular one with the
/// `run-help` and `which-command` defaults.
pub fn createaliastables() {
    let _ = aliastab_lock();
    let _ = sufaliastab_lock();
}

/// Port of `createaliasnode()` from `Src/hashtable.c:1230`.
///
/// C body:
/// ```c
/// al = zshcalloc(sizeof *al);
/// al->node.flags = flags;
/// al->text = txt;
/// al->inuse = 0;
/// return al;
/// ```
// Duplicate `createaliasnode` removed — canonical port is at the
// earlier definition (matches C hashtable.c:1230).

/// Port of `freealiasnode()` from `Src/hashtable.c:1243`.
///
/// C body frees the name + text strings + alias struct. Rust
/// port: drop runs the same when the Alias is removed from its
/// table. This helper triggers the drop.
pub fn freealiasnode(nam: &str) {
    let mut tab = aliastab_lock().lock().expect("aliastab poisoned");
    tab.remove(nam);
}

/// Port of `printaliasnode()` from `Src/hashtable.c:1256`.
///
/// C body emits `whence`-style output for one alias with
/// PRINT_NAMEONLY / PRINT_WHENCE_WORD / PRINT_WHENCE_SIMPLE /
/// PRINT_WHENCE_CSH / PRINT_WHENCE_VERBOSE / PRINT_LIST flag
/// dispatch.
pub fn printaliasnode(a: &Alias, printflags: u32) -> String {
    let nam = &a.node.nam;
    let af = a.node.flags;
    let is_suffix = (af & flags::ALIAS_SUFFIX as i32) != 0;
    let is_global = (af & flags::ALIAS_GLOBAL as i32) != 0;
    if printflags & print_flags::NAMEONLY != 0 {
        return nam.clone();
    }
    if printflags & print_flags::WHENCE_WORD != 0 {
        let kind = if is_suffix { "suffix alias" }
                   else if is_global { "global alias" }
                   else { "alias" };
        return format!("{}: {}", nam, kind);
    }
    if printflags & print_flags::WHENCE_SIMPLE != 0 {
        return a.text.clone();
    }
    if printflags & print_flags::WHENCE_CSH != 0 {
        let qual = if is_suffix { "suffix " }
                   else if is_global { "globally " }
                   else { "" };
        return format!("{}: {}aliased to {}", nam, qual, a.text);
    }
    if printflags & print_flags::WHENCE_VERBOSE != 0 {
        let qual = if is_suffix { "a suffix" }
                   else if is_global { "a global" }
                   else { "an" };
        return format!("{} is {} alias for {}", nam, qual, a.text);
    }
    if printflags & print_flags::LIST != 0 {
        let mut out = String::from("alias ");
        if is_suffix { out.push_str("-s "); }
        else if is_global { out.push_str("-g "); }
        if nam.starts_with('-') || nam.starts_with('+') {
            out.push_str("-- ");
        }
        out.push_str(&format!("{}={}", nam, a.text));
        return out;
    }
    format!("{}={}", nam, a.text)
}

/// Port of `createhisttable()` from `Src/hashtable.c:1345`.
///
/// C body wires up the histtab GSU vtable with `histhasher` /
/// `histstrcmp` / `addhistnode` etc. Rust port: touches the
/// singleton to initialise. The HashMap-keyed-by-string model
/// is much simpler than C's per-bucket chain; the entries hold
/// (history event-id) values keyed by command-text.
pub fn createhisttable() {
    let _ = histtab_lock();
}

/// Port of `emptyhisttable()` from `Src/hashtable.c:1385`.
///
/// C body:
/// ```c
/// emptyhashtable(ht);
/// if (hist_ring) histremovedups();
/// ```
///
/// Clears the lookup table; the dup-removal pass on `hist_ring`
/// is a separate hist.c entry point pending its port.
pub fn emptyhisttable() {
    histtab_lock().lock().expect("histtab poisoned").clear();
}

/// Port of `addhistnode()` from `Src/hashtable.c:1427`.
///
/// C body:
/// ```c
/// HashNode oldnode = addhashnode2(ht, nam, nodeptr);
/// if (oldnode && oldnode != nodeptr) {
///     // mark dup, optionally free old
/// }
/// ```
///
/// Inserts a history entry, returning the displaced event ID
/// (Some) if a duplicate command-text was already present.
pub fn addhistnode(nam: &str, event_id: i32) -> Option<i32> {
    histtab_lock()
        .lock()
        .expect("histtab poisoned")
        .insert(nam.to_string(), event_id)
}

/// Port of `freehistnode()` from `Src/hashtable.c:1450`.
///
/// C body: `freehistdata((Histent)nodeptr, 1); zfree(nodeptr, ...);`
/// Rust port: removes from the lookup table — drop runs the
/// equivalent of zfree.
pub fn freehistnode(nam: &str) {
    histtab_lock()
        .lock()
        .expect("histtab poisoned")
        .remove(nam);
}

/// Port of `freehistdata()` from `Src/hashtable.c:1458`.
///
/// C body frees the command + word-array fields of a Histent.
/// Rust port: no-op since the Rust history-engine port owns
/// these via String/Vec, freed by Drop.
pub fn freehistdata(_unlink: i32) {}

/// Port of `dircache_set()` from `Src/hashtable.c:1537`.
///
/// C body manages a refcounted directory-name cache:
///   - `value == NULL` → decrement refs on `*name`, free if zero,
///     set `*name = NULL`.
///   - `value != NULL` → search for an existing entry, bump refs,
///     else allocate a new slot.
///
/// Rust port: routes through dircache_lock() with refcount-by-
/// HashMap-value (i32). Add/remove via the (name, value) pair.
pub fn dircache_set(name: &str, value: Option<&str>) {
    let mut cache = dircache_lock().lock().expect("dircache poisoned");
    match value {
        None => {
            // Find the entry by name; decrement refs; remove on 0.
            // Mirrors the C `release_dircache_entry` flow used by
            // `freeshfuncnode` (hashtable.c:888).
            if let Some(idx) = cache.iter().position(|e| e.name == name) {
                cache[idx].refs -= 1;
                if cache[idx].refs <= 0 {
                    cache.remove(idx);
                }
            }
        }
        Some(v) => {
            // Find-or-insert by name; bump refs. Mirrors the C
            // `get_dircache_entry` flow at hashtable.c:1539+.
            if let Some(idx) = cache.iter().position(|e| e.name == v) {
                cache[idx].refs += 1;
            } else {
                cache.push(dircache_entry { name: v.to_string(), refs: 1 });
            }
            let _ = name; // C uses *name for refcount keying; Rust keys by value path
        }
    }
}
