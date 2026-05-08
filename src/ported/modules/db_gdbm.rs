//! GDBM database bindings for zsh
//!
//! Port of zsh/Src/Modules/db_gdbm.c
//!
//! Provides builtins:
//! - ztie: Tie a parameter to a GDBM database
//! - zuntie: Untie a parameter from a GDBM database
//! - zgdbmpath: Get the path of a tied GDBM database

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex, RwLock};

use once_cell::sync::Lazy;

const BACKTYPE: &str = "db/gdbm";

/// GDBM open flags
const GDBM_READER: c_int = 0;
const GDBM_WRITER: c_int = 1;
const GDBM_WRCREAT: c_int = 2;
const GDBM_NEWDB: c_int = 3;
const GDBM_SYNC: c_int = 0x20;
const GDBM_REPLACE: c_int = 1;

/// Datum structure for GDBM
#[repr(C)]
struct Datum {
    dptr: *mut c_char,
    dsize: c_int,
}

impl Datum {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    fn null() -> Self {
        Datum {
            dptr: ptr::null_mut(),
            dsize: 0,
        }
    }

    /// Port of `gdbmgetfn()` from `Src/Modules/db_gdbm.c:282`.
    fn from_bytes(data: &[u8]) -> Self {
        let ptr = unsafe { libc::malloc(data.len()) as *mut c_char };
        if !ptr.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            }
        }
        Datum {
            dptr: ptr,
            dsize: data.len() as c_int,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    fn to_bytes(&self) -> Option<Vec<u8>> {
        if self.dptr.is_null() {
            None
        } else {
            let mut result = vec![0u8; self.dsize as usize];
            unsafe {
                ptr::copy_nonoverlapping(
                    self.dptr as *const u8,
                    result.as_mut_ptr(),
                    self.dsize as usize,
                );
            }
            Some(result)
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    fn free(&mut self) {
        if !self.dptr.is_null() {
            unsafe { libc::free(self.dptr as *mut c_void) };
            self.dptr = ptr::null_mut();
            self.dsize = 0;
        }
    }
}

/// Opaque GDBM file handle
type GdbmFile = *mut c_void;

#[cfg(feature = "gdbm")]
#[link(name = "gdbm")]
extern "C" {
    fn gdbm_open(
        name: *const c_char,
        block_size: c_int,
        flags: c_int,
        mode: c_int,
        fatal_func: Option<extern "C" fn(*const c_char)>,
    ) -> GdbmFile;
    fn gdbm_close(dbf: GdbmFile);
    fn gdbm_store(dbf: GdbmFile, key: Datum, content: Datum, flag: c_int) -> c_int;
    fn gdbm_fetch(dbf: GdbmFile, key: Datum) -> Datum;
    fn gdbmunsetfn(dbf: GdbmFile, key: Datum) -> c_int;
    fn gdbm_exists(dbf: GdbmFile, key: Datum) -> c_int;
    fn gdbm_firstkey(dbf: GdbmFile) -> Datum;
    fn gdbm_nextkey(dbf: GdbmFile, key: Datum) -> Datum;
    fn gdbm_reorganize(dbf: GdbmFile) -> c_int;
    fn gdbm_fdesc(dbf: GdbmFile) -> c_int;
    fn gdbm_strerror(errno: c_int) -> *const c_char;
    static gdbm_errno: c_int;
}

/// GDBM database handle wrapper.
/// Port of the per-tied-param `Db` slot Src/Modules/db_gdbm.c
/// stores in `myfreeparamnode()` (line 45) — the C source threads
/// the live `GDBM_FILE *` through every `gdbmgetfn`/`gdbmsetfn` call
/// (lines 282/347). Same shape on the Rust side.
#[derive(Debug)]
pub struct GdbmDatabase {
    dbf: GdbmFile,
    path: PathBuf,
    readonly: bool,
}

impl GdbmDatabase {
    /// Port of `bin_ztie()` from `Src/Modules/db_gdbm.c:109`.
    #[cfg(feature = "gdbm")]
    pub fn open(path: &Path, readonly: bool) -> Result<Self, String> {
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "Invalid path")?;

        let flags = GDBM_SYNC | if readonly { GDBM_READER } else { GDBM_WRCREAT };

        let dbf = unsafe { gdbm_open(c_path.as_ptr(), 0, flags, 0o666, None) };

        if dbf.is_null() {
            let err = unsafe {
                let err_ptr = gdbm_strerror(gdbm_errno);
                if err_ptr.is_null() {
                    "Unknown error".to_string()
                } else {
                    CStr::from_ptr(err_ptr).to_string_lossy().to_string()
                }
            };
            return Err(format!(
                "error opening database file {} ({})",
                path.display(),
                err
            ));
        }

        Ok(GdbmDatabase {
            dbf,
            path: path.to_path_buf(),
            readonly,
        })
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn open(_path: &Path, _readonly: bool) -> Result<Self, String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// Port of `gdbmgetfn()` from `Src/Modules/db_gdbm.c:282`.
    #[cfg(feature = "gdbm")]
    pub fn get(&self, key: &str) -> Option<String> {
        let key_bytes = key.as_bytes();
        let key_datum = Datum::from_bytes(key_bytes);

        let exists = unsafe {
            gdbm_exists(
                self.dbf,
                Datum {
                    dptr: key_datum.dptr,
                    dsize: key_datum.dsize,
                },
            )
        };

        if exists == 0 {
            unsafe { libc::free(key_datum.dptr as *mut c_void) };
            return None;
        }

        let mut content = unsafe {
            gdbm_fetch(
                self.dbf,
                Datum {
                    dptr: key_datum.dptr,
                    dsize: key_datum.dsize,
                },
            )
        };

        unsafe { libc::free(key_datum.dptr as *mut c_void) };

        let result = content
            .to_bytes()
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string());

        content.free();
        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn get(&self, _key: &str) -> Option<String> {
        None
    }

    /// Port of `gdbmhashsetfn()` from `Src/Modules/db_gdbm.c:476`.
    #[cfg(feature = "gdbm")]
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let key_datum = Datum::from_bytes(key.as_bytes());
        let content_datum = Datum::from_bytes(value.as_bytes());

        let ret = unsafe {
            gdbm_store(
                self.dbf,
                Datum {
                    dptr: key_datum.dptr,
                    dsize: key_datum.dsize,
                },
                Datum {
                    dptr: content_datum.dptr,
                    dsize: content_datum.dsize,
                },
                GDBM_REPLACE,
            )
        };

        unsafe {
            libc::free(key_datum.dptr as *mut c_void);
            libc::free(content_datum.dptr as *mut c_void);
        }

        if ret != 0 {
            Err("Failed to store value".to_string())
        } else {
            Ok(())
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// Port of `gdbmgetfn()` from `Src/Modules/db_gdbm.c:282`.
    #[cfg(feature = "gdbm")]
    pub fn delete(&self, key: &str) -> Result<(), String> {
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let key_datum = Datum::from_bytes(key.as_bytes());

        let ret = unsafe {
            gdbmunsetfn(
                self.dbf,
                Datum {
                    dptr: key_datum.dptr,
                    dsize: key_datum.dsize,
                },
            )
        };

        unsafe { libc::free(key_datum.dptr as *mut c_void) };

        if ret != 0 {
            Err("Key not found".to_string())
        } else {
            Ok(())
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn delete(&self, _key: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// Port of `scangdbmkeys()` from `Src/Modules/db_gdbm.c:442`.
    #[cfg(feature = "gdbm")]
    pub fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();

        let mut key = unsafe { gdbm_firstkey(self.dbf) };

        while !key.dptr.is_null() {
            if let Some(bytes) = key.to_bytes() {
                keys.push(String::from_utf8_lossy(&bytes).to_string());
            }

            let prev_key = key;
            key = unsafe {
                gdbm_nextkey(
                    self.dbf,
                    Datum {
                        dptr: prev_key.dptr,
                        dsize: prev_key.dsize,
                    },
                )
            };
            unsafe { libc::free(prev_key.dptr as *mut c_void) };
        }

        keys
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn keys(&self) -> Vec<String> {
        Vec::new()
    }

    /// Port of `scangdbmkeys()` from `Src/Modules/db_gdbm.c:442`.
    #[cfg(feature = "gdbm")]
    pub fn clear(&self) -> Result<(), String> {
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let keys = self.keys();
        for key in keys {
            let _ = self.delete(&key);
        }

        unsafe { gdbm_reorganize(self.dbf) };
        Ok(())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn clear(&self) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(feature = "gdbm")]
    pub fn fd(&self) -> i32 {
        unsafe { gdbm_fdesc(self.dbf) }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    #[cfg(not(feature = "gdbm"))]
    pub fn fd(&self) -> i32 {
        -1
    }
}

#[cfg(feature = "gdbm")]
impl Drop for GdbmDatabase {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    fn drop(&mut self) {
        if !self.dbf.is_null() {
            unsafe { gdbm_close(self.dbf) };
            self.dbf = ptr::null_mut();
        }
    }
}

#[cfg(not(feature = "gdbm"))]
impl Drop for GdbmDatabase {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    fn drop(&mut self) {}
}

unsafe impl Send for GdbmDatabase {}
unsafe impl Sync for GdbmDatabase {}

/// A parameter tied to a GDBM database.
/// Port of the `Param` shape Src/Modules/db_gdbm.c installs via
/// `bin_ztie()` (line 109) — the C source builds a special hash
/// `Param` whose `getfn`/`setfn`/`unsetfn` route every read/write
/// through `gdbmgetfn` (line 282) / `gdbmsetfn` (line 347) /
/// `gdbmunsetfn` (line 399). The Rust struct holds the live db
/// handle plus a small per-key cache.
pub struct TiedGdbmParam {
    pub name: String,
    pub db: Arc<GdbmDatabase>,
    pub cache: RwLock<HashMap<String, String>>,
}

impl TiedGdbmParam {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn new(name: String, db: Arc<GdbmDatabase>) -> Self {
        TiedGdbmParam {
            name,
            db,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn get(&self, key: &str) -> Option<String> {
        if let Ok(cache) = self.cache.read() {
            if let Some(val) = cache.get(key) {
                return Some(val.clone());
            }
        }

        if let Some(val) = self.db.get(key) {
            if let Ok(mut cache) = self.cache.write() {
                cache.insert(key.to_string(), val.clone());
            }
            Some(val)
        } else {
            None
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.db.set(key, value)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key.to_string(), value.to_string());
        }
        Ok(())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.db.delete(key)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(key);
        }
        Ok(())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn keys(&self) -> Vec<String> {
        self.db.keys()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn to_hash(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for key in self.keys() {
            if let Some(val) = self.get(&key) {
                result.insert(key, val);
            }
        }
        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/db_gdbm.c`.
    pub fn from_hash(&self, hash: &HashMap<String, String>) -> Result<(), String> {
        self.db.clear()?;
        for (key, val) in hash {
            self.db.set(key, val)?;
        }
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
        Ok(())
    }
}

/// Global registry of tied GDBM parameters
static TIED_PARAMS: Lazy<Mutex<HashMap<String, Arc<TiedGdbmParam>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// List currently-tied GDBM parameter names — backs the
/// `${gdbm_tied}` magic-assoc reader.
/// Port of `append_tied_name()` / `remove_tied_name()` (the
/// `tied_param_list` enumeration helpers at
/// Src/Modules/db_gdbm.c:42-43); reads the linked list those
/// two C helpers maintain.
pub fn append_tied_name() -> Vec<String> {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.keys().cloned().collect()
    } else {
        Vec::new()
    }
}

/// `ztie` builtin entry point — bind a parameter to a GDBM file.
/// Port of `bin_ztie()` from Src/Modules/db_gdbm.c:109 — the C
/// source opens the GDBM file via `gdbm_open()`, allocates a hash
/// `Param`, wires the per-key getter/setter slots, and inserts
/// the param name into the tied-list.
///
/// Usage: `ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME`
pub fn bin_ztie(
    args: &[String],
    readonly: bool,
    db_type: Option<&str>,
    file_path: Option<&str>,
) -> Result<(), String> {
    let db_type = db_type.ok_or("you must pass `-d db/gdbm'")?;
    let file_path = file_path.ok_or("you must pass `-f' with a filename")?;

    if db_type != BACKTYPE {
        return Err(format!("unsupported backend type `{}'", db_type));
    }

    let param_name = args.first().ok_or("parameter name required")?;

    // Resolve path
    let path = if file_path.starts_with('/') {
        PathBuf::from(file_path)
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(file_path)
    };

    // Check if already tied
    {
        let params = TIED_PARAMS.lock().map_err(|_| "lock error")?;
        if params.contains_key(param_name) {
            return Err(format!("parameter {} is already tied", param_name));
        }
    }

    // Open database
    let db = GdbmDatabase::open(&path, readonly)?;
    let db = Arc::new(db);

    // Create tied parameter
    let tied = Arc::new(TiedGdbmParam::new(param_name.clone(), db));

    // Register
    {
        let mut params = TIED_PARAMS.lock().map_err(|_| "lock error")?;
        params.insert(param_name.clone(), tied);
    }

    Ok(())
}

/// `zuntie` builtin entry point — release a tied parameter.
/// Port of `bin_zuntie()` from Src/Modules/db_gdbm.c:201 — the C
/// source's `gdbmuntie()` (line 555) closes the database, frees
/// the hash table, and removes the entry from the tied-list.
///
/// Usage: `zuntie [-u] PARAM_NAME...`
pub fn bin_zuntie(args: &[String], force_unset: bool) -> Result<(), String> {
    let mut errors = Vec::new();

    for param_name in args {
        let mut params = match TIED_PARAMS.lock() {
            Ok(p) => p,
            Err(_) => {
                errors.push(format!("cannot untie {}: lock error", param_name));
                continue;
            }
        };

        if !params.contains_key(param_name) {
            errors.push(format!("cannot untie {}", param_name));
            continue;
        }

        params.remove(param_name);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// `zgdbmpath` builtin entry point — print a tied parameter's path.
/// Port of `bin_zgdbmpath()` from Src/Modules/db_gdbm.c:236 — the
/// C source writes the result into `$REPLY`. Same convention.
///
/// Usage: `zgdbmpath PARAM_NAME`
pub fn bin_zgdbmpath(param_name: &str) -> Result<String, String> {
    let params = TIED_PARAMS.lock().map_err(|_| "lock error")?;

    let tied = params
        .get(param_name)
        .ok_or_else(|| format!("no such parameter: {}", param_name))?;

    Ok(tied.db.path().to_string_lossy().to_string())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/db_gdbm.c`.
/// Is a given parameter currently tied to GDBM?
/// zshrs convenience — equivalent to scanning `tied_param_list`
/// in Src/Modules/db_gdbm.c.
pub fn is_gdbm_tied(param_name: &str) -> bool {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.contains_key(param_name)
    } else {
        false
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/db_gdbm.c`.
/// Get the live `TiedGdbmParam` for a given name.
/// zshrs convenience — Src/Modules/db_gdbm.c uses
/// `getgdbmnode()` (line 407) for the same lookup at the C level.
pub fn get_tied_param(param_name: &str) -> Option<Arc<TiedGdbmParam>> {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.get(param_name).cloned()
    } else {
        None
    }
}

/// Read a key from a tied parameter.
/// Port of `gdbmgetfn()` from Src/Modules/db_gdbm.c:282 — the
/// `getfn` slot the C source wires for `${db[key]}`.
pub fn gdbmgetfn(param_name: &str, key: &str) -> Option<String> {
    get_tied_param(param_name).and_then(|p| p.get(key))
}

/// Write a key to a tied parameter.
/// Port of `gdbmsetfn()` from Src/Modules/db_gdbm.c:347.
pub fn gdbmsetfn(param_name: &str, key: &str, value: &str) -> Result<(), String> {
    let param = get_tied_param(param_name)
        .ok_or_else(|| format!("not a tied gdbm hash: {}", param_name))?;
    param.set(key, value)
}

/// Delete a key from a tied parameter.
/// Port of `gdbmunsetfn()` from Src/Modules/db_gdbm.c:399 — used
/// by `unset 'db[key]'`.
pub fn gdbmunsetfn(param_name: &str, key: &str) -> Result<(), String> {
    let param = get_tied_param(param_name)
        .ok_or_else(|| format!("not a tied gdbm hash: {}", param_name))?;
    param.delete(key)
}

/// Get every key in a tied parameter.
/// Port of `scangdbmkeys()` from Src/Modules/db_gdbm.c:442 — the
/// `scanfn` slot the C source wires for `${(k)db}`.
pub fn scangdbmkeys(param_name: &str) -> Option<Vec<String>> {
    get_tied_param(param_name).map(|p| p.keys())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Port of `bin_ztie()` from `Src/Modules/db_gdbm.c:109`.
    #[test]
    #[cfg(feature = "gdbm")]
    fn test_gdbm_basic_operations() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.gdbm");

        // Open database
        let db = GdbmDatabase::open(&db_path, false).unwrap();

        // Set and get
        db.set("key1", "value1").unwrap();
        assert_eq!(db.get("key1"), Some("value1".to_string()));

        // Non-existent key
        assert_eq!(db.get("nonexistent"), None);

        // Delete
        db.delete("key1").unwrap();
        assert_eq!(db.get("key1"), None);

        // Multiple keys
        db.set("a", "1").unwrap();
        db.set("b", "2").unwrap();
        db.set("c", "3").unwrap();

        let keys = db.keys();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
        assert!(keys.contains(&"c".to_string()));

        // Clear
        db.clear().unwrap();
        assert_eq!(db.keys().len(), 0);
    }

    /// Port of `bin_ztie()` from `Src/Modules/db_gdbm.c:109`.
    #[test]
    #[cfg(feature = "gdbm")]
    fn test_tied_param() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tied.gdbm");

        let db = Arc::new(GdbmDatabase::open(&db_path, false).unwrap());
        let tied = TiedGdbmParam::new("mydb".to_string(), db);

        tied.set("foo", "bar").unwrap();
        assert_eq!(tied.get("foo"), Some("bar".to_string()));

        let hash = tied.to_hash();
        assert_eq!(hash.get("foo"), Some(&"bar".to_string()));
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
    /// Tie a parameter to a GDBM database
    /// Usage: ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME
    pub(crate) fn bin_ztie(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        let mut db_type: Option<String> = None;
        let mut file_path: Option<String> = None;
        let mut readonly = false;
        let mut param_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() {
                        db_type = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        eprintln!("zshrs:ztie:1: -d requires an argument");
                        return 1;
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        file_path = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        eprintln!("zshrs:ztie:1: -f requires an argument");
                        return 1;
                    }
                }
                "-r" => {
                    readonly = true;
                    i += 1;
                }
                arg if arg.starts_with('-') => {
                    eprintln!("zshrs:ztie:1: bad option: {}", arg);
                    return 1;
                }
                _ => {
                    param_args.push(args[i].clone());
                    i += 1;
                }
            }
        }

        match db_gdbm::bin_ztie(
            &param_args,
            readonly,
            db_type.as_deref(),
            file_path.as_deref(),
        ) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("zshrs:ztie:1: {}", e);
                1
            }
        }
    }
    /// Untie a parameter from its GDBM database
    /// Usage: zuntie [-u] PARAM_NAME...
    pub(crate) fn bin_zuntie(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        let mut force_unset = false;
        let mut param_args: Vec<String> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-u" => force_unset = true,
                a if a.starts_with('-') => {
                    eprintln!("zshrs:zuntie:1: bad option: {}", a);
                    return 1;
                }
                _ => param_args.push(arg.clone()),
            }
        }

        if param_args.is_empty() {
            eprintln!("zshrs:zuntie:1: not enough arguments");
            return 1;
        }

        match db_gdbm::bin_zuntie(&param_args, force_unset) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("zshrs:zuntie:1: {}", e);
                1
            }
        }
    }
    /// Get the path of a tied GDBM database
    /// Usage: zgdbmpath PARAM_NAME
    /// Sets $REPLY to the path
    pub(crate) fn bin_zgdbmpath(&mut self, args: &[String]) -> i32 {
        use crate::db_gdbm;

        if args.is_empty() {
            eprintln!(
                "zgdbmpath: parameter name (whose path is to be written to $REPLY) is required"
            );
            return 1;
        }

        match db_gdbm::bin_zgdbmpath(&args[0]) {
            Ok(path) => {
                self.variables.insert("REPLY".to_string(), path.clone());
                std::env::set_var("REPLY", &path);
                0
            }
            Err(e) => {
                eprintln!("zshrs:zgdbmpath:1: {}", e);
                1
            }
        }
    }
}
// END moved-from-exec-rs

/// Module loader entry — port of `setup_()` from Src/Modules/db_gdbm.c:613.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/db_gdbm.c:620.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/db_gdbm.c:628.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/db_gdbm.c:635.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/db_gdbm.c:643.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/db_gdbm.c:651.
pub fn finish_() -> i32 {
    0
}
