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
    fn null() -> Self {
        Datum {
            dptr: ptr::null_mut(),
            dsize: 0,
        }
    }

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
    fn gdbm_delete(dbf: GdbmFile, key: Datum) -> c_int;
    fn gdbm_exists(dbf: GdbmFile, key: Datum) -> c_int;
    fn gdbm_firstkey(dbf: GdbmFile) -> Datum;
    fn gdbm_nextkey(dbf: GdbmFile, key: Datum) -> Datum;
    fn gdbm_reorganize(dbf: GdbmFile) -> c_int;
    fn gdbm_fdesc(dbf: GdbmFile) -> c_int;
    fn gdbm_strerror(errno: c_int) -> *const c_char;
    static gdbm_errno: c_int;
}

/// A GDBM database handle wrapper
#[derive(Debug)]
pub struct GdbmDatabase {
    dbf: GdbmFile,
    path: PathBuf,
    readonly: bool,
}

impl GdbmDatabase {
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

    #[cfg(not(feature = "gdbm"))]
    pub fn open(_path: &Path, _readonly: bool) -> Result<Self, String> {
        Err("GDBM support not compiled in".to_string())
    }

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

    #[cfg(not(feature = "gdbm"))]
    pub fn get(&self, _key: &str) -> Option<String> {
        None
    }

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

    #[cfg(not(feature = "gdbm"))]
    pub fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    #[cfg(feature = "gdbm")]
    pub fn delete(&self, key: &str) -> Result<(), String> {
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let key_datum = Datum::from_bytes(key.as_bytes());

        let ret = unsafe {
            gdbm_delete(
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

    #[cfg(not(feature = "gdbm"))]
    pub fn delete(&self, _key: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

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

    #[cfg(not(feature = "gdbm"))]
    pub fn keys(&self) -> Vec<String> {
        Vec::new()
    }

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

    #[cfg(not(feature = "gdbm"))]
    pub fn clear(&self) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(feature = "gdbm")]
    pub fn fd(&self) -> i32 {
        unsafe { gdbm_fdesc(self.dbf) }
    }

    #[cfg(not(feature = "gdbm"))]
    pub fn fd(&self) -> i32 {
        -1
    }
}

#[cfg(feature = "gdbm")]
impl Drop for GdbmDatabase {
    fn drop(&mut self) {
        if !self.dbf.is_null() {
            unsafe { gdbm_close(self.dbf) };
            self.dbf = ptr::null_mut();
        }
    }
}

#[cfg(not(feature = "gdbm"))]
impl Drop for GdbmDatabase {
    fn drop(&mut self) {}
}

unsafe impl Send for GdbmDatabase {}
unsafe impl Sync for GdbmDatabase {}

/// A tied parameter backed by GDBM
pub struct TiedGdbmParam {
    pub name: String,
    pub db: Arc<GdbmDatabase>,
    pub cache: RwLock<HashMap<String, String>>,
}

impl TiedGdbmParam {
    pub fn new(name: String, db: Arc<GdbmDatabase>) -> Self {
        TiedGdbmParam {
            name,
            db,
            cache: RwLock::new(HashMap::new()),
        }
    }

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

    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.db.set(key, value)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key.to_string(), value.to_string());
        }
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.db.delete(key)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(key);
        }
        Ok(())
    }

    pub fn keys(&self) -> Vec<String> {
        self.db.keys()
    }

    pub fn to_hash(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for key in self.keys() {
            if let Some(val) = self.get(&key) {
                result.insert(key, val);
            }
        }
        result
    }

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

/// Get list of tied parameter names
pub fn zgdbm_tied() -> Vec<String> {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.keys().cloned().collect()
    } else {
        Vec::new()
    }
}

/// Tie a parameter to a GDBM database
///
/// Usage: ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME
pub fn ztie(
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

/// Untie a parameter from its GDBM database
///
/// Usage: zuntie [-u] PARAM_NAME...
pub fn zuntie(args: &[String], force_unset: bool) -> Result<(), String> {
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

/// Get the path of a tied GDBM database
///
/// Usage: zgdbmpath PARAM_NAME
/// Sets $REPLY to the path
pub fn zgdbmpath(param_name: &str) -> Result<String, String> {
    let params = TIED_PARAMS.lock().map_err(|_| "lock error")?;

    let tied = params
        .get(param_name)
        .ok_or_else(|| format!("no such parameter: {}", param_name))?;

    Ok(tied.db.path().to_string_lossy().to_string())
}

/// Check if a parameter is tied to GDBM
pub fn is_gdbm_tied(param_name: &str) -> bool {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.contains_key(param_name)
    } else {
        false
    }
}

/// Get a tied parameter by name
pub fn get_tied_param(param_name: &str) -> Option<Arc<TiedGdbmParam>> {
    if let Ok(params) = TIED_PARAMS.lock() {
        params.get(param_name).cloned()
    } else {
        None
    }
}

/// Get value from a tied parameter
pub fn gdbm_get(param_name: &str, key: &str) -> Option<String> {
    get_tied_param(param_name).and_then(|p| p.get(key))
}

/// Set value in a tied parameter
pub fn gdbm_set(param_name: &str, key: &str, value: &str) -> Result<(), String> {
    let param = get_tied_param(param_name)
        .ok_or_else(|| format!("not a tied gdbm hash: {}", param_name))?;
    param.set(key, value)
}

/// Delete key from a tied parameter
pub fn gdbm_delete(param_name: &str, key: &str) -> Result<(), String> {
    let param = get_tied_param(param_name)
        .ok_or_else(|| format!("not a tied gdbm hash: {}", param_name))?;
    param.delete(key)
}

/// Get all keys from a tied parameter
pub fn gdbm_keys(param_name: &str) -> Option<Vec<String>> {
    get_tied_param(param_name).map(|p| p.keys())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

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
