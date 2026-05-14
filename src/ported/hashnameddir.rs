//! Port of `Src/hashnameddir.c` — named-directory hash table.
//!
//! C: `mod_export HashTable nameddirtab;` plus a file-static
//! `int allusersadded;` and seven hash-table callbacks
//! (`createnameddirtable` / `emptynameddirtable` / `fillnameddirtable` /
//! `addnameddirnode` / `removenameddirnode` / `freenameddirnode` /
//! `printnameddirnode`).
//!
//! The Rust port keeps the `nameddir` struct definition canonical
//! (it lives in `zsh_h.rs`) and stores entries in a global
//! `OnceLock<Mutex<HashMap<String, nameddir>>>`, since the full
//! `HashTable` substrate (vtable callbacks, intrusive `next` chain)
//! is not yet wired. Names match C 1:1.

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ported::zsh_h::{nameddir, ND_USERNAME, PRINT_LIST, PRINT_NAMEONLY};

/****************************************/
/* Named Directory Hash Table Functions */
/****************************************/

// hash table containing named directories                                 // c:45
//
// C: `mod_export HashTable nameddirtab;` (c:48). Rust port stores the
// table as a `Mutex<HashMap<String, nameddir>>` keyed on `node.nam`.
static NAMEDDIRTAB_INNER: OnceLock<Mutex<HashMap<String, nameddir>>> = OnceLock::new();

/* Create new hash table for named directories */                          // c:59

/// Port of `createnameddirtable()` from `Src/hashnameddir.c:59`.
/// C builds a `HashTable`, wires 12 callbacks, then resets
/// `allusersadded` and clears the `finddir()` cache.
pub fn createnameddirtable() {                                             // c:59
    // c:59 — `nameddirtab = newhashtable(201, "nameddirtab", NULL);`
    // OnceLock-backed HashMap is initialised lazily; touch it here so
    // first-access timing matches the eager C allocation.
    let _ = nameddirtab();
    // c:63-74 — assign 12 callback slots. Static-link path: callbacks
    // are the free fns in this module; no vtable to populate.
    allusersadded.store(0, Ordering::Relaxed);                             // c:84
    // c:84 — `finddir(NULL);` clear the finddir cache. The Rust
    // `finddir` port has no cache, so the call is a no-op here.
}

// != 0 if all the usernames have already been *
// added to the named directory hash table.    *                           // c:59-51
#[allow(non_upper_case_globals)]
pub static allusersadded: AtomicI32 = AtomicI32::new(0);                   // c:59

/* Empty the named directories table */                                    // c:84

/// Port of `emptynameddirtable(HashTable ht)` from `Src/hashnameddir.c:84`.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptynameddirtable() {                                              // c:84
    if let Ok(mut t) = nameddirtab().lock() {                              // c:84
        t.clear();                                                         // c:86 emptyhashtable
    }
    allusersadded.store(0, Ordering::Relaxed);                             // c:96
    // c:96 — `finddir(NULL);` clear the finddir cache (no-op).
}

/* Add all the usernames in the password file/database *
 * to the named directories table.                     */                  // c:96-92

/// Port of `fillnameddirtable(UNUSED(HashTable ht))` from `Src/hashnameddir.c:96`.
/// C signature is `static void fillnameddirtable(UNUSED(HashTable ht))`;
/// Rust drops the unused parameter since `nameddirtab` is the only
/// table this is wired to.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn fillnameddirtable() {                                               // c:96
    if allusersadded.load(Ordering::Relaxed) != 0 {                        // c:96
        return;
    }
    // c:99-110 — `#ifdef USE_GETPWENT` block.
    #[cfg(unix)]
    unsafe {
        libc::setpwent();                                                  // c:102
        // c:106 — `while ((pw = getpwent()) && !errflag)`
        loop {
            let pw = libc::getpwent();
            if pw.is_null() {
                break;
            }
            if crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned();
            let dir = std::ffi::CStr::from_ptr((*pw).pw_dir)
                .to_string_lossy()
                .into_owned();
            // c:107 — `adduserdir(pw->pw_name, pw->pw_dir, ND_USERNAME, 1);`
            crate::ported::utils::adduserdir(&name, &dir, ND_USERNAME, true);
        }
        libc::endpwent();                                                  // c:109
    }
    allusersadded.store(1, Ordering::Relaxed);                             // c:111
}

/* Add an entry to the named directory hash *
 * table, clearing the finddir() cache and  *
 * initialising the `diff' member.          */                             // c:121-117

/// Port of `addnameddirnode(HashTable ht, char *nam, void *nodeptr)` from `Src/hashnameddir.c:121`.
/// C: `static void addnameddirnode(HashTable ht, char *nam, void *nodeptr)`.
/// Caller constructs the `nameddir` (with `dir` + `flags` already
/// set); this fn computes `diff`, clears the finddir cache, then
/// installs the entry. Rust drops the unused `HashTable ht` since
/// `nameddirtab` is the only target.
/// WARNING: param names don't match C — Rust=(nam, nd) vs C=(ht, nam, nodeptr)
pub fn addnameddirnode(nam: &str, mut nd: nameddir) {                      // c:121
    // c:121 — `nd->diff = strlen(nd->dir) - strlen(nam);`
    nd.diff = nd.dir.len() as i32 - nam.len() as i32;
    // c:126 — `finddir(NULL);` clear cache (no-op in Rust port).
    // c:127 — `addhashnode(ht, nam, nodeptr);`
    if let Ok(mut t) = nameddirtab().lock() {
        nd.node.nam = nam.to_string();
        t.insert(nam.to_string(), nd);
    }
}

/* Remove an entry from the named directory  *
 * hash table, clearing the finddir() cache. */                            // c:135-131

/// Port of `removenameddirnode(HashTable ht, const char *nam)` from `Src/hashnameddir.c:135`.
/// C: `static HashNode removenameddirnode(HashTable ht, const char *nam)`.
/// WARNING: param names don't match C — Rust=(nam) vs C=(ht, nam)
pub fn removenameddirnode(nam: &str) -> Option<nameddir> {                 // c:135
    // c:135 — `HashNode hn = removehashnode(ht, nam);`
    let removed = nameddirtab().lock().ok().and_then(|mut t| t.remove(nam));
    if removed.is_some() {                                                 // c:148
        // c:148 — `finddir(NULL);` clear cache (no-op in Rust port).
    }
    removed                                                                // c:148
}

/* Free up the memory used by a named directory hash node. */              // c:148

/// Port of `freenameddirnode(HashNode hn)` from `Src/hashnameddir.c:148`.
/// C frees the two embedded `char*`s plus the struct; in Rust the
/// `Drop` impl for `nameddir` (which owns its `String`s) covers
/// the same teardown.
pub fn freenameddirnode(hn: nameddir) {                                   // c:148
    // c:161-154 — `zsfree(nd->node.nam); zsfree(nd->dir); zfree(nd, …);`
    // Rust drop covers all three.
}

/* Print a named directory */                                              // c:161

/// Port of `printnameddirnode(HashNode hn, int printflags)` from `Src/hashnameddir.c:161`.
pub fn printnameddirnode(hn: &nameddir, printflags: i32) {                 // c:161
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if (printflags & PRINT_NAMEONLY) != 0 {                                // c:165
        let _ = writeln!(out, "{}", hn.node.nam);                          // c:166-168
        return;
    }
    if (printflags & PRINT_LIST) != 0 {                                    // c:171
        let _ = write!(out, "hash -d ");                                   // c:172
        if hn.node.nam.starts_with('-') {                                  // c:174
            let _ = write!(out, "-- ");                                    // c:175
        }
    }
    let _ = write!(
        out,
        "{}={}",
        crate::ported::utils::quotedzputs(&hn.node.nam),                   // c:178
        crate::ported::utils::quotedzputs(&hn.dir),                        // c:180
    );
    let _ = writeln!(out);                                                 // c:181
}

/// Accessor for the global `nameddirtab`. Mirrors the C global
/// dereference (`nameddirtab->...`) by returning the underlying
/// mutex; callers `.lock()` and operate on the map directly.
#[allow(non_snake_case)]
pub fn nameddirtab() -> &'static Mutex<HashMap<String, nameddir>> {        // c:48
    NAMEDDIRTAB_INNER.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::{hashnode, nameddir};

    fn fresh_table() {
        if let Ok(mut t) = nameddirtab().lock() {
            t.clear();
        }
        allusersadded.store(0, Ordering::Relaxed);
    }

    fn make_nd(name: &str, dir: &str, flags: i32) -> nameddir {
        nameddir {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags,
            },
            dir: dir.to_string(),
            diff: 0,
        }
    }

    #[test]
    fn addnameddirnode_sets_diff_and_inserts() {
        // c:125 — `nd->diff = strlen(nd->dir) - strlen(nam);`
        fresh_table();
        addnameddirnode("p", make_nd("p", "/home/user/projects", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("p").expect("entry inserted");
        assert_eq!(nd.dir, "/home/user/projects");
        // strlen("/home/user/projects") - strlen("p") == 19 - 1 == 18.
        assert_eq!(nd.diff, 18);
        assert_eq!(nd.node.nam, "p");
    }

    #[test]
    fn removenameddirnode_returns_node_and_clears_entry() {
        // c:137-141 — removehashnode + finddir(NULL).
        fresh_table();
        addnameddirnode("k", make_nd("k", "/tmp/k", 0));
        assert!(nameddirtab().lock().unwrap().contains_key("k"));
        let dropped = removenameddirnode("k");
        assert!(dropped.is_some());
        assert_eq!(dropped.unwrap().dir, "/tmp/k");
        assert!(!nameddirtab().lock().unwrap().contains_key("k"));
    }

    #[test]
    fn removenameddirnode_missing_returns_none() {
        // c:139 — `if(hn) finddir(NULL);` only fires when a node
        // was actually present; the function still returns NULL
        // when it wasn't.
        fresh_table();
        assert!(removenameddirnode("absent").is_none());
    }

    #[test]
    fn emptynameddirtable_clears_and_resets_allusersadded() {
        // c:86-87 — emptyhashtable + allusersadded = 0.
        fresh_table();
        addnameddirnode("a", make_nd("a", "/a", 0));
        addnameddirnode("b", make_nd("b", "/b", 0));
        allusersadded.store(1, Ordering::Relaxed);
        emptynameddirtable();
        assert!(nameddirtab().lock().unwrap().is_empty());
        assert_eq!(allusersadded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn createnameddirtable_resets_allusersadded() {
        // c:76 — `allusersadded = 0;`
        allusersadded.store(1, Ordering::Relaxed);
        createnameddirtable();
        assert_eq!(allusersadded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn nd_username_value_matches_zsh_h() {
        // Src/zsh.h:2157 — `#define ND_USERNAME (1<<1)`.
        assert_eq!(ND_USERNAME, 1 << 1);
    }
}
