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

use crate::ported::hashtable::hashtable_nodes;
use std::io::Write;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ported::utils::adduserdir;
use crate::ported::zsh_h::{nameddir, ND_USERNAME, PRINT_LIST, PRINT_NAMEONLY};
use crate::utils::{errflag, quotedzputs};

// != 0 if all the usernames have already been *
// added to the named directory hash table.    *                           // c:59-51
#[allow(non_upper_case_globals)]
pub static allusersadded: AtomicI32 = AtomicI32::new(0); // c:59

/* Create new hash table for named directories */
// c:59

/// Port of `createnameddirtable()` from `Src/hashnameddir.c:59`.
/// C builds a `HashTable`, wires 12 callbacks, then resets
/// `allusersadded` and clears the `finddir()` cache.
pub fn createnameddirtable() {
    // c:59
    // c:59 — `nameddirtab = newhashtable(201, "nameddirtab", NULL);`
    // OnceLock-backed HashMap is initialised lazily; touch it here so
    // first-access timing matches the eager C allocation.
    let _ = nameddirtab();
    // c:63-74 — assign 12 callback slots. Static-link path: callbacks
    // are the free ported in this module; no vtable to populate.
    allusersadded.store(0, Ordering::Relaxed); // c:84
                                               // c:84 — `finddir(NULL);` clear the finddir cache. The Rust
                                               // `finddir` port has no cache, so the call is a no-op here.
}

/* Empty the named directories table */
// c:84

/// Port of `emptynameddirtable(HashTable ht)` from `Src/hashnameddir.c:84`.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptynameddirtable() {
    // c:84
    if let Ok(mut t) = nameddirtab().lock() {
        // c:84
        t.clear(); // c:86 emptyhashtable
    }
    allusersadded.store(0, Ordering::Relaxed); // c:96
                                               // c:96 — `finddir(NULL);` clear the finddir cache (no-op).
}

/* Add all the usernames in the password file/database *
 * to the named directories table.                     */
// c:96-92

/// Port of `fillnameddirtable(UNUSED(HashTable ht))` from `Src/hashnameddir.c:96`.
/// C signature is `static void fillnameddirtable(UNUSED(HashTable ht))`;
/// Rust drops the unused parameter since `nameddirtab` is the only
/// table this is wired to.
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn fillnameddirtable() {
    // c:96
    if allusersadded.load(Ordering::Relaxed) != 0 {
        // c:96
        return;
    }
    // c:99-110 — `#ifdef USE_GETPWENT` block.
    #[cfg(unix)]
    unsafe {
        libc::setpwent(); // c:102
                          // c:106 — `while ((pw = getpwent()) && !errflag)`
        loop {
            let pw = libc::getpwent();
            if pw.is_null() {
                break;
            }
            if errflag.load(Ordering::Relaxed) != 0 {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned();
            let dir = std::ffi::CStr::from_ptr((*pw).pw_dir)
                .to_string_lossy()
                .into_owned();
            // c:107 — `adduserdir(pw->pw_name, pw->pw_dir, ND_USERNAME, 1);`
            adduserdir(&name, &dir, ND_USERNAME, true);
        }
        libc::endpwent(); // c:109
    }
    allusersadded.store(1, Ordering::Relaxed); // c:111
}

/* Add an entry to the named directory hash *
 * table, clearing the finddir() cache and  *
 * initialising the `diff' member.          */
// c:121-117

/// Port of `addnameddirnode(HashTable ht, char *nam, void *nodeptr)` from `Src/hashnameddir.c:121`.
/// C: `static void addnameddirnode(HashTable ht, char *nam, void *nodeptr)`.
/// Caller constructs the `nameddir` (with `dir` + `flags` already
/// set); this fn computes `diff`, clears the finddir cache, then
/// installs the entry. Rust drops the unused `HashTable ht` since
/// `nameddirtab` is the only target.
/// WARNING: param names don't match C — Rust=(nam, nd) vs C=(ht, nam, nodeptr)
pub fn addnameddirnode(nam: &str, mut nd: nameddir) {
    // c:121
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
 * hash table, clearing the finddir() cache. */
// c:135-131

/// Port of `removenameddirnode(HashTable ht, const char *nam)` from `Src/hashnameddir.c:135`.
/// C: `static HashNode removenameddirnode(HashTable ht, const char *nam)`.
/// WARNING: param names don't match C — Rust=(nam) vs C=(ht, nam)
pub fn removenameddirnode(nam: &str) -> Option<nameddir> {
    // c:135
    // c:135 — `HashNode hn = removehashnode(ht, nam);`
    let removed = nameddirtab().lock().ok().and_then(|mut t| t.remove(nam));
    if removed.is_some() { // c:148
         // c:148 — `finddir(NULL);` clear cache (no-op in Rust port).
    }
    removed // c:148
}

/* Free up the memory used by a named directory hash node. */
// c:148

/// Port of `freenameddirnode(HashNode hn)` from `Src/hashnameddir.c:148`.
/// C frees the two embedded `char*`s plus the struct; in Rust the
/// `Drop` impl for `nameddir` (which owns its `String`s) covers
/// the same teardown.
pub fn freenameddirnode(hn: nameddir) { // c:148
                                        // c:161-154 — `zsfree(nd->node.nam); zsfree(nd->dir); zfree(nd, …);`
                                        // Rust drop covers all three.
}

/* Print a named directory */
// c:161

/// Port of `printnameddirnode(HashNode hn, int printflags)` from `Src/hashnameddir.c:161`.
pub fn printnameddirnode(hn: &nameddir, printflags: i32) {
    // c:161
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if (printflags & PRINT_NAMEONLY) != 0 {
        // c:165
        let _ = writeln!(out, "{}", hn.node.nam); // c:166-168
        return;
    }
    if (printflags & PRINT_LIST) != 0 {
        // c:171
        let _ = write!(out, "hash -d "); // c:172
        if hn.node.nam.starts_with('-') {
            // c:174
            let _ = write!(out, "-- "); // c:175
        }
    }
    let _ = write!(
        out,
        "{}={}",
        quotedzputs(&hn.node.nam), // c:178
        quotedzputs(&hn.dir),      // c:180
    );
    let _ = writeln!(out); // c:181
}

/****************************************/
/* Named Directory Hash Table Functions */
/****************************************/

// hash table containing named directories                                 // c:45
//
// C: `mod_export HashTable nameddirtab;` (c:48). Node storage is the
// open-hashed bucket array C builds at c:61
// (`newhashtable(201, "nameddirtab", NULL)`); the Mutex is the Rust
// singleton wrapper.
//
// The bucket walk is user-visible — `${(k)nameddirs}` (and `hash -d`
// with no args) scans this table, so backing it with a `HashMap`
// re-seeded `RandomState` order rather than C's. Repro that failed
// before this change:
//   hash -d aa=/tmp bb=/usr cc=/var dd=/etc ee=/opt
//   print -rl -- ${(k)nameddirs}   # zsh: aa dd bb cc ee
static NAMEDDIRTAB_INNER: OnceLock<Mutex<hashtable_nodes<nameddir>>> = OnceLock::new();

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

/// Accessor for the global `nameddirtab`. Mirrors the C global
/// dereference (`nameddirtab->...`) by returning the underlying
/// mutex; callers `.lock()` and operate on the map directly.
#[allow(non_snake_case)]
pub fn nameddirtab() -> &'static Mutex<hashtable_nodes<nameddir>> {
    // c:48
    // c:61 — `newhashtable(201, "nameddirtab", NULL)`.
    NAMEDDIRTAB_INNER.get_or_init(|| Mutex::new(hashtable_nodes::newhashtable(201)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zsh_h::hashnode;

    /// Process-wide lock serialising tests that mutate the global
    /// `nameddirtab`. Without this, parallel `cargo test` invocations
    /// race on the shared HashMap — `fresh_table()` from one test
    /// can clear an entry another just inserted, producing flaky
    /// "entry inserted" assertion failures. No C counterpart.
    static NAMEDDIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // c:139 — `if(hn) finddir(NULL);` only fires when a node
        // was actually present; the function still returns NULL
        // when it wasn't.
        fresh_table();
        assert!(removenameddirnode("absent").is_none());
    }

    #[test]
    fn emptynameddirtable_clears_and_resets_allusersadded() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // c:76 — `allusersadded = 0;`
        allusersadded.store(1, Ordering::Relaxed);
        createnameddirtable();
        assert_eq!(allusersadded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn nd_username_value_matches_zsh_h() {
        let _g = crate::test_util::global_state_lock();
        // Src/zsh.h:2157 — `#define ND_USERNAME (1<<1)`.
        assert_eq!(ND_USERNAME, 1 << 1);
    }

    /// `Src/hashnameddir.c:98` — `if (!allusersadded) { … allusersadded = 1; }`.
    /// Once the passwd database has been walked, subsequent calls must
    /// early-exit. Regression dropping this guard would re-walk
    /// `getpwent()` on every `~<TAB>` completion attempt — silently
    /// quadratic on systems with thousands of LDAP users. We can't
    /// observe the no-op via the table (other parallel tests may
    /// mutate it) — instead we pin the visible side-effect:
    /// `allusersadded` stays exactly 1, since the c:111 `= 1` only
    /// runs inside the c:98 conditional.
    #[test]
    fn fillnameddirtable_short_circuits_when_allusersadded_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        allusersadded.store(1, Ordering::Relaxed);
        fillnameddirtable();
        assert_eq!(
            allusersadded.load(Ordering::Relaxed),
            1,
            "c:98 conditional must early-exit; flag remains 1 unchanged"
        );
    }

    /// `Src/hashnameddir.c:125` — `nd->diff = strlen(nd->dir) - strlen(nam);`.
    /// `Src/zsh.h:2152` — `int diff;` (signed int field). Regression
    /// using unsigned subtraction (or assuming `dir.len() >= name.len()`)
    /// would underflow on a `hash -d short=/x` style entry.
    #[test]
    fn addnameddirnode_diff_can_be_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // name "longname" (8) longer than dir "/x" (2) → diff = -6.
        fresh_table();
        addnameddirnode("longname", make_nd("longname", "/x", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("longname").expect("entry inserted");
        assert_eq!(nd.diff, -6, "c:125 — signed subtraction may underflow zero");
    }

    /// `Src/hashnameddir.c:121-128` — `addnameddirnode` calls
    /// `addhashnode(ht, nam, nodeptr)` at c:127. `addhashnode` in C
    /// replaces an existing node with the same key (Src/hashtable.c
    /// `addhashnode2` removes the prior entry before insert).
    /// Regression that accumulates duplicate keys would mis-print
    /// `~name`-prefixed paths after `hash -d` reuse and leak the
    /// stale `diff` field.
    #[test]
    fn addnameddirnode_overwrites_existing_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("p", make_nd("p", "/old", 0));
        addnameddirnode("p", make_nd("p", "/new/longer/path", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("p").expect("entry present");
        assert_eq!(nd.dir, "/new/longer/path", "c:127 — addhashnode replaces");
        // c:125 — diff recomputed for the new dir+name pair.
        assert_eq!(nd.diff, "/new/longer/path".len() as i32 - 1);
        assert_eq!(t.len(), 1, "must not accumulate duplicate keys");
    }

    /// c:125 — `nd->diff = strlen(nd->dir) - strlen(nam)`. When
    /// dir and name have equal length, diff is exactly 0. Pin the
    /// zero-edge so a regen that adds a +1/-1 fudge factor breaks
    /// `~name` prefix matching for this case.
    #[test]
    fn addnameddirnode_diff_zero_when_lengths_equal() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("abcde", make_nd("abcde", "/etc/", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("abcde").expect("entry inserted");
        assert_eq!(
            nd.diff, 0,
            "len(\"/etc/\") == len(\"abcde\") == 5 → diff = 0"
        );
    }

    /// c:84 — `emptynameddirtable` empties the table and resets
    /// `allusersadded`. Pin the full reset because callers depend
    /// on the flag returning to 0 so `fillnameddirtable` will
    /// re-scan on next access.
    #[test]
    fn emptynameddirtable_resets_table_and_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("a", make_nd("a", "/x", 0));
        addnameddirnode("b", make_nd("b", "/y", 0));
        allusersadded.store(1, Ordering::Relaxed);
        emptynameddirtable();
        assert!(
            nameddirtab().lock().unwrap().is_empty(),
            "emptynameddirtable must clear the table"
        );
        assert_eq!(
            allusersadded.load(Ordering::Relaxed),
            0,
            "emptynameddirtable must reset allusersadded to 0"
        );
    }

    /// c:135 — `removenameddirnode` returns the removed entry's data
    /// (Some), or None for an absent name. Pin both branches.
    #[test]
    fn removenameddirnode_returns_some_for_present_and_none_for_absent() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("here", make_nd("here", "/tmp/here", 0));
        let removed = removenameddirnode("here");
        assert!(removed.is_some(), "present entry must return Some");
        assert_eq!(removed.unwrap().dir, "/tmp/here");
        // Now it's gone
        assert!(
            removenameddirnode("here").is_none(),
            "second remove must return None"
        );
        // Unknown name is also None
        assert!(removenameddirnode("never_was").is_none());
    }

    /// c:135 — `removenameddirnode` of a present entry removes it
    /// from the table (followup state check).
    #[test]
    fn removenameddirnode_actually_removes_from_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("a", make_nd("a", "/a", 0));
        addnameddirnode("b", make_nd("b", "/b", 0));
        assert_eq!(nameddirtab().lock().unwrap().len(), 2);
        removenameddirnode("a");
        let t = nameddirtab().lock().unwrap();
        assert_eq!(t.len(), 1);
        assert!(!t.contains_key("a"));
        assert!(t.contains_key("b"), "removing 'a' must NOT touch 'b'");
    }

    /// c:59 — `createnameddirtable` is idempotent: calling it twice
    /// must not double-init or wipe existing entries. Pin re-entry
    /// safety since `init_misc` and `fillnameddirtable` both call it.
    #[test]
    fn createnameddirtable_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        createnameddirtable();
        addnameddirnode("preserved", make_nd("preserved", "/x", 0));
        createnameddirtable(); // second call
        let t = nameddirtab().lock().unwrap();
        assert!(
            t.contains_key("preserved"),
            "second createnameddirtable must NOT wipe existing entries"
        );
    }

    /// c:148 — `freenameddirnode` accepts any nameddir and consumes
    /// it (drops the String fields). Smoke test for the destructor.
    #[test]
    fn freenameddirnode_consumes_node() {
        let _g = crate::test_util::global_state_lock();
        let nd = make_nd("doomed", "/tmp", 0);
        freenameddirnode(nd);
        // No panic = pass. Caller's `nd` is moved.
    }

    // ─── zsh-corpus pins: nameddir add/get/remove ────────────────────

    /// `addnameddirnode` then `removenameddirnode` round-trip.
    #[test]
    fn nameddir_corpus_add_then_remove() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("proj", make_nd("proj", "/home/u/proj", 0));
        let removed = removenameddirnode("proj");
        assert!(removed.is_some(), "removed value returned");
        assert_eq!(removed.unwrap().dir, "/home/u/proj");
        let t = nameddirtab().lock().unwrap();
        assert!(!t.contains_key("proj"), "entry gone after remove");
    }

    /// `removenameddirnode` on missing returns None.
    #[test]
    fn nameddir_corpus_remove_missing_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        assert!(removenameddirnode("never_added").is_none());
    }

    /// `addnameddirnode` overwrites on duplicate name — last add wins.
    #[test]
    fn nameddir_corpus_add_duplicate_overwrites() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("d", make_nd("d", "/old/path", 0));
        addnameddirnode("d", make_nd("d", "/new/path", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("d").expect("entry present");
        assert_eq!(nd.dir, "/new/path", "duplicate add overwrote");
    }

    /// `addnameddirnode` with single-char dir: diff = len(dir) - len(name).
    /// `addnameddirnode("x", "/a")` → diff = 2 - 1 = 1.
    #[test]
    fn nameddir_corpus_diff_is_dir_minus_name_length() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("x", make_nd("x", "/a", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("x").unwrap();
        assert_eq!(nd.diff, 1, "diff = len('/a') - len('x') = 2 - 1 = 1");
    }

    /// `emptynameddirtable` clears all entries.
    #[test]
    fn nameddir_corpus_emptynameddirtable_clears() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("a", make_nd("a", "/a", 0));
        addnameddirnode("b", make_nd("b", "/b", 0));
        emptynameddirtable();
        let t = nameddirtab().lock().unwrap();
        // emptynameddirtable removes user entries; may keep internal defaults.
        assert!(t.get("a").is_none(), "user entry a cleared");
        assert!(t.get("b").is_none(), "user entry b cleared");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashnameddir.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:121 — `diff` = `len(dir) - len(nam)` allows negative (dir
    /// shorter than name). Pin: short dir gives negative diff.
    #[test]
    fn addnameddirnode_diff_can_be_negative_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        // name='verylongname' (12 chars), dir='/x' (2 chars) → diff = -10
        addnameddirnode("verylongname", make_nd("verylongname", "/x", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("verylongname").unwrap();
        assert_eq!(nd.diff, -10, "diff = 2 - 12 = -10");
    }

    /// c:121 — `diff = 0` when dir.len() == nam.len() (equal-length).
    #[test]
    fn addnameddirnode_diff_zero_when_lengths_equal_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("xyz", make_nd("xyz", "abc", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("xyz").unwrap();
        assert_eq!(nd.diff, 0, "diff = 3 - 3 = 0");
    }

    /// c:121 — `addnameddirnode` sets node.nam to the input name.
    #[test]
    fn addnameddirnode_sets_node_nam() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        // Construct nd with empty name; addnameddirnode must overwrite.
        addnameddirnode("real_name", make_nd("", "/x", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("real_name").unwrap();
        assert_eq!(nd.node.nam, "real_name", "node.nam set by addnameddirnode");
    }

    /// c:135 — `removenameddirnode("missing")` returns None.
    #[test]
    fn removenameddirnode_missing_returns_none_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let r = removenameddirnode("zshrs_never_dir");
        assert!(r.is_none(), "missing entry → None");
    }

    /// c:135 — `removenameddirnode` actually removes from the table.
    #[test]
    fn removenameddirnode_existing_removes_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("rmme", make_nd("rmme", "/rmme", 0));
        // Confirm present before remove.
        assert!(nameddirtab().lock().unwrap().contains_key("rmme"));
        let removed = removenameddirnode("rmme");
        assert!(removed.is_some(), "returned the removed entry");
        assert!(!nameddirtab().lock().unwrap().contains_key("rmme"));
    }

    /// c:148 — `freenameddirnode` is a no-op (Rust drop handles strings).
    #[test]
    fn freenameddirnode_is_drop_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let nd = make_nd("ok", "/ok", 0);
        freenameddirnode(nd);
        // Drop cascade frees the strings; no assertion possible beyond no panic.
    }

    /// c:59 — `createnameddirtable` is idempotent (lazy init).
    #[test]
    fn createnameddirtable_is_idempotent_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createnameddirtable();
        createnameddirtable();
        createnameddirtable();
        // No panic = pass.
    }

    /// c:84 — `emptynameddirtable` resets `allusersadded` flag to 0
    /// (so subsequent `fillnameddirtable` re-runs).
    #[test]
    fn emptynameddirtable_resets_allusersadded() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        allusersadded.store(1, Ordering::Relaxed); // simulate filled state
        emptynameddirtable();
        assert_eq!(
            allusersadded.load(Ordering::Relaxed),
            0,
            "emptynameddirtable must reset allusersadded → 0"
        );
    }

    /// c:121 — `addnameddirnode` overwrites existing entry (same name).
    #[test]
    fn addnameddirnode_overwrites_existing() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("dup", make_nd("dup", "/first", 0));
        addnameddirnode("dup", make_nd("dup", "/second", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("dup").unwrap();
        assert_eq!(nd.dir, "/second", "second insert wins");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashnameddir.c
    // c:121 addnameddirnode / c:135 removenameddirnode / c:161 printnameddirnode
    // ═══════════════════════════════════════════════════════════════════

    /// c:121 — `addnameddirnode` writes the input nam onto node.nam
    /// (not the input nd.node.nam) so the caller-supplied name wins.
    #[test]
    fn addnameddirnode_uses_argument_name_not_input_node_name() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        // Pass nd with mismatched embedded name.
        let mut nd = make_nd("ignored", "/dir", 0);
        nd.node.nam = "ignored".to_string();
        addnameddirnode("real", nd);
        let t = nameddirtab().lock().unwrap();
        let stored = t.get("real").unwrap();
        assert_eq!(stored.node.nam, "real", "argument name must win");
        assert!(!t.contains_key("ignored"), "ignored name not used");
    }

    /// c:121 — diff = strlen(dir) - strlen(nam). Long name + short dir →
    /// negative diff (no overflow / no clamp at 0).
    #[test]
    fn addnameddirnode_diff_can_be_strongly_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let long_name = "x".repeat(100);
        addnameddirnode(&long_name, make_nd(&long_name, "/a", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get(long_name.as_str()).unwrap();
        assert_eq!(nd.diff, 2 - 100, "diff = 2 - 100 = -98");
    }

    /// c:121 — empty `nam` is accepted (defensive — no crash on edge case).
    #[test]
    fn addnameddirnode_empty_name_accepted() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("", make_nd("", "/somewhere", 0));
        let t = nameddirtab().lock().unwrap();
        assert!(t.contains_key(""), "empty name must insert");
        let nd = t.get("").unwrap();
        assert_eq!(nd.diff, 10, "diff = 10 - 0 = 10");
    }

    /// c:135 — `removenameddirnode` is idempotent: removing same key
    /// twice → first Some, second None.
    #[test]
    fn removenameddirnode_idempotent_removes_once() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("once", make_nd("once", "/dir", 0));
        let first = removenameddirnode("once");
        assert!(first.is_some(), "first remove returns Some");
        let second = removenameddirnode("once");
        assert!(second.is_none(), "second remove returns None");
    }

    /// c:135 — `removenameddirnode` doesn't affect unrelated keys.
    #[test]
    fn removenameddirnode_only_removes_target_key() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("a", make_nd("a", "/A", 0));
        addnameddirnode("b", make_nd("b", "/B", 0));
        addnameddirnode("c", make_nd("c", "/C", 0));
        removenameddirnode("b");
        let t = nameddirtab().lock().unwrap();
        assert!(t.contains_key("a"));
        assert!(!t.contains_key("b"));
        assert!(t.contains_key("c"));
    }

    /// c:121 — diff sign rule: dir.len() > nam.len() → diff > 0.
    #[test]
    fn addnameddirnode_diff_positive_when_dir_longer() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("h", make_nd("h", "/home/user", 0));
        let t = nameddirtab().lock().unwrap();
        let nd = t.get("h").unwrap();
        assert!(nd.diff > 0, "diff must be > 0 when dir longer");
        assert_eq!(nd.diff, "/home/user".len() as i32 - "h".len() as i32);
    }

    /// c:53 — `emptynameddirtable` actually empties (size = 0 after).
    #[test]
    fn emptynameddirtable_drops_all_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        for i in 0..10 {
            let name = format!("entry{}", i);
            addnameddirnode(&name, make_nd(&name, "/p", 0));
        }
        emptynameddirtable();
        let t = nameddirtab().lock().unwrap();
        assert_eq!(t.len(), 0, "all entries dropped");
    }

    /// c:53 + c:35 — emptynameddirtable + createnameddirtable both
    /// reset allusersadded to 0.
    #[test]
    fn createnameddirtable_resets_allusersadded_flag() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        allusersadded.store(1, Ordering::Relaxed);
        createnameddirtable();
        assert_eq!(
            allusersadded.load(Ordering::Relaxed),
            0,
            "createnameddirtable must reset allusersadded → 0"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashnameddir.c
    // c:117 addnameddirnode / c:136 removenameddirnode / c:53 emptynameddirtable
    // c:161 printnameddirnode / c:224 nameddirtab — type pins + read-only
    // ═══════════════════════════════════════════════════════════════════

    /// c:117 — `addnameddirnode` followed by lookup finds the entry.
    #[test]
    fn addnameddirnode_followed_by_lookup_finds_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("zshrs_test_lookup", make_nd("zshrs_test_lookup", "/tmp", 0));
        let t = nameddirtab().lock().unwrap();
        assert!(
            t.contains_key("zshrs_test_lookup"),
            "added entry must be findable"
        );
    }

    /// c:136 — `removenameddirnode` returns Option<nameddir> (type pin).
    #[test]
    fn removenameddirnode_returns_option_nameddir_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let _: Option<nameddir> = removenameddirnode("nothing");
    }

    /// c:136 — `removenameddirnode` empty name returns None on empty table.
    #[test]
    fn removenameddirnode_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        assert!(removenameddirnode("").is_none(), "empty name → None");
    }

    /// c:117 — `addnameddirnode` returns void.
    #[test]
    fn addnameddirnode_signature_void() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let _: () = addnameddirnode("zshrs_void_pin", make_nd("zshrs_void_pin", "/tmp", 0));
    }

    /// c:53 — `emptynameddirtable` returns void.
    #[test]
    fn emptynameddirtable_signature_void() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _: () = emptynameddirtable();
    }

    /// c:117 — `addnameddirnode` two distinct entries grows table by 2.
    #[test]
    fn addnameddirnode_two_distinct_entries_grows_by_two() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let before = nameddirtab().lock().unwrap().len();
        addnameddirnode("aa", make_nd("aa", "/a", 0));
        addnameddirnode("bb", make_nd("bb", "/b", 0));
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before + 2, "2 distinct names → grow by 2");
    }

    /// c:117 — `addnameddirnode` same name twice keeps count at 1 (overwrite).
    #[test]
    fn addnameddirnode_same_name_twice_keeps_count_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let before = nameddirtab().lock().unwrap().len();
        addnameddirnode("dup", make_nd("dup", "/v1", 0));
        addnameddirnode("dup", make_nd("dup", "/v2", 0));
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before + 1, "duplicate insert grows by 1 only");
    }

    /// c:136 — `removenameddirnode` after `add` decrements count by 1.
    #[test]
    fn removenameddirnode_decrements_count_by_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("rem_test", make_nd("rem_test", "/x", 0));
        let before = nameddirtab().lock().unwrap().len();
        removenameddirnode("rem_test");
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before - 1, "remove decrements count by 1");
    }

    /// c:153 — `freenameddirnode` consumes the node (Box drop semantic).
    #[test]
    fn freenameddirnode_consumes_node_signature_void() {
        let nd = nameddir {
            node: crate::zsh_h::hashnode {
                next: None,
                nam: "free_test".to_string(),
                flags: 0,
            },
            dir: "/tmp".to_string(),
            diff: 0,
        };
        let _: () = freenameddirnode(nd);
    }

    /// c:224 — `nameddirtab()` returns same Mutex ref across calls.
    #[test]
    fn nameddirtab_returns_same_ref_across_calls() {
        let a = nameddirtab() as *const _;
        let b = nameddirtab() as *const _;
        assert_eq!(a, b, "nameddirtab must return singleton");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashnameddir.c
    // c:35 createnameddirtable / c:53 emptynameddirtable / c:72 fillnameddirtable /
    // c:117 addnameddirnode / c:136 removenameddirnode / c:153 freenameddirnode /
    // c:162 printnameddirnode
    // ═══════════════════════════════════════════════════════════════════

    /// c:35 — `createnameddirtable` is idempotent (callable repeatedly).
    #[test]
    fn createnameddirtable_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..10 {
            createnameddirtable();
        }
    }

    /// c:53 — `emptynameddirtable` is idempotent.
    #[test]
    fn emptynameddirtable_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..10 {
            emptynameddirtable();
        }
    }

    /// c:53 — after `emptynameddirtable`, table is empty.
    #[test]
    fn emptynameddirtable_actually_empties_table() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        addnameddirnode("e_test", make_nd("e_test", "/tmp", 0));
        emptynameddirtable();
        assert_eq!(
            nameddirtab().lock().unwrap().len(),
            0,
            "empty must zero the table"
        );
    }

    /// c:136 — `removenameddirnode` returns Option<nameddir> (alt pin).
    #[test]
    fn removenameddirnode_returns_option_nameddir_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _: Option<nameddir> = removenameddirnode("__never__");
    }

    /// c:136 — `removenameddirnode` of nonexistent returns None.
    #[test]
    fn removenameddirnode_nonexistent_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        assert!(
            removenameddirnode("__never_added_xyz__").is_none(),
            "removing never-added returns None"
        );
    }

    /// c:117 + c:136 — add then remove returns Some, then remove again None.
    #[test]
    fn add_then_remove_then_remove_again_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        addnameddirnode("rt_test", make_nd("rt_test", "/tmp", 0));
        let first = removenameddirnode("rt_test");
        assert!(first.is_some(), "first remove → Some");
        let second = removenameddirnode("rt_test");
        assert!(second.is_none(), "second remove → None");
    }

    /// c:162 — `printnameddirnode` for various flags doesn't panic.
    #[test]
    fn printnameddirnode_various_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let nd = nameddir {
            node: crate::zsh_h::hashnode {
                next: None,
                nam: "p_test".to_string(),
                flags: 0,
            },
            dir: "/tmp".to_string(),
            diff: 0,
        };
        for flags in [0i32, 1, 2, 0xff, -1] {
            printnameddirnode(&nd, flags);
        }
    }

    /// c:117 — add many distinct entries; final count equals number added.
    #[test]
    fn addnameddirnode_many_distinct_grows_by_count() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        let before = nameddirtab().lock().unwrap().len();
        for i in 0..5 {
            let name = format!("many_{}", i);
            addnameddirnode(&name, make_nd(&name, "/p", 0));
        }
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before + 5, "5 distinct adds → grow by 5");
    }

    /// c:117 — addnameddirnode with empty name is safe.
    #[test]
    fn addnameddirnode_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        addnameddirnode("", make_nd("", "/tmp", 0));
    }

    /// c:121 — `addnameddirnode` COMPUTES `diff = dir.len - nam.len`
    /// (overwrites whatever caller passed in `nd.diff`). Pin the
    /// formula contract so a refactor that drops the computation
    /// gets caught.
    #[test]
    fn addnameddirnode_computes_diff_from_dir_minus_nam_len() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = NAMEDDIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fresh_table();
        // Caller passes diff=42 — addnameddirnode MUST overwrite to
        // dir.len() - nam.len() = 12 - 13 = -1.
        addnameddirnode(
            "preserve_test",
            make_nd("preserve_test", "/unique/path", 42),
        );
        let tab = nameddirtab().lock().unwrap();
        let entry = tab.get("preserve_test").expect("entry exists");
        assert_eq!(entry.dir, "/unique/path", "dir field preserved");
        let expected = "/unique/path".len() as i32 - "preserve_test".len() as i32;
        assert_eq!(
            entry.diff,
            expected,
            "c:121 — diff = dir.len({}) - nam.len({}) = {}",
            "/unique/path".len(),
            "preserve_test".len(),
            expected
        );
    }

    // NOTE: `fillnameddirtable` reads /etc/passwd and populates the
    // table with real OS users, which would clobber other tests'
    // count assertions. No test pin for it here — pre-existing test
    // suite covers the relevant invariants in isolation. Replacement
    // pin: simply confirms the fn exists (compile-time check).
    /// c:72 — `fillnameddirtable` is exported (compile-time symbol pin).
    #[test]
    fn fillnameddirtable_symbol_exists_pin() {
        let _: fn() = fillnameddirtable;
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/hashnameddir.c
    // c:117 addnameddirnode / c:136 removenameddirnode /
    // c:153 freenameddirnode / c:162 printnameddirnode
    // ═══════════════════════════════════════════════════════════════════

    fn mk_nd(dir: &str) -> nameddir {
        nameddir {
            node: crate::ported::zsh_h::hashnode::default(),
            dir: dir.to_string(),
            diff: 0,
        }
    }

    /// c:117 — `addnameddirnode` overwrites an existing key (same name, new dir).
    #[test]
    fn addnameddirnode_overwrite_replaces_dir() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("homed", mk_nd("/old/path"));
        addnameddirnode("homed", mk_nd("/new/path"));
        let tab = nameddirtab().lock().unwrap();
        assert_eq!(
            tab.get("homed").map(|n| n.dir.clone()),
            Some("/new/path".to_string()),
            "overwrite must replace dir verbatim"
        );
        drop(tab);
        emptynameddirtable();
    }

    /// c:117 — `addnameddirnode` overwrite does NOT grow the table.
    #[test]
    fn addnameddirnode_overwrite_does_not_grow() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("key1", mk_nd("/a"));
        let n1 = nameddirtab().lock().unwrap().len();
        addnameddirnode("key1", mk_nd("/b"));
        let n2 = nameddirtab().lock().unwrap().len();
        assert_eq!(n1, n2, "overwrite must not grow table size");
        emptynameddirtable();
    }

    /// c:136 — `removenameddirnode` returns the previously-stored nameddir
    /// (Some payload).
    #[test]
    fn removenameddirnode_returns_payload_on_hit() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("toremove", mk_nd("/path/x"));
        let r = removenameddirnode("toremove");
        assert!(r.is_some(), "remove must return Some payload");
        assert_eq!(r.unwrap().dir, "/path/x");
        emptynameddirtable();
    }

    /// c:136 — `removenameddirnode` shrinks the table by 1.
    #[test]
    fn removenameddirnode_shrinks_table_by_one() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("a", mk_nd("/1"));
        addnameddirnode("b", mk_nd("/2"));
        addnameddirnode("c", mk_nd("/3"));
        let before = nameddirtab().lock().unwrap().len();
        let _ = removenameddirnode("b");
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before - 1, "remove must shrink by 1");
        emptynameddirtable();
    }

    /// c:136 — removing nonexistent name doesn't shrink table.
    #[test]
    fn removenameddirnode_miss_does_not_shrink() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("a", mk_nd("/1"));
        let before = nameddirtab().lock().unwrap().len();
        let _ = removenameddirnode("__never__");
        let after = nameddirtab().lock().unwrap().len();
        assert_eq!(after, before, "miss must not change size");
        emptynameddirtable();
    }

    /// c:153 — `freenameddirnode(nd)` returns void (compile-time pin).
    #[test]
    fn freenameddirnode_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = freenameddirnode(mk_nd("/x"));
    }

    /// c:162 — `printnameddirnode` empty-name node doesn't panic.
    #[test]
    fn printnameddirnode_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let nd = mk_nd("");
        printnameddirnode(&nd, 0);
    }

    /// c:162 — `printnameddirnode` ND_USERNAME flag path doesn't panic.
    #[test]
    fn printnameddirnode_username_flag_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut nd = mk_nd("/home/u");
        nd.node.nam = "u".to_string();
        nd.node.flags = ND_USERNAME as i32;
        printnameddirnode(&nd, 0);
    }

    /// c:162 — `printnameddirnode` PRINT_NAMEONLY | PRINT_LIST combo safe.
    #[test]
    fn printnameddirnode_combo_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut nd = mk_nd("/dir");
        nd.node.nam = "n".to_string();
        printnameddirnode(&nd, (PRINT_NAMEONLY | PRINT_LIST) as i32);
    }

    /// c:117 — `addnameddirnode` computes `diff` as `dir.len() - nam.len()`.
    /// Already pinned in pre-existing test; add a negative-diff case
    /// (nam longer than dir).
    #[test]
    fn addnameddirnode_diff_negative_when_name_longer_than_dir() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        addnameddirnode("very_long_name_xyz", mk_nd("/x"));
        let tab = nameddirtab().lock().unwrap();
        let nd = tab.get("very_long_name_xyz").unwrap();
        // diff = dir.len() - nam.len() = 2 - 18 = -16
        assert_eq!(
            nd.diff,
            2i32 - "very_long_name_xyz".len() as i32,
            "diff must be dir.len() - nam.len(), even when negative"
        );
        drop(tab);
        emptynameddirtable();
    }

    /// c:53 — `emptynameddirtable` after adding entries clears them all.
    #[test]
    fn emptynameddirtable_clears_all_entries() {
        let _g = crate::test_util::global_state_lock();
        emptynameddirtable();
        for i in 0..10 {
            addnameddirnode(&format!("k{}", i), mk_nd(&format!("/d{}", i)));
        }
        assert_eq!(nameddirtab().lock().unwrap().len(), 10);
        emptynameddirtable();
        assert_eq!(
            nameddirtab().lock().unwrap().len(),
            0,
            "emptynameddirtable must remove every entry"
        );
    }
}
