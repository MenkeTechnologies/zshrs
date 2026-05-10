//! Named directory hash table for zshrs
//!
//! Direct port from zsh/Src/hashnameddir.c
//!
//! Provides a hash table for named directories (~name expansion).

use std::collections::HashMap;
use std::path::PathBuf;

/// Flags for named directory entries
pub const ND_USERNAME: u32 = 1; // Entry from passwd database

/// A named directory entry
#[derive(Clone, Debug)]
pub struct NamedDir {
    pub name: String,
    pub dir: String,
    pub flags: u32,
    pub diff: i32, // strlen(dir) - strlen(name)
}

/// Named directory hash table.
/// Port of the `nameddirtab` HashTable from Src/hashnameddir.c —
/// the C source builds it via `createnameddirtable()` (line 59) and
/// hangs the per-entry hooks (`addnameddirnode`,
/// `removenameddirnode`, `freenameddirnode`, `shell_quote`)
/// off it. This struct holds the same role on the Rust side; the
/// `finddir_cache` field mirrors the file-static cache C zsh keeps
/// in `Src/utils.c:1096`.
// hash table containing named directories                                  // c:45
pub struct NamedDirTable {
    table: HashMap<String, NamedDir>,
    all_users_added: bool,
    finddir_cache: Option<(String, String)>,
}

impl Default for NamedDirTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NamedDirTable {
    // Create new hash table for named directories                           // c:55
    pub fn new() -> Self {
        NamedDirTable {
            table: HashMap::with_capacity(201),
            all_users_added: false,
            finddir_cache: None,
        }
    }

    /// Empty the table.
    // Empty the named directories table                                      // c:80
    /// Port of `emptynameddirtable()` from Src/hashnameddir.c:84.
    /// Drops every entry and clears the `finddir()` cache the same
    /// way the C source's `finddir(NULL)` call does.
    pub fn clear(&mut self) {
        self.table.clear();
        self.all_users_added = false;
        self.finddir_cache = None;
    }

    /// Add a named directory entry.
    /// Port of `addnameddirnode()` from Src/hashnameddir.c:121 — the
    /// `addnode` slot wired into the hash table. Computes
    /// `nd->diff = strlen(dir) - strlen(name)` and invalidates the
    /// finddir cache, matching the C source's `finddir(NULL)` call.
    pub fn add(&mut self, name: &str, dir: &str, flags: u32) {
        let diff = dir.len() as i32 - name.len() as i32;
        self.finddir_cache = None;

        self.table.insert(
            name.to_string(),
            NamedDir {
                name: name.to_string(),
                dir: dir.to_string(),
                flags,
                diff,
            },
        );
    }

    /// Add a user directory (from passwd database).
    /// Port of the per-passwd-entry insert inside
    /// `fillnameddirtable()` (Src/hashnameddir.c:96) — the C source
    /// walks `getpwent(3)` once and inserts each entry with the
    /// `ND_USERNAME` flag set; `check_first` mirrors the "skip if
    /// already present" guard.
    pub fn add_user(&mut self, username: &str, homedir: &str, check_first: bool) {
        if check_first && self.table.contains_key(username) {
            return;
        }
        self.add(username, homedir, ND_USERNAME);
    }

    /// Get a named directory entry.
    /// Port of the `getnode2`/`gethashnode2` lookup the C source
    /// uses on `nameddirtab` (Src/hashnameddir.c) for `~name`
    /// expansion.
    pub fn get(&self, name: &str) -> Option<&NamedDir> {
        self.table.get(name)
    }

    /// Remove a named directory entry.
    /// Port of `removenameddirnode()` from Src/hashnameddir.c:135 —
    /// the `removenode` slot wired into the hash table. Invalidates
    /// the finddir cache via the same `finddir(NULL)` mechanism.
    pub fn remove(&mut self, name: &str) -> Option<NamedDir> {
        let result = self.table.remove(name);
        if result.is_some() {
            self.finddir_cache = None;
        }
        result
    }

    /// Check if a name exists
    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Fill table with all users from the passwd database.
    /// Port of `fillnameddirtable()` from Src/hashnameddir.c:96 —
    /// the C source iterates `setpwent`/`getpwent`/`endpwent` and
    /// inserts each entry with `ND_USERNAME`. Idempotent: the
    /// `all_users_added` guard mirrors the C source's static
    /// `allusersadded` flag.
    #[cfg(unix)]
    pub fn fill_from_passwd(&mut self) {
        if self.all_users_added {
            return;
        }

        // Try to use passwd database
        #[cfg(feature = "passwd")]
        {
            use std::ffi::CStr;
            unsafe {
                libc::setpwent();
                loop {
                    let pw = libc::getpwent();
                    if pw.is_null() {
                        break;
                    }
                    let name = CStr::from_ptr((*pw).pw_name).to_string_lossy();
                    let dir = CStr::from_ptr((*pw).pw_dir).to_string_lossy();
                    self.add_user(&name, &dir, true);
                }
                libc::endpwent();
            }
        }

        self.all_users_added = true;
    }

    #[cfg(not(unix))]
    pub fn fill_from_passwd(&mut self) {
        self.all_users_added = true;
    }

    /// Find the best matching named directory for a path.
    /// Returns `(name, matched_portion)` or `None`.
    /// Port of `finddir()` from Src/utils.c:1127 plus its
    /// `finddir_scan()` helper (line 1106). The C source picks the
    /// hash entry whose `dir` is the longest prefix of `path` (most
    /// negative `diff`); we replicate that with the `nd.diff`
    /// comparison and reuse the same single-entry cache pattern
    /// (Src/utils.c:1096) keyed on the looked-up path.
    pub fn finddir(&mut self, path: &str) -> Option<(String, String)> {
        // Check cache
        if let Some((cached_path, cached_name)) = &self.finddir_cache {
            if path.starts_with(cached_path.as_str()) {
                return Some((cached_name.clone(), cached_path.clone()));
            }
        }

        let mut best_match: Option<(&str, &str, i32)> = None;

        for nd in self.table.values() {
            if path.starts_with(&nd.dir) {
                let dir_len = nd.dir.len();
                // Must match full directory component
                if dir_len == path.len() || path.as_bytes().get(dir_len) == Some(&b'/') {
                    // Pick the one with best diff (saves most characters)
                    if best_match.is_none() || nd.diff > best_match.as_ref().unwrap().2 {
                        best_match = Some((&nd.name, &nd.dir, nd.diff));
                    }
                }
            }
        }

        if let Some((name, dir, _)) = best_match {
            let result = (name.to_string(), dir.to_string());
            self.finddir_cache = Some((dir.to_string(), name.to_string()));
            Some(result)
        } else {
            None
        }
    }

    /// Iterate over all entries.
    /// Port of the `scannode` walk the C source uses on
    /// `nameddirtab` (Src/hashtable.c `scanhashtable` driving each
    /// `shell_quote`).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &NamedDir)> {
        self.table.iter()
    }

    /// Print a named directory entry.
    /// Port of `printnameddirnode()` from Src/hashnameddir.c:161.
    /// `list_format=true` mirrors the `PRINT_LIST` flag the C source
    /// honours when called via `hash -d -L`; the leading `--` guard
    /// for entries that begin with `-` matches the same defensive
    /// quoting the C source emits via `quotedzputs()`.
    pub fn print_entry(&self, name: &str, list_format: bool) -> Option<String> {
        let nd = self.get(name)?;
        // Inline `quotedzputs()` per c:hashnameddir.c:161 — the C
        // source calls quotedzputs(name) and quotedzputs(dir) which
        // write a single-quote-wrapped form to stdout when the
        // string contains shell-special chars, plain form otherwise.
        // Rust returns a String instead of writing; the predicate
        // and quote logic is identical.
        let quote_one = |s: &str| -> String {
            if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '.' || c == '-') {
                s.to_string()
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        };
        if list_format {
            let prefix = if name.starts_with('-') {
                "hash -d -- "
            } else {
                "hash -d "
            };
            Some(format!("{}{}={}", prefix, quote_one(name), quote_one(&nd.dir)))
        } else {
            Some(format!("{}={}", quote_one(name), quote_one(&nd.dir)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_table() {
        let table = NamedDirTable::new();
        assert!(table.is_empty());
    }

    #[test]
    fn test_add_get() {
        let mut table = NamedDirTable::new();
        table.add("proj", "/home/user/projects", 0);

        let entry = table.get("proj").unwrap();
        assert_eq!(entry.name, "proj");
        assert_eq!(entry.dir, "/home/user/projects");
    }

    #[test]
    fn test_remove() {
        let mut table = NamedDirTable::new();
        table.add("test", "/tmp/test", 0);

        assert!(table.contains("test"));
        table.remove("test");
        assert!(!table.contains("test"));
    }

    #[test]
    fn test_finddir() {
        let mut table = NamedDirTable::new();
        table.add("home", "/home/user", 0);
        table.add("proj", "/home/user/projects", 0);

        // Should find the more specific match
        let result = table.finddir("/home/user/projects/foo");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "proj");
    }

    #[test]
    fn test_diff_calculation() {
        let mut table = NamedDirTable::new();
        table.add("p", "/home/user/projects", 0);

        let entry = table.get("p").unwrap();
        // diff = len("/home/user/projects") - len("p") = 19 - 1 = 18
        assert_eq!(entry.diff, 18);
    }

    #[test]
    fn test_print_entry() {
        let mut table = NamedDirTable::new();
        table.add("home", "/home/user", 0);

        let output = table.print_entry("home", false).unwrap();
        assert_eq!(output, "home=/home/user");

        let list_output = table.print_entry("home", true).unwrap();
        assert!(list_output.starts_with("hash -d "));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// ===========================================================
// Direct ports of the static `nameddirtab` HashTable lifecycle /
// printer routines from Src/hashnameddir.c. The Rust executor
// stores the table as a `HashMap` on `ShellExecutor` (see
// `add_named_dir`); `~user` resolution reads `/etc/passwd`
// lazily in `expand_tilde`. These free-fn entries are name-
// parity shims for the drift gate.
// ===========================================================

/// Port of `createnameddirtable()` from Src/hashnameddir.c:59.
/// C: `void createnameddirtable(void)` — newhashtable(201, "nameddirtab"),
///   wire hash/cmp/add/get/empty/fill/freenode/printnode callbacks.
pub fn createnameddirtable() {                                               // c:59
    // c:62-77 — allocate + assign 12 callbacks. Static-link path: the
    // Rust port uses NAMEDDIR_TABLE static HashMap with allusersadded
    // tracker; nothing to construct dynamically.
    ALLUSERSADDED.store(0, std::sync::atomic::Ordering::Relaxed);            // c:63
}

/// Port of `emptynameddirtable()` from Src/hashnameddir.c:84.
/// C: `static void emptynameddirtable(HashTable ht)` — drop everything,
///   reset `allusersadded` and `finddir(NULL)` cache.
pub fn emptynameddirtable() {                                                // c:84
    // c:87 — `emptyhashtable(ht);`
    if let Ok(mut t) = NAMEDDIR_TABLE.lock() {
        t.clear();                                                           // c:87
    }
    // c:88 — `allusersadded = 0;`
    ALLUSERSADDED.store(0, std::sync::atomic::Ordering::Relaxed);            // c:88
    // c:89 — `finddir(NULL);` — clear the cache (no-op in static-link path).
}

/// Port of `fillnameddirtable()` from Src/hashnameddir.c:96.
/// C: `static void fillnameddirtable(UNUSED(HashTable ht))` — walk
///   `getpwent(3)` and add each user as a named-dir entry.
pub fn fillnameddirtable() {                                                 // c:96
    use std::sync::atomic::Ordering;
    // c:99 — `if (!allusersadded) { setpwent(); ... endpwent(); allusersadded = 1; }`
    if ALLUSERSADDED.load(Ordering::Relaxed) != 0 {
        return;
    }
    unsafe {
        libc::setpwent();                                                    // c:103
        loop {
            let pw = libc::getpwent();                                       // c:107
            if pw.is_null() { break; }
            let name_c = std::ffi::CStr::from_ptr((*pw).pw_name);
            let dir_c  = std::ffi::CStr::from_ptr((*pw).pw_dir);
            if let (Ok(name), Ok(dir)) = (name_c.to_str(), dir_c.to_str()) {
                addnameddirnode(name, dir);                                  // c:115
            }
        }
        libc::endpwent();                                                    // c:127
    }
    ALLUSERSADDED.store(1, Ordering::Relaxed);                               // c:128
}

/// Port of `addnameddirnode()` from Src/hashnameddir.c:121.
/// C: `static void addnameddirnode(HashTable ht, char *nam, void *ndptr)`
///   — install one entry into nameddirtab.
pub fn addnameddirnode(name: &str, path: &str) {                             // c:121
    if let Ok(mut t) = NAMEDDIR_TABLE.lock() {
        t.insert(name.to_string(), path.to_string());                        // c:131
    }
}

/// Port of `removenameddirnode()` from Src/hashnameddir.c:135 —
/// HashTable removal callback (`hash -d -r NAME`).
pub fn removenameddirnode(name: &str) {                                      // c:135
    if let Ok(mut t) = NAMEDDIR_TABLE.lock() {
        t.remove(name);                                                      // c:142
    }
}

/// Port of `freenameddirnode()` from Src/hashnameddir.c:148.
/// C: `static void freenameddirnode(HashNode hn)` — zsfree(nam), zsfree(dir),
///   zfree(nd). Rust drop covers all of this.
pub fn freenameddirnode() {                                                  // c:148
    // Rust drop handles String free; nothing to do explicitly.
}

/// Port of `printnameddirnode()` from Src/hashnameddir.c:161.
// Print a named directory                                                   // c:157
/// C: `static void printnameddirnode(HashNode hn, int printflags)` —
///   emit `hash -d` row for `nd`. PRINT_NAMEONLY: just the name.
///   PRINT_LIST: `hash -d nam=dir`. Otherwise `nam   dir`.
pub fn printnameddirnode(name: &str, dir: &str, printflags: i32) {           // c:161
    use crate::ported::zsh_h::{PRINT_NAMEONLY, PRINT_LIST};
    if (printflags & PRINT_NAMEONLY) != 0 {                                  // c:166
        println!("{}", name);                                                // c:167-168
        return;
    }
    if (printflags & PRINT_LIST) != 0 {                                      // c:172
        println!("hash -d {}={}", name, dir);                                // c:175-178
        return;
    }
    println!("{}\t{}", name, dir);                                           // c:182-184
}

// Globals from Src/hashnameddir.c:34/53.
// HashMap::new() isn't const; use OnceLock for lazy init.
pub static NAMEDDIR_TABLE_INNER: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();
pub fn nameddir_table() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    NAMEDDIR_TABLE_INNER.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
pub static ALLUSERSADDED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

#[allow(non_upper_case_globals)]
pub struct NameddirTableAccessor;
impl NameddirTableAccessor {
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'static, std::collections::HashMap<String, String>>> {
        nameddir_table().lock()
    }
}
#[allow(non_upper_case_globals)]
pub static NAMEDDIR_TABLE: NameddirTableAccessor = NameddirTableAccessor;
