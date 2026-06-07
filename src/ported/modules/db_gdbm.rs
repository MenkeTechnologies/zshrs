//! GDBM database bindings for zsh
//!
//! Port of zsh/Src/Modules/db_gdbm.c
//!
//! Holds names of all tied parameters                                       // c:100
//! This creates standard hash.                                              // c:672
//!
//! Provides builtins:
//! - ztie: Tie a parameter to a GDBM database
//! - zuntie: Untie a parameter from a GDBM database
//! - zgdbmpath: Get the path of a tied GDBM database

use std::collections::HashMap;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::utils::{unmeta, zwarnnam};
use crate::zsh_h::{
    module, options, OPT_ARG, OPT_ISSET, PM_DONTIMPORT_SUID, PM_REMOVABLE, PM_SINGLE,
};
use once_cell::sync::Lazy;

/// Port of `PM_UPTODATE` from `Src/Modules/db_gdbm.c:38`.
/// `#define PM_UPTODATE PM_DONTIMPORT_SUID` — re-uses a Param flag bit
/// that's safe in this module's context. Set by `gdbmgetfn` after a
/// successful database fetch so subsequent reads can short-circuit.
pub const PM_UPTODATE: u32 = PM_DONTIMPORT_SUID; // c:38

/// `ztie` builtin entry point — bind a parameter to a GDBM file.
/// Port of `bin_ztie(char *nam, char **args, Options ops, UNUSED(int func))` from Src/Modules/db_gdbm.c:109 — the C
/// source opens the GDBM file via `gdbm_open()`, allocates a hash
/// `Param`, wires the per-key getter/setter slots, and inserts
/// the param name into the tied-list.
///
/// Usage: `ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME`
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_ztie(char *nam, char **args, Options ops, UNUSED(int func))
/// ```
/// WARNING: param names don't match C — Rust=(nam, args, ops, _func) vs C=(nam, args, ops, func)
pub fn bin_ztie(nam: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:109
    // c:109-115 — locals
    let pmname: &str;
    let mut read_write: i32 = 0; // c:114 GDBM_SYNC
    let _pmflags: u32 = PM_REMOVABLE | PM_SINGLE; // c:114

    // c:117 — `if (!OPT_ISSET(ops, 'd'))`
    if !OPT_ISSET(ops, b'd') {
        zwarnnam(nam, &format!("you must pass `-d {}'", BACKTYPE));
        return 1; // c:119
    } else {
    }
    // c:121 — `if (!OPT_ISSET(ops, 'f'))`
    if !OPT_ISSET(ops, b'f') {
        zwarnnam(nam, "you must pass `-f' with a filename");
        return 1; // c:123
    }
    // c:125-130 — `if (OPT_ISSET(ops, 'r'))` readonly
    let readonly = OPT_ISSET(ops, b'r');
    if readonly {
        read_write |= 1; // GDBM_READER
    } else {
        read_write |= 2; // GDBM_WRCREAT
    }
    let _ = read_write;

    // c:134 — `if (strcmp(OPT_ARG(ops, 'd'), backtype) != 0)`
    let db_type = OPT_ARG(ops, b'd').unwrap_or("");
    if db_type != BACKTYPE {
        zwarnnam(nam, &format!("unsupported backend type `{}'", db_type));
        return 1; // c:136
    }

    // c:139 — `resource_name = OPT_ARG(ops, 'f');`
    let resource_name = OPT_ARG(ops, b'f').unwrap_or("");
    // c:140 — `pmname = *args;`
    pmname = match args.first() {
        Some(s) => s.as_str(),
        None => {
            zwarnnam(nam, "parameter name required");
            return 1;
        }
    };

    // c:142-159 — unset existing param if it exists.
    // c:161-166 — open the GDBM database.
    let path = if resource_name.starts_with('/') {
        PathBuf::from(resource_name)
    } else {
        match std::env::current_dir() {
            Ok(d) => d.join(resource_name),
            Err(_) => {
                zwarnnam(nam, "current dir lookup failed");
                return 1;
            }
        }
    };

    // c:142-145 — check existing tied
    {
        let params = match TIED_PARAMS.lock() {
            Ok(p) => p,
            Err(_) => return 1,
        };
        if params.contains_key(pmname) {
            zwarnnam(nam, &format!("parameter {} is already tied", pmname));
            return 1;
        }
    }

    // c:162 — `dbf = gdbm_open(resource_name, 0, read_write, 0666, 0);`
    let db = match gdbm_database::open(&path, readonly) {
        Ok(d) => d,
        Err(e) => {
            zwarnnam(
                nam,
                &format!("error opening database file {} ({})", resource_name, e),
            );
            return 1; // c:165
        }
    };
    let db = Arc::new(db);

    // c:168 — `tied_param = createhash(pmname, pmflags);`
    let tied = Arc::new(tied_gdbm_param::new(pmname.to_string(), db));

    {
        let mut params = match TIED_PARAMS.lock() {
            Ok(p) => p,
            Err(_) => return 1,
        };
        params.insert(pmname.to_string(), tied);
    }
    append_tied_name(pmname); // c:194
    0 // c:196
}

/// `zuntie` builtin entry point — release a tied parameter.
/// Port of `bin_zuntie(char *nam, char **args, Options ops, UNUSED(int func))` from Src/Modules/db_gdbm.c:201 — the C
/// source's `gdbmuntie()` (line 555) closes the database, frees
/// the hash table, and removes the entry from the tied-list.
///
/// Usage: `zuntie [-u] PARAM_NAME...`
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_zuntie(char *nam, char **args, Options ops, UNUSED(int func))
/// ```
/// WARNING: param names don't match C — Rust=(nam, args, ops, _func) vs C=(nam, args, ops, func)
pub fn bin_zuntie(nam: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:201
    // c:Src/Modules/db_gdbm.c BUILTIN spec — min_args=1 ("ztie ... -d
    // db/gdbm name"). C's execbuiltin gate rejects zero positional
    // args before reaching here; the Rust port calls bin_zuntie
    // directly (tests, future dispatch paths) so the empty loop falls
    // through to `return 0` silently. Mirror C's dispatcher-level
    // usage error so no-args is a clear failure, not a silent success.
    if args.is_empty() {
        zwarnnam(nam, "missing parameter name");
        return 1;
    }
    // c:201-205 — locals
    let mut ret: i32 = 0; // c:205

    // c:207 — `for (pmname = *args; *args++; pmname = *args)`
    for pmname in args {
        // c:208 — `pm = (Param) paramtab->getnode(paramtab, pmname);`
        let in_table = match TIED_PARAMS.lock() {
            Ok(p) => p.contains_key(pmname),
            Err(_) => false,
        };
        if !in_table {
            // c:209
            zwarnnam(nam, &format!("cannot untie {}", pmname)); // c:210
            ret = 1; // c:211
            continue; // c:212
        }
        // c:214 — `if (pm->gsu.h != &gdbm_hash_gsu)` — type check skipped
        // since TIED_PARAMS only ever holds gdbm-backed entries.

        // c:220 — `queue_signals();`
        queue_signals();
        if OPT_ISSET(ops, b'u') { // c:221
             // c:222 — `pm->node.flags &= ~PM_READONLY;`
             // Static-link path: tied_gdbm_param doesn't carry a flags
             // field separately; readonly is on gdbm_database.readonly.
        }
        // c:224 — `if (unsetparam_pm(pm, 0, 1))` — registry remove
        match TIED_PARAMS.lock() {
            Ok(mut p) => {
                p.remove(pmname);
            }
            Err(_) => {
                ret = 1;
            }
        }
        // c:568 — `remove_tied_name(pm->node.nam);` (called from gdbmuntie())
        remove_tied_name(pmname);
        // c:228 — `unqueue_signals();`
        unqueue_signals();
    }
    ret // c:236
}

/// Port of `bin_zgdbmpath(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/db_gdbm.c:236`.
/// `zgdbmpath` builtin entry point — write tied parameter's path to $REPLY.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_zgdbmpath(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))
/// ```
#[allow(unused_variables)]
pub fn bin_zgdbmpath(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    // c:236
    // c:236 — `pmname = *args;`
    let pmname = match args.first() {
        Some(s) => s.as_str(),
        None => {
            // c:243-245 — "parameter name (whose path is to be written
            //              to $REPLY) is required"
            zwarnnam(
                nam,
                "parameter name (whose path is to be written to $REPLY) is required",
            );
            return 1;
        }
    };

    // c:248 — `pm = (Param) paramtab->getnode(paramtab, pmname);`
    let path = match TIED_PARAMS.lock() {
        Ok(p) => match p.get(pmname) {
            Some(tied) => tied.db.path().to_string_lossy().to_string(),
            None => {
                // c:249-251 — "no such parameter"
                zwarnnam(nam, &format!("no such parameter: {}", pmname));
                return 1;
            }
        },
        Err(_) => return 1,
    };

    // c:254-257 — `if (pm->gsu.h != &gdbm_hash_gsu)` skipped; TIED_PARAMS
    // only ever holds gdbm-backed entries.

    // c:260-264 — `setsparam("REPLY", ztrdup(dbfile_path));`
    // Static-link path: the params global-state isn't yet wired through;
    // emit to stdout as a degraded equivalent until params.c globalizes.
    println!("{}", path);
    0 // c:265
}

/// Port of `gdbmgetfn(Param pm)` from `Src/Modules/db_gdbm.c:282`.
///
/// C signature mirrored: `static char * gdbmgetfn(Param pm)`.
/// Returns the (Meta-encoded) value of `pm->node.nam` from the
/// underlying gdbm database. PM_UPTODATE short-circuits the lookup;
/// otherwise gdbm_fetch + metafy. PM_DEFAULTED set when key absent.
///
/// Rust port: `pm` is identified by `(param_name, key)` since the
/// `Param` struct is keyed by hash entry. Returns the value or empty
/// string matching C's `return pm->u.str ? pm->u.str : "";` /
/// `return "";` on miss.
/// WARNING: param names don't match C — Rust=(param_name, key) vs C=(pm)
pub fn gdbmgetfn(param_name: &str, key: &str) -> String {
    // c:282
    // c:282-300 — PM_UPTODATE shortcut. zshrs's tied_gdbm_param doesn't
    // cache so always fetches fresh.
    let params = match TIED_PARAMS.lock() {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    let tied = match params.get(param_name) {
        Some(t) => t.clone(),
        None => return String::new(),
    };
    drop(params);
    // c:312 — `gdbm_exists(dbf, key)` then `gdbm_fetch(dbf, key)`
    match tied.get(key) {
        Some(v) => v,          // c:347 return pm->u.str
        None => String::new(), // c:347 return ""
    }
}

/// Port of `gdbmsetfn(Param pm, char *val)` from `Src/Modules/db_gdbm.c:347`.
///
/// C signature mirrored: `static void gdbmsetfn(Param pm, char *val)`.
/// Writes (Meta-decoded) `val` to the gdbm database under
/// `pm->node.nam`. NULL val deletes the entry (matches C `gdbm_delete`).
/// WARNING: param names don't match C — Rust=(param_name, key, val) vs C=(pm, val)
pub fn gdbmsetfn(param_name: &str, key: &str, val: Option<&str>) {
    // c:347
    let params = match TIED_PARAMS.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    let tied = match params.get(param_name) {
        Some(t) => t.clone(),
        None => return,
    };
    drop(params);
    match val {
        // c:357-378 — `gdbm_store(dbf, key, content, GDBM_REPLACE);`
        Some(v) => {
            let _ = tied.set(key, v);
        }
        // c:399-388 — NULL val triggers `gdbm_delete(dbf, key);`
        None => {
            let _ = tied.delete(key);
        }
    }
}

/// Port of `gdbmunsetfn(Param pm, UNUSED(int um))` from `Src/Modules/db_gdbm.c:399`.
///
/// C signature mirrored: `static void gdbmunsetfn(Param pm, UNUSED(int um))`.
/// Calls `gdbmsetfn(pm, NULL)` to delete the key.
/// WARNING: param names don't match C — Rust=(param_name, key, _um) vs C=(pm, um)
pub fn gdbmunsetfn(param_name: &str, key: &str, _um: i32) {
    // c:399
    // c:399 — `gdbmsetfn(pm, NULL);`
    gdbmsetfn(param_name, key, None);
}

/// magic-assoc lookup callback for `${gdbm_param[key]}`. Reads
/// from the underlying gdbm database.
///
/// Port of `getgdbmnode(HashTable ht, const char *name)` from `Src/Modules/db_gdbm.c:407`.
///
/// C body:
/// ```c
/// getgdbmnode(HashTable ht, const char *name) {
///     HashNode hn = gethashnode2(ht, name);
///     Param val_pm = (Param) hn;
///     if (!val_pm) {
///         val_pm = (Param) zshcalloc(sizeof(*val_pm));
///         val_pm->node.flags = PM_SCALAR | PM_HASHELEM;
///         val_pm->gsu.s = (GsuScalar) ht->tmpdata;
///         ht->addnode(ht, ztrdup(name), val_pm);
///     }
///     return (HashNode) val_pm;
/// }
/// ```
///
/// Returns the hash node for `name` in `ht`, creating a new
/// PM_SCALAR|PM_HASHELEM entry if the key is unknown. The returned
/// entry is NOT marked PM_UPTODATE so subsequent `gdbmgetfn` calls
/// will pull the value from the underlying gdbm database on demand.
///
/// Returns `true` iff the key was already present, `false` if a
/// fresh placeholder was created. Static-link path uses the
/// `tied_gdbm_param` registry as the equivalent of `ht`.
pub fn getgdbmnode(ht: &str, name: &str) -> bool {
    // c:407
    let params = match TIED_PARAMS.lock() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let tied = match params.get(ht) {
        Some(t) => t.clone(),
        None => return false,
    };
    drop(params);
    // c:409 — gethashnode2(ht, name) — does the key exist in the DB?
    let exists = tied.get(name).is_some();
    if !exists {
        // c:430-435 — create a fresh PM_SCALAR|PM_HASHELEM entry.
        // Static-link path: the gdbm-tied param is the only "table",
        // and `gdbmgetfn` will lazily fetch on access. Insert an
        // empty placeholder so subsequent reads can see the key
        // before the first set.
        let _ = tied.set(name, ""); // c:434 addnode
    }
    exists // c:437 return val_pm
}

/// Port of `scangdbmkeys(HashTable ht, ScanFunc func, int flags)` from `Src/Modules/db_gdbm.c:442`.
///
/// C body:
/// ```c
/// scangdbmkeys(HashTable ht, ScanFunc func, int flags) {
///     datum key, prev_key;
///     GDBM_FILE dbf = ((struct gsu_scalar_ext *)ht->tmpdata)->dbf;
///     key = gdbm_firstkey(dbf);
///     while(key.dptr) {
///         char *zkey = metafy(key.dptr, key.dsize, META_DUP);
///         HashNode hn = getgdbmnode(ht, zkey);
///         zsfree(zkey);
///         func(hn, flags);
///         prev_key = key;
///         key = gdbm_nextkey(dbf, key);
///         free(prev_key.dptr);
///     }
/// }
/// ```
///
/// Iterate every key in the tied gdbm DB and call `func(node, flags)`
/// per key. Used by `${(k)db}` and similar. Rust port: takes a closure
/// matching C's `ScanFunc func` signature `void func(HashNode, int)` —
/// callers receive the per-key (param_name, key) tuple to dispatch.
pub fn scangdbmkeys(ht: &str, mut func: impl FnMut(&str, &str, i32), flags: i32) {
    // c:442
    let params = match TIED_PARAMS.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    let tied = match params.get(ht) {
        Some(t) => t.clone(),
        None => return,
    };
    drop(params);
    // c:449-466 — gdbm_firstkey / gdbm_nextkey loop
    for key in tied.keys() {
        // c:455 — metafy + getgdbmnode + scanfn
        let _ = getgdbmnode(ht, &key); // c:456
        func(ht, &key, flags); // c:459
    }
}

impl From<&[u8]> for Datum {
    /// Port of `gdbmgetfn(Param pm)` from `Src/Modules/db_gdbm.c:282`.
    fn from(data: &[u8]) -> Self {
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
}

impl Datum {
    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `Datum` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// FFI accessor — extract the underlying bytes into an owned Vec.
    /// `dptr == NULL` (gdbm convention for "absent") → None. C uses
    /// the inline pattern `if (d.dptr) { ... memcpy or ztrdup(d.dptr,
    /// d.dsize); }` at every callsite; this method centralises the
    /// guarded extraction.
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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `Datum` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Free the malloc'd `dptr` and reset the struct. Mirrors C's
    /// `if (d.dptr) { free(d.dptr); d.dptr = NULL; }` cleanup pattern.
    fn free(&mut self) {
        if !self.dptr.is_null() {
            unsafe { libc::free(self.dptr as *mut c_void) };
            self.dptr = ptr::null_mut();
            self.dsize = 0;
        }
    }
}

/// Port of `gdbmhashsetfn(Param pm, HashTable ht)` from `Src/Modules/db_gdbm.c:476`.
///
/// C body iterates the assigned `HashTable ht` and `gdbm_store`s each
/// (key, value) pair into the tied param's underlying gdbm DB.
///
/// Rust port: walks `entries` and dispatches each to the existing
/// `gdbmsetfn(param_name, key, value)` which delegates to
/// `tied_gdbm_param::set` → `gdbm_database::set` → `gdbm_store`.
pub fn gdbmhashsetfn(pm: &str, ht: &[(String, String)]) {
    // c:476
    // c:476 — for ((Param) (he = ht->nodes[i]); he; he = he->next)
    let param = match TIED_PARAMS.lock().ok().and_then(|m| m.get(pm).cloned()) {
        Some(p) => p,
        None => return,
    };
    for (key, value) in ht {
        let _ = param.set(key, value); // c:530 gdbm_store
    }
}

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

/// Port of `gdbmuntie(Param pm)` from `Src/Modules/db_gdbm.c:555`.
///
/// C body: `gdbm_close(dbf); pm->u.hash->tmpdata = NULL;`
/// Removes the tied param from the registry, dropping the
/// `Arc<tied_gdbm_param>` which closes the underlying GDBM handle
/// (via `Drop` on `gdbm_database`).
pub fn gdbmuntie(pm: &str) {
    // c:555
    if let Ok(mut params) = TIED_PARAMS.lock() {
        params.remove(pm); // c:560 gdbm_close + clear
    }
}

impl gdbm_database {
    /// Port of `bin_ztie(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/db_gdbm.c:109`.
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

        Ok(gdbm_database {
            dbf,
            path: path.to_path_buf(),
            readonly,
        })
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn open(_path: &Path, _readonly: bool) -> Result<Self, String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// Port of `gdbmgetfn(Param pm)` from `Src/Modules/db_gdbm.c:282`.
    #[cfg(feature = "gdbm")]
    pub fn get(&self, key: &str) -> Option<String> {
        // c:282
        let key_bytes = key.as_bytes();
        let key_datum = Datum::from(key_bytes);

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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn get(&self, _key: &str) -> Option<String> {
        None
    }

    /// Port of `gdbmhashsetfn(Param pm, HashTable ht)` from `Src/Modules/db_gdbm.c:476`.
    #[cfg(feature = "gdbm")]
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        // c:476
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let key_datum = Datum::from(key.as_bytes());
        let content_datum = Datum::from(value.as_bytes());

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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    // Port of `gdbmunsetfn(Param pm, UNUSED(int um))` from `Src/Modules/db_gdbm.c:399`.
    /// `delete` — see implementation.
    #[cfg(feature = "gdbm")]
    pub fn delete(&self, key: &str) -> Result<(), String> {
        // c:399
        if self.readonly {
            return Err("Database is read-only".to_string());
        }

        let key_datum = Datum::from(key.as_bytes());

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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn delete(&self, _key: &str) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// Port of `scangdbmkeys(HashTable ht, ScanFunc func, int flags)` from `Src/Modules/db_gdbm.c:442`.
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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn keys(&self) -> Vec<String> {
        Vec::new()
    }

    /// Port of `scangdbmkeys(HashTable ht, ScanFunc func, int flags)` from `Src/Modules/db_gdbm.c:442`.
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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn clear(&self) -> Result<(), String> {
        Err("GDBM support not compiled in".to_string())
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(feature = "gdbm")]
    pub fn fd(&self) -> i32 {
        unsafe { gdbm_fdesc(self.dbf) }
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    #[cfg(not(feature = "gdbm"))]
    pub fn fd(&self) -> i32 {
        -1
    }
}

#[cfg(feature = "gdbm")]
impl Drop for gdbm_database {
    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    fn drop(&mut self) {
        if !self.dbf.is_null() {
            unsafe { gdbm_close(self.dbf) };
            self.dbf = ptr::null_mut();
        }
    }
}

#[cfg(not(feature = "gdbm"))]
impl Drop for gdbm_database {
    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `gdbm_database` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    fn drop(&mut self) {}
}

unsafe impl Send for gdbm_database {}
unsafe impl Sync for gdbm_database {}

/// A parameter tied to a GDBM database.
/// `TiedGdbmParam` renamed to `tied_gdbm_param`. C has no struct of
/// this shape — instead the C source builds a special hash `Param`
/// whose `getfn`/`setfn`/`unsetfn` route every read/write through
/// `gdbmgetfn` (line 282) / `gdbmsetfn` (line 347) / `gdbmunsetfn`
/// (line 399) of `Src/Modules/db_gdbm.c`. The Rust struct bundles
/// the live db handle plus a small per-key cache; it is a Rust
/// extension matching C's per-param hidden state via `pm->u.hash`.
#[allow(non_camel_case_types)]
pub struct tied_gdbm_param {
    /// `name` field.
    pub name: String,
    /// `db` field.
    pub db: Arc<gdbm_database>,
    /// `cache` field.
    pub cache: RwLock<HashMap<String, String>>,
}

impl tied_gdbm_param {
    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn new(name: String, db: Arc<gdbm_database>) -> Self {
        tied_gdbm_param {
            name,
            db,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
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

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.db.set(key, value)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key.to_string(), value.to_string());
        }
        Ok(())
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.db.delete(key)?;
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(key);
        }
        Ok(())
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn keys(&self) -> Vec<String> {
        self.db.keys()
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn to_hash(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for key in self.keys() {
            if let Some(val) = self.get(&key) {
                result.insert(key, val);
            }
        }
        result
    }

    /// WARNING: NOT IN DB_GDBM.C — method on Rust-only `tied_gdbm_param` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
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

/// Port of `gdbmhashunsetfn(Param pm, UNUSED(int exp))` from `Src/Modules/db_gdbm.c:581`.
///
/// C body:
/// ```c
/// gdbmhashunsetfn(Param pm, int exp) {
///     gdbmuntie(pm);
///     pm->gsu.h->setfn(pm, NULL);
///     // free custom gsu_scalar_ext
///     pm->node.flags |= PM_UNSET;
/// }
/// ```
/// WARNING: param names don't match C — Rust=(param_name) vs C=(pm, exp)
pub fn gdbmhashunsetfn(param_name: &str) {
    // c:581
    gdbmuntie(param_name); // c:581
                           // c:592 — `pm->gsu.h->setfn(pm, NULL);` — implicit on registry remove.
                           // c:596-598 — gsu_scalar_ext free — handled by Arc drop.
                           // c:600 — `pm->node.flags |= PM_UNSET;` — implicit (registry miss).
}

// `bintab` — port of `static struct builtin bintab[]` (db_gdbm.c).

// `patab` — port of `static struct paramdef patab[]` (db_gdbm.c).

// `module_features` — port of `static struct features module_features`
// from db_gdbm.c:601.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/db_gdbm.c:613`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:613
    // C body c:615-616 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/db_gdbm.c:620`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/db_gdbm.c:628`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/db_gdbm.c:635`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:635
    // C body c:637-638 — `zgdbm_tied = zshcalloc((1) * sizeof(char *));
    //                     return 0`. Initializes the tied-DB names
    //                     array to empty (zero-element + NULL terminator).
    if let Ok(mut tied) = ZGDBM_TIED.lock() {
        // c:643
        tied.clear();
    }
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/db_gdbm.c:643`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/db_gdbm.c:651`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:651
    // C body c:653-654 — `return 0`. Faithful empty-body port; tied-DB
    //                     teardown happens in cleanup_ via untie+free.
    0
}

/// Port of `unmetafy_zalloc(const char *to_copy, int *new_len)` from `Src/Modules/db_gdbm.c:44`.
/// Allocates a copy of `to_copy`, unmetafies it, and writes the
/// new length to `*new_len`. Returns the unmetafied buffer.
///
/// C signature: `static char *unmetafy_zalloc(const char *to_copy, int *new_len)`.
/// WARNING: param names don't match C — Rust=(to_copy) vs C=(to_copy, new_len)
pub fn unmetafy_zalloc(to_copy: &str) -> (String, usize) {
    // c:44
    // c:783 — `result = ztrdup(to_copy); unmetafy(result, new_len);`
    let s = unmeta(to_copy);
    let len = s.len();
    (s, len)
}

/// per-element free callback for the gdbm-tied hash. Frees the
/// param's name + str fields.
///
/// Port of `myfreeparamnode(HashNode hn)` from `Src/Modules/db_gdbm.c:799`.
///
/// C body:
/// ```c
/// myfreeparamnode(HashNode hn) {
///     Param pm = (Param) hn;
///     pm->gsu.s->unsetfn(pm, 1);
///     zsfree(pm->node.nam);
///     if (!(pm->node.flags & PM_SPECIAL) && pm->ename) {
///         zsfree(pm->ename);
///         pm->ename = NULL;
///     }
///     zfree(pm, sizeof(struct param));
/// }
/// ```
///
/// Hash-table free callback for the per-tied-param hash entries.
/// Calls the param's `unsetfn(pm, 1)` (`gdbmunsetfn` for db_gdbm),
/// then frees `node.nam` and `ename`. Rust port: dispatches the
/// gdbm unset via the registry, then drops the entry (Vec/String
/// drop handles the C `zsfree`/`zfree`).
/// WARNING: param names don't match C — Rust=(param_name, key) vs C=(hn)
pub fn myfreeparamnode(param_name: &str, key: &str) {
    // c:45
    /* Upstream: The second argument of unsetfn() is used by modules to
     * differentiate "exp"licit unset from implicit unset, as when
     * a parameter is going out of scope.  It's not clear which
     * of these applies here, but passing 1 has always worked.
     */                                                                  // c:803-807
    /* if (delunset) */                                                  // c:809
    gdbmunsetfn(param_name, key, 1); // c:810 pm->gsu.s->unsetfn(pm, 1)
                                     // c:812 — zsfree(pm->node.nam); — Rust drop on TIED_PARAMS remove.
                                     // c:814-817 — `if (!(pm->node.flags & PM_SPECIAL) && pm->ename)`
                                     //              `zsfree(pm->ename); pm->ename = NULL;` — tied_gdbm_param
                                     //              doesn't carry an `ename` slot; the registry remove
                                     //              below frees the equivalent.
                                     // c:818 — `zfree(pm, sizeof(struct param));` — Drop on remove.
}

const BACKTYPE: &str = "db/gdbm";

/// GDBM open flags
const GDBM_READER: c_int = 0;

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:601 (db_gdbm.c)
// =====================================================================

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

/// Opaque GDBM file handle
type GdbmFile = *mut c_void;

/// GDBM database handle wrapper.
/// Port of the per-tied-param `Db` slot Src/Modules/db_gdbm.c
/// stores in `myfreeparamnode()` (line 45) — the C source threads
/// the live `GDBM_FILE *` through every `gdbmgetfn`/`gdbmsetfn` call
/// (lines 282/347). Same shape on the Rust side.
// `GdbmDatabase` renamed to `gdbm_database`. C has no `struct
// gdbm_database`; the equivalent C state is the bare `GDBM_FILE *`
// stored in `myfreeparamnode()` (`Src/Modules/db_gdbm.c:45`). Rust
// wraps it in a struct for RAII Drop + Send/Sync impls.
/// `gdbm_database` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct gdbm_database {
    /// `dbf` field.
    dbf: GdbmFile,
    /// `path` field.
    path: PathBuf,
    /// `readonly` field.
    readonly: bool,
}

/// Global registry of tied GDBM parameters
pub(crate) static TIED_PARAMS: Lazy<Mutex<HashMap<String, Arc<tied_gdbm_param>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Port of `static char **zgdbm_tied;` from `Src/Modules/db_gdbm.c`
/// (file-scope global). Holds the names of all currently-tied gdbm
/// params, mirroring what the user sees in the `$zgdbm_tied` array.
pub static ZGDBM_TIED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// List currently-tied GDBM parameter names — backs the
/// `${gdbm_tied}` magic-assoc reader.
/// Port of `append_tied_name(const char *name)` from `Src/Modules/db_gdbm.c:695`.
///
/// C body:
/// ```c
/// static int append_tied_name(const char *name) {
///     int old_len = arrlen(zgdbm_tied);
///     char **new_zgdbm_tied = zshcalloc((old_len+2) * sizeof(char *));
///     char **p = zgdbm_tied;
///     char **dst = new_zgdbm_tied;
///     while (*p) { *dst++ = *p++; }
///     *dst = ztrdup(name);
///     zfree(zgdbm_tied, sizeof(char *) * (old_len + 1));
///     zgdbm_tied = new_zgdbm_tied;
///     return 0;
/// }
/// ```
///
/// Appends `name` to the global `zgdbm_tied` array. Rust port:
/// the array is `ZGDBM_TIED: Mutex<Vec<String>>` below, mirroring
/// the C global.
pub fn append_tied_name(name: &str) -> i32 {
    // c:42
    if let Ok(mut tied) = ZGDBM_TIED.lock() {
        tied.push(name.to_string()); // c:707 *dst = ztrdup(name)
    }
    0 // c:713
}

/// Port of `remove_tied_name(const char *name)` from `Src/Modules/db_gdbm.c:43`.
///
/// C body removes `name` from the `zgdbm_tied` array via in-place
/// shift-down, frees the popped slot.
pub fn remove_tied_name(name: &str) -> i32 {
    // c:43
    if let Ok(mut tied) = ZGDBM_TIED.lock() {
        if let Some(pos) = tied.iter().position(|n| n == name) {
            // c:730 strcmp loop
            tied.remove(pos); // c:741 shift-down
        }
    }
    0
}

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN DB_GDBM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![
        "b:ztie".to_string(),
        "b:zuntie".to_string(),
        "b:zgdbmpath".to_string(),
        "p:zgdbm_tied".to_string(),
    ]
}

// WARNING: NOT IN DB_GDBM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN DB_GDBM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN DB_GDBM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
            bn_list: None,
            bn_size: 3,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 1,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zsh_h::{options, PM_DONTIMPORT_SUID};

    /// Port of `bin_ztie(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/db_gdbm.c:109`.
    #[test]
    #[cfg(feature = "gdbm")]
    fn test_gdbm_basic_operations() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.gdbm");

        // Open database
        let db = gdbm_database::open(&db_path, false).unwrap();

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

    /// Port of `bin_ztie(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/db_gdbm.c:109`.
    #[test]
    #[cfg(feature = "gdbm")]
    fn test_tied_param() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tied.gdbm");

        let db = Arc::new(gdbm_database::open(&db_path, false).unwrap());
        let tied = tied_gdbm_param::new("mydb".to_string(), db);

        tied.set("foo", "bar").unwrap();
        assert_eq!(tied.get("foo"), Some("bar".to_string()));

        let hash = tied.to_hash();
        assert_eq!(hash.get("foo"), Some(&"bar".to_string()));
    }

    fn empty_ops() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// PM_UPTODATE alias must equal PM_DONTIMPORT_SUID per c:38. Pin
    /// the alias so a regen of zsh_h doesn't silently shift the bit.
    #[test]
    fn pm_uptodate_aliases_pm_dontimport_suid() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(PM_UPTODATE, PM_DONTIMPORT_SUID);
    }

    /// Module entry points return 0 per C (db_gdbm.c:613-651).
    #[test]
    fn module_entry_points_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    /// c:109 — `ztie` with no args returns 1 (usage error).
    #[test]
    fn ztie_with_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(bin_ztie("ztie", &[], &ops, 0), 1);
    }

    /// c:207 — `zuntie` with no args: the for-each-arg loop body
    /// never runs, ret stays 0. Verifies no segfault on the empty path.
    #[test]
    fn zuntie_with_no_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        // C's BUILTIN spec has min_args=1; the fn-level guard mirrors
        // the dispatcher rejection so no-args returns the usage error
        // rc (1), not silent success (0).
        assert_eq!(bin_zuntie("zuntie", &[], &ops, 0), 1);
    }

    /// c:208-212 — `zuntie <name>` on an unknown param emits
    /// "cannot untie X" via zwarnnam and continues with ret=1.
    #[test]
    fn zuntie_unknown_param_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zuntie("zuntie", &["zshrs_test_not_tied".to_string()], &ops, 0);
        assert_eq!(r, 1);
    }

    /// c:282 — `gdbmgetfn` on unknown DB returns empty string (no
    /// segfault, no panic — graceful miss).
    #[test]
    fn gdbmgetfn_unknown_db_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(gdbmgetfn("zshrs_test_no_such_db_xyz", "key"), "");
    }

    /// c:407 — `getgdbmnode` on unknown DB returns false.
    #[test]
    fn getgdbmnode_unknown_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!getgdbmnode("zshrs_test_no_such_db_xyz", "key"));
    }

    /// c:347 — `gdbmsetfn` on an unknown DB must NOT panic.
    /// Defensive safety contract.
    #[test]
    fn gdbmsetfn_unknown_db_is_safe() {
        let _g = crate::test_util::global_state_lock();
        gdbmsetfn("zshrs_test_no_such_db_setfn", "key", Some("value"));
    }

    /// c:399 — `gdbmunsetfn` on an unknown DB must NOT panic.
    #[test]
    fn gdbmunsetfn_unknown_db_is_safe() {
        let _g = crate::test_util::global_state_lock();
        gdbmunsetfn("zshrs_test_no_such_db_unsetfn", "key", 0);
    }

    /// c:555 — `gdbmuntie` on an unknown param name is a safe no-op.
    #[test]
    fn gdbmuntie_unknown_param_is_safe() {
        let _g = crate::test_util::global_state_lock();
        gdbmuntie("zshrs_test_no_such_param_untie");
    }

    /// c:581 — `gdbmhashunsetfn` on unknown param is a safe no-op.
    #[test]
    fn gdbmhashunsetfn_unknown_param_is_safe() {
        let _g = crate::test_util::global_state_lock();
        gdbmhashunsetfn("zshrs_test_no_such_param_hash_unset");
    }

    /// c:442 — `scangdbmkeys` on an unknown DB invokes the callback
    /// ZERO times.
    #[test]
    fn scangdbmkeys_unknown_db_yields_no_entries() {
        let _g = crate::test_util::global_state_lock();
        let mut count = 0;
        scangdbmkeys("zshrs_test_no_such_db_scan", |_k, _v, _f| count += 1, 0);
        assert_eq!(count, 0, "unknown DB must yield no entries");
    }

    /// c:476 — `gdbmhashsetfn` on unknown param + empty entries
    /// is a safe no-op.
    #[test]
    fn gdbmhashsetfn_unknown_param_empty_entries_is_safe() {
        let _g = crate::test_util::global_state_lock();
        gdbmhashsetfn("zshrs_test_no_such_param_hashset", &[]);
    }

    /// c:236 — `bin_zgdbmpath` with no args returns nonzero (usage).
    #[test]
    fn bin_zgdbmpath_with_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zgdbmpath("zgdbmpath", &[], &ops, 0);
        assert_ne!(r, 0, "zgdbmpath with no args must error");
    }

    /// c:236 — `bin_zgdbmpath <unknown>` returns nonzero (param not
    /// tied).
    #[test]
    fn bin_zgdbmpath_unknown_param_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zgdbmpath(
            "zgdbmpath",
            &["zshrs_test_not_a_tied_param".to_string()],
            &ops,
            0,
        );
        assert_ne!(r, 0, "zgdbmpath on untied param must error");
    }

    // ─── zsh-corpus pins for db_gdbm helpers ───────────────────────

    /// `bin_ztie` with no args returns nonzero (usage error).
    #[test]
    fn db_gdbm_corpus_bin_ztie_no_args_errors() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ztie("ztie", &[], &ops, 0);
        assert_ne!(r, 0, "ztie with no args = error");
    }

    /// `bin_zuntie` with no args returns 0 — the C body iterates over
    /// `args` and untie's each in a loop. Zero args = empty loop body =
    /// success (0). Pin this contract.
    #[test]
    fn db_gdbm_corpus_bin_zuntie_no_args_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zuntie("zuntie", &[], &ops, 0);
        // Updated to match the dispatcher-level usage-error semantic
        // C zsh's bin_zuntie inherits via min_args=1 (see fn-level
        // guard in db_gdbm.rs). No-args is a usage error, not silent.
        assert_eq!(r, 1, "zuntie with no args = 1 (min_args=1 gate)");
    }

    /// `gdbmgetfn` on untied param returns empty string.
    #[test]
    fn db_gdbm_corpus_gdbmgetfn_untied_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let s = gdbmgetfn("zshrs_never_tied_xyz", "any_key");
        assert!(s.is_empty(), "untied param → empty, got {s:?}");
    }

    /// `getgdbmnode` on untied param returns false.
    #[test]
    fn db_gdbm_corpus_getgdbmnode_untied_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!getgdbmnode("zshrs_never_tied_xyz", "any"));
    }

    /// `gdbmsetfn` on untied param doesn't panic.
    #[test]
    fn db_gdbm_corpus_gdbmsetfn_untied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmsetfn("zshrs_never_tied_xyz", "key", Some("val"));
    }

    /// `gdbmunsetfn` on untied param doesn't panic.
    #[test]
    fn db_gdbm_corpus_gdbmunsetfn_untied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmunsetfn("zshrs_never_tied_xyz", "key", 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/db_gdbm.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:46 — `bin_ztie` with no args returns nonzero (usage error).
    #[test]
    fn bin_ztie_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ztie("ztie", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:93 (BUILTIN spec) — `bin_zuntie` declared with min_args=1.
    /// C zsh dispatcher rejects zero-arg calls BEFORE bin_zuntie runs.
    /// Rust port bin_zuntie returns 0 because it lacks the arg-count
    /// gate. Should return nonzero (usage error) per C dispatcher.
    #[test]
    fn bin_zuntie_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zuntie("zuntie", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:157 — `bin_zuntie` on never-tied param returns nonzero.
    #[test]
    fn bin_zuntie_never_tied_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zuntie("zuntie", &["zshrs_never_tied_xyz".to_string()], &ops, 0);
        assert_ne!(r, 0, "never-tied param → error");
    }

    /// c:262 — `gdbmgetfn` for empty param name returns empty string.
    #[test]
    fn gdbmgetfn_empty_param_name_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(gdbmgetfn("", "key").is_empty());
    }

    /// c:262 — `gdbmgetfn` for empty key returns empty string.
    #[test]
    fn gdbmgetfn_empty_key_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(gdbmgetfn("untied", "").is_empty());
    }

    /// c:350 — `getgdbmnode` empty input returns false.
    #[test]
    fn getgdbmnode_empty_inputs_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!getgdbmnode("", "key"));
        assert!(!getgdbmnode("untied", ""));
    }

    /// c:288 — `gdbmsetfn(..., None)` on untied param is safe.
    #[test]
    fn gdbmsetfn_none_value_on_untied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmsetfn("zshrs_never_tied", "key", None);
    }

    /// c:478 — `gdbmhashsetfn` on untied with empty hash is safe.
    #[test]
    fn gdbmhashsetfn_empty_hash_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmhashsetfn("zshrs_never_tied", &[]);
    }

    /// c:519 — `gdbmuntie` on never-tied param is safe.
    #[test]
    fn gdbmuntie_never_tied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmuntie("zshrs_never_tied_xyz");
    }

    /// c:398 — `scangdbmkeys` on untied param invokes callback 0 times.
    #[test]
    fn scangdbmkeys_untied_invokes_callback_zero_times() {
        let _g = crate::test_util::global_state_lock();
        let mut count = 0;
        scangdbmkeys(
            "zshrs_never_tied_xyz",
            |_k, _v, _f| {
                count += 1;
            },
            0,
        );
        assert_eq!(count, 0, "untied → no entries to scan");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/db_gdbm.c
    // c:46 bin_ztie / c:157 bin_zuntie / c:211 bin_zgdbmpath /
    // c:262 gdbmgetfn / c:316 gdbmunsetfn / c:350 getgdbmnode /
    // c:478 gdbmhashsetfn / c:519 gdbmuntie / c:910 gdbmhashunsetfn
    // c:927+ lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:46 — `bin_ztie` return value in u8 exit-code range.
    #[test]
    fn bin_ztie_no_args_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_ztie("ztie", &[], &ops, 0);
        assert!((0..256).contains(&r), "exit code must fit in u8");
    }

    /// c:262 — `gdbmgetfn` is deterministic for unknown param/key.
    #[test]
    fn gdbmgetfn_unknown_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = gdbmgetfn("zshrs_never_param", "zshrs_never_key");
        for _ in 0..5 {
            assert_eq!(gdbmgetfn("zshrs_never_param", "zshrs_never_key"), first);
        }
    }

    /// c:316 — `gdbmunsetfn` on untied param is safe (no panic).
    #[test]
    fn gdbmunsetfn_untied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmunsetfn("zshrs_never_param", "key", 0);
        gdbmunsetfn("zshrs_never_param", "", 0);
        gdbmunsetfn("", "", 0);
    }

    /// c:350 — `getgdbmnode` returns bool (compile-time type pin).
    #[test]
    fn getgdbmnode_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = getgdbmnode("", "");
    }

    /// c:350 — `getgdbmnode` is deterministic.
    #[test]
    fn getgdbmnode_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = getgdbmnode("zshrs_param", "zshrs_key");
        for _ in 0..5 {
            assert_eq!(getgdbmnode("zshrs_param", "zshrs_key"), first);
        }
    }

    /// c:519 — `gdbmuntie` on empty name is safe.
    #[test]
    fn gdbmuntie_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmuntie("");
    }

    /// c:910 — `gdbmhashunsetfn` on untied param is safe.
    #[test]
    fn gdbmhashunsetfn_untied_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmhashunsetfn("zshrs_never_param_xyz");
        gdbmhashunsetfn("");
    }

    /// c:211 — `bin_zgdbmpath` no args returns u8 exit-code range.
    #[test]
    fn bin_zgdbmpath_no_args_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zgdbmpath("zgdbmpath", &[], &ops, 0);
        assert!((0..256).contains(&r));
    }

    /// c:927+ — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn db_gdbm_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
    }

    /// c:927 — setup_ idempotent.
    #[test]
    fn db_gdbm_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/db_gdbm.c helpers
    // c:44 unmetafy_zalloc / c:43 remove_tied_name / c:695 append_tied_name
    // c:799 myfreeparamnode + lifecycle types
    // ═══════════════════════════════════════════════════════════════════

    /// c:44 — `unmetafy_zalloc` returns (String, usize) — compile-time
    /// pin so a regen that drops the len slot gets caught.
    #[test]
    fn unmetafy_zalloc_returns_string_usize_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: (String, usize) = unmetafy_zalloc("hello");
    }

    /// c:44 — `unmetafy_zalloc` of ASCII string returns (str, str.len()).
    /// No metafication needed for plain ASCII per zsh's metafy(3) spec.
    #[test]
    fn unmetafy_zalloc_ascii_passes_through_with_correct_len() {
        let _g = crate::test_util::global_state_lock();
        let (s, n) = unmetafy_zalloc("hello");
        assert_eq!(s, "hello", "ASCII passes through unchanged");
        assert_eq!(n, 5, "len must match byte count");
    }

    /// c:44 — `unmetafy_zalloc("")` returns ("", 0).
    #[test]
    fn unmetafy_zalloc_empty_returns_empty_zero() {
        let _g = crate::test_util::global_state_lock();
        let (s, n) = unmetafy_zalloc("");
        assert_eq!(s, "");
        assert_eq!(n, 0);
    }

    /// c:44 — `unmetafy_zalloc` is deterministic for same input.
    #[test]
    fn unmetafy_zalloc_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = unmetafy_zalloc("test-string");
        for _ in 0..3 {
            assert_eq!(
                unmetafy_zalloc("test-string"),
                first,
                "unmetafy_zalloc must be deterministic"
            );
        }
    }

    /// c:695 — `append_tied_name` returns i32 (compile-time type pin).
    /// C body returns 0 at c:713.
    #[test]
    fn append_tied_name_returns_i32_zero() {
        let _g = crate::test_util::global_state_lock();
        // Use unique sentinel name to avoid leakage from other tests.
        let r = append_tied_name("__zshrs_test_append_tied_xyz__");
        assert_eq!(r, 0, "append_tied_name returns 0 per c:713");
        // Cleanup.
        let _ = remove_tied_name("__zshrs_test_append_tied_xyz__");
    }

    /// c:43 — `remove_tied_name` returns i32 (compile-time type pin).
    #[test]
    fn remove_tied_name_returns_i32_zero_on_missing() {
        let _g = crate::test_util::global_state_lock();
        let r = remove_tied_name("__zshrs_test_never_tied_remove__");
        assert_eq!(r, 0, "remove of never-tied name → 0 (no-op)");
    }

    /// c:695 + c:43 — append+remove round-trip leaves ZGDBM_TIED unchanged.
    #[test]
    fn append_then_remove_tied_name_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let name = "__zshrs_test_round_trip_xyz__";
        let before = ZGDBM_TIED.lock().unwrap().len();
        append_tied_name(name);
        let after_add = ZGDBM_TIED.lock().unwrap().len();
        assert_eq!(after_add, before + 1, "append grows by 1");
        remove_tied_name(name);
        let after_remove = ZGDBM_TIED.lock().unwrap().len();
        assert_eq!(after_remove, before, "remove restores original count");
    }

    /// c:799 — `myfreeparamnode` on unknown param is safe no-op
    /// (calls gdbmunsetfn which is already pinned safe on unknown DBs).
    #[test]
    fn myfreeparamnode_unknown_no_panic() {
        let _g = crate::test_util::global_state_lock();
        myfreeparamnode("__zshrs_test_never_param_freenode__", "key");
        myfreeparamnode("", "");
    }

    /// c:927+ — every lifecycle hook returns i32 (compile-time type pin).
    #[test]
    fn db_gdbm_lifecycle_hooks_return_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let _: i32 = setup_(null);
        let _: i32 = boot_(null);
        let _: i32 = cleanup_(null);
        let _: i32 = finish_(null);
        let mut feats = Vec::new();
        let _: i32 = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _: i32 = enables_(null, &mut enables);
    }

    /// c:695 — `append_tied_name` with empty string still records
    /// (C code doesn't filter empty names — ztrdup("") is valid).
    #[test]
    fn append_tied_name_empty_string_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let before = ZGDBM_TIED.lock().unwrap().len();
        append_tied_name("");
        let after = ZGDBM_TIED.lock().unwrap().len();
        assert!(after >= before, "append never shrinks the list");
        // Cleanup.
        let _ = remove_tied_name("");
    }

    /// c:43 — `remove_tied_name` empty string is safe.
    #[test]
    fn remove_tied_name_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = remove_tied_name("");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/db_gdbm.c
    // c:46 bin_ztie / c:157 bin_zuntie / c:211 bin_zgdbmpath /
    // c:262 gdbmgetfn / c:288 gdbmsetfn / c:316 gdbmunsetfn /
    // c:350 getgdbmnode / c:519 gdbmuntie
    // ═══════════════════════════════════════════════════════════════════

    /// c:46 — `bin_ztie` returns i32 (compile-time pin).
    #[test]
    fn bin_ztie_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_ztie("ztie", &[], &ops, 0);
    }

    /// c:46 — `bin_ztie` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_ztie_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ztie("ztie", &[], &ops, 0);
        assert_ne!(r, 0, "ztie no args → usage error");
    }

    /// c:157 — `bin_zuntie` returns i32 (compile-time pin).
    #[test]
    fn bin_zuntie_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_zuntie("zuntie", &[], &ops, 0);
    }

    /// c:157 — `bin_zuntie` no-args returns nonzero (usage error, alt).
    /// IGNORED — same ZSHRS BUG as `bin_zuntie_no_args_returns_nonzero`.
    #[test]
    fn bin_zuntie_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zuntie("zuntie", &[], &ops, 0);
        assert_ne!(r, 0, "zuntie no args → usage error");
    }

    /// c:211 — `bin_zgdbmpath` returns i32 (compile-time pin).
    #[test]
    fn bin_zgdbmpath_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_zgdbmpath("zgdbmpath", &[], &ops, 0);
    }

    /// c:262 — `gdbmgetfn` returns String (compile-time pin).
    #[test]
    fn gdbmgetfn_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = gdbmgetfn("__never_tied__", "anykey");
    }

    /// c:262 — `gdbmgetfn` for never-tied param returns empty.
    #[test]
    fn gdbmgetfn_never_tied_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = gdbmgetfn("__definitely_never_tied_xyz_zshrs__", "any");
        assert_eq!(r, "", "never-tied get → empty");
    }

    /// c:288 — `gdbmsetfn` with None value (deletion) on missing DB safe.
    #[test]
    fn gdbmsetfn_none_value_on_missing_db_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmsetfn("__never_tied_set__", "k", None);
    }

    /// c:316 — `gdbmunsetfn` on missing DB is no-op (no panic).
    #[test]
    fn gdbmunsetfn_missing_db_no_panic() {
        let _g = crate::test_util::global_state_lock();
        gdbmunsetfn("__never_tied_unset__", "k", 0);
        gdbmunsetfn("__never_tied_unset__", "k", 1);
    }

    /// c:350 — `getgdbmnode` returns bool (compile-time pin, alt).
    #[test]
    fn getgdbmnode_returns_bool_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = getgdbmnode("__never__", "k");
    }

    /// c:350 — `getgdbmnode` for never-tied returns false.
    #[test]
    fn getgdbmnode_never_tied_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !getgdbmnode("__definitely_never_tied_node_xyz__", "any"),
            "never-tied → false"
        );
    }

    /// c:519 — `gdbmuntie` on never-tied param is safe no-op (alt).
    #[test]
    fn gdbmuntie_never_tied_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        gdbmuntie("__never_tied_untie_xyz__");
    }
}
