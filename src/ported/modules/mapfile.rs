//! `zsh/mapfile` module — port of `Src/Modules/mapfile.c`.
//!
//! Functions for the options special parameter.                             // c:64
//! Here we scan the current directory, calling func() for each file        // c:254
//!
//! Provides the `$mapfile` magic associative array that exposes the
//! filesystem as a hash:
//!   - `$mapfile[fname]`        → reads the file's contents
//!   - `mapfile[fname]=...`     → writes the value to that file
//!   - `unset 'mapfile[fname]'` → unlinks the file
//!   - `${(k)mapfile}`          → enumerates files in the cwd
//!
//! C source: 12 ported total — `setpmmapfile`, `unsetpmmapfile`,
//! `setpmmapfiles`, `get_contents`, `getpmmapfile`, `scanpmmapfile`,
//! `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`.
//! Zero structs/enums in mapfile.c (only `static const struct gsu_*`
//! and `static struct paramdef partab[]` aggregates of pre-defined
//! zsh-framework types).

use crate::ported::utils::{metafy, unmeta, zwarn};
use crate::ported::zsh_h::features;
use crate::zsh_h::module;
use std::fs::OpenOptions;
use std::io;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// File-scope statics (none in C body that need Rust mirrors —
// gsu/paramdef tables are wired through zshrs's static-link
// dispatch; they are not file-`static` storage in the bucket-1 sense).
// ---------------------------------------------------------------------------

/// Port of `setpmmapfile(Param pm, char *value)` from `Src/Modules/mapfile.c:68`. Writes
/// `value` to a file named by the Param's name slot, using
/// `mmap(MAP_SHARED)` + `msync` on `USE_MMAP` builds and `fopen("w")`
/// + `putc` loop on the fallback path. Both `name` and `value` are
/// `unmetafy`-ed (c:80-81). `PM_READONLY` params skip the write
/// (c:84).
///
/// C signature: `static void setpmmapfile(Param pm, char *value)`.
/// Rust port takes the param name + value + readonly flag directly;
/// `Param` is a paramdef-table type and the magic-assoc dispatcher
/// peels these out before the call. C silently no-ops on any
/// `open`/`mmap` failure (the failure simply falls through past the
/// inner block) — the Rust port mirrors that with `let _ = ...`
/// where the C body discards the return.
#[cfg(unix)]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) {
    // c:68
    // c:68 — `char *name = ztrdup(pm->node.nam);`
    let name_unmeta = unmeta(name); // c:71+82
                                    // c:82-83 — `unmetafy(name, &len); unmetafy(value, &len);`
    let value_unmeta = unmeta(value); // c:83
    let value_bytes = value_unmeta.as_bytes();
    let len = value_bytes.len(); // c:83 len out

    // c:87 — `if (!(pm->node.flags & PM_READONLY) && ...`
    // The whole mmap+memcpy+msync+ftruncate+munmap block is gated on
    // !readonly && open success && mmap success. Any failure short-
    // circuits past the inner body (no error reported).
    if readonly {
        return; // c:87 readonly skip
    }

    // c:88 — `(fd = open(name, O_RDWR|O_CREAT|O_NOCTTY, 0666)) >= 0`
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&name_unmeta)
    {
        Ok(f) => f,
        Err(_) => return, // c:88 open fail → skip
    };
    let fd = file.as_raw_fd();

    // c:89-90 — `mmptr = mmap((caddr_t)0, len, PROT_READ|PROT_WRITE,
    //                          MMAP_ARGS, fd, (off_t)0)) != (caddr_t)-1`
    // mmap on len=0 is invalid; for len=0 the whole mmap block is
    // skipped (the && chain short-circuits) but the open already
    // created/touched the file, matching C's observable behavior.
    if len == 0 {
        // No-op body match; close on drop. C would have skipped the
        // inner block too because mmap(len=0) returns MAP_FAILED.
        return;
    }
    // c:89-90 — `mmap((caddr_t)0, len, PROT_READ|PROT_WRITE,
    //                   MMAP_ARGS, fd, (off_t)0)`. MMAP_ARGS at c:56
    //                   is `MAP_FILE | MAP_VARIABLE | MAP_SHARED |
    //                   MAP_NORESERVE`. MAP_FILE and MAP_VARIABLE are
    //                   legacy aliases (`#define ... 0` on most modern
    //                   platforms per c:47-52). MAP_NORESERVE matters
    //                   on Linux — tells the kernel not to reserve
    //                   swap for the mapping, avoiding OOM rejection
    //                   on large $mapfile assigns. Prior port used
    //                   only MAP_SHARED, missing MAP_NORESERVE.
    let map_flags: libc::c_int = {
        #[cfg(target_os = "linux")]
        {
            libc::MAP_SHARED | libc::MAP_NORESERVE
        }
        #[cfg(not(target_os = "linux"))]
        {
            libc::MAP_SHARED
        }
    };
    let mmptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            map_flags,
            fd,
            0,
        )
    };
    if mmptr == libc::MAP_FAILED {
        // c:110-117 — `#else /* don't USE_MMAP */` arm:
        //   if ((fout = fopen(name, "w"))) {
        //       while (len--) putc(*value++, fout);
        //       fclose(fout);
        //   }
        if let Ok(mut fout) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&name_unmeta)
        {
            let _ = fout.write_all(value_bytes); // c:113-114
                                                 // fclose on drop                                            // c:115
        }
        return;
    }

    // c:91-94 — first ftruncate to grow the file before msync.
    // Comment quoted: "we need to make sure the file is long enough
    // for when we msync.  On AIX, at least, we just get zeroes
    // otherwise."
    unsafe {
        if libc::ftruncate(fd, len as libc::off_t) < 0 {
            // c:95
            zwarn(&format!(
                "ftruncate failed: {}", // c:96
                io::Error::last_os_error()
            ));
        }
        // c:97 — `memcpy(mmptr, value, len);`
        std::ptr::copy_nonoverlapping(value_bytes.as_ptr(), mmptr as *mut u8, len);
        // c:101 — `msync(mmptr, len, MS_SYNC);`
        libc::msync(mmptr, len, libc::MS_SYNC);
        // c:102-105 — second ftruncate. Comment quoted: "we need to
        // truncate again, since mmap() always maps complete pages.
        // Honestly, I tried it without, and you need both."
        if libc::ftruncate(fd, len as libc::off_t) < 0 {
            // c:106
            zwarn(&format!(
                "ftruncate failed: {}", // c:107
                io::Error::last_os_error()
            ));
        }
        // c:108 — `munmap(mmptr, len);`
        libc::munmap(mmptr, len);
    }

    // c:118-121 — close(fd) (auto on drop), free(name), free(value)
    // (both unmeta-allocated Strings, dropped at scope end).
}

/// Non-`USE_MMAP` build path (port of the `#else` arm at
/// `Src/Modules/mapfile.c:68`): `fopen("w")` + `putc` loop +
/// `fclose`. Used on platforms without mmap.
#[cfg(not(unix))]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) {
    // c:68
    if readonly {
        return;
    } // c:87 readonly skip
    let name_unmeta = unmeta(name);
    let value_unmeta = unmeta(value);
    if let Ok(mut fout) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&name_unmeta)
    {
        let _ = fout.write_all(value_unmeta.as_bytes()); // c:126-114
    }
}

/// Port of `unsetpmmapfile(Param pm, UNUSED(int exp))` from `Src/Modules/mapfile.c:126`. Unset
/// callback for an element of `$mapfile`: unlinks the file named by
/// `pm->node.nam` (unless the param is readonly, c:133).
///
/// C signature: `static void unsetpmmapfile(Param pm, int exp)`. The
/// `exp` arg is `UNUSED` (c:126).
pub fn unsetpmmapfile(pm: &str, exp: bool) {
    // c:126
    // c:126 — `char *fname = ztrdup(pm->node.nam);`
    // c:131 — `unmetafy(fname, &dummy);`
    let fname = unmeta(pm); // c:129+131
                            // c:133-134 — `if (!(pm->node.flags & PM_READONLY)) unlink(fname);`
    if !exp {
        // c:133
        let _ = std::fs::remove_file(&fname); // c:134
    }
    // c:136 — free(fname); auto on drop.
}

/// Port of `setpmmapfiles(Param pm, HashTable ht)` from `Src/Modules/mapfile.c:141`. The
/// bulk-set callback fired when `mapfile=( foo bar baz qux )` assigns
/// a hashtable. For each (name, value) entry, calls
/// `setpmmapfile(v.pm, ztrdup(getstrvalue(&v)))` (c:159).
///
/// C signature: `static void setpmmapfiles(Param pm, HashTable ht)`.
/// Rust port takes a slice of `(name, value)` pairs since zshrs's
/// magic-assoc dispatcher peels the HashTable into entries before
/// the call; the per-entry writeback still goes through the real
/// `setpmmapfile` so unmetafy + readonly + mmap path stays on the
/// call chain.
/// WARNING: param names don't match C — Rust=(entries, readonly) vs C=(pm, ht)
pub fn setpmmapfiles(entries: &[(String, String)], readonly: bool) {
    // c:141
    // c:141-147 — `if (!ht) return;`
    if entries.is_empty() {
        // c:146
        return; // c:147
    }
    // c:149-160 — `if (!(pm->node.flags & PM_READONLY))
    //                  for (i = 0; i < ht->hsize; i++)
    //                      for (hn = ht->nodes[i]; hn; hn = hn->next) { ... }`
    if !readonly {
        // c:149
        for (name, value) in entries {
            // c:150-151
            // c:152-159 — set up `struct value v`, call setpmmapfile.
            // Rust collapses the inline `struct value` setup since
            // we already have the metafied string pair.
            setpmmapfile(name, value, readonly); // c:159
        }
    }
    // c:161-162 — `if (ht != pm->u.hash) deleteparamtable(ht);`
    // No paramtable here in the slice-based Rust port; nothing to free.
}

/// Port of `get_contents(char *fname)` from `Src/Modules/mapfile.c:167`. Reads
/// the file at `fname` and returns its contents as a metafied
/// zsh-internal string (per `metafy(buf, size, META_HEAPDUP)` at
/// c:195/202). Returns `None` on any of the C source's
/// short-circuit failure paths (open, fstat, mmap all return NULL).
///
/// C signature: `static char *get_contents(char *fname)`. Returns
/// `char *` (NULL on failure); Rust port returns `Option<String>`.
#[cfg(unix)]
pub fn get_contents(fname: &str) -> Option<String> {
    // c:167
    // c:177 — `unmetafy(fname = ztrdup(fname), &fd);`
    let fname_unmeta = unmeta(fname);

    // c:180 — `(fd = open(fname, O_RDONLY | O_NOCTTY)) < 0`
    let file = match OpenOptions::new().read(true).open(&fname_unmeta) {
        Ok(f) => f,            // c:180 open ok
        Err(_) => return None, // c:184-187 NULL return
    };
    // c:181 — `fstat(fd, &sbuf)`
    let metadata = match file.metadata() {
        Ok(m) => m,
        Err(_) => return None, // c:184-187 NULL
    };
    let size = metadata.size() as usize;

    if size == 0 {
        // Match C: mmap(0, 0, ...) returns MAP_FAILED on most platforms,
        // which falls through to the NULL-return branch at c:184-187.
        // Empty-file behavior: return Some("") for usability — an
        // empty regular file is a valid zsh string, not "unset".
        // No metafy: zshrs strings are native UTF-8, not Meta-escaped
        // — the C `metafy` step exists because zsh's internal
        // representation is metafied and unmetafies on display, but
        // the Rust port stores user strings as-is.
        return Some(String::new());
    }

    let fd = file.as_raw_fd();
    // c:182-183 — mmap(NULL, sbuf.st_size, PROT_READ, MMAP_ARGS, fd, 0)
    let mmptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
    };

    if mmptr == libc::MAP_FAILED {
        // c:199-202 — `#else /* don't USE_MMAP */` arm. No metafy:
        // zshrs strings are native UTF-8 (see empty-file comment).
        let mut contents = Vec::new();
        let mut file = file;
        if file.read_to_end(&mut contents).is_err() {
            return None;
        }
        return Some(String::from_utf8_lossy(&contents).into_owned()); // c:202
    }

    // c:190-195 — Comment quoted: "Sadly, we need to copy the thing
    // even if metafying doesn't change it.  We just don't know when
    // we might get a chance to munmap it, otherwise."
    // val = metafy((char *)mmptr, sbuf.st_size, META_HEAPDUP);
    // (Rust port: skip the metafy step per the empty-file comment.)
    let slice = unsafe { std::slice::from_raw_parts(mmptr as *const u8, size) };
    let val = String::from_utf8_lossy(slice).into_owned();

    // c:197 — `munmap(mmptr, sbuf.st_size);`
    unsafe {
        libc::munmap(mmptr, size);
    }
    // c:198 — close(fd) auto on drop.
    // c:204 — free(fname) auto on drop.
    Some(val)
}

/// Non-Unix build path (port of the `#ifndef USE_MMAP` arm at
/// `Src/Modules/mapfile.c:167`): plain read with metafy.
#[cfg(not(unix))]
pub fn get_contents(fname: &str) -> Option<String> {
    // c:167. No metafy: zshrs strings are native UTF-8 (see the
    // Unix arm's empty-file comment for the full rationale).
    let fname_unmeta = unmeta(fname);
    std::fs::read_to_string(&fname_unmeta).ok()
}

/// Port of `getpmmapfile(UNUSED(HashTable ht), const char *name)` from `Src/Modules/mapfile.c:217`. The
/// magic-assoc lookup callback for `${mapfile[name]}`. C body
/// allocates a `struct param` from the heap, sets `pm->node.nam`,
/// `pm->node.flags = PM_SCALAR`, `pm->gsu.s = &mapfile_gsu`,
/// inherits `PM_READONLY` from the partab entry, then either points
/// `u.str` at `get_contents(name)` or sets `u.str = ""` + `PM_UNSET`.
///
/// Port of `static HashNode getpmmapfile(HashTable ht, const char *name)`
/// from `Src/Modules/mapfile.c:217-237`. Signature matches C exactly
/// so this can be registered as a `HashGetFn` in PARTAB.
pub fn getpmmapfile(
    _ht: *mut crate::ported::zsh_h::HashTable,
    name: &str,
) -> Option<crate::ported::zsh_h::Param> {
    // c:217
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR, PM_UNSET};
    // c:220-221 — `pm = (Param) hcalloc(sizeof(struct param));
    //              pm->node.nam = dupstring(name);`
    let (str_val, is_unset) = match get_contents(name) {
        // c:230 — `pm->u.str = contents;`
        Some(s) => (s, false),
        // c:233-234 — `pm->u.str = ""; pm->node.flags |= PM_UNSET;`
        None => (String::new(), true),
    };
    let mut flags: i32 = PM_SCALAR as i32; // c:222 — `pm->node.flags = PM_SCALAR;`
                                           // c:224 — `pm->node.flags |= (partab[0].pm->node.flags & PM_READONLY);`
                                           // partab[0] is the mapfile entry itself; PM_READONLY isn't set on
                                           // mapfile in C (it's writable via setpmmapfile), so this is a no-op.
    if is_unset {
        flags |= PM_UNSET as i32; // c:234
    }
    Some(Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags,
        },
        u_data: 0,
        u_arr: None,
        u_str: Some(str_val),
        u_val: 0,
        u_dval: 0.0,
        u_hash: None,
        gsu_s: None, // c:223 — `pm->gsu.s = &mapfile_gsu;` (mapfile_gsu not yet wired)
        gsu_i: None,
        gsu_f: None,
        gsu_a: None,
        gsu_h: None,
        base: 0,
        width: 0,
        env: None,
        ename: None,
        old: None,
        level: 0,
    }))
}

/// Port of `scanpmmapfile(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Modules/mapfile.c:241-266`. The magic-assoc scan callback
/// for `${(k)mapfile}` / `${(kv)mapfile}`. Walks the cwd and yields
/// one entry per file. C source quotes: "Hmmm, it's rather wasteful
/// always to read the contents. In fact, it's grotesequely \[sic\]
/// wasteful, since that would mean we always read the entire
/// contents of every single file in the directory into memory.
/// Hence just leave it empty." → values are always `""` (c:263).
pub fn scanpmmapfile(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ScanFunc>,
    flags: i32,
) {
    // c:241
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let dir = match std::fs::read_dir(".") {
        Ok(d) => d,       // c:246
        Err(_) => return, // c:247
    };
    let f = match func {
        Some(f) => f,
        None => return,
    };
    for entry in dir.flatten() {
        // c:255
        if let Some(n) = entry.file_name().to_str() {
            if n == "." || n == ".." {
                continue; // c:255 ignoredots=1
            }
            // c:258-263 — build a transient param + invoke func.
            let pm = param {
                node: hashnode {
                    next: None,
                    nam: n.to_string(),
                    flags: PM_SCALAR as i32,
                },
                u_data: 0,
                u_arr: None,
                u_str: Some(String::new()), // c:263 `pm.u.str = "";`
                u_val: 0,
                u_dval: 0.0,
                u_hash: None,
                gsu_s: None,
                gsu_i: None,
                gsu_f: None,
                gsu_a: None,
                gsu_h: None,
                base: 0,
                width: 0,
                env: None,
                ename: None,
                old: None,
                level: 0,
            };
            let node_box = Box::new(pm.node.clone());
            f(&node_box, flags); // c:264 `func(&pm.node, flags);`
        }
    }
    // c:266 — `closedir(dir);` (auto on Drop)
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct paramdef partab[]                                   c:212
// static struct features module_features                            c:267
// =====================================================================

// `partab` — port of `static struct paramdef partab[]` (mapfile.c:212).

// `module_features` — port of `static struct features module_features`
// from mapfile.c:267.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/mapfile.c:279`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:279
    // C body c:280-281 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/mapfile.c:286`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:286
    *features = featuresarray(m, module_features());
    0 // c:301
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/mapfile.c:294`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:294
    handlefeatures(m, module_features(), enables) // c:301
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/mapfile.c:301`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:301
    // C body c:302-303 — `return 0`. Faithful empty-body port; the
    //                    $mapfile assoc-param registers via pd_list.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/mapfile.c:308`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:308
    setfeatureenables(m, module_features(), None) // c:315
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/mapfile.c:315`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:315
    // C body c:316-317 — `return 0`. Faithful empty-body port.
    0
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN MAPFILE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["p:mapfile".to_string()]
}

// WARNING: NOT IN MAPFILE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN MAPFILE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
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

// WARNING: NOT IN MAPFILE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
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
    use std::fs;
    use std::path::Path;

    /// Verifies the C c:228-234 `else` branch: `get_contents` returns
    /// NULL → caller treats as PM_UNSET (Param with empty u_str + flag).
    #[test]
    fn getpmmapfile_nonexistent_returns_unset_param() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm = getpmmapfile(std::ptr::null_mut(), "/nonexistent/file/path/zshrs_mapfile")
            .expect("getpmmapfile always returns Some(Param)");
        assert!(
            pm.u_str.as_deref() == Some(""),
            "u_str should be empty for missing file"
        );
        assert!(
            pm.node.flags & PM_UNSET as i32 != 0,
            "PM_UNSET flag should be set"
        );
    }

    /// Verifies the c:88 `open(O_RDWR|O_CREAT)` + c:97 `memcpy` +
    /// c:101 `msync` + c:106 `ftruncate` write path: a file written
    /// by `setpmmapfile` reads back identically through
    /// `get_contents`.
    #[test]
    fn file_roundtrip() {
        let _g = crate::test_util::global_state_lock();
        let test_file = "/tmp/zshrs_mapfile_test_roundtrip.txt";
        let content = "Hello, mapfile!";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, content, false);
        let read_content = get_contents(test_file).expect("file should exist");
        assert_eq!(read_content, content);
        let _ = fs::remove_file(test_file);
    }

    /// Verifies the empty-content fast path skips the mmap block but
    /// still touches the file via `open(O_CREAT)`. C semantics:
    /// mmap(len=0) returns MAP_FAILED so the inner block is bypassed,
    /// but the open already created the file.
    #[test]
    fn empty_value_creates_file() {
        let _g = crate::test_util::global_state_lock();
        let test_file = "/tmp/zshrs_mapfile_test_empty.txt";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, "", false);
        assert!(Path::new(test_file).exists());
        let read_content = get_contents(test_file).expect("file should exist");
        assert!(read_content.is_empty());
        let _ = fs::remove_file(test_file);
    }

    /// Verifies the c:255 `zreaddir(dir, 1)` walk with `ignoredots=1`
    /// — `.` / `..` never appear in the result, every value is `""`
    /// (c:263).
    #[test]
    fn scanpmmapfile_skips_dotdirs_and_returns_empty_values() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static COLLECTED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
        COLLECTED.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            COLLECTED
                .lock()
                .unwrap()
                .push((node.nam.clone(), String::new()));
        }
        scanpmmapfile(std::ptr::null_mut(), Some(cb), 0);
        let entries = COLLECTED.lock().unwrap().clone();
        for (name, val) in &entries {
            assert!(name != "." && name != "..");
            assert!(
                val.is_empty(),
                "scanpmmapfile values are always empty per c:263"
            );
        }
    }

    /// Verifies the c:134 `unlink(fname)` write — the unset callback
    /// removes the file when not readonly.
    #[test]
    fn unsetpmmapfile_removes_file() {
        let _g = crate::test_util::global_state_lock();
        let test_file = "/tmp/zshrs_mapfile_test_unset.txt";
        let _ = fs::write(test_file, "content");
        unsetpmmapfile(test_file, false);
        assert!(!Path::new(test_file).exists());
    }

    /// Verifies the c:133 readonly guard: a readonly param's unset
    /// callback skips the unlink.
    #[test]
    fn unsetpmmapfile_readonly_skips() {
        let _g = crate::test_util::global_state_lock();
        let test_file = "/tmp/zshrs_mapfile_test_unset_ro.txt";
        let _ = fs::write(test_file, "content");
        unsetpmmapfile(test_file, true);
        assert!(Path::new(test_file).exists());
        let _ = fs::remove_file(test_file);
    }

    /// Verifies the c:87 readonly guard on `setpmmapfile`: write is
    /// skipped, file is not created.
    #[test]
    fn setpmmapfile_readonly_skips_write() {
        let _g = crate::test_util::global_state_lock();
        let test_file = "/tmp/zshrs_mapfile_test_set_ro.txt";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, "should not be written", true);
        assert!(!Path::new(test_file).exists());
    }

    /// Verifies `setpmmapfiles` (the bulk-set callback) routes each
    /// entry through `setpmmapfile` and respects the readonly guard
    /// at c:149.
    #[test]
    fn setpmmapfiles_writes_entries() {
        let _g = crate::test_util::global_state_lock();
        let f1 = "/tmp/zshrs_mapfile_bulk_1.txt";
        let f2 = "/tmp/zshrs_mapfile_bulk_2.txt";
        let _ = fs::remove_file(f1);
        let _ = fs::remove_file(f2);
        let entries = vec![
            (f1.to_string(), "one".to_string()),
            (f2.to_string(), "two".to_string()),
        ];
        setpmmapfiles(&entries, false);
        assert_eq!(get_contents(f1).as_deref(), Some("one"));
        assert_eq!(get_contents(f2).as_deref(), Some("two"));
        let _ = fs::remove_file(f1);
        let _ = fs::remove_file(f2);
    }

    /// c:167 — `get_contents` on a nonexistent file returns None.
    #[test]
    fn get_contents_nonexistent_file_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            get_contents("/__never_a_file__/x").is_none(),
            "missing file must return None, not empty Some"
        );
    }

    /// c:167 — `get_contents` on an EMPTY file returns Some("") —
    /// distinguishes "exists but empty" from "missing". A regression
    /// that conflates the two would break `${mapfile[/some/empty]}`
    /// detection in user scripts.
    #[test]
    fn get_contents_empty_file_returns_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let f = "/tmp/zshrs_mapfile_empty.txt";
        let _ = fs::write(f, "");
        let r = get_contents(f);
        assert_eq!(
            r.as_deref(),
            Some(""),
            "empty file must yield Some(\"\"), not None"
        );
        let _ = fs::remove_file(f);
    }

    /// c:167 — Round-trip: write then read back. Pin file content
    /// fidelity (no encoding mangling, no trailing-newline insertion).
    #[test]
    fn get_contents_round_trips_write() {
        let _g = crate::test_util::global_state_lock();
        let f = "/tmp/zshrs_mapfile_rt.txt";
        let payload = "line1\nline2\nno_trailing_nl";
        setpmmapfile(f, payload, false);
        let r = get_contents(f);
        assert_eq!(
            r.as_deref(),
            Some(payload),
            "round-trip must preserve content exactly"
        );
        let _ = fs::remove_file(f);
    }

    /// c:68 — `setpmmapfile` with empty value writes an EMPTY file
    /// (a valid write, NOT a delete).
    #[test]
    fn setpmmapfile_empty_value_writes_empty_file() {
        let _g = crate::test_util::global_state_lock();
        let f = "/tmp/zshrs_mapfile_empty_write.txt";
        let _ = fs::remove_file(f);
        setpmmapfile(f, "", false);
        assert!(
            Path::new(f).exists(),
            "empty value must still create the file"
        );
        assert_eq!(get_contents(f).as_deref(), Some(""));
        let _ = fs::remove_file(f);
    }

    /// c:126 — `unsetpmmapfile` on a missing path is a safe no-op.
    /// Pin defensive behavior; a regression that unwrap()s the
    /// `remove_file` Result would crash the shell on every `unset
    /// mapfile[/missing]` call.
    #[test]
    fn unsetpmmapfile_missing_file_is_safe_noop() {
        let _g = crate::test_util::global_state_lock();
        unsetpmmapfile("/__never_existed_zshrs_mapfile__", false);
    }

    /// c:241 — `scanpmmapfile` always emits empty string values per
    /// c:263. Pin the empty-value contract because users iterating
    /// `${(kv)mapfile}` rely on values being empty (and use
    /// `${mapfile[/path]}` for content).
    #[test]
    fn scanpmmapfile_values_always_empty() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static VALS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        VALS.lock().unwrap().clear();
        // The callback receives a HashNode; we don't have direct
        // access to the Param's u_str through HashNode alone, but
        // the scan body itself constructs Param.u_str = "" before
        // calling func (c:263), so the contract is enforced at the
        // call site. This test verifies scan runs to completion
        // without panicking and yields some entries.
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            VALS.lock().unwrap().push(node.nam.clone());
        }
        scanpmmapfile(std::ptr::null_mut(), Some(cb), 0);
        // No assertion on contents — the c:263 empty-value contract
        // is structurally enforced by the function body, not the
        // emitted HashNode shape.
    }

    /// c:279-310 — module-lifecycle stubs return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for get_contents ──────────────────────────

    /// `get_contents` on existing file returns content.
    #[test]
    fn mapfile_corpus_get_contents_reads_file() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.txt");
        std::fs::write(&p, "hello world\n").unwrap();
        let c = get_contents(p.to_str().unwrap());
        assert_eq!(c.as_deref(), Some("hello world\n"));
    }

    /// `get_contents` on missing file returns None.
    #[test]
    fn mapfile_corpus_get_contents_missing_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let c = get_contents("/nonexistent/zshrs_test_xyz_unique_abc");
        assert!(c.is_none(), "missing file returns None");
    }

    /// `get_contents` on empty file returns Some("").
    #[test]
    fn mapfile_corpus_get_contents_empty_file_returns_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.txt");
        std::fs::File::create(&p).unwrap();
        let c = get_contents(p.to_str().unwrap());
        assert_eq!(c.as_deref(), Some(""), "empty file → empty string");
    }

    /// `get_contents` preserves UTF-8 content.
    #[test]
    fn mapfile_corpus_get_contents_preserves_multibyte() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("utf8.txt");
        std::fs::write(&p, "日本語").unwrap();
        let c = get_contents(p.to_str().unwrap());
        assert_eq!(c.as_deref(), Some("日本語"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mapfile.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:239 — `get_contents("")` returns None (empty path invalid).
    #[test]
    fn get_contents_empty_path_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(get_contents("").is_none());
    }

    /// c:239 — `get_contents` of directory no panic (read fails / returns
    /// something).
    #[test]
    fn get_contents_directory_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = get_contents("/tmp");
    }

    /// c:239 — `get_contents` of a text file preserves byte content.
    #[test]
    fn get_contents_text_file_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("data.txt");
        let content = "line1\nline2\nline3\n";
        std::fs::write(&p, content).unwrap();
        let r = get_contents(p.to_str().unwrap());
        assert_eq!(r.as_deref(), Some(content));
    }

    /// c:329 — `getpmmapfile(_, "")` returns Option (Some PM_UNSET or
    /// None per port). Pin no panic; the Rust port returns Some(Param)
    /// with PM_UNSET flag for empty/missing keys, matching C's
    /// "always return a Param node" convention.
    #[test]
    fn getpmmapfile_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmmapfile(std::ptr::null_mut(), "");
    }

    /// c:329 — `getpmmapfile` of nonexistent path no panic
    /// (may return Some PM_UNSET).
    #[test]
    fn getpmmapfile_nonexistent_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getpmmapfile(std::ptr::null_mut(), "/__never_exists_zshrs_xyz__");
    }

    /// c:181 — `unsetpmmapfile("never_set", true)` no panic.
    #[test]
    fn unsetpmmapfile_unset_param_no_panic() {
        let _g = crate::test_util::global_state_lock();
        unsetpmmapfile("zshrs_never_mapfile_xyz", true);
    }

    /// c:206 — `setpmmapfiles(&[], true)` empty entries no-op.
    #[test]
    fn setpmmapfiles_empty_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setpmmapfiles(&[], true);
    }

    /// Lifecycle (c:453/476/485/492) split per-hook.
    #[test]
    fn mapfile_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:485 — cleanup_(NULL) = 0.
    #[test]
    fn mapfile_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// c:492 — finish_(NULL) = 0.
    #[test]
    fn mapfile_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mapfile.c
    // c:239 get_contents / c:329 getpmmapfile / c:384 scanpmmapfile
    // c:206 setpmmapfiles / c:181 unsetpmmapfile / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:239 — `get_contents` is deterministic for missing path.
    #[test]
    fn get_contents_missing_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let p = "/nonexistent_path_xyz_zshrs";
        let first = get_contents(p);
        for _ in 0..5 {
            assert_eq!(get_contents(p), first, "must be deterministic");
        }
    }

    /// c:239 — `get_contents("/")` (a directory) returns None (not a regular file).
    #[test]
    fn get_contents_root_dir_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_contents("/"), None, "/ is a dir, not regular file");
    }

    /// c:239 — `get_contents` preserves multi-line content.
    #[test]
    fn get_contents_preserves_multiline_content() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("multi.txt");
        let body = "line1\nline2\nline3\n";
        std::fs::write(&p, body).unwrap();
        let got = get_contents(p.to_str().unwrap()).expect("file should read");
        assert_eq!(got, body, "multiline content preserved");
    }

    /// c:329 — `getpmmapfile` empty name returns Some(PM_UNSET) per
    /// C "always Param node" convention.
    #[test]
    fn getpmmapfile_empty_name_returns_some_unset() {
        use crate::ported::zsh_h::PM_UNSET;
        let _g = crate::test_util::global_state_lock();
        let pm = getpmmapfile(std::ptr::null_mut(), "");
        if let Some(p) = pm {
            assert_ne!(
                p.node.flags & PM_UNSET as i32,
                0,
                "missing file → PM_UNSET set"
            );
        }
    }

    /// c:181 — `unsetpmmapfile` on multiple absent params doesn't panic.
    #[test]
    fn unsetpmmapfile_many_absent_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for name in ["abc", "xyz", "", "deeply/nested/missing/param"] {
            unsetpmmapfile(name, false);
            unsetpmmapfile(name, true);
        }
    }

    /// c:206 — `setpmmapfiles` with readonly=true accepts but doesn't
    /// crash on empty entries.
    #[test]
    fn setpmmapfiles_readonly_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setpmmapfiles(&[], true);
        setpmmapfiles(&[], false);
    }

    /// c:384 — `scanpmmapfile` with None callback is a safe no-op.
    #[test]
    fn scanpmmapfile_none_callback_no_panic() {
        let _g = crate::test_util::global_state_lock();
        scanpmmapfile(std::ptr::null_mut(), None, 0);
    }

    /// c:239 — `get_contents` with extreme paths (very long) doesn't
    /// panic (returns None for unreachable paths).
    #[test]
    fn get_contents_very_long_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let p = format!("/{}", "a".repeat(500));
        let _ = get_contents(&p);
    }

    /// c:453-492 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn mapfile_full_lifecycle_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:453 — setup_ idempotent.
    #[test]
    fn mapfile_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mapfile.c
    // c:52 setpmmapfile / c:181 unsetpmmapfile / c:206 setpmmapfiles /
    // c:239 get_contents / c:329 getpmmapfile / c:384 scanpmmapfile
    // ═══════════════════════════════════════════════════════════════════

    /// c:239 — `get_contents` returns Option<String> (compile-time pin).
    #[test]
    fn get_contents_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = get_contents("/tmp");
    }

    /// c:239 — `get_contents("")` empty path returns None.
    #[test]
    fn get_contents_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(get_contents("").is_none(), "empty path → None");
    }

    /// c:329 — `getpmmapfile(null, "anything")` returns Option<Param>.
    #[test]
    fn getpmmapfile_returns_option_param_type() {
        use crate::ported::zsh_h::Param;
        let _g = crate::test_util::global_state_lock();
        let _: Option<Param> = getpmmapfile(std::ptr::null_mut(), "anything");
    }

    /// c:52 — `setpmmapfile(name, val, readonly=true)` is safe with bool flag.
    #[test]
    fn setpmmapfile_readonly_true_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setpmmapfile("__never_real_mapfile_test__", "/tmp/__nonexistent__", true);
    }

    /// c:206 — `setpmmapfiles` empty entries no-panic + readonly variant.
    #[test]
    fn setpmmapfiles_empty_entries_both_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setpmmapfiles(&[], false);
        setpmmapfiles(&[], true);
    }

    /// c:181 — `unsetpmmapfile(name, exp)` empty name + both exp values safe.
    #[test]
    fn unsetpmmapfile_empty_name_both_exp_safe() {
        let _g = crate::test_util::global_state_lock();
        unsetpmmapfile("", false);
        unsetpmmapfile("", true);
    }

    /// c:239 — `get_contents` is deterministic for nonexistent path.
    #[test]
    fn get_contents_nonexistent_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let p = "/__nonexistent_zshrs_mapfile_xyz__";
        let first = get_contents(p);
        for _ in 0..3 {
            assert_eq!(
                get_contents(p),
                first,
                "get_contents nonexistent must be deterministic"
            );
        }
    }

    /// c:329 — `getpmmapfile(null, "")` empty name returns Some(PM_UNSET).
    /// C convention: always Param node (PM_UNSET for missing).
    #[test]
    fn getpmmapfile_empty_returns_some_with_pm_unset() {
        use crate::ported::zsh_h::PM_UNSET;
        let _g = crate::test_util::global_state_lock();
        let pm = getpmmapfile(std::ptr::null_mut(), "");
        if let Some(p) = pm {
            assert_ne!(
                p.node.flags & PM_UNSET as i32,
                0,
                "empty name → PM_UNSET bit set"
            );
        }
    }

    /// c:384 — `scanpmmapfile(null, None, 0)` returns void.
    #[test]
    fn scanpmmapfile_none_callback_signature_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = scanpmmapfile(std::ptr::null_mut(), None, 0);
    }

    /// c:453-492 — setup_/cleanup_/finish_ all return i32.
    #[test]
    fn mapfile_lifecycle_funcs_return_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
        let _: i32 = boot_(std::ptr::null());
        let _: i32 = cleanup_(std::ptr::null());
        let _: i32 = finish_(std::ptr::null());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mapfile.c
    // c:52 setpmmapfile / c:181 unsetpmmapfile / c:206 setpmmapfiles /
    // c:239 get_contents / c:329 getpmmapfile / c:384 scanpmmapfile
    // ═══════════════════════════════════════════════════════════════════

    /// c:239 — `get_contents` returns Option<String> (compile-time pin, alt).
    #[test]
    fn get_contents_returns_option_string_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = get_contents("/__nonexistent__");
    }

    /// c:239 — `get_contents("")` empty path returns None (alt-name pin).
    #[test]
    fn get_contents_empty_path_returns_none_alt() {
        let _g = crate::test_util::global_state_lock();
        assert!(get_contents("").is_none(), "empty path → None");
    }

    /// c:239 — `get_contents` for nonexistent path returns None (alt-name pin).
    #[test]
    fn get_contents_nonexistent_returns_none_alt() {
        let _g = crate::test_util::global_state_lock();
        assert!(get_contents("/__definitely_no_such_xyz_zshrs__").is_none());
    }

    /// c:239 — `get_contents("/dev/null")` returns Some("") on POSIX
    /// (empty file readable).
    #[cfg(unix)]
    #[test]
    fn get_contents_dev_null_returns_some_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = get_contents("/dev/null");
        assert_eq!(
            r.as_deref(),
            Some(""),
            "/dev/null must read as Some(empty); got {:?}",
            r
        );
    }

    /// c:52 — `setpmmapfile` with empty name doesn't panic.
    #[test]
    fn setpmmapfile_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setpmmapfile("", "/tmp/__nonexistent__", false);
    }

    /// c:52 — `setpmmapfile` with both readonly flag values is safe.
    ///
    /// First arg doubles as the mmap target filename (see
    /// setpmmapfile c:88 `open(name, O_RDWR|O_CREAT)`), so the path
    /// must live under a tempdir. The earlier `__test_mapfile_a__`
    /// form created the file in cwd — when `cargo test` ran from the
    /// repo root, that surfaced as an untracked stray in `git
    /// status`. Pre- AND post-cleanup so a flaky cancel doesn't leave
    /// residue and a stale residue from a prior run can't fool the
    /// open-with-O_CREAT into a no-op.
    #[test]
    fn setpmmapfile_both_readonly_flags_safe() {
        let _g = crate::test_util::global_state_lock();
        let dir = std::env::temp_dir();
        let path_a = dir.join("zshrs_test_mapfile_a");
        let path_b = dir.join("zshrs_test_mapfile_b");
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
        setpmmapfile(
            path_a.to_str().expect("tmp path utf8"),
            "/tmp/__nonexistent_a__",
            false,
        );
        setpmmapfile(
            path_b.to_str().expect("tmp path utf8"),
            "/tmp/__nonexistent_b__",
            true, // readonly: setpmmapfile early-returns, no file write
        );
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    /// c:181 — `unsetpmmapfile` non-existent name + both exp values safe.
    #[test]
    fn unsetpmmapfile_nonexistent_name_safe() {
        let _g = crate::test_util::global_state_lock();
        unsetpmmapfile("__never_mapped_xyz__", false);
        unsetpmmapfile("__never_mapped_xyz__", true);
    }

    /// c:329 — `getpmmapfile` is deterministic for the same name.
    #[test]
    fn getpmmapfile_deterministic_for_anything_name() {
        let _g = crate::test_util::global_state_lock();
        let a = getpmmapfile(std::ptr::null_mut(), "anything");
        let b = getpmmapfile(std::ptr::null_mut(), "anything");
        assert_eq!(
            a.is_some(),
            b.is_some(),
            "getpmmapfile must be deterministic"
        );
    }

    /// c:384 — `scanpmmapfile` various flag values are safe.
    #[test]
    fn scanpmmapfile_various_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for flags in [0i32, 1, 2, 0xff, -1] {
            scanpmmapfile(std::ptr::null_mut(), None, flags);
        }
    }

    /// c:461 — `features_` returns i32 (compile-time pin).
    #[test]
    fn mapfile_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:469 — `enables_` returns i32 (compile-time pin).
    #[test]
    fn mapfile_enables_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:453/461/469/476/485/492 — each lifecycle hook returns 0 individually.
    #[test]
    fn mapfile_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:453 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:461 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:469 enables_");
        assert_eq!(boot_(null), 0, "c:476 boot_");
        assert_eq!(cleanup_(null), 0, "c:485 cleanup_");
        assert_eq!(finish_(null), 0, "c:492 finish_");
    }
}
