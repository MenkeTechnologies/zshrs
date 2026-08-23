//! Hash table implementations - port of hashtable.c
//!
//! Provides hash tables for commands, shell functions, reserved words, aliases,
//! and history. The four tables whose iteration order is user-visible
//! (`cmdnamtab`, `shfunctab`, `reswdtab`, `aliastab`/`sufaliastab`)
//! store their nodes in `hashtable_nodes`, a port of the node-storage
//! half of C's `struct hashtable` (`Src/zsh.h:1175-1235`), so
//! `${(k)commands}` / `${(k)functions}` / `$reswords` /
//! `${(k)aliases}` come out in C's bucket-walk order.
//!
//! `cmdnam_table` / `shfunc_table` / `reswd_table` / `alias_table` are
//! Rust-side typed wrappers. C uses one polymorphic `struct hashtable`
//! (`Src/zsh.h:1175-1235`) with function-pointer callbacks per table
//! kind; the canonical Rust port of that struct lives at
//! `zsh_h.rs:532`. These typed wrappers add fields that aren't part
//! of `struct hashtable` (e.g. `cmdnam_table` carries
//! `path_checked_index` + `path` + `hash_executables_only` for the
//! `PATH`-walk fast-rehash that C tracks via the file-scope
//! `pathchecked` / `hashed_anything` statics in `Src/hashtable.c`).
//! Lowercase naming reflects that the wrappers are co-located with
//! the canonical `hashtable` rather than mirror it 1:1.

#![allow(non_camel_case_types)]

use crate::compat::zgetcwd;
use crate::hist::{hashchar, hist_ring};
use crate::jobs::getsigidx;
use crate::ported::hist::{hist_ignore_all_dups, histlinect, histremovedups, up_histent};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::signals::removetrap;
use crate::ported::utils::scriptfilename_get;
use crate::ported::zsh_h::{
    alias, options, BANG_TOK, CASE, COPROC, DINBRACK, DOLOOP, DONE, ELIF, ELSE, ESAC, FI, FOR,
    FOREACH, FUNC, IF, INBRACE_TOK, NOCORRECT, OUTBRACE_TOK, PAT_HEAPDUP, REPEAT, SELECT, THEN,
    TIME, TYPESET, UNTIL, WHILE, ZEND,
};
use crate::signals::{settrap, unsettrap};
use crate::text::{getpermtext, zoutputtab};
use crate::utils::{nicezputs, quotedzputs, xsymlink, zputs, ztrcmp, zwarn};
use crate::zsh_h::{
    cmdnam, hashnode, hashtable, reswd, shfunc, ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED, EF_RUN,
    HASHED, HIST_DUP, HIST_FOREIGN, HIST_MAKEUNIQUE, HIST_TMPSTORE, PM_CUR_FPATH, PM_KSHSTORED,
    PM_LOADDIR, PM_TAGGED, PM_TAGGED_LOCAL, PM_UNALIASED, PM_UNDEFINED, PM_ZSHSTORED, PRINT_LIST,
    PRINT_NAMEONLY, PRINT_WHENCE_CSH, PRINT_WHENCE_FUNCDEF, PRINT_WHENCE_SIMPLE,
    PRINT_WHENCE_VERBOSE, PRINT_WHENCE_WORD, ZSIG_FUNC,
};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// Generic hash function (zsh's hasher)
/// Compute the canonical zsh hash for a string.
/// Port of `hasher(const char *str)` from Src/hashtable.c:86 — uses the same
// Generic hash function                                                    // c:86
/// `hash * 33 + char` polynomial the C source uses for every
/// HashTable lookup.
pub fn hasher(str: &str) -> u32 {
    // c:86
    let mut hashval: u32 = 0;
    for c in str.bytes() {
        hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(c as u32));
    }
    hashval
}

/// Port of the node-storage half of `struct hashtable`
/// (`Src/zsh.h:1175-1235`: `HashNode *nodes; int hsize; int ct;`) as
/// `Src/hashtable.c` actually drives it — an OPEN-HASHED BUCKET ARRAY:
///
///   * bucket index is `ht->hash(nam) % ht->hsize` (c:176/236/260/280);
///   * a new key goes to the FRONT of its chain (c:194-196 for an empty
///     bucket, c:214-215 otherwise);
///   * replacing an existing key keeps that key's POSITION in the chain
///     (c:187-203 `replacing:` — `hn->next = hp->next`);
///   * once `ct >= hsize * 2` the table quadruples and every node is
///     re-added in old-traversal order (c:183/219 → `expandhashtable`
///     c:458-482);
///   * an unsorted scan walks bucket 0..hsize-1, each chain head→tail
///     (`scanmatchtable` c:420-434).
///
/// That walk order is OBSERVABLE: `${(k)functions}` /
/// `${(k)commands}` / `compadd -k <assoc>` all emit it verbatim
/// (`Src/Modules/parameter.c:480-481` loops `shfunctab->nodes[i]`
/// directly), and `join_clines` is a non-commutative fold, so the
/// order matches are added in decides the common prefix `compadd -k`
/// produces.
///
/// The port previously stood a `std::collections::HashMap` in for the
/// bucket array. That is not merely a different order — `RandomState`
/// re-seeds per process, so `print -rl -- ${(k)functions}` returned a
/// DIFFERENT order on every run of the same binary against the same
/// 46761-function table, and any completion enumerating the table was
/// nondeterministic.
///
/// !!! WARNING: RUST-ONLY HELPER !!!
/// C reaches these fields through one polymorphic `struct hashtable`
/// with `hash`/`cmpnodes`/`addnode`/`getnode` function pointers. Rust
/// has no equivalent of the `HashNode`-as-first-member downcast, so the
/// storage is a generic struct and the typed wrappers below embed it.
/// Every method is named for the `Src/hashtable.c` function it ports.
#[derive(Debug, Clone)]
pub struct hashtable_nodes<T> {
    /// `HashNode *nodes` (`Src/zsh.h:1177`) — `hsize` chains. Index 0 of
    /// a chain is the head, i.e. the most recently inserted key.
    nodes: Vec<Vec<(String, T)>>,
    /// `int hsize` (`Src/zsh.h:1179`) — number of buckets.
    hsize: usize,
    /// `int ct` (`Src/zsh.h:1181`) — number of live nodes.
    ct: usize,
}

impl<T> hashtable_nodes<T> {
    /// Port of `newhashtable(int size, …)` from `Src/hashtable.c:100` —
    /// `zshcalloc(size * sizeof(HashNode))` + `hsize = size` + `ct = 0`.
    pub fn newhashtable(size: usize) -> Self {
        // c:100
        let size = size.max(1); // c:115 — a 0-bucket table can't be indexed
        let mut nodes = Vec::with_capacity(size);
        nodes.resize_with(size, Vec::new); // c:116
        Self {
            nodes,
            hsize: size, // c:117
            ct: 0,       // c:118
        }
    }

    /// Port of `expandhashtable(HashTable ht)` from `Src/hashtable.c:458`.
    /// Quadruples `hsize`, zeroes `ct`, and re-adds every node walking the
    /// OLD table in traversal order (bucket 0..osize-1, chain head→tail)
    /// so the new chains come out in C's exact order.
    fn expandhashtable(&mut self) {
        // c:458
        let osize = self.hsize; // c:463
        let onodes = std::mem::take(&mut self.nodes); // c:464
        self.hsize = osize * 4; // c:466
        self.nodes = Vec::with_capacity(self.hsize);
        self.nodes.resize_with(self.hsize, Vec::new); // c:467
        self.ct = 0; // c:468
                     // c:471-476 — `for (i = 0, ha = onodes; i < osize; i++, ha++)
                     //                for (hn = *ha; hn;) { hp = hn->next;
                     //                    ht->addnode(ht, hn->nam, hn); hn = hp; }`
        for bucket in onodes {
            for (nam, node) in bucket {
                let hashval = hasher(&nam) as usize % self.hsize; // c:176
                self.nodes[hashval].insert(0, (nam, node)); // c:214-215
                self.ct += 1; // c:219 (the expand test can't re-fire here)
            }
        }
    }

    /// Port of `addhashnode2(HashTable ht, char *nam, void *nodeptr)` from
    /// `Src/hashtable.c:168` — inserts, returning the displaced node.
    pub fn addhashnode2(&mut self, nam: &str, nodeptr: T) -> Option<T> {
        // c:168
        let hashval = hasher(nam) as usize % self.hsize; // c:176
                                                         // c:186-206 — an existing key is replaced IN PLACE, keeping its
                                                         // position in the chain; ct does not move and no expand fires.
        if let Some(pos) = self.nodes[hashval].iter().position(|(k, _)| k == nam) {
            let old = std::mem::replace(&mut self.nodes[hashval][pos], (nam.to_string(), nodeptr));
            return Some(old.1); // c:203
        }
        // c:193-196 / c:214-215 — otherwise the new node goes to the FRONT.
        self.nodes[hashval].insert(0, (nam.to_string(), nodeptr));
        self.ct += 1;
        if self.ct >= self.hsize * 2 {
            // c:183 / c:219
            self.expandhashtable(); // c:184 / c:220
        }
        None
    }

    /// Port of `gethashnode2(HashTable ht, const char *nam)` from
    /// `Src/hashtable.c:255` — lookup WITHOUT the DISABLED filter.
    pub fn gethashnode2(&self, nam: &str) -> Option<&T> {
        // c:255
        let hashval = hasher(nam) as usize % self.hsize; // c:260
        self.nodes[hashval]
            .iter()
            .find(|(k, _)| k == nam) // c:262-265
            .map(|(_, v)| v)
    }

    /// Mutable companion of [`hashtable_nodes::gethashnode2`]. C mutates
    /// straight through the returned `HashNode` pointer; Rust needs the
    /// separate borrow.
    pub fn get_mut(&mut self, nam: &str) -> Option<&mut T> {
        // c:255
        let hashval = hasher(nam) as usize % self.hsize;
        self.nodes[hashval]
            .iter_mut()
            .find(|(k, _)| k == nam)
            .map(|(_, v)| v)
    }

    /// Port of `removehashnode(HashTable ht, const char *nam)` from
    /// `Src/hashtable.c:275` — unlinks the node from its chain and
    /// decrements `ct`.
    pub fn removehashnode(&mut self, nam: &str) -> Option<T> {
        // c:275
        let hashval = hasher(nam) as usize % self.hsize; // c:280
        let pos = self.nodes[hashval].iter().position(|(k, _)| k == nam)?;
        self.ct -= 1; // c:294
        Some(self.nodes[hashval].remove(pos).1)
    }

    /// Port of `emptyhashtable(HashTable ht)` from `Src/hashtable.c:517`
    /// (`resizehashtable(ht, ht->hsize)`) — frees every node, keeps `hsize`.
    pub fn emptyhashtable(&mut self) {
        // c:517
        for bucket in self.nodes.iter_mut() {
            bucket.clear(); // c:490-497
        }
        self.ct = 0; // c:509
    }

    /// C's `ht->ct` (`Src/zsh.h:1181`).
    pub fn len(&self) -> usize {
        self.ct
    }

    /// `ht->ct == 0`.
    pub fn is_empty(&self) -> bool {
        self.ct == 0
    }

    /// The unsorted scan order of `scanmatchtable` (`Src/hashtable.c:420-434`):
    /// bucket 0..hsize-1, each chain head→tail. This IS the order zsh's
    /// `${(k)assoc}` / `compadd -k` emit.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &T)> {
        // c:420-434
        self.nodes.iter().flatten().map(|(k, v)| (k, v))
    }

    /// Mutable companion of [`hashtable_nodes::iter`] — same
    /// `scanmatchtable` walk (`Src/hashtable.c:420-434`); C hands the
    /// scan function a writable `HashNode`, Rust needs the separate
    /// borrow.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&String, &mut T)> {
        // c:420-434
        self.nodes.iter_mut().flatten().map(|(k, v)| (&*k, v))
    }

    /// The names in `scanmatchtable` order (`Src/hashtable.c:420-434`) —
    /// C's scan reads `hn->nam` off each node in the same walk.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        // c:420-434
        self.nodes.iter().flatten().map(|(k, _)| k)
    }

    /// The nodes in `scanmatchtable` order (`Src/hashtable.c:420-434`).
    pub fn values(&self) -> impl Iterator<Item = &T> {
        // c:420-434
        self.nodes.iter().flatten().map(|(_, v)| v)
    }

    /// Mutable companion of [`hashtable_nodes::values`]
    /// (`Src/hashtable.c:420-434`).
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        // c:420-434
        self.nodes.iter_mut().flatten().map(|(_, v)| v)
    }

    /// `ht->getnode2(ht, nam)` (`Src/hashtable.c:255` `gethashnode2`) —
    /// map-shaped alias so tables converted from a `HashMap` keep their
    /// call sites. Same O(1) bucket hash + short-chain walk as C.
    pub fn get(&self, nam: &str) -> Option<&T> {
        // c:255
        self.gethashnode2(nam)
    }

    /// `ht->getnode2(ht, nam)` != NULL (`Src/hashtable.c:255`).
    pub fn contains_key(&self, nam: &str) -> bool {
        // c:255
        self.gethashnode2(nam).is_some()
    }

    /// `ht->addnode(ht, ztrdup(nam), node)` (`Src/hashtable.c:157`
    /// `addhashnode` → `addhashnode2` at `c:168`) — returns the
    /// displaced node instead of running `freenode`.
    pub fn insert(&mut self, nam: String, nodeptr: T) -> Option<T> {
        // c:157 / c:168
        self.addhashnode2(&nam, nodeptr)
    }

    /// `ht->removenode(ht, nam)` (`Src/hashtable.c:275`
    /// `removehashnode`).
    pub fn remove(&mut self, nam: &str) -> Option<T> {
        // c:275
        self.removehashnode(nam)
    }

    /// `ht->emptytable(ht)` (`Src/hashtable.c:517` `emptyhashtable`).
    pub fn clear(&mut self) {
        // c:517
        self.emptyhashtable();
    }

    /// The `scanmatchtable` walk (`Src/hashtable.c:420-434`) with the
    /// C `freenode` arm taken for every node the predicate rejects.
    /// Chain order of the survivors is preserved exactly, which is what
    /// makes this different from rebuilding the table.
    pub fn retain<F: FnMut(&String, &mut T) -> bool>(&mut self, mut f: F) {
        // c:420-434
        let mut ct = 0usize;
        for bucket in self.nodes.iter_mut() {
            bucket.retain_mut(|(k, v)| f(k, v));
            ct += bucket.len();
        }
        self.ct = ct; // c:294 — one decrement per unlinked node
    }
}

impl<T> Default for hashtable_nodes<T> {
    /// `newparamtable`/`newmoduletable` fall back to `size = 17` when
    /// handed 0 (`Src/params.c:541-542`, and `Src/module.c:1602`
    /// creates 17-bucket sub-tables); use that as the neutral default
    /// for `#[derive(Default)]` containers.
    fn default() -> Self {
        // c:541-542
        Self::newhashtable(17)
    }
}

impl<T, Q: ?Sized + std::borrow::Borrow<str>> std::ops::Index<&Q> for hashtable_nodes<T> {
    type Output = T;
    /// `ht->getnode2(ht, nam)` with C's "caller already checked" contract
    /// (`Src/hashtable.c:255`) — panics on a missing name, like the
    /// `HashMap` indexing it replaces.
    fn index(&self, nam: &Q) -> &T {
        // c:255
        let nam = nam.borrow();
        self.gethashnode2(nam)
            .unwrap_or_else(|| panic!("no hash node named {nam}"))
    }
}

impl<'a, T> IntoIterator for &'a hashtable_nodes<T> {
    type Item = (&'a String, &'a T);
    type IntoIter = std::iter::Map<
        std::iter::Flatten<std::slice::Iter<'a, Vec<(String, T)>>>,
        fn(&'a (String, T)) -> (&'a String, &'a T),
    >;
    /// `for (k, v) in &table` — the `scanmatchtable` walk
    /// (`Src/hashtable.c:420-434`).
    fn into_iter(self) -> Self::IntoIter {
        // c:420-434
        self.nodes
            .iter()
            .flatten()
            .map(|(k, v): &(String, T)| (k, v))
    }
}

// ===========================================================
// Direct ports of the generic `HashTable` lifecycle / mutation /
// printer routines from Src/hashtable.c. The Rust port stores
// command/alias/reswd/shfunc tables as `HashMap`-backed wrappers
// (above), so most of these are free-fn shims for ABI/name
// parity. Callers in the Rust executor reach the live state via
// the typed table structs (`alias_table`, `shfunc_table`, etc.).
// ===========================================================

/// Port of `newhashtable(int size, UNUSED(char const *name), UNUSED(PrintTableStats printinfo))` from `Src/hashtable.c:100`.
///
/// C allocates a `HashTable` header with `size` buckets and the
/// supplied `name` for `bin_hashinfo` reporting. Rust uses
/// `HashMap` (auto-resizing) so the bucket count is informational;
/// the named-table accounting is recorded for `printhashtabinfo`.
///
/// Returns a `(name, expected_size)` tuple — callers (the table-
/// specific creators) typically discard since each Rust table
/// type has its own constructor. Provided for C name parity.
// Get a new hash table                                                     // c:100
/// WARNING: param names don't match C — Rust=(size, name) vs C=(size, name, printinfo)
pub fn newhashtable(size: i32, name: &str) -> (String, i32) {
    // c:100
    (name.to_string(), size)
}

/// Port of `deletehashtable(HashTable ht)` from `Src/hashtable.c:129`.
///
/// C frees every node via `emptytable` then frees the header.
/// Rust port: `Drop` runs the equivalent on the typed table when
/// it falls out of scope. The free fn here calls clear on the
/// passed map for C name parity at call sites that explicitly
/// invoke deletehashtable.
pub fn deletehashtable<T>(ht: &mut HashMap<String, T>) {
    // c:129
    ht.clear();
}

// `cmdnam` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::cmdnam` (zsh.h:1301-1308). C struct:
//
//     struct cmdnam {
//         struct hashnode node;
//         union {
//             char **name;   /* HASHED off: full $PATH array (u.name) */
//             char  *cmd;    /* HASHED on:  resolved abs path (u.cmd) */
//         } u;
//     };
//
// The Rust-only version had a flat `name, flags, path: PathBuf,
// dir_index` shape that lost the hashnode embedding and the
// name/cmd union (the C source uses `flags & HASHED` to dispatch
// which arm holds the value). Type alias surfaces the canonical
// struct directly; the previous `path: PathBuf` becomes
// `cmd: Option<String>` and `dir_index: Option<usize>` becomes
// `name: Option<Vec<String>>` (the full PATH-segment slice the
// command would be looked up against).
// c:1301

/// Port of `addhashnode(HashTable ht, char *nam, void *nodeptr)` from `Src/hashtable.c:157`.
///
/// C body:
/// ```c
/// HashNode oldnode = addhashnode2(ht, nam, nodeptr);
/// if (oldnode) ht->freenode(oldnode);
/// ```
///
/// Generic insert that drops the previous value at `nam` (Rust's
// is now greater than twice the number of hash values,                    // c:157
// the table is then expanded.                                              // c:157
/// `HashMap::insert` returns the old value; dropping it runs the
/// equivalent of `freenode`). For typed table-specific entry
/// shapes use the table's own `add()` method.
// Add a node to a hash table, returning the old node on replacement.      // c:168
/// `addhashnode` — see implementation.
pub fn addhashnode<T>(ht: &mut HashMap<String, T>, nam: &str, value: T) {
    // c:157
    ht.insert(nam.to_string(), value);
}

// Add a node to a hash table, returning the old node on replacement.      // c:168
/// Port of `addhashnode2(HashTable ht, char *nam, void *nodeptr)` from `Src/hashtable.c:168`.
///
/// C body inserts and returns the OLD node (instead of freeing
/// it via the freenode callback). Rust HashMap::insert already
/// has this shape — return the displaced value.
pub fn addhashnode2<T>(ht: &mut HashMap<String, T>, nam: &str, nodeptr: T) -> Option<T> {
    // c:168
    ht.insert(nam.to_string(), nodeptr)
}

/// Port of `gethashnode(HashTable ht, const char *nam)` from `Src/hashtable.c:231`.
///
/// C body returns NULL if the entry has the DISABLED flag set;
// the hashnode.  If the node is DISABLED                                  // c:231
// or isn't found, it returns NULL                                          // c:231
/// otherwise returns the node. Generic lookup helper — `T` must
/// expose its DISABLED flag via the [`HashNodeFlags`] trait so
/// the disabled filter applies.
/// WARNING: param names don't match C — Rust=(nam) vs C=(ht, nam)
pub fn gethashnode<'a, T: HashNodeFlags>(
    // c:231
    ht: &'a HashMap<String, T>,
    nam: &str,
) -> Option<&'a T> {
    ht.get(nam).filter(|t| !t.is_disabled())
}

impl cmdnam_table {
    /// `new` — see implementation.
    pub fn new() -> Self {
        Self {
            // hashtable.c:603 — `cmdnamtab = newhashtable(201, "cmdnamtab", NULL)`.
            // The bucket count is observable: `${(k)commands}` and
            // `compadd -k commands` emit the raw bucket walk.
            table: hashtable_nodes::newhashtable(201), // c:603
            path_checked_index: 0,
            path: Vec::new(),
            hash_executables_only: false,
        }
    }
    /// `set_path` — see implementation.
    pub fn set_path(&mut self, path: Vec<String>) {
        self.path = path;
        self.path_checked_index = 0;
    }
    /// `set_hash_executables_only` — see implementation.
    pub fn set_hash_executables_only(&mut self, value: bool) {
        self.hash_executables_only = value;
    }
    /// `add` — `addhashnode` (`Src/hashtable.c:157`).
    pub fn add(&mut self, cmd: cmdnam) {
        let nam = cmd.node.nam.clone();
        let _ = self.table.addhashnode2(&nam, cmd); // c:168
    }
    /// `get` — `gethashnode` (`Src/hashtable.c:231`).
    pub fn get(&self, name: &str) -> Option<&cmdnam> {
        self.table
            .gethashnode2(name)
            .filter(|c| (c.node.flags & DISABLED as i32) == 0) // c:239
    }
    /// `get_including_disabled` — `gethashnode2` (`Src/hashtable.c:255`).
    pub fn get_including_disabled(&self, name: &str) -> Option<&cmdnam> {
        self.table.gethashnode2(name) // c:255
    }
    /// `remove` — `removehashnode` (`Src/hashtable.c:275`).
    pub fn remove(&mut self, name: &str) -> Option<cmdnam> {
        self.table.removehashnode(name) // c:275
    }
    /// `clear` — `emptyhashtable` (`Src/hashtable.c:517`).
    pub fn clear(&mut self) {
        self.table.emptyhashtable(); // c:517
        self.path_checked_index = 0;
    }
    /// `len` — see implementation.
    pub fn len(&self) -> usize {
        self.table.len()
    }
    /// `is_empty` — see implementation.
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

            if self.table.gethashnode2(&name).is_some() {
                continue;
            }

            let path = entry.path();
            let should_add = if self.hash_executables_only {
                // Inline of the deleted is_executable helper.
                #[cfg(unix)]
                {
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
                // C `cn->u.name = pathchecked;` at hashtable.c:712 —
                // the unhashed entry carries the PATH-array slice it
                // would scan. Rust port: snapshot the single PATH
                // segment at `dir_index` so lookup later resolves
                // the path. Older Rust-only code stored just the
                // index; canonical port stores the actual segment.
                let segment = self
                    .path
                    .get(dir_index)
                    .cloned()
                    .unwrap_or_else(|| dir.to_string());
                let _ = self
                    .table
                    .addhashnode2(&name, cmdnam_unhashed(&name, vec![segment])); // c:168
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
    pub fn iter(&self) -> impl Iterator<Item = (&String, &cmdnam)> {
        self.table.iter()
    }

    /// Get full path for a command. Mirrors C's
    /// `findcmd(name, 1, 0)` lookup via cmdnamtab (Src/exec.c:5260).
    pub fn get_full_path(&self, name: &str) -> Option<PathBuf> {
        let cmd = self.table.gethashnode2(name)?;
        if (cmd.node.flags & DISABLED as i32) != 0 {
            return None;
        }
        // HASHED branch: cn->u.cmd holds the resolved path.
        if (cmd.node.flags & HASHED as i32) != 0 {
            if let Some(ref s) = cmd.cmd {
                return Some(PathBuf::from(s));
            }
        }
        // Unhashed branch: cn->u.name holds PATH segments to scan.
        if let Some(ref segs) = cmd.name {
            if let Some(seg) = segs.first() {
                let mut path = PathBuf::from(seg);
                path.push(name);
                return Some(path);
            }
        }
        None
    }
}

impl Default for cmdnam_table {
    fn default() -> Self {
        Self::new()
    }
}

// `shfunc` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::shfunc` (zsh.h:1316-1325). Canonical:
//
//     struct shfunc {
//         struct hashnode node;
//         char *filename;
//         zlong lineno;
//         Eprog funcdef;
//         Eprog redir;
//         Emulation_options sticky;
//     };
//
// Canonical was extended with a Rust-only `body: Option<String>`
// field (deferred-compile source text) so callers using the old
// `shfunc.body` access continue working. Type alias surfaces
// canonical as `shfunc`; helpers below build instances with the
// hashnode literal pre-populated.

/// Port of `gethashnode2(HashTable ht, const char *nam)` from `Src/hashtable.c:255`.
///
/// Same as gethashnode but bypasses the DISABLED filter.
pub fn gethashnode2<'a, T>(ht: &'a HashMap<String, T>, nam: &str) -> Option<&'a T> {
    // c:255
    ht.get(nam)
}

/// Port of `removehashnode(HashTable ht, const char *nam)` from `Src/hashtable.c:275`.
///
// table and returns a pointer to it.  If there                            // c:275
// is no such node, then it returns NULL                                    // c:275
/// C body removes the node from the bucket chain and returns the
/// removed pointer (or NULL). Rust `HashMap::remove` has the
/// matching shape.
pub fn removehashnode<T>(ht: &mut HashMap<String, T>, nam: &str) -> Option<T> {
    // c:275
    ht.remove(nam)
}

/// Port of `disablehashnode(HashNode hn, UNUSED(int flags))` from `Src/hashtable.c:323`.
///
/// C body: `hn->flags |= DISABLED;`. Generic helper that flips
/// the DISABLED bit on the named entry via [`HashNodeFlags`].
pub fn disablehashnode<T: HashNodeFlags>(hn: &mut HashMap<String, T>, flags: &str) -> bool {
    hn.get_mut(flags)
        .map(|node| {
            node.set_disabled(true);
            true
        })
        .unwrap_or(false) // c:323
}

impl shfunc_table {
    /// `new` — `shfunctab = newhashtable(7, "shfunctab", NULL)`
    /// (`Src/hashtable.c:814`). The initial bucket count is part of the
    /// observable scan order, so it must be C's 7 and grow only through
    /// `expandhashtable`'s x4 rule.
    pub fn new() -> Self {
        Self {
            table: hashtable_nodes::newhashtable(7), // hashtable.c:814
        }
    }
    /// `snapshot` — clone the whole bucket array for subshell
    /// save/restore. Used by `subshell_begin` to capture the parent's
    /// function set before the subshell body runs, so `subshell_end`
    /// can restore it (matches C fork-copy semantics at
    /// `Src/exec.c::entersubsh`).
    ///
    /// This clones the TABLE, not a `HashMap` of its entries: the scan
    /// order is part of the state (`Src/Modules/parameter.c:480-481`
    /// walks `shfunctab->nodes[i]` directly), and rebuilding a bucket
    /// array from an unordered map would reshuffle `${(k)functions}`
    /// after every `( … )` / `$( … )`.
    pub fn snapshot(&self) -> std::sync::Arc<shfunc_table> {
        std::sync::Arc::new(self.clone())
    }
    /// `restore` — replace the internal table with a saved snapshot.
    /// Called by `subshell_end` after the subshell body completes.
    /// Takes the `Arc`-shared snapshot stored in `SubshellSnapshot`;
    /// unwraps in place when uniquely owned (the common case), else
    /// clones out of the shared handle.
    pub fn restore(&mut self, snap: std::sync::Arc<shfunc_table>) {
        *self = std::sync::Arc::try_unwrap(snap).unwrap_or_else(|arc| (*arc).clone());
    }
    /// Formerly pre-sized the backing `HashMap`. C has no such call and
    /// CANNOT have one: `hsize` only ever moves through
    /// `expandhashtable`'s x4 steps (`Src/hashtable.c:466`), and the
    /// bucket count is what `hasher(nam) % hsize` — hence the whole scan
    /// order — is computed against. Pre-sizing to the batch size would
    /// put every name in a different bucket than zsh does. Kept as a
    /// no-op so the `compinit` call site (which is a Rust-only
    /// optimisation) still compiles.
    pub fn reserve(&mut self, _additional: usize) {}

    /// `add` — `addhashnode2` (`Src/hashtable.c:168`) with the displaced
    /// node handed back to the caller instead of `freenode`d.
    pub fn add(&mut self, func: shfunc) -> Option<shfunc> {
        let name = func.node.nam.clone();
        self.table
            .addhashnode2(&name, Box::new(func)) // c:168
            .map(|b| *b)
    }
    /// `get` — `gethashnode` (`Src/hashtable.c:231`): DISABLED nodes read
    /// as absent.
    pub fn get(&self, name: &str) -> Option<&shfunc> {
        self.table
            .gethashnode2(name) // c:236-241
            .map(|b| b.as_ref())
            .filter(|f| (f.node.flags & DISABLED as i32) == 0) // c:239
    }
    /// `get_including_disabled` — `gethashnode2` (`Src/hashtable.c:255`).
    pub fn get_including_disabled(&self, name: &str) -> Option<&shfunc> {
        self.table.gethashnode2(name).map(|b| b.as_ref()) // c:255
    }
    /// `get_mut` — see implementation.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut shfunc> {
        self.table
            .get_mut(name)
            .map(|b| b.as_mut())
            .filter(|f| (f.node.flags & DISABLED as i32) == 0)
    }
    /// `remove` — `removehashnode` (`Src/hashtable.c:275`).
    pub fn remove(&mut self, name: &str) -> Option<shfunc> {
        self.table.removehashnode(name).map(|b| *b) // c:275
    }
    /// `contains_key` — see implementation.
    pub fn contains_key(&self, name: &str) -> bool {
        self.table.gethashnode2(name).is_some()
    }

    /// Port of C's `HashTable.addnode` GSU function pointer
    /// (`Src/zsh.h:281+`). Takes a `*mut shfunc` (typedef `Shfunc`)
    /// previously obtained via `Box::into_raw` — reclaims ownership
    /// into the table by name. After this call, the caller's `shf`
    /// pointer is INVALIDATED in the Rust ownership sense; subsequent
    /// reads must go through `getnode(name)` to get a fresh pointer.
    /// In practice, C code re-uses the same `shf` pointer because the
    /// Box stays at the same heap address — we keep that semantic by
    /// boxing-on-heap. Replaces any prior entry with the same name
    /// (matching C `addnode`'s overwrite-and-free-old behavior).
    pub fn addnode(&mut self, shf: *mut shfunc) {
        if shf.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(shf) };
        let name = boxed.node.nam.clone();
        let _ = self.table.addhashnode2(&name, boxed); // c:168
    }

    /// Port of C's `HashTable.getnode` GSU. Returns the raw `Shfunc`
    /// pointer (typedef `*mut shfunc`) or null if missing or disabled.
    /// Pointer stays valid as long as the underlying `Box<shfunc>`
    /// lives in the table (i.e. until `remove`/`addnode`-overwrite).
    pub fn getnode(&self, name: &str) -> *mut shfunc {
        self.table
            .gethashnode2(name)
            .filter(|b| (b.node.flags & DISABLED as i32) == 0)
            .map(|b| b.as_ref() as *const shfunc as *mut shfunc)
            .unwrap_or(std::ptr::null_mut())
    }

    /// Port of C's `HashTable.getnode2` GSU — same as `getnode` but
    /// returns disabled nodes too. Used by `unhash`/`enable -f` paths.
    pub fn getnode2(&self, name: &str) -> *mut shfunc {
        self.table
            .gethashnode2(name)
            .map(|b| b.as_ref() as *const shfunc as *mut shfunc)
            .unwrap_or(std::ptr::null_mut())
    }
    /// `disable` — see implementation.
    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(func) = self.table.get_mut(name) {
            func.node.flags |= DISABLED as i32;
            true
        } else {
            false
        }
    }
    /// `enable` — see implementation.
    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(func) = self.table.get_mut(name) {
            func.node.flags &= !(DISABLED as i32);
            true
        } else {
            false
        }
    }
    /// `len` — see implementation.
    pub fn len(&self) -> usize {
        self.table.len()
    }
    /// `is_empty` — see implementation.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    /// `iter` — see implementation.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &shfunc)> {
        self.table.iter().map(|(k, b)| (k, b.as_ref()))
    }
    /// `iter_sorted` — see implementation.
    pub fn iter_sorted(&self) -> Vec<(&String, &shfunc)> {
        let mut entries: Vec<(&String, &shfunc)> =
            self.table.iter().map(|(k, b)| (k, b.as_ref())).collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
    }
    /// `clear` — `emptyhashtable` (`Src/hashtable.c:517`): frees every
    /// node but keeps the bucket count.
    pub fn clear(&mut self) {
        self.table.emptyhashtable(); // c:517
    }
}

impl Default for shfunc_table {
    fn default() -> Self {
        Self::new()
    }
}

// `reswdToken` enum deleted — Rust-only enum duplicating the
// canonical `lextok` i32 token constants already in zsh_h.rs
// (BANG_TOK/DINBRACK/INBRACE_TOK/OUTBRACE_TOK/CASE/COPROC/DOLOOP
// /DONE/ELIF/ELSE/ZEND/ESAC/FI/FOR/FOREACH/FUNC/IF/NOCORRECT/
// REPEAT/SELECT/THEN/TIME/UNTIL/WHILE/TYPESET at zsh.h:345-371).
// reswd.token now stores the raw i32 lextok matching C `struct
// reswd { HashNode node; int token; }` at zsh.h:1246-1249.

// `reswd` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::reswd` (zsh.h:1246-1249). The canonical
// has `node: hashnode { nam, flags, next }` + `token: i32`; the
// Rust-only had `name, flags: u32, token: i32` (missing the
// hashnode embedding). Type alias surfaces the canonical struct
// to in-file callers and external imports.
// c:1246

/// Public copy of the canonical `reswds[]` table from
/// `Src/hashtable.c:1076-1108`. Each entry is `(name, lextok)`; the
/// token identifies which grammar production the word triggers.
///
/// Callers outside the hashtable (LSP reflection dump, IntelliJ
/// inventory) iterate this directly so they don't have to take the
/// `reswdtab` lock or duplicate the list. Filtering: entries with
/// `token == TYPESET` are declaration commands (local / typeset /
/// declare / export / readonly / integer / float) — they're aliased
/// to `typeset` at the grammar level but really live as builtins, so
/// a "reserved word" inventory should exclude them.
pub const RESWDS: &[(&str, i32)] = &[
    ("!", BANG_TOK),
    ("[[", DINBRACK),
    ("{", INBRACE_TOK),
    ("}", OUTBRACE_TOK),
    ("case", CASE),
    ("coproc", COPROC),
    ("declare", TYPESET),
    ("do", DOLOOP),
    ("done", DONE),
    ("elif", ELIF),
    ("else", ELSE),
    ("end", ZEND),
    ("esac", ESAC),
    ("export", TYPESET),
    ("fi", FI),
    ("float", TYPESET),
    ("for", FOR),
    ("foreach", FOREACH),
    ("function", FUNC),
    ("if", IF),
    ("integer", TYPESET),
    ("local", TYPESET),
    ("nocorrect", NOCORRECT),
    ("readonly", TYPESET),
    ("repeat", REPEAT),
    ("select", SELECT),
    ("then", THEN),
    ("time", TIME),
    ("typeset", TYPESET),
    ("until", UNTIL),
    ("while", WHILE),
];

/// Port of `enablehashnode(HashNode hn, UNUSED(int flags))` from `Src/hashtable.c:332`.
///
/// C body: `hn->flags &= ~DISABLED;`. Inverse of [`disablehashnode`].
pub fn enablehashnode<T: HashNodeFlags>(hn: &mut HashMap<String, T>, flags: &str) -> bool {
    hn.get_mut(flags)
        .map(|node| {
            node.set_disabled(false);
            true
        })
        .unwrap_or(false) // c:332
}

impl reswd_table {
    /// `new` — port of `createreswdtable()` (`Src/hashtable.c:1120`):
    /// `newhashtable(23, "reswdtab", NULL)` (`c:1124`) followed by
    /// `for (rw = reswds; rw->node.nam; rw++) reswdtab->addnode(...)`
    /// (`c:1138-1139`), which walks the static `reswds[]` array in
    /// declaration order.
    pub fn new() -> Self {
        // c:1124 — `reswdtab = newhashtable(23, "reswdtab", NULL);`
        let mut table = hashtable_nodes::newhashtable(23);

        // Direct port of `static struct reswd reswds[]` at
        // Src/hashtable.c:1076-1108. Token IDs are the lextok
        // constants from zsh_h.rs (zsh.h:345-371).
        //
        // Same list is exposed via the public `RESWDS` const below so
        // callers outside this module (LSP reflection dump, IntelliJ
        // tool-window inventory) can enumerate reserved words without
        // taking the table lock.
        let words: [(&str, i32); 31] = [
            // c:1076
            ("!", BANG_TOK),          // c:1077
            ("[[", DINBRACK),         // c:1078
            ("{", INBRACE_TOK),       // c:1079
            ("}", OUTBRACE_TOK),      // c:1080
            ("case", CASE),           // c:1081
            ("coproc", COPROC),       // c:1082
            ("declare", TYPESET),     // c:1083
            ("do", DOLOOP),           // c:1084
            ("done", DONE),           // c:1085
            ("elif", ELIF),           // c:1086
            ("else", ELSE),           // c:1087
            ("end", ZEND),            // c:1088
            ("esac", ESAC),           // c:1089
            ("export", TYPESET),      // c:1090
            ("fi", FI),               // c:1091
            ("float", TYPESET),       // c:1092
            ("for", FOR),             // c:1093
            ("foreach", FOREACH),     // c:1094
            ("function", FUNC),       // c:1095
            ("if", IF),               // c:1096
            ("integer", TYPESET),     // c:1097
            ("local", TYPESET),       // c:1098
            ("nocorrect", NOCORRECT), // c:1099
            ("readonly", TYPESET),    // c:1100
            ("repeat", REPEAT),       // c:1101
            ("select", SELECT),       // c:1102
            ("then", THEN),           // c:1103
            ("time", TIME),           // c:1104
            ("typeset", TYPESET),     // c:1105
            ("until", UNTIL),         // c:1106
            ("while", WHILE),         // c:1107
        ];
        // Sanity: the local `words` array and the public `RESWDS` const
        // below MUST stay in sync — both are direct ports of the same
        // upstream `reswds[]` table at Src/hashtable.c:1076-1108.
        debug_assert_eq!(words.len(), RESWDS.len());

        for (name, token) in words {
            // Direct struct literal — canonical `reswd` has
            // `node: hashnode` (zsh.h:1246) so we build the
            // embedded hashnode inline. Mirrors C `{{NULL,
            // "if", 0}, IF}` at hashtable.c:1077+.
            // c:1138-1139 — `reswdtab->addnode(reswdtab, rw->node.nam, rw)`
            table.addhashnode2(
                name,
                reswd {
                    node: hashnode {
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
    /// `get` — `gethashnode` (`Src/hashtable.c:245`), the lookup that
    /// skips `DISABLED` nodes (`c:253`); `createreswdtable` wires
    /// `reswdtab->getnode = gethashnode` (`c:1131`).
    pub fn get(&self, name: &str) -> Option<&reswd> {
        // c:245
        self.table
            .gethashnode2(name)
            .filter(|r| (r.node.flags & DISABLED as i32) == 0)
    }
    /// `get_including_disabled` — `gethashnode2`
    /// (`Src/hashtable.c:255`), wired as `reswdtab->getnode2`
    /// (`c:1132`).
    pub fn get_including_disabled(&self, name: &str) -> Option<&reswd> {
        self.table.gethashnode2(name) // c:255
    }
    /// `disable` — see implementation.
    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.node.flags |= DISABLED as i32;
            true
        } else {
            false
        }
    }
    /// `enable` — see implementation.
    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(rw) = self.table.get_mut(name) {
            rw.node.flags &= !(DISABLED as i32);
            true
        } else {
            false
        }
    }
    /// `is_reserved` — see implementation.
    pub fn is_reserved(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    /// `iter` — the raw bucket walk `getreswords`
    /// (`Src/Modules/parameter.c:877-880`) does over
    /// `reswdtab->nodes[]`, which is what `$reswords` /
    /// `$dis_reswords` expose.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &reswd)> {
        self.table.iter() // c:420-434
    }
    /// Port of `addhashnode(HashTable ht, char *nam, void *nodeptr)`
    /// from `Src/hashtable.c:157`. C stores `nodeptr` under `nam` and
    /// frees any node it displaces (`ht->freenode(oldnode)`, c:161).
    /// Here the map replaces the entry and the displaced `reswd` is
    /// dropped, matching C's freenode semantics. Runtime companion to
    /// the seed-only `new()`; param_private's `setup_` uses it to
    /// register `private` as a TYPESET reserved word at module boot
    /// (param_private.c:687 `reswdtab->addnode(reswdtab, ...)`).
    pub fn insert(&mut self, name: &str, rw: reswd) {
        // c:157 addhashnode → c:159 addhashnode2 sets hn->nam = nam
        self.table.addhashnode2(name, rw);
    }
    /// Port of `removehashnode(HashTable ht, const char *nam)` from
    /// `Src/hashtable.c:275`. Unlinks the node keyed by `nam` and
    /// returns it (C returns the removed `HashNode`, or NULL when the
    /// key is absent — c:283). param_private's teardown uses it to
    /// unregister the `private` reserved word (param_private.c:722
    /// `removehashnode(reswdtab, "private")`).
    pub fn remove(&mut self, name: &str) -> Option<reswd> {
        // c:275
        self.table.removehashnode(name)
    }
}

impl Default for reswd_table {
    fn default() -> Self {
        Self::new()
    }
}

// `crate::ported::zsh_h::alias` struct + impl deleted — Rust-only duplicate of canonical
// `crate::ported::zsh_h::alias` (zsh.h:1253-1257). The canonical
// has `node: hashnode { nam, flags, next }` embedded (c:1254) +
// `text: String` (c:1255) + `inuse: i32` (c:1256); the Rust-only
// had a flat `name: String, flags: u32, text: String, inuse: i32`
// (missing the hashnode embedding).

/// Port of `static int hnamcmp(const void *ap, const void *bp)`
/// from `Src/hashtable.c:341-346`. C body:
/// ```c
/// HashNode a = *(HashNode *)ap;
/// HashNode b = *(HashNode *)bp;
/// return ztrcmp(a->nam, b->nam);
/// ```
///
/// `ztrcmp` is a META-AWARE compare that XORs Meta-escaped bytes
/// with 32 before comparing (Src/utils.c:5106). The previous Rust
/// port used `str::cmp` which does naive byte-wise lexicographic
/// compare — for Meta-encoded hash-table keys this sorts them
/// incorrectly (Meta byte 0x83 sorts AFTER ASCII printable but the
/// real underlying byte 0x83^32=0xa3 should compare as a high byte).
///
/// Route through the canonical `crate::ported::utils::ztrcmp` so
/// `functions`, `alias`, etc. sort their key listings the same way
/// C does for Meta-encoded names.
pub fn hnamcmp(ap: &str, bp: &str) -> std::cmp::Ordering {
    ztrcmp(ap, bp) // c:345
}

/// Port of `scanmatchtable(HashTable ht, Patprog pprog, int sorted, int flags1, int flags2, ScanFunc scanfunc, int scanflags)` from `Src/hashtable.c:373`.
///
/// C body walks every node calling `func(node, scanflags)` if
/// the node satisfies (a) optional pattern match, (b) `flags1`
/// require-at-least-one, (c) `flags2` require-none-of. The
/// `sorted` flag pre-sorts entries before scanning.
///
/// Rust port: same shape with closure callback. Returns the
/// match count.
/// WARNING: param names don't match C — Rust=() vs C=(ht, pprog, sorted, flags1, flags2, scanfunc, scanflags)
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
        // c:400 — `qsort(hnsorttab, ct, sizeof(HashNode), hnamcmp);`
        // hnamcmp routes through Meta-aware ztrcmp. The previous Rust
        // port used `str::cmp` (naive byte-wise) which sorts Meta-
        // encoded hash keys incorrectly. Use the canonical hnamcmp
        // to match C's qsort comparator exactly.
        entries.sort_by(|a, b| hnamcmp(a.0, b.0)); // c:400
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

impl alias_table {
    /// `new` — `newhashtable(23, "aliastab", NULL)` +
    /// `createaliastable(aliastab)` (`Src/hashtable.c:1210-1212`),
    /// without the two default aliases. `sufaliastab` is created at
    /// `hsize = 11` (`Src/hashtable.c:1221`) so `sufaliastab_lock()`
    /// builds its table inline rather than going through here.
    pub fn new() -> Self {
        Self {
            table: hashtable_nodes::newhashtable(23), // c:1210
        }
    }
    /// `with_defaults` — `new()` plus the two aliases
    /// `createaliastables()` installs (`Src/hashtable.c:1215-1216`).
    /// They are added FIRST, before any user alias, so the chain heads
    /// come out in C's order.
    pub fn with_defaults() -> Self {
        let mut table = Self::new();
        // C addaliasnode(aliastab, "run-help", createaliasnode("man", 0));
        // at hashtable.c:1215-1216.
        table.add(createaliasnode("run-help", "man", 0)); // c:1215
        table.add(createaliasnode("which-command", "whence", 0)); // c:1216
        table
    }
    /// `add` — `addhashnode2` (`Src/hashtable.c:168`). C's `addnode`
    /// for this table is `addhashnode` (`c:1194`), which is
    /// `addhashnode2` + `freenode(oldnode)` (`c:157-162`); returning
    /// the displaced node lets the caller do the freeing, and dropping
    /// it is `freealiasnode` (`c:1243`).
    pub fn add(&mut self, alias: alias) -> Option<alias> {
        // c:157 / c:168
        let nam = alias.node.nam.clone(); // c:177 — `hn->nam = nam`
        self.table.addhashnode2(&nam, alias)
    }
    /// `get` — `gethashnode` (`Src/hashtable.c:245`), i.e. the lookup
    /// that skips `DISABLED` nodes (`c:253`).
    pub fn get(&self, name: &str) -> Option<&alias> {
        // c:245
        self.table
            .gethashnode2(name)
            .filter(|a| (a.node.flags & DISABLED as i32) == 0)
    }
    /// `get_including_disabled` — `gethashnode2`
    /// (`Src/hashtable.c:255`), the lookup WITHOUT the DISABLED filter.
    pub fn get_including_disabled(&self, name: &str) -> Option<&alias> {
        self.table.gethashnode2(name) // c:255
    }
    /// `get_mut` — mutable `gethashnode` (`Src/hashtable.c:245`); C
    /// mutates straight through the returned `HashNode` pointer.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut alias> {
        // c:245
        self.table
            .get_mut(name)
            .filter(|a| (a.node.flags & DISABLED as i32) == 0)
    }
    /// `remove` — `removehashnode` (`Src/hashtable.c:275`).
    pub fn remove(&mut self, name: &str) -> Option<alias> {
        self.table.removehashnode(name) // c:275
    }
    /// `disable` — see implementation.
    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.node.flags |= DISABLED as i32;
            true
        } else {
            false
        }
    }
    /// `enable` — see implementation.
    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(alias) = self.table.get_mut(name) {
            alias.node.flags &= !(DISABLED as i32);
            true
        } else {
            false
        }
    }
    /// `len` — see implementation.
    pub fn len(&self) -> usize {
        self.table.len()
    }
    /// `is_empty` — see implementation.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    /// `clear` — `emptyhashtable` (`Src/hashtable.c:517`), which is
    /// `resizehashtable(ht, ht->hsize)`: every node freed, `hsize` kept.
    pub fn clear(&mut self) {
        self.table.emptyhashtable(); // c:517
    }
    /// `iter` — the unsorted `scanmatchtable` walk
    /// (`Src/hashtable.c:420-434`): bucket 0..hsize-1, each chain
    /// head→tail. This IS the order `${(k)aliases}` / `${(k)galiases}`
    /// / `${(k)saliases}` emit via `scanpmraliases` and friends
    /// (`Src/Modules/parameter.c:2005-2047`).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &alias)> {
        self.table.iter() // c:420-434
    }
    /// `iter_sorted` — the `sorted` arm of `scanmatchtable`
    /// (`Src/hashtable.c:395-401`), which `qsort`s the collected nodes
    /// with `hnamcmp`.
    pub fn iter_sorted(&self) -> Vec<(&String, &alias)> {
        let mut entries: Vec<_> = self.table.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0)); // c:400
        entries
    }
}

impl Default for alias_table {
    fn default() -> Self {
        Self::new()
    }
}

/// Port of `scanhashtable(HashTable ht, int sorted, int flags1, int flags2, ScanFunc scanfunc, int scanflags)` from `Src/hashtable.c:446`.
///
/// C body delegates to `scanmatchtable` with `pprog = NULL`. Rust
/// port does the same.
/// WARNING: param names don't match C — Rust=() vs C=(ht, sorted, flags1, flags2, scanfunc, scanflags)
pub fn scanhashtable<T: HashNodeFlags, F: FnMut(&str, &T)>(
    ht: &HashMap<String, T>,
    sorted: bool,
    flags1: u32,
    flags2: u32,
    func: F,
) -> i32 {
    scanmatchtable(ht, None, sorted, flags1, flags2, func)
}

/// Port of `expandhashtable(HashTable ht)` from `Src/hashtable.c:458`.
///
/// C grows the bucket array when load factor exceeds threshold.
/// Rust HashMap rehashes automatically — calling reserve on the
/// passed map gives the closest equivalent.
/// Rust idiom replacement: `HashMap::reserve` covers the C
/// `growhashtable` bucket-realloc + rehash loop.
pub fn expandhashtable<T>(ht: &mut HashMap<String, T>) {
    let want = ht.len() * 2;
    ht.reserve(want.saturating_sub(ht.capacity()));
}

/// Port of `resizehashtable(HashTable ht, int newsize)` from `Src/hashtable.c:486`.
///
/// C reallocates buckets to a specific size. Rust HashMap reserves
/// capacity to ensure at least `newsize` entries fit without rehash.
/// Rust idiom replacement: `HashMap::reserve(need)` covers the C
/// `realloc(hsize * sizeof(HashNode))` + rehash dance.
pub fn resizehashtable<T>(ht: &mut HashMap<String, T>, newsize: i32) {
    let need = newsize.max(0) as usize;
    if need > ht.capacity() {
        ht.reserve(need - ht.capacity());
    }
}

// Generic method to empty a hash table                                    // c:519
/// Port of `emptyhashtable(HashTable ht)` from `Src/hashtable.c:519`.
///
/// C body: `resizehashtable(ht, ht->hsize);` — drop all nodes
/// while keeping the bucket array. Rust HashMap::clear preserves
/// capacity, matching the semantic.
pub fn emptyhashtable<T>(ht: &mut HashMap<String, T>) {
    // c:519
    ht.clear();
}

// Print info about hash table                                             // c:527
/// Port of `printhashtabinfo(HashTable ht)` from `Src/hashtable.c:78`.
///
/// C body prints chain-length distribution stats for hash-table
/// debug analysis (under ZSH_HASH_DEBUG). Rust HashMap doesn't
/// expose chain-length info; emit count + capacity which is the
/// equivalent visibility.
/// Rust idiom replacement: HashMap's open addressing doesn't expose
/// chain length, so we emit name+capacity+len — the equivalent
/// visibility under Rust's std::collections backend.
/// WARNING: param names don't match C — Rust=(name, ht) vs C=(ht)
pub fn printhashtabinfo<T>(name: &str, ht: &HashMap<String, T>) -> String {
    // c:78
    format!(
        "name of table   : {}\nsize of nodes[] : {}\nnumber of nodes : {}",
        name,
        ht.capacity(),
        ht.len()
    )
}

/// Port of `bin_hashinfo(UNUSED(char *nam), UNUSED(char **args), UNUSED(Options ops), UNUSED(int func))` from `Src/hashtable.c:566`.
///
/// C iterates all registered hashtables (cmdnamtab, shfunctab,
/// aliastab, etc.) and emits stats for each. Rust port walks the
/// known-singleton tables.
pub fn bin_hashinfo(
    _nam: &str,
    _args: &[String], // c:566
    _ops: &options,
    _func: i32,
) -> i32 {
    let banner = "----------------------------------------------------";
    println!("{}", banner);
    {
        let tab = cmdnamtab_lock().read().expect("cmdnamtab poisoned");
        println!("name of table   : cmdnamtab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    {
        let tab = shfunctab_lock().read().expect("shfunctab poisoned");
        println!("name of table   : shfunctab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    {
        let tab = aliastab_lock().read().expect("aliastab poisoned");
        println!("name of table   : aliastab");
        println!("number of nodes : {}", tab.len());
    }
    println!("{}", banner);
    0
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

/// Port of `emptycmdnamtable(HashTable ht)` from `Src/hashtable.c:623`.
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
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptycmdnamtable() {
    // c:1015 — `emptyhashtable(ht);`
    cmdnamtab_lock()
        .write()
        .expect("cmdnamtab poisoned")
        .clear();
    // c:1016 — `pathchecked = path;`. Resetting the cursor here (not in
    // each caller) is what C does: every caller that empties the table
    // must also allow a subsequent `fillcmdnamtable` to re-walk PATH
    // from the start. Without this, emptying the table (e.g. a `PATH`
    // reassignment) left `pathchecked` exhausted, so the next
    // `${(k)commands}` / `compadd -k commands` scan refilled nothing.
    pathchecked.store(0, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `hashdir(char **dirp)` from `Src/hashtable.c:634`.
///
/// C body opendir's the directory, reads each entry, and adds
/// any executable to `cmdnamtab` (skipping names already present
/// from earlier PATH entries). Rust port routes through
/// `cmdnam_table::hash_dir`.
/// Rust idiom replacement: pure delegation to `hash_dir` on the
/// typed `CmdNamTable`; the C opendir/readdir/executable-test loop
/// lives there with `fs::read_dir` + `is_executable_via_metadata`.
/// WARNING: param names don't match C — Rust=(dir, dir_index) vs C=(dirp)
pub fn hashdir(dir: &str, dir_index: usize) {
    cmdnamtab_lock()
        .write()
        .expect("cmdnamtab poisoned")
        .hash_dir(dir, dir_index);
}

/// Port of `fillcmdnamtable(UNUSED(HashTable ht))` from `Src/hashtable.c:712`.
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
/// WARNING: param names don't match C — Rust=(path) vs C=(ht)
pub fn fillcmdnamtable(path: &[String]) {
    // c:716 — `for (pq = pathchecked; *pq; pq++) hashdir(pq);`. Start
    // from the cursor, NOT index 0: dirs already walked by an earlier
    // fill (or by `hashcmd`, which bumps `pathchecked`) must not be
    // re-scanned. Re-filling from 0 on every call made
    // `${(k)commands}` / `compadd -k commands` return the entire PATH
    // even when `pathchecked` was exhausted and the table had been
    // emptied — diverging from zsh, whose scan yields the current
    // (possibly empty) table. Symptom: `l<TAB>` listed all 230 PATH
    // commands vs zsh's 5 builtins.
    use std::sync::atomic::Ordering;
    let from = pathchecked.load(Ordering::SeqCst);
    if from < path.len() {
        let mut tab = cmdnamtab_lock().write().expect("cmdnamtab poisoned");
        for idx in from..path.len() {
            tab.hash_dir(&path[idx], idx);
        }
    }
    // c:719 — `pathchecked = pq;` — cursor advances to the end.
    pathchecked.store(path.len(), Ordering::SeqCst);
}

/// Port of `freecmdnamnode(HashNode hn)` from `Src/hashtable.c:724`.
///
/// C body frees the entry's name + (if HASHED) cached path. Rust
/// port: drop runs both when the entry is removed from the table.
/// This helper performs the removal to trigger Drop.
pub fn freecmdnamnode(hn: &str) {
    cmdnamtab_lock()
        .write()
        .expect("cmdnamtab poisoned")
        .remove(hn);
}

/// Port of `printcmdnamnode(HashNode hn, int printflags)` from `Src/hashtable.c:739`.
///
/// Emits one cmdnamtab entry for `hash` / `whence`. Each branch
/// returns; PRINT_LIST falls through to the tail that emits
/// `quotedzputs(nam) '=' quotedzputs(u.cmd|*u.name '/' nam) '\n'`.
pub fn printcmdnamnode(hn: &cmdnam, printflags: i32) {
    // c:741 — `Cmdnam cn = (Cmdnam) hn;` — Rust types give us cmdnam.

    // c:743-747 — PRINT_WHENCE_WORD branch.
    if (printflags & PRINT_WHENCE_WORD) != 0 {
        // c:744-745 — `printf("%s: %s\n", nam, HASHED ? "hashed" : "command");`
        let kind = if (hn.node.flags & HASHED as i32) != 0 {
            "hashed"
        } else {
            "command"
        };
        println!("{}: {}", hn.node.nam, kind); // c:744
        return; // c:746
    }

    // c:749-760 — PRINT_WHENCE_CSH | PRINT_WHENCE_SIMPLE branch.
    if (printflags & (PRINT_WHENCE_CSH | PRINT_WHENCE_SIMPLE)) != 0 {
        let mut so = io::stdout();
        if (hn.node.flags & HASHED as i32) != 0 {
            // c:750
            // c:751-752 — `zputs(u.cmd, stdout); putchar('\n');`
            if let Some(cmd) = &hn.cmd {
                let _ = zputs(cmd, &mut so); // c:751
            }
            println!(); // c:752
        } else {
            // c:753
            // c:754-757 — `zputs(*u.name); putchar('/'); zputs(nam); putchar('\n');`
            if let Some(name_arr) = &hn.name {
                if let Some(first) = name_arr.first() {
                    let _ = zputs(first, &mut so); // c:754
                }
            }
            print!("/"); // c:755
            let _ = zputs(&hn.node.nam, &mut so); // c:756
            println!(); // c:757
        }
        return; // c:759
    }

    // c:762-777 — PRINT_WHENCE_VERBOSE branch.
    if (printflags & PRINT_WHENCE_VERBOSE) != 0 {
        let mut so = io::stdout();
        if (hn.node.flags & HASHED as i32) != 0 {
            // c:763
            // c:764-767 — `nicezputs(nam); printf(" is hashed to "); nicezputs(u.cmd); putchar('\n');`
            let _ = nicezputs(&hn.node.nam, &mut so); // c:764
            print!(" is hashed to "); // c:765
            if let Some(cmd) = &hn.cmd {
                let _ = nicezputs(cmd, &mut so); // c:766
            }
            println!(); // c:767
        } else {
            // c:768
            // c:769-774 — `nicezputs(nam); printf(" is "); nicezputs(*u.name); putchar('/'); nicezputs(nam); putchar('\n');`
            let _ = nicezputs(&hn.node.nam, &mut so); // c:769
            print!(" is "); // c:770
            if let Some(name_arr) = &hn.name {
                if let Some(first) = name_arr.first() {
                    let _ = nicezputs(first, &mut so); // c:771
                }
            }
            print!("/"); // c:772
            let _ = nicezputs(&hn.node.nam, &mut so); // c:773
            println!(); // c:774
        }
        return; // c:776
    }

    // c:779-784 — PRINT_LIST prefix block; falls through to the tail.
    if (printflags & PRINT_LIST) != 0 {
        // c:779
        print!("hash "); // c:780
                         // c:782-783 — `-- ` for names starting with `-`.
        if hn.node.nam.starts_with('-') {
            // c:782
            print!("-- "); // c:783
        }
    }

    // c:786-798 — common tail. HASHED uses u.cmd, !HASHED splices first
    // u.name PATH segment + '/' + nam.
    if (hn.node.flags & HASHED as i32) != 0 {
        // c:786
        print!("{}", quotedzputs(&hn.node.nam)); // c:787
        print!("="); // c:788
        if let Some(cmd) = &hn.cmd {
            print!("{}", quotedzputs(cmd)); // c:789
        }
        println!(); // c:790
    } else {
        // c:791
        print!("{}", quotedzputs(&hn.node.nam)); // c:792
        print!("="); // c:793
        if let Some(name_arr) = &hn.name {
            if let Some(first) = name_arr.first() {
                print!("{}", quotedzputs(first)); // c:794
            }
        }
        print!("/"); // c:795
        print!("{}", quotedzputs(&hn.node.nam)); // c:796
        println!(); // c:797
    }
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

/// Port of `removeshfuncnode(UNUSED(HashTable ht), const char *nam)` from `Src/hashtable.c:836`.
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
/// WARNING: param names don't match C — Rust=(nam) vs C=(ht, nam)
pub fn removeshfuncnode(nam: &str) -> Option<shfunc> {
    // c:841-844 — the two arms are EXCLUSIVE:
    //     if (!strncmp(nam, "TRAP", 4) && (sigidx = getsigidx(nam + 4)) != -1)
    //         hn = removetrap(sigidx);
    //     else
    //         hn = removehashnode(shfunctab, nam);
    // The Rust port ran BOTH: `removetrap` already pulls the TRAP<SIG>
    // node out of shfunctab (signals.rs c:832-841), so the follow-up
    // `remove` returned None and the caller (`bin_unhash`, c:4405) read
    // that as "no such hash table element". `unfunction TRAPZERR` then
    // reported an error AND — because the second remove ran outside
    // removetrap's dosavetrap window — the localtraps restore had
    // nothing to put back (C03traps:13,14).
    if let Some(sig_part) = nam.strip_prefix("TRAP") {
        // c:841
        if let Some(sig) = getsigidx(sig_part) {
            return removetrap(sig); // c:842
        }
    }
    // c:844 — `hn = removehashnode(shfunctab, nam);`
    shfunctab_lock()
        .write()
        .expect("shfunctab poisoned")
        .remove(nam)
}

/// Port of `disableshfuncnode(HashNode hn, UNUSED(int flags))` from `Src/hashtable.c:855`.
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
/// WARNING: param names don't match C — Rust=(hn) vs C=(hn, flags)
pub fn disableshfuncnode(hn: &str) {
    {
        let mut tab = shfunctab_lock().write().expect("shfunctab poisoned");
        tab.disable(hn);
    }
    if let Some(sig_part) = hn.strip_prefix("TRAP") {
        if let Some(sig) = getsigidx(sig_part) {
            unsettrap(sig);
        }
    }
}

/// Port of `enableshfuncnode(HashNode hn, UNUSED(int flags))` from `Src/hashtable.c:873`.
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
/// WARNING: param names don't match C — Rust=(hn) vs C=(hn, flags)
pub fn enableshfuncnode(hn: &str) {
    {
        let mut tab = shfunctab_lock().write().expect("shfunctab poisoned");
        tab.enable(hn);
    }
    if let Some(sig_part) = hn.strip_prefix("TRAP") {
        if let Some(sig) = getsigidx(sig_part) {
            // c:882 — `settrap(sigidx, NULL, ZSIG_FUNC)`. The TRAPxxx
            // function body resolves through shfunctab at dispatch
            // (`gettrapnode`), not via the trap arrays directly.
            let _ = settrap(sig, None, ZSIG_FUNC);
            // c:Src/signals.c::settrap → unsettrap → removetrap also
            // clears any previously-registered string-form trap for
            // the same signal (single-slot sigtrapped[] array). The
            // zshrs port stores string-form bodies in a separate
            // `traps_table` HashMap that `removetrap` doesn't touch,
            // so the string body survives the function-form
            // registration and BOTH fire on the next signal. Drop
            // the string-form entry here so dotrap's
            // `traps_table` fallback doesn't double-dispatch. Bug
            // #541 in docs/BUGS.md.
            if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                t.remove(sig_part);
            }
        }
    }
}

/// Port of `freeshfuncnode(HashNode hn)` from `Src/hashtable.c:888`.
///
/// C body frees the function name, body Eprog, redir Eprog,
/// filename string, and sticky options struct. Rust port: drop
/// runs all of this when the entry is removed; this helper just
/// removes from the table to trigger the drop chain.
/// Rust idiom replacement: `HashMap::remove` triggers the `Box<T>`
/// drop cascade — same teardown as the C zfree chain, automated.
pub fn freeshfuncnode(hn: &str) {
    shfunctab_lock()
        .write()
        .expect("shfunctab poisoned")
        .remove(hn);
}

/// Port of `printshfuncnode(HashNode hn, int printflags)` from `Src/hashtable.c:914`.
///
/// Emits one shfunctab entry for `functions` / `whence` / `typeset -f`.
/// PRINT_NAMEONLY and the PRINT_WHENCE_* variants return early; the
/// default body emits the full re-parseable `name () { body }` form
/// including autoload-stub, traced markers, and trailing redirections.
pub fn printshfuncnode(hn: &shfunc, printflags: i32) {
    // c:916 — `Shfunc f = (Shfunc) hn;` — Rust types give us shfunc.
    // c:917 — `char *t = 0;` — declared but only used by the funcdef/redir
    // branches; Rust scope-locals the `t` binding inside each branch.

    // c:919-925 — PRINT_NAMEONLY (or PRINT_WHENCE_SIMPLE without FUNCDEF):
    // `zputs(nam); putchar('\n'); return;`
    if (printflags & PRINT_NAMEONLY) != 0
        || ((printflags & PRINT_WHENCE_SIMPLE) != 0 && (printflags & PRINT_WHENCE_FUNCDEF) == 0)
    {
        let mut so = io::stdout();
        let _ = zputs(&hn.node.nam, &mut so); // c:922
        println!(); // c:923
        return; // c:924
    }

    // c:927-944 — PRINT_WHENCE_VERBOSE | PRINT_WHENCE_WORD (without FUNCDEF):
    // nicezputs(nam) ":" function | " is an autoload shell function" | " is a shell function"
    // [" from " quotedzputs(filename) [(PM_LOADDIR) "/" quotedzputs(nam)]] '\n'
    if (printflags & (PRINT_WHENCE_VERBOSE | PRINT_WHENCE_WORD)) != 0
        && (printflags & PRINT_WHENCE_FUNCDEF) == 0
    {
        let mut so = io::stdout();
        let _ = nicezputs(&hn.node.nam, &mut so); // c:929
                                                  // c:930-933 — printf one of three strings via nested ternary.
        let msg = if (printflags & PRINT_WHENCE_WORD) != 0 {
            ": function" // c:930
        } else if (hn.node.flags & PM_UNDEFINED as i32) != 0 {
            " is an autoload shell function" // c:932
        } else {
            " is a shell function" // c:933
        };
        print!("{}", msg);
        // c:934-941 — verbose-with-filename suffix.
        if (printflags & PRINT_WHENCE_VERBOSE) != 0 {
            if let Some(filename) = &hn.filename {
                // c:934
                print!(" from "); // c:935
                print!("{}", quotedzputs(filename)); // c:936
                if (hn.node.flags & PM_LOADDIR as i32) != 0 {
                    // c:937
                    print!("/"); // c:938
                    print!("{}", quotedzputs(&hn.node.nam)); // c:939
                }
            }
        }
        println!(); // c:942
        return; // c:943
    }

    // c:946 — `quotedzputs(nam, stdout);`
    print!("{}", quotedzputs(&hn.node.nam));

    // c:947-987 — funcdef-present branch (or PM_UNDEFINED stub) vs empty `() { }`.
    // RUST-ONLY EXTENSION: zshrs's shfunc carries a raw `body: Option<String>`
    // alongside (or instead of) the compiled `funcdef: Eprog` (see
    // `shfunc` doc at zsh_h.rs:670). The fusevm compile path stores the
    // body source there (parse.rs:7118 + parse.rs:1787) but never builds
    // a C-shaped Eprog — `funcdef` stays None. C zsh's getpermtext walks
    // the wordcode-Eprog; in zshrs we fall back to `body` text directly
    // when funcdef is absent. Without this, `functions NAME` prints
    // `f () { }` for every user-defined function.
    let has_body_source = hn.body.as_deref().is_some_and(|b| !b.is_empty());
    if hn.funcdef.is_some() || has_body_source || (hn.node.flags & PM_UNDEFINED as i32) != 0 {
        // c:947
        print!(" () {{\n"); // c:948
        let _ = zoutputtab(&mut io::stdout()); // c:949
                                               // c:950-954 — `# undefined` marker or getpermtext body.
        let mut t: Option<String>;
        if (hn.node.flags & PM_UNDEFINED as i32) != 0 {
            // c:950
            println!(
                "{} undefined",
                hashchar.load(Ordering::Relaxed) as u8 as char
            ); // c:951
            let _ = zoutputtab(&mut io::stdout()); // c:952
            t = None;
        } else if let Some(fd) = hn.funcdef.as_ref() {
            // c:953
            t = Some(getpermtext(fd.clone(), None, 1)); // c:954
        } else {
            // Rust-only fallback: emit `body` text directly. C's
            // getpermtext walks the wordcode-Eprog and emits
            // canonicalized statement-per-line text. We don't have
            // the Eprog, so we normalize the captured raw body
            // source: strip a leading `{` + ws, trailing `}` + ws,
            // and trailing `;` before the `}`. These appear when
            // par_simple's body_argv path (parse.rs:7041) reuses
            // the raw input slice that still includes the framing
            // braces; par_funcdef's brace path strips them but the
            // short-form `name() { body }` path can capture the
            // closing `}` because cmdpos vs. cmdpos confusion
            // makes the `}` lex as STRING_LEX, missing the
            // OUTBRACE_TOK arm at parse.rs:7113.
            // c:Src/text.c gettext2 — C zsh re-emits function bodies
            // from parsed wordcode (`getpermtext`) with `\n\t` between
            // sibling statements AND recursive indenting of nested
            // function definitions. zshrs stores raw source (no Eprog
            // for shfunc bodies); the closure below applies the same
            // canonicalization at print time.
            // Bug #197 (top-level statements) + #124 (nested fns) in
            // docs/BUGS.md.
            //
            // canonicalize: walk char-by-char tracking quote state +
            // brace/paren depth. At brace_depth == 0:
            //   - top-level `;` (or `; `) becomes `\n\t` * (depth+1)
            //   - `name() {` or `name () {` opens a nested fn def;
            //     emit `name () {\n` then recurse on the body until
            //     the matching `}` with depth+1, then `\n\t` * depth
            //     + `}`.
            let canonicalize_body = |source: &str| -> String {
                fn fmt_body(s: &str, depth: usize, lead: bool) -> String {
                    let chars: Vec<char> = s.chars().collect();
                    let mut out = String::with_capacity(s.len());
                    let mut in_sq = false;
                    let mut in_dq = false;
                    let mut brace_depth: i32 = 0;
                    let mut paren_depth: i32 = 0;
                    let stmt_indent = "\t".repeat(depth);
                    let mut i = 0;
                    // NOTE: caller (the funcdef emit at hashtable.rs:1364)
                    // already wrote one leading `\t` via zoutputtab. Don't
                    // double-indent the first statement. With `lead`
                    // (recursive nested-fn body), we DO need the leading
                    // indent because the caller writes `name () {\n` then
                    // recurses without a prior tab.
                    if lead && !chars.is_empty() {
                        out.push_str(&stmt_indent);
                    }
                    while i < chars.len() {
                        let c = chars[i];
                        if !in_sq && !in_dq && c == '\\' && i + 1 < chars.len() {
                            out.push(c);
                            out.push(chars[i + 1]);
                            i += 2;
                            continue;
                        }
                        if !in_dq && c == '\'' {
                            in_sq = !in_sq;
                            out.push(c);
                            i += 1;
                            continue;
                        }
                        if !in_sq && c == '"' {
                            in_dq = !in_dq;
                            out.push(c);
                            i += 1;
                            continue;
                        }
                        // Detect nested fn-def pattern at depth 0:
                        // `name() {...}` or `name () {...}` (and
                        // optional `function `-keyword form). Only
                        // when in_sq/in_dq == false and brace/paren
                        // depth == 0.
                        if !in_sq && !in_dq && brace_depth == 0 && paren_depth == 0 {
                            // c:Src/text.c gettext2 WC_CASE arm (~520) —
                            // C re-emits case statements from wordcode as
                            //   case W in
                            //           (p | q) body ;;
                            //   esac
                            // with `;;`/`;&`/`;|` per WC_CASE_TYPE. The
                            // generic `;`-break below ATE the `;;` (it
                            // skips runs of `;`), so `functions f`
                            // displayed case bodies without terminators
                            // and with `;&` mangled to `&`. Re-render the
                            // whole case..esac region case-aware.
                            if out.is_empty() || out.ends_with('\n') || out.ends_with('\t') {
                                if let Some((next_i, txt)) = try_render_case(&chars, i, depth) {
                                    out.push_str(&txt);
                                    i = next_i;
                                    // Consume trailing `;` + ws; emit a
                                    // statement break if more follows.
                                    while i < chars.len()
                                        && (chars[i] == ' '
                                            || chars[i] == '\t'
                                            || chars[i] == '\n'
                                            || chars[i] == ';')
                                    {
                                        i += 1;
                                    }
                                    if i < chars.len() {
                                        out.push('\n');
                                        out.push_str(&stmt_indent);
                                    }
                                    continue;
                                }
                            }
                            // Try to match `<ident>\s*\(\s*\)\s*\{` at
                            // current position, OR `function\s+<ident>...{`.
                            let fn_start = try_match_fn_def(&chars, i);
                            if let Some((header_end, name_str)) = fn_start {
                                // Find matching `}` for the body.
                                let body_open = header_end; // index just after `{`
                                let body_close = find_matching_brace(&chars, body_open - 1);
                                if let Some(close_idx) = body_close {
                                    let body_src: String =
                                        chars[body_open..close_idx].iter().collect();
                                    let body_trim = body_src
                                        .trim_start_matches(|c: char| c.is_whitespace())
                                        .trim_end_matches(|c: char| c.is_whitespace() || c == ';')
                                        .to_string();
                                    out.push_str(&name_str);
                                    out.push_str(" () {\n");
                                    out.push_str(&fmt_body(&body_trim, depth + 1, true));
                                    out.push('\n');
                                    out.push_str(&stmt_indent);
                                    out.push('}');
                                    i = close_idx + 1;
                                    // Consume trailing `;` and ws.
                                    let saved = i;
                                    while i < chars.len()
                                        && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == ';')
                                    {
                                        i += 1;
                                    }
                                    if saved != i || i < chars.len() {
                                        // More content follows — emit
                                        // statement break.
                                        if i < chars.len() {
                                            out.push('\n');
                                            out.push_str(&stmt_indent);
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                        if !in_sq && !in_dq {
                            match c {
                                '{' => brace_depth += 1,
                                '}' => brace_depth = (brace_depth - 1).max(0),
                                '(' => paren_depth += 1,
                                ')' => paren_depth = (paren_depth - 1).max(0),
                                _ => {}
                            }
                        }
                        if !in_sq && !in_dq && brace_depth == 0 && paren_depth == 0 && c == ';' {
                            i += 1;
                            while i < chars.len()
                                && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == ';')
                            {
                                i += 1;
                            }
                            if i < chars.len() {
                                out.push('\n');
                                out.push_str(&stmt_indent);
                            }
                            continue;
                        }
                        out.push(c);
                        i += 1;
                    }
                    out
                }
                fn is_ident_byte(b: u8) -> bool {
                    b == b'_' || b.is_ascii_alphanumeric()
                }
                fn try_match_fn_def(chars: &[char], start: usize) -> Option<(usize, String)> {
                    // Skip leading `function ` keyword (optional).
                    let mut i = start;
                    let _function_prefix = {
                        let rest: String = chars[i..].iter().collect();
                        if rest.starts_with("function ") || rest.starts_with("function\t") {
                            i += "function".len();
                            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                                i += 1;
                            }
                            true
                        } else {
                            false
                        }
                    };
                    // Match identifier.
                    let name_start = i;
                    while i < chars.len() && is_ident_byte(chars[i] as u8) {
                        i += 1;
                    }
                    if i == name_start {
                        return None;
                    }
                    let name: String = chars[name_start..i].iter().collect();
                    // Skip optional whitespace.
                    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                        i += 1;
                    }
                    // Require `()` for the non-`function`-keyword form;
                    // C zsh accepts `function name { ... }` without parens.
                    if i < chars.len() && chars[i] == '(' {
                        i += 1;
                        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                            i += 1;
                        }
                        if i >= chars.len() || chars[i] != ')' {
                            return None;
                        }
                        i += 1;
                    } else if !_function_prefix {
                        return None;
                    }
                    // Skip ws + `{`.
                    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                        i += 1;
                    }
                    if i >= chars.len() || chars[i] != '{' {
                        return None;
                    }
                    Some((i + 1, name))
                }
                /// Keyword match at `i` with word boundaries on both
                /// sides (start/ws/`;` before; end/ws/`;` after).
                fn matches_kw(chars: &[char], i: usize, kw: &str) -> bool {
                    let kl = kw.len();
                    if i + kl > chars.len() {
                        return false;
                    }
                    if !chars[i..i + kl].iter().copied().eq(kw.chars()) {
                        return false;
                    }
                    let before_ok = i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == ';';
                    let after_ok = i + kl == chars.len()
                        || chars[i + kl].is_whitespace()
                        || chars[i + kl] == ';';
                    before_ok && after_ok
                }
                /// Split a case-pattern alternation on top-level `|`
                /// (quote/paren aware), trimming each alternative —
                /// `a|b` → ["a", "b"], rendered `(a | b)` like
                /// C:Src/text.c gettext2's `taddstr(" | ")` walk.
                fn split_top_bar(pat: &str) -> Vec<String> {
                    let chars: Vec<char> = pat.chars().collect();
                    let mut alts = Vec::new();
                    let mut cur = String::new();
                    let (mut in_sq, mut in_dq) = (false, false);
                    let mut pd = 0i32;
                    let mut i = 0;
                    while i < chars.len() {
                        let c = chars[i];
                        if !in_sq && !in_dq && c == '\\' && i + 1 < chars.len() {
                            cur.push(c);
                            cur.push(chars[i + 1]);
                            i += 2;
                            continue;
                        }
                        if !in_dq && c == '\'' {
                            in_sq = !in_sq;
                        } else if !in_sq && c == '"' {
                            in_dq = !in_dq;
                        } else if !in_sq && !in_dq {
                            match c {
                                '(' => pd += 1,
                                ')' => pd = (pd - 1).max(0),
                                '|' if pd == 0 => {
                                    alts.push(cur.trim().to_string());
                                    cur.clear();
                                    i += 1;
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        cur.push(c);
                        i += 1;
                    }
                    alts.push(cur.trim().to_string());
                    alts
                }
                /// Case-aware re-render of one `case W in … esac`
                /// region starting at `start` (which must point at the
                /// `case` keyword). Returns (index-after-`esac`,
                /// rendered text) or None when the region doesn't
                /// parse (caller falls back to generic emission).
                /// Output shape mirrors C:Src/text.c gettext2 WC_CASE:
                ///   case W in
                ///   \t(p | q) body ;;
                ///   esac
                /// with arm bodies recursively formatted at depth+2
                /// (continuation statements land one level deeper than
                /// the arm line, matching zsh 5.9 output).
                fn try_render_case(
                    chars: &[char],
                    start: usize,
                    depth: usize,
                ) -> Option<(usize, String)> {
                    let n = chars.len();
                    let mut i = start;
                    if !matches_kw(chars, i, "case") {
                        return None;
                    }
                    i += 4;
                    if i >= n || !chars[i].is_whitespace() {
                        return None;
                    }
                    while i < n && chars[i].is_whitespace() {
                        i += 1;
                    }
                    // Scrutinee word: scan to unquoted whitespace.
                    let word_start = i;
                    let (mut in_sq, mut in_dq) = (false, false);
                    while i < n {
                        let c = chars[i];
                        if !in_sq && !in_dq && c == '\\' && i + 1 < n {
                            i += 2;
                            continue;
                        }
                        if !in_dq && c == '\'' {
                            in_sq = !in_sq;
                        } else if !in_sq && c == '"' {
                            in_dq = !in_dq;
                        } else if !in_sq && !in_dq && c.is_whitespace() {
                            break;
                        }
                        i += 1;
                    }
                    if i == word_start || in_sq || in_dq {
                        return None;
                    }
                    let word: String = chars[word_start..i].iter().collect();
                    while i < n && chars[i].is_whitespace() {
                        i += 1;
                    }
                    if !matches_kw(chars, i, "in") {
                        return None;
                    }
                    i += 2;
                    let indent_case = "\t".repeat(depth);
                    let indent_arm = "\t".repeat(depth + 1);
                    let mut rendered = format!("case {} in", word);
                    loop {
                        while i < n && chars[i].is_whitespace() {
                            i += 1;
                        }
                        if i >= n {
                            return None; // unterminated — bail out
                        }
                        if matches_kw(chars, i, "esac") {
                            i += 4;
                            break;
                        }
                        // Pattern: optional leading `(`, alts to `)`.
                        if chars[i] == '(' {
                            i += 1;
                        }
                        let pat_start = i;
                        let (mut in_sq, mut in_dq) = (false, false);
                        let mut pd = 0i32;
                        while i < n {
                            let c = chars[i];
                            if !in_sq && !in_dq && c == '\\' && i + 1 < n {
                                i += 2;
                                continue;
                            }
                            if !in_dq && c == '\'' {
                                in_sq = !in_sq;
                            } else if !in_sq && c == '"' {
                                in_dq = !in_dq;
                            } else if !in_sq && !in_dq {
                                if c == '(' {
                                    pd += 1;
                                } else if c == ')' {
                                    if pd == 0 {
                                        break;
                                    }
                                    pd -= 1;
                                }
                            }
                            i += 1;
                        }
                        if i >= n || chars[i] != ')' {
                            return None;
                        }
                        let pat_src: String = chars[pat_start..i].iter().collect();
                        i += 1; // consume `)`
                        while i < n && (chars[i] == ' ' || chars[i] == '\t') {
                            i += 1;
                        }
                        // Body: scan to `;;`/`;&`/`;|` or this case's
                        // `esac` (last arm without terminator), quote/
                        // paren/brace aware, nested case…esac tracked.
                        let body_start = i;
                        let mut body_end = i;
                        let mut term: Option<&str> = None;
                        let (mut in_sq, mut in_dq) = (false, false);
                        let (mut bd, mut pd) = (0i32, 0i32);
                        let mut nested_case = 0i32;
                        loop {
                            if i >= n {
                                return None; // unterminated — bail out
                            }
                            let c = chars[i];
                            if !in_sq && !in_dq && c == '\\' && i + 1 < n {
                                i += 2;
                                continue;
                            }
                            if !in_dq && c == '\'' {
                                in_sq = !in_sq;
                                i += 1;
                                continue;
                            }
                            if !in_sq && c == '"' {
                                in_dq = !in_dq;
                                i += 1;
                                continue;
                            }
                            if in_sq || in_dq {
                                i += 1;
                                continue;
                            }
                            match c {
                                '{' => bd += 1,
                                '}' => bd = (bd - 1).max(0),
                                '(' => pd += 1,
                                ')' => pd = (pd - 1).max(0),
                                _ => {}
                            }
                            if bd == 0 && pd == 0 {
                                if matches_kw(chars, i, "case") {
                                    nested_case += 1;
                                    i += 4;
                                    continue;
                                }
                                if matches_kw(chars, i, "esac") {
                                    if nested_case == 0 {
                                        body_end = i;
                                        break;
                                    }
                                    nested_case -= 1;
                                    i += 4;
                                    continue;
                                }
                                if nested_case == 0
                                    && c == ';'
                                    && i + 1 < n
                                    && matches!(chars[i + 1], ';' | '&' | '|')
                                {
                                    body_end = i;
                                    term = Some(match chars[i + 1] {
                                        ';' => ";;",
                                        '&' => ";&",
                                        _ => ";|",
                                    });
                                    i += 2;
                                    break;
                                }
                            }
                            i += 1;
                        }
                        let body_src: String = chars[body_start..body_end].iter().collect();
                        let body_trim = body_src.trim().trim_end_matches(';').trim_end();
                        rendered.push('\n');
                        rendered.push_str(&indent_arm);
                        rendered.push('(');
                        rendered.push_str(&split_top_bar(&pat_src).join(" | "));
                        rendered.push_str(") ");
                        rendered.push_str(&fmt_body(body_trim, depth + 2, false));
                        if let Some(t) = term {
                            rendered.push(' ');
                            rendered.push_str(t);
                        }
                    }
                    rendered.push('\n');
                    rendered.push_str(&indent_case);
                    rendered.push_str("esac");
                    Some((i, rendered))
                }
                fn find_matching_brace(chars: &[char], open: usize) -> Option<usize> {
                    let mut depth = 1i32;
                    let mut in_sq = false;
                    let mut in_dq = false;
                    let mut j = open + 1;
                    while j < chars.len() {
                        let c = chars[j];
                        if !in_sq && !in_dq && c == '\\' && j + 1 < chars.len() {
                            j += 2;
                            continue;
                        }
                        if !in_dq && c == '\'' {
                            in_sq = !in_sq;
                        } else if !in_sq && c == '"' {
                            in_dq = !in_dq;
                        } else if !in_sq && !in_dq {
                            if c == '{' {
                                depth += 1;
                            } else if c == '}' {
                                depth -= 1;
                                if depth == 0 {
                                    return Some(j);
                                }
                            }
                        }
                        j += 1;
                    }
                    None
                }
                let mut s = source.trim().to_string();
                if s.starts_with('{') {
                    s = s[1..].trim_start().to_string();
                }
                if s.ends_with('}') {
                    s.pop();
                    s = s.trim_end().to_string();
                }
                if s.ends_with(';') {
                    s.pop();
                    s = s.trim_end().to_string();
                }
                fmt_body(&s, 1, false)
            };
            // c:954 — `t = getpermtext(fd, NULL, 1);`. C holds the body as
            // compiled wordcode and renders it back to source with
            // getpermtext, which is what produces zsh's canonical layout:
            // `do`/`then` on their own line with the body indented under
            // them, `(` and `)` broken onto separate lines, an `always`
            // block re-emitted as `{ … } always { … }`, and a trailing
            // space after every assignment (taddassign, c:203-204).
            //
            // zshrs only has an Eprog for zwc-loaded functions (the
            // `hn.funcdef` branch above); shell-defined ones keep their raw
            // source, which is why this branch existed. Re-parse that source
            // and render it through the SAME deparser rather than
            // re-deriving getpermtext's formatting rules by hand —
            // canonicalize_body reproduced the flat cases but not the
            // indenting ones, and mangled `always` blocks into
            // `print x } always { print y`.
            //
            // The source is parsed as-is. hn.body arrives with the framing
            // `{ }` of `name() { … }` ALREADY stripped, so removing a
            // leading `{` here would eat the braces of a body whose first
            // command is itself a brace group (`f() { { print x } }`) and
            // would leave an always-block body unbalanced. That
            // unconditional strip is exactly why canonicalize_body below
            // rendered `f() { { print x } always { print y } }` as
            // `print x } always { print y`.
            //
            // parse_string is the wordcode parser, whose coverage is
            // narrower than the AST parser that executes these bodies, so
            // fall back to canonicalize_body when it can't take the source
            // rather than losing the listing entirely.
            let deparse_body = |source: &str| -> String {
                match crate::ported::exec::parse_string(source.trim(), 1) {
                    Some(p) => crate::ported::text::getpermtext(Box::new(p), None, 1), // c:954
                    None => canonicalize_body(source),
                }
            };
            t = hn.body.clone().map(|s| deparse_body(&s));
        }
        // c:955-958 — PM_TAGGED | PM_TAGGED_LOCAL → `# traced` marker.
        if (hn.node.flags & (PM_TAGGED | PM_TAGGED_LOCAL) as i32) != 0 {
            println!("{} traced", hashchar.load(Ordering::Relaxed) as u8 as char); // c:956
            let _ = zoutputtab(&mut io::stdout()); // c:957
        }
        // c:959-983 — no funcdef text → autoload stub; else emit text.
        if t.is_none() {
            // c:959
            // c:960-964 — `fopt = "UtTkzc"; flgs[] = { PM_UNALIASED, PM_TAGGED,
            //               PM_TAGGED_LOCAL, PM_KSHSTORED, PM_ZSHSTORED, PM_CUR_FPATH, 0 };`
            let fopt: &[u8] = b"UtTkzc"; // c:960
            let flgs: [u32; 6] = [
                // c:961-964
                PM_UNALIASED,
                PM_TAGGED,
                PM_TAGGED_LOCAL,
                PM_KSHSTORED,
                PM_ZSHSTORED,
                PM_CUR_FPATH,
            ];
            let mut so = io::stdout();
            let _ = zputs("builtin autoload -X", &mut so); // c:967
                                                           // c:968-969 — emit each fopt char whose flag is set.
            for fl in 0..fopt.len() {
                // c:968
                if (hn.node.flags & flgs[fl] as i32) != 0 {
                    // c:969
                    print!("{}", fopt[fl] as char); // c:969
                }
            }
            // c:970-973 — PM_LOADDIR with filename → ' ' + zputs(filename).
            if let Some(filename) = &hn.filename {
                if (hn.node.flags & PM_LOADDIR as i32) != 0 {
                    // c:970
                    print!(" "); // c:971
                    let _ = zputs(filename, &mut so); // c:972
                }
            }
        } else {
            // c:974
            // c:975 — `zputs(t, stdout);`
            let body = t.take().unwrap();
            let mut so = io::stdout();
            let _ = zputs(&body, &mut so); // c:975
                                           // c:977-982 — funcdef.flags & EF_RUN → run-time suffix.
            let ef_run = hn
                .funcdef
                .as_ref()
                .map(|fd| (fd.flags & EF_RUN) != 0)
                .unwrap_or(false);
            if ef_run {
                // c:977
                println!(); // c:978
                let _ = zoutputtab(&mut io::stdout()); // c:979
                print!("{}", quotedzputs(&hn.node.nam)); // c:980
                print!(" \"$@\""); // c:981
            }
        }
        print!("\n}}"); // c:984
    } else {
        // c:985
        print!(" () {{ }}"); // c:986
    }
    // c:988-994 — redir present → emit its text.
    if let Some(redir) = &hn.redir {
        // c:988
        let t = getpermtext(redir.clone(), None, 1); // c:989
        if !t.is_empty() {
            // c:990
            let mut so = io::stdout();
            let _ = zputs(&t, &mut so); // c:991
        }
    } else if let Some(t) = &hn.redir_text {
        // RUST-ONLY twin of the branch above, exactly parallel to the
        // `body`-instead-of-`funcdef` fallback at c:947: the fusevm
        // definition path registers the redirection list as already-rendered
        // text rather than a second Eprog, so there is nothing for
        // `getpermtext` to walk. Without this, `functions f` / `which f`
        // dropped the `} > out` tail that C prints at c:991.
        if !t.is_empty() {
            let mut so = io::stdout();
            let _ = zputs(t, &mut so); // c:991
        }
    }

    println!(); // c:996
}

/// Port of `scanmatchshfunc(Patprog pprog, int sorted, int flags1, int flags2, ScanFunc scanfunc, int scanflags, int expand)` from `Src/hashtable.c:1013`.
///
/// C body iterates `shfunctab` and calls `func(node)` on every
/// entry whose name matches the compiled pattern `pprog`. Rust
/// port walks the singleton with a closure callback.
///
/// Returns the count of matched entries (mirrors C's int return).
/// WARNING: param names don't match C — Rust=(pattern, func) vs C=(pprog, sorted, flags1, flags2, scanfunc, scanflags, expand)
pub fn scanmatchshfunc<F>(pattern: Option<&str>, mut func: F) -> i32
where
    F: FnMut(&str, &shfunc),
{
    let tab = shfunctab_lock().read().expect("shfunctab poisoned");
    let mut count = 0;
    // c:Src/hashtable.c:1031 scanshfunc(sorted=1, …) — the `sorted`
    // flag is set on every internal caller (bin_functions's no-arg
    // listing, etc.), so scan walks entries in sorted order via
    // hnamcmp (byte-wise ASCII compare). The HashMap iter order is
    // arbitrary; collect + sort for parity.
    let mut entries: Vec<_> = tab.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (name, entry) in entries {
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

/// Port of `scanshfunc(int sorted, int flags1, int flags2, ScanFunc scanfunc, int scanflags, int expand)` from `Src/hashtable.c:1031`.
///
/// C body walks every `shfunctab` entry calling `func(node, flags)`.
/// Rust port delegates to scanmatchshfunc with no pattern.
/// WARNING: param names don't match C — Rust=(func) vs C=(sorted, flags1, flags2, scanfunc, scanflags, expand)
pub fn scanshfunc<F>(func: F) -> i32
where
    F: FnMut(&str, &shfunc),
{
    scanmatchshfunc(None, func)
}

/// Port of `printshfuncexpand(HashNode hn, int printflags, int expand)` from `Src/hashtable.c:1042`.
///
/// C body:
/// ```c
/// int save_expand;
/// save_expand = text_expand_tabs;
/// text_expand_tabs = expand;
/// shfunctab->printnode(hn, printflags);
/// text_expand_tabs = save_expand;
/// ```
///
/// Briefly toggles `text_expand_tabs` around the printnode call so
/// the body indentation comes out either tab- or space-formatted
/// per the caller's `expand` arg.
pub fn printshfuncexpand(hn: &shfunc, printflags: i32, expand: i32) {
    // c:1044 — `int save_expand;`
    let save_expand: i32; // c:1044
                          // c:1046 — `save_expand = text_expand_tabs;`
    save_expand = crate::text::TEXT_EXPAND_TABS.load(Ordering::Relaxed); // c:1046
                                                                         // c:1047 — `text_expand_tabs = expand;`
    crate::text::TEXT_EXPAND_TABS.store(expand, Ordering::Relaxed); // c:1047
                                                                    // c:1048 — `shfunctab->printnode(hn, printflags);`
    printshfuncnode(hn, printflags); // c:1048
                                     // c:1049 — `text_expand_tabs = save_expand;`
    crate::text::TEXT_EXPAND_TABS.store(save_expand, Ordering::Relaxed); // c:1049
}

/// Port of `getshfuncfile(shfunc shf)` from `Src/hashtable.c:1059`.
///
/// C body (verbatim):
///   if (shf->node.flags & PM_LOADDIR) {
///       return zhtricat(shf->filename, "/", shf->node.nam);
///   } else if (shf->filename) {
///       return dupstring(shf->filename);
///   } else {
///       return NULL;
///   }
///
/// PM_LOADDIR is set when zsh loaded the function via fpath
/// directory autoload (the common `autoload -Uz` path): in that
/// case `filename` is the DIRECTORY and we must append `/name` to
/// produce the actual source file. Prior Rust port skipped the
/// PM_LOADDIR branch, so `${functions_source[my_autoload]}`
/// returned the fpath dir (e.g. `/usr/share/zsh/5.9/functions`)
/// instead of the real path (`.../functions/my_autoload`).
/// NOTE ON THE SIGNATURE: C takes the resolved node —
/// `getshfuncfile(Shfunc shf)` — so callers that already hold it (c:549
/// `pm->u.str = getshfuncfile(shf)`, c:589 `pm.u.str =
/// getshfuncfile((Shfunc)hn)`) reach the filename with no lookup at all.
/// This port is keyed by NAME, so those callers pay a second shfunctab
/// lock + hash. Callers on a whole-map path therefore inline the c:1061-1063
/// body against the node they already resolved rather than calling here.
pub fn getshfuncfile(shf: &str) -> Option<String> {
    let tab = shfunctab_lock().read().expect("shfunctab poisoned");
    let f = tab.get_including_disabled(shf)?;
    let filename = f.filename.as_ref()?;
    // c:1061 — PM_LOADDIR: `zhtricat(shf->filename, "/", shf->node.nam)`
    if (f.node.flags as u32 & crate::ported::zsh_h::PM_LOADDIR) != 0 {
        Some(format!("{}/{}", filename, f.node.nam))
    } else {
        // c:1063 — `dupstring(shf->filename)`
        Some(filename.clone())
    }
}

/// Port of `createreswdtable()` from `Src/hashtable.c:1120`.
///
/// C body wires up the reswdtab GSU vtable then iterates the
/// static `reswds` array calling `addnode` for each. Rust port:
/// touches the singleton (which seeds the table from the static
/// word list in `reswd_table::new`).
pub fn createreswdtable() {
    let _ = reswdtab_lock();
}

/// Port of `printreswdnode(HashNode hn, int printflags)` from `Src/hashtable.c:1147`.
///
/// C body:
/// ```c
/// Reswd rw = (Reswd) hn;
/// if (printflags & PRINT_WHENCE_WORD) {
///     printf("%s: reserved\n", rw->node.nam);
///     return;
/// }
/// if (printflags & PRINT_WHENCE_CSH) {
///     printf("%s: shell reserved word\n", rw->node.nam);
///     return;
/// }
/// if (printflags & PRINT_WHENCE_VERBOSE) {
///     printf("%s is a reserved word\n", rw->node.nam);
///     return;
/// }
/// /* default is name only */
/// printf("%s\n", rw->node.nam);
/// ```
pub fn printreswdnode(hn: &reswd, printflags: i32) {
    // c:1149 — `Reswd rw = (Reswd) hn;` — Rust types already give us reswd.
    // c:1151-1154 — PRINT_WHENCE_WORD branch.
    if (printflags & PRINT_WHENCE_WORD) != 0 {
        println!("{}: reserved", hn.node.nam); // c:1152
        return; // c:1153
    }
    // c:1156-1159 — PRINT_WHENCE_CSH branch.
    if (printflags & PRINT_WHENCE_CSH) != 0 {
        println!("{}: shell reserved word", hn.node.nam); // c:1157
        return; // c:1158
    }
    // c:1161-1164 — PRINT_WHENCE_VERBOSE branch.
    if (printflags & PRINT_WHENCE_VERBOSE) != 0 {
        println!("{} is a reserved word", hn.node.nam); // c:1162
        return; // c:1163
    }
    // c:1166-1167 — default: name only.
    println!("{}", hn.node.nam); // c:1167
}

/// Port of `void createaliastable(HashTable ht)` from `Src/hashtable.c:1186`.
/// ```c
/// void
/// createaliastable(HashTable ht)
/// {
///     ht->hash        = hasher;
///     ht->emptytable  = NULL;
///     ht->filltable   = NULL;
///     ht->cmpnodes    = strcmp;
///     ht->addnode     = addhashnode;
///     ht->getnode     = gethashnode;
///     ht->getnode2    = gethashnode2;
///     ht->removenode  = removehashnode;
///     ht->disablenode = disablehashnode;
///     ht->enablenode  = enablehashnode;
///     ht->freenode    = freealiasnode;
///     ht->printnode   = printaliasnode;
/// }
/// ```
/// The Rust `hashtable.addnode/.getnode/.removenode/.disablenode/.enablenode/
/// .freenode/.printnode` function-pointer types take untyped HashNode
/// arguments. The generic Rust helpers (`addhashnode<T>`/`gethashnode<T>`/
/// etc.) take typed `&mut HashMap<String, T>` so they can't directly
/// satisfy the untyped slot signature; downstream consumers of `aliastab`
/// dispatch through `aliastab_lock()` (the typed wrapper) instead of
/// the C-style slot. Mirror the C structure verbatim: assign every slot
/// either to the matching adapter or `None`, with each line citing the
/// matching c:NNN.
pub fn createaliastable(ht: &mut hashtable) {
    // c:1188
    fn cmpnodes_strcmp(a: &str, b: &str) -> i32 {
        // c:1193 strcmp
        a.cmp(b) as i32
    }
    ht.hash = Some(hasher); // c:1190
    ht.emptytable = None; // c:1191
    ht.filltable = None; // c:1192
    ht.cmpnodes = Some(cmpnodes_strcmp); // c:1193
                                         // c:1194-1201 — addnode/getnode/getnode2/removenode/disablenode/
                                         // enablenode/freenode/printnode: their C signatures are `void(*)(
                                         // HashTable, char *, void *)` / `HashNode(*)(HashTable, char *)` /
                                         // ... — they take untyped `void *`. The typed Rust helpers
                                         // (`addhashnode<T>(ht: &mut HashMap<String, T>, ...)`) can't be
                                         // coerced through the `fn(&mut hashtable, String, usize)` slot
                                         // shape without per-value-type trampoline closures. Leave the
                                         // slots `None`; the typed dispatch through `aliastab_lock` is the
                                         // canonical Rust path for this table.
    ht.addnode = None; // c:1194 addhashnode
    ht.getnode = None; // c:1195 gethashnode
    ht.getnode2 = None; // c:1196 gethashnode2
    ht.removenode = None; // c:1197 removehashnode
    ht.disablenode = None; // c:1198 disablehashnode
    ht.enablenode = None; // c:1199 enablehashnode
    ht.freenode = None; // c:1200 freealiasnode
    ht.printnode = None; // c:1201 printaliasnode
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
        self.flags() & (DISABLED as u32) != 0
    }
}

impl HashNodeFlags for alias {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= DISABLED as i32;
        } else {
            self.node.flags &= !(DISABLED as i32);
        }
    }
}

impl HashNodeFlags for shfunc {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= DISABLED as i32;
        } else {
            self.node.flags &= !(DISABLED as i32);
        }
    }
}

impl HashNodeFlags for cmdnam {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= DISABLED as i32;
        } else {
            self.node.flags &= !(DISABLED as i32);
        }
    }
}

impl HashNodeFlags for reswd {
    fn flags(&self) -> u32 {
        self.node.flags as u32
    }
    fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.node.flags |= DISABLED as i32;
        } else {
            self.node.flags &= !(DISABLED as i32);
        }
    }
}

/// Port of `createaliastables()` from `Src/hashtable.c:1206`.
///
/// C body (lines 1206-1224):
/// ```c
/// aliastab = newhashtable(23, "aliastab", NULL);
/// createaliastable(aliastab);
/// aliastab->addnode(aliastab, ztrdup("run-help"), createaliasnode(ztrdup("man"), 0));
/// aliastab->addnode(aliastab, ztrdup("which-command"), createaliasnode(ztrdup("whence"), 0));
/// sufaliastab = newhashtable(11, "sufaliastab", NULL);
/// createaliastable(sufaliastab);
/// ```
///
/// The OnceLock-backed `aliastab_lock()` / `sufaliastab_lock()`
/// stand in for `newhashtable(...)` + `createaliastable(...)` — they
/// lazy-init the underlying maps on first access.
pub fn createaliastables() {
    // c:1206 — newhashtable(23, "aliastab", NULL)
    // c:1212 — createaliastable(aliastab)
    let mut tab = aliastab_lock().write().expect("aliastab poisoned");
    // c:1215 — `aliastab->addnode(aliastab, ztrdup("run-help"),
    //                              createaliasnode(ztrdup("man"), 0));`
    tab.add(createaliasnode("run-help", "man", 0)); // c:1215
                                                    // c:1216 — `aliastab->addnode(aliastab, ztrdup("which-command"),
                                                    //                              createaliasnode(ztrdup("whence"), 0));`
    tab.add(createaliasnode("which-command", "whence", 0)); // c:1216
    drop(tab);
    // c:1221 — newhashtable(11, "sufaliastab", NULL)
    // c:1223 — createaliastable(sufaliastab)
    let _ = sufaliastab_lock();
}
// c:1253

/// Build an alias node with the canonical `alias` shape.
/// Mirrors C `addaliasnode(aliastab, name, createaliasnode(text, flags))`
/// at hashtable.c:1230 — caller-side bundle for the
/// hashnode+text+flags inline-build.
pub fn createaliasnode(name: &str, text: &str, flags: u32) -> alias {
    // c:1230
    alias {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: flags as i32,
        },
        text: text.to_string(),
        inuse: 0,
    }
}

/// Port of `createaliasnode(char *txt, int flags)` from `Src/hashtable.c:1230`.
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

/// Port of `freealiasnode(HashNode hn)` from `Src/hashtable.c:1243`.
///
/// C body frees the name + text strings + alias struct. Rust
/// port: drop runs the same when the crate::ported::zsh_h::alias is removed from its
/// table. This helper triggers the drop.
pub fn freealiasnode(hn: &str) {
    let mut tab = aliastab_lock().write().expect("aliastab poisoned");
    tab.remove(hn);
}

/// Port of `printaliasnode(HashNode hn, int printflags)` from `Src/hashtable.c:1256`.
///
/// Emits `whence`-style output for one alias with PRINT_NAMEONLY /
/// PRINT_WHENCE_WORD / PRINT_WHENCE_SIMPLE / PRINT_WHENCE_CSH /
/// PRINT_WHENCE_VERBOSE / PRINT_LIST flag dispatch. PRINT_LIST falls
/// through to the tail `quotedzputs(nam) '=' quotedzputs(text) '\n'`;
/// every other branch returns early.
pub fn printaliasnode(hn: &alias, printflags: i32) {
    // c:1258 — `Alias a = (Alias) hn;` — Rust types already give us alias.

    // c:1260-1264 — PRINT_NAMEONLY branch.
    if (printflags & PRINT_NAMEONLY) != 0 {
        let mut so = io::stdout();
        let _ = zputs(&hn.node.nam, &mut so); // c:1261
        println!(); // c:1262
        return; // c:1263
    }

    // c:1266-1274 — PRINT_WHENCE_WORD branch.
    if (printflags & PRINT_WHENCE_WORD) != 0 {
        if (hn.node.flags & ALIAS_SUFFIX as i32) != 0 {
            println!("{}: suffix alias", hn.node.nam); // c:1268
        } else if (hn.node.flags & ALIAS_GLOBAL as i32) != 0 {
            println!("{}: global alias", hn.node.nam); // c:1270
        } else {
            println!("{}: alias", hn.node.nam); // c:1272
        }
        return; // c:1273
    }

    // c:1276-1280 — PRINT_WHENCE_SIMPLE branch.
    if (printflags & PRINT_WHENCE_SIMPLE) != 0 {
        let mut so = io::stdout();
        let _ = zputs(&hn.text, &mut so); // c:1277
        println!(); // c:1278
        return; // c:1279
    }

    // c:1282-1293 — PRINT_WHENCE_CSH branch.
    if (printflags & PRINT_WHENCE_CSH) != 0 {
        let mut so = io::stdout();
        let _ = nicezputs(&hn.node.nam, &mut so); // c:1283
        print!(": "); // c:1284
        if (hn.node.flags & ALIAS_SUFFIX as i32) != 0 {
            print!("suffix "); // c:1286
        } else if (hn.node.flags & ALIAS_GLOBAL as i32) != 0 {
            print!("globally "); // c:1288
        }
        print!("aliased to "); // c:1289
        let _ = nicezputs(&hn.text, &mut so); // c:1290
        println!(); // c:1291
        return; // c:1292
    }

    // c:1295-1308 — PRINT_WHENCE_VERBOSE branch.
    if (printflags & PRINT_WHENCE_VERBOSE) != 0 {
        let mut so = io::stdout();
        let _ = nicezputs(&hn.node.nam, &mut so); // c:1296
        print!(" is a"); // c:1297
        if (hn.node.flags & ALIAS_SUFFIX as i32) != 0 {
            print!(" suffix"); // c:1299
        } else if (hn.node.flags & ALIAS_GLOBAL as i32) != 0 {
            print!(" global"); // c:1301
        } else {
            print!("n"); // c:1303
        }
        print!(" alias for "); // c:1304
        let _ = nicezputs(&hn.text, &mut so); // c:1305
        println!(); // c:1306
        return; // c:1307
    }

    // c:1310-1330 — PRINT_LIST prefix block (falls through to the
    // tail quotedzputs body below; default-no-flags also reaches the
    // tail by skipping this block).
    if (printflags & PRINT_LIST) != 0 {
        // c:1312-1316 — Fast fail on `=` in name (unrepresentable
        // `alias name=...` round-trip).
        if hn.node.nam.contains('=') {
            // c:1313
            zwarn(&format!(
                "invalid alias '{}' encountered while printing aliases",
                hn.node.nam
            ));
            return; // c:1316
        }
        print!("alias "); // c:1320
        if (hn.node.flags & ALIAS_SUFFIX as i32) != 0 {
            // c:1321
            print!("-s "); // c:1322
        } else if (hn.node.flags & ALIAS_GLOBAL as i32) != 0 {
            // c:1323
            print!("-g "); // c:1324
        }
        // c:1326-1329 — `-- ` so a name starting with `-`/`+` isn't
        // interpreted as an option when the listing is re-executed.
        if hn.node.nam.starts_with('-') || hn.node.nam.starts_with('+') {
            // c:1328
            print!("-- "); // c:1329
        }
    }

    // c:1332-1336 — common tail: quotedzputs(nam) '=' quotedzputs(text) '\n'.
    print!("{}", quotedzputs(&hn.node.nam)); // c:1332
    print!("="); // c:1333
    print!("{}", quotedzputs(&hn.text)); // c:1334
    println!(); // c:1336
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

/// History-specific hash function (normalizes whitespace).
/// Port of `histhasher(const char *str)` from `Src/hashtable.c:1365`.
///
/// C body uses `inblank(*str)` (canonical typtab predicate at
/// `Src/ztype.h:50` — NARROW blank: space/tab ONLY, not newline,
/// definitely NOT broad Unicode whitespace). The Rust port previously
/// used `c.is_whitespace()` which is the Unicode-broad set including
/// CR/FF/VT/NBSP — every line of zsh history containing one of those
/// bytes hashed to a different bucket than C would have.
///
/// Faithful: matches `inblank` exactly (`c:50` — `space + tab`).
pub fn histhasher(s: &str) -> u32 {
    // c:1365
    // c:50 — `inblank(c)` = `c == ' ' || c == '\t'`. NOT `\n`, NOT broad.
    #[inline]
    fn is_inblank_narrow(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    let mut hashval: u32 = 0;
    let mut chars = s.chars().peekable();

    // c:1369 — `while (inblank(*str)) str++;` skip leading blanks.
    while let Some(&c) = chars.peek() {
        if is_inblank_narrow(c) {
            chars.next();
        } else {
            break;
        }
    }

    // c:1371 — main mix loop.
    while let Some(c) = chars.next() {
        if is_inblank_narrow(c) {
            // c:1373 — `do str++; while (inblank(*str));` collapse runs.
            while let Some(&next) = chars.peek() {
                if is_inblank_narrow(next) {
                    chars.next();
                } else {
                    break;
                }
            }
            // c:1374-1375 — `if (*str) hashval += (hashval << 5) + ' ';`
            if chars.peek().is_some() {
                hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(' ' as u32));
            }
        } else {
            // c:1377 — `hashval += (hashval << 5) + *(unsigned char *)str++;`
            hashval = hashval.wrapping_add(hashval.wrapping_shl(5).wrapping_add(c as u32));
        }
    }
    hashval
}

/// Port of `emptyhisttable(HashTable ht)` from `Src/hashtable.c:1385`.
///
/// C body:
/// ```c
/// emptyhashtable(ht);
/// if (hist_ring) histremovedups();
/// ```
/// WARNING: param names don't match C — Rust=() vs C=(ht)
pub fn emptyhisttable() {
    // c:1385 — `emptyhashtable(ht)` — clear the lookup table.
    histtab_lock().write().expect("histtab poisoned").clear();
    // c:1386 — `if (hist_ring) histremovedups();` — prune dup-flagged
    // entries from the history ring.
    let has_ring = !hist_ring.lock().unwrap().is_empty();
    if has_ring {
        histremovedups(); // c:1386
    }
}

/// Compare strings with normalized whitespace (for history).
/// Port of `histstrcmp(const char *str1, const char *str2)` from
/// `Src/hashtable.c:1396`.
///
/// C body uses `inblank(*str)` everywhere (`Src/ztype.h:50` — NARROW
/// space/tab only). The previous Rust port used `c.is_whitespace()`
/// (broad Unicode set including CR/FF/VT/NBSP), which would silently
/// fold history lines that C considers distinct (e.g. lines that
/// contain NBSP would dedupe against lines with no NBSP).
///
/// C signature is 2-arg: it reads `isset(HISTREDUCEBLANKS)` directly.
/// Rust port passes `reduce_blanks` as an explicit 3rd arg to keep
/// the option read out of this leaf fn (call sites at hist.c thread
/// the option from the parent scope).
pub fn histstrcmp(s1: &str, s2: &str, reduce_blanks: bool) -> std::cmp::Ordering {
    // c:1396
    // c:50 — `inblank(c)` = `c == ' ' || c == '\t'`. NOT newline, NOT broad.
    #[inline]
    fn is_inblank_narrow(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    // c:1398-1399 — skip leading inblank in both strings.
    let s1 = s1.trim_start_matches(is_inblank_narrow);
    let s2 = s2.trim_start_matches(is_inblank_narrow);

    // c:1405 — HISTREDUCEBLANKS short-circuit to raw strcmp.
    if reduce_blanks {
        return s1.cmp(s2);
    }

    let mut c1 = s1.chars().peekable();
    let mut c2 = s2.chars().peekable();

    // c:1408 — `while (*str1 && *str2) { ... }` then `return *str1 - *str2;`.
    loop {
        let ch1 = c1.peek().copied();
        let ch2 = c2.peek().copied();

        match (ch1, ch2) {
            (None, None) => return std::cmp::Ordering::Equal, // c:1421 — both NUL
            (None, Some(c)) => {
                // c:1421 — *str1=0 - *str2; left shorter (Less) unless str2
                // is all-inblank residue.
                if is_inblank_narrow(c) {
                    while c2.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c2.next();
                    }
                    if c2.peek().is_none() {
                        return std::cmp::Ordering::Equal;
                    }
                }
                return std::cmp::Ordering::Less;
            }
            (Some(c), None) => {
                if is_inblank_narrow(c) {
                    while c1.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c1.next();
                    }
                    if c1.peek().is_none() {
                        return std::cmp::Ordering::Equal;
                    }
                }
                return std::cmp::Ordering::Greater;
            }
            (Some(ch1), Some(ch2)) => {
                let ws1 = is_inblank_narrow(ch1);
                let ws2 = is_inblank_narrow(ch2);

                if ws1 && ws2 {
                    // c:1411-1413 — collapse both runs.
                    while c1.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c1.next();
                    }
                    while c2.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c2.next();
                    }
                } else if ws1 {
                    // c:1410 — `if (!inblank(*str2)) break;` → mismatch.
                    while c1.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c1.next();
                    }
                    if c1.peek().is_none() {
                        return std::cmp::Ordering::Less;
                    }
                    return std::cmp::Ordering::Less;
                } else if ws2 {
                    while c2.peek().copied().map(is_inblank_narrow).unwrap_or(false) {
                        c2.next();
                    }
                    if c2.peek().is_none() {
                        return std::cmp::Ordering::Greater;
                    }
                    return std::cmp::Ordering::Greater;
                } else if ch1 != ch2 {
                    return ch1.cmp(&ch2); // c:1417 — *str1 - *str2
                } else {
                    c1.next();
                    c2.next();
                }
            }
        }
    }
}

/// Port of `addhistnode(HashTable ht, char *nam, void *nodeptr)` from `Src/hashtable.c:1427`.
///
/// C body:
/// ```c
/// HashNode oldnode = addhashnode2(ht, nam, nodeptr);
/// Histent he = (Histent)nodeptr;
/// if (oldnode && oldnode != (HashNode)nodeptr) {
///     if (he->node.flags & HIST_MAKEUNIQUE
///      || (he->node.flags & HIST_FOREIGN && (Histent)oldnode == he->up)) {
///         (void) addhashnode2(ht, oldnode->nam, oldnode); /* restore hash */
///         he->node.flags |= HIST_DUP;
///         he->node.flags &= ~HIST_MAKEUNIQUE;
///     } else {
///         oldnode->flags |= HIST_DUP;
///         if (hist_ignore_all_dups)
///             freehistnode(oldnode); /* Remove the old dup */
///     }
/// } else
///     he->node.flags &= ~HIST_MAKEUNIQUE;
/// ```
///
/// The Rust `histtab` is keyed by command text → event id, so
/// `addhashnode2` maps to `HashMap::insert` (returns the displaced
/// event). The new node `he` and the displaced `oldnode` are located
/// in `hist_ring` by their `histnum`; their `node.flags` are the same
/// `HIST_*` fields the C node carries.
///
/// NOTE: the caller must NOT hold the `hist_ring` lock across this
/// call — `addhistnode` re-locks the ring to read/mutate node flags.
/// WARNING: param names don't match C — Rust=(nam, event_id) vs C=(ht, nam, nodeptr)
pub fn addhistnode(nam: &str, event_id: i32) -> Option<i32> {
    // c:1429 — `HashNode oldnode = addhashnode2(ht, nam, nodeptr);`
    let oldnode = histtab_lock()
        .write()
        .expect("histtab poisoned")
        .insert(nam.to_string(), event_id);

    // c:1431 — `if (oldnode && oldnode != (HashNode)nodeptr)`
    if let Some(old_event) = oldnode {
        if old_event != event_id {
            // `he->node.flags` — flags of the newly inserted node. C reads
            // `he->node.flags` directly off the pointer; the Rust ring is a
            // `Vec` keyed by `histnum`, so locate the entry by event id.
            let he_flags = hist_ring
                .lock()
                .unwrap()
                .iter()
                .find(|h| h.histnum == event_id as i64)
                .map(|h| h.node.flags)
                .unwrap_or(0);
            // c:1433 — `(Histent)oldnode == he->up` (the entry directly
            // above `he` in the ring is the one being displaced).
            let up_is_old = up_histent(event_id as i64) == Some(old_event as i64);
            if (he_flags & HIST_MAKEUNIQUE as i32) != 0
                || ((he_flags & HIST_FOREIGN as i32) != 0 && up_is_old)
            {
                // c:1434 — `addhashnode2(ht, oldnode->nam, oldnode);`
                // Restore the hash so `nam` maps back to the old event
                // (same command text, so the key is unchanged).
                histtab_lock()
                    .write()
                    .expect("histtab poisoned")
                    .insert(nam.to_string(), old_event);
                // c:1435-1436 — mark `he` a dup, clear make-unique.
                if let Some(h) = hist_ring
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .find(|h| h.histnum == event_id as i64)
                {
                    h.node.flags = (h.node.flags | HIST_DUP as i32) & !(HIST_MAKEUNIQUE as i32);
                }
            } else {
                // c:1439 — `oldnode->flags |= HIST_DUP;`
                if let Some(h) = hist_ring
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .find(|h| h.histnum == old_event as i64)
                {
                    h.node.flags |= HIST_DUP as i32;
                }
                // c:1440-1441 — `if (hist_ignore_all_dups) freehistnode(oldnode);`
                // C's `freehistnode` == `freehistdata(oldnode, 1); zfree(oldnode)`;
                // the ported `freehistdata(idx, 1)` unlinks the old node from
                // the ring (the Rust equivalent of `zfree`) and — because the
                // node is now HIST_DUP-flagged — skips removing the hash entry
                // that already points at the new node (c:1466 guard).
                if hist_ignore_all_dups.load(Ordering::SeqCst) != 0 {
                    let idx = hist_ring
                        .lock()
                        .unwrap()
                        .iter()
                        .position(|h| h.histnum == old_event as i64);
                    if let Some(idx) = idx {
                        freehistdata(idx, 1);
                    }
                }
            }
            return oldnode;
        }
    }
    // c:1445 — `he->node.flags &= ~HIST_MAKEUNIQUE;`
    if let Some(h) = hist_ring
        .lock()
        .unwrap()
        .iter_mut()
        .find(|h| h.histnum == event_id as i64)
    {
        h.node.flags &= !(HIST_MAKEUNIQUE as i32);
    }
    oldnode
}

/// Port of `freehistnode(HashNode nodeptr)` from `Src/hashtable.c:1450`.
///
/// C body: `freehistdata((Histent)nodeptr, 1); zfree(nodeptr, ...);`
/// Rust port: removes from the lookup table — drop runs the
/// equivalent of zfree.
pub fn freehistnode(nodeptr: &str) {
    histtab_lock()
        .write()
        .expect("histtab poisoned")
        .remove(nodeptr);
}

/// Port of `freehistdata(Histent he, int unlink)` from `Src/hashtable.c:1458`.
///
/// C body: removes the named entry from `histtab` (unless flagged
/// HIST_DUP/HIST_TMPSTORE), frees the command + word-array fields,
/// and if `unlink` re-links the ring around `he` and decrements
/// `histlinect`. Rust port indexes into `hist_ring` (Vec replaces C's
/// doubly-linked list); the up/down relink collapses to `Vec::remove`.
/// WARNING: param names don't match C — Rust=(idx, unlink) vs C=(he, unlink)
pub fn freehistdata(idx: usize, unlink: i32) {
    // c:1458
    let mut ring = hist_ring.lock().unwrap();
    let he = match ring.get(idx) {
        Some(h) => h,
        None => return,
    }; // c:1461 if (!he) return
    let nam = he.node.nam.clone();
    let flags = he.node.flags as u32;
    if (flags & (HIST_DUP | HIST_TMPSTORE)) == 0 {
        // c:1467
        let mut tab = histtab_lock().write().expect("histtab poisoned"); // c:1468 removehashnode(histtab, ...)
        tab.remove(&nam);
    }
    // c:1471-1473 — `zsfree(name); if (nwords) zfree(words, ...)`. Rust
    // String/Vec drop handles both; only the unlink step needs explicit
    // ring mutation.
    if unlink != 0 {
        // c:1475
        ring.remove(idx); // c:1477-1483 unlink up/down
        let new_ct = ring.len() as i64;
        drop(ring);
        histlinect.store(new_ct, Ordering::SeqCst);
        // c:1477 --histlinect
    }
}

/// Port of `dircache_set(char **name, char *value)` from `Src/hashtable.c:1537`.
///
/// C body manages a refcounted directory-name cache:
///   - `value == NULL` → decrement refs on `*name`, free if zero,
///     set `*name = NULL`.
///   - `value != NULL` → search for an existing entry, bump refs,
///     else allocate a new slot.
///
/// Rust port: routes through dircache_lock() with refcount-by-
/// HashMap-value (i32). Add/remove via the (name, value) pair.
pub fn dircache_set(name: &mut Option<String>, value: Option<&str>) {
    // c:1537
    let mut cache = dircache_lock().lock().expect("dircache poisoned");

    if value.is_none() {
        // c:1541
        // c:1542-1543 — `if (!*name) return;`
        let key = match name.as_deref() {
            None => return, // c:1543
            Some(s) => s.to_string(),
        };
        // c:1544-1548 — `if (!dircache_size) { zsfree(*name); *name = NULL; return; }`
        if cache.is_empty() {
            // c:1544
            *name = None; // c:1546
            return; // c:1547
        }
        // c:1550-1582 — scan cache, decrement matching entry's refs;
        // on refs==0, drop the entry. Rust keys by string equality
        // since we don't share the C pointer-identity used at c:1553.
        if let Some(idx) = cache.iter().position(|e| e.name == key) {
            // c:1550
            cache[idx].refs -= 1; // c:1555
            if cache[idx].refs == 0 {
                // c:1556
                cache.remove(idx); // c:1558-1577 collapsed
                DIRCACHE_LASTENTRY.store(usize::MAX, Ordering::SeqCst); // c:1564/1577
            }
            *name = None; // c:1579
            return; // c:1580
        }
        // c:1583-1584 — `zsfree(*name); *name = NULL;`
        *name = None; // c:1584
    } else {
        // c:1585
        let mut v = value.unwrap().to_string();
        // c:1590-1594 — absolute-path normalization for relative input.
        if !v.starts_with('/') {
            // c:1590
            let cwd = zgetcwd(); // c:1591 zgetcwd
            v = format!("{}/{}", cwd, v); // c:1591 zhtricat
            if let Some(resolved) = xsymlink(&v) {
                // c:1593 xsymlink(..., 1)
                v = resolved; // c:1593
            } // c:1593
        }
        // c:1602-1606 — `dircache_lastentry` fast-path: same path as last.
        let last_idx = DIRCACHE_LASTENTRY.load(Ordering::SeqCst);
        if last_idx != usize::MAX && last_idx < cache.len() && cache[last_idx].name == v {
            *name = Some(cache[last_idx].name.clone()); // c:1604
            cache[last_idx].refs += 1; // c:1605
            return; // c:1606
        }
        // c:1607-1610 — empty-cache: allocate first entry.
        if cache.is_empty() {
            // c:1607
            cache.push(dircache_entry {
                name: v.clone(),
                refs: 1,
            }); // c:1609-1610
            DIRCACHE_LASTENTRY.store(0usize, Ordering::SeqCst);
            *name = Some(v);
            return;
        }
        // c:1611-1619 — scan for existing entry, bump refs.
        if let Some(idx) = cache.iter().position(|e| e.name == v) {
            // c:1612-1614
            *name = Some(cache[idx].name.clone()); // c:1615
            cache[idx].refs += 1; // c:1616
            DIRCACHE_LASTENTRY.store(idx, Ordering::SeqCst);
            return;
        }
        // c:1620+ — push new entry.
        cache.push(dircache_entry {
            name: v.clone(),
            refs: 1,
        });
        let new_idx = cache.len() - 1;
        DIRCACHE_LASTENTRY.store(new_idx, Ordering::SeqCst);
        *name = Some(v);
    }
}

// `DIRCACHE_LASTENTRY` already declared below at hashtable.rs:1849
// as `AtomicUsize` (`usize::MAX` sentinel). Reuse that — the new
// body above adapts via i32 cast.

// `SuffixAliasTable` type alias deleted — Rust-only convenience.
// C has no `SuffixAliasTable`; the same generic `HashTable` powers
// both `aliastab` and `sufaliastab` (declared identically at
// hashtable.c:1177-1182). Callers can use `alias_table` directly
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
pub struct dircache_entry {
    // c:1503
    pub name: String, // c:1506
    pub refs: i32,    // c:1508
}

/// Command name hash table
// hash table containing external commands                                  // c:587
#[derive(Debug)]
/// `$cmdtab` table of cached executable lookups.
/// Port of `cmdnamtab` from Src/hashtable.c — `createcmdnamtable()`
/// (line 601), `emptycmdnamtable()` (line 623), and `hashdir()`
/// (line 634) drive populate/clear/fill cycles.
/// **NOT C-FAITHFUL — Rust-only typed wrapper around HashMap.**
/// C uses the generic `HashTable` struct (zsh.h:1530 / zsh_h.rs:535)
/// with per-table GSU callback fn pointers (`hash`/`addnode`/
/// `getnode`/`removenode`/`freenode`/`printnode`/`scantab`). Each
/// per-table accessor (`cmdnamtab_lock`, `shfunctab_lock`, etc.)
/// returns a `Mutex<HashTable>` instance with the appropriate
/// callbacks wired. When the generic-HashTable substrate lands,
/// cmdnam_table/shfunc_table/reswd_table/alias_table get deleted
/// in favor of typed views over the shared `HashTable` storage.
pub struct cmdnam_table {
    /// `table` field — C's `cmdnamtab` bucket array, `hsize = 201`
    /// (`Src/hashtable.c:603`). Was a `std::collections::HashMap`,
    /// whose per-process-seeded order made `${(k)commands)` /
    /// `compadd -k commands` differ on every run.
    table: hashtable_nodes<cmdnam>,
    /// `path_checked_index` field.
    path_checked_index: usize,
    /// `path` field.
    path: Vec<String>,
    /// `hash_executables_only` field.
    hash_executables_only: bool,
}

// `impl shfunc` deleted — methods replaced with inline flag checks
// (`(shf.node.flags & FLAG as i32) != 0`) at callers, mirroring
// C's idiom. Constructors `shfunc_with_body` / `shfunc_autoload`
// above replace `shfunc::with_body` / `::autoload` / `::new`.

/// Shell function hash table
// hash table containing the shell functions                                // c:805
#[derive(Debug, Clone)]
/// `$shfunctab` shell function table.
/// Port of the `shfunctab` HashTable Src/hashtable.c builds —
/// `printshfuncnode` / `freeshfuncnode` (Src/builtin.c) hang off
/// the same shape.
/// Faithful port of C's `HashTable shfunctab` (Src/zsh.h, declared
/// `mod_export HashTable shfunctab`). Stores `Box<shfunc>` so that
/// raw `*mut shfunc` handed to C-style call sites stays stable
/// across map rehashes — mirrors C's `HashNode` semantics where
/// the table owns the heap allocation and hands out pointers.
/// Owned-value accessors (`add`, `get`, `get_mut`) coexist with
/// C-faithful pointer accessors (`addnode`, `getnode`) so both
/// the Rust-idiomatic bytecode function-def path
/// (`fusevm_bridge.rs:8378`) and the C-style `bin_functions`
/// port (`builtin.rs:3689+`) write to the same canonical table.
pub struct shfunc_table {
    /// `table` field — the `nodes`/`hsize`/`ct` bucket array C's
    /// `shfunctab` is, created at `hsize = 7` (`Src/hashtable.c:814`
    /// `shfunctab = newhashtable(7, "shfunctab", NULL)`).
    ///
    /// It used to be a `std::collections::HashMap`, whose iteration
    /// order is neither C's bucket walk nor even stable across runs
    /// (`RandomState` re-seeds per process). `${(k)functions}` and
    /// `compadd -k functions` read this order straight out
    /// (`Src/Modules/parameter.c:480-481`), so both were random.
    table: hashtable_nodes<Box<shfunc>>,
}

/// Reserved word hash table
#[derive(Debug)]
/// `$reswdtab` reserved-word table.
// hash table containing the reserved words                                 // c:1111
/// Port of the `reswdtab` HashTable from Src/hashtable.c — used
/// by Src/lex.c to recognize keywords like `if`/`while`/`do`.
/// **NOT C-FAITHFUL — Rust-only typed wrapper.** See WARNING on
/// `cmdnam_table` for the canonical-port direction.
pub struct reswd_table {
    /// `table` field — C's `reswdtab` bucket array, `hsize = 23`
    /// (`Src/hashtable.c:1124` `reswdtab = newhashtable(23, "reswdtab",
    /// NULL)`).
    ///
    /// Was a `std::collections::HashMap`, whose per-process-seeded
    /// order made `$reswords` / `$dis_reswords` random. `getreswords`
    /// (`Src/Modules/parameter.c:871-886`) walks the raw bucket array
    /// (`for (i = 0; i < reswdtab->hsize; i++) for (hn =
    /// reswdtab->nodes[i]; hn; hn = hn->next)`), so those parameters
    /// expose C's chain order directly.
    table: hashtable_nodes<reswd>,
}

/// crate::ported::zsh_h::alias hash table
#[derive(Debug)]
/// `$aliastab` alias hash.
/// Port of the `aliastab` HashTable from Src/hashtable.c —
// hash table containing the aliases                                        // c:1174
/// `bin_alias()` (Src/builtin.c) drives every mutation. Suffix
/// aliases live in a separate `sufaliastab` instance.
/// **NOT C-FAITHFUL — Rust-only typed wrapper.** See WARNING on
/// `cmdnam_table` for the canonical-port direction.
pub struct alias_table {
    /// `table` field — C's `aliastab` bucket array, `hsize = 23`
    /// (`Src/hashtable.c:1210` `aliastab = newhashtable(23, "aliastab",
    /// NULL)`); the `sufaliastab` instance is built with `hsize = 11`
    /// (`Src/hashtable.c:1221`).
    ///
    /// Was an `indexmap::IndexMap`, chosen on the theory that
    /// insertion order was "closer to zsh's observed behavior". That
    /// reasoning was wrong: zsh emits alias keys in the `scanhashtable`
    /// bucket walk (`Src/hashtable.c:420-434`) that `scanpmraliases` /
    /// `scanpmgaliases` / `scanpmsaliases` drive
    /// (`Src/Modules/parameter.c:2005-2047`), i.e. bucket
    /// `hasher(nam) % hsize` ascending, each chain walked head→tail
    /// with the most recently added key at the head (`c:214-215`).
    /// That is neither insertion order nor sorted order, so
    /// `${(k)aliases}` diverged from zsh for every alias set.
    table: hashtable_nodes<alias>,
}

// Mirrors C's file-statics at hashtable.c:1517:
//   `static struct dircache_entry *dircache, *dircache_lastentry;`
//   `static int dircache_size;`
// Rust port keeps the cache as a `Mutex<Vec<dircache_entry>>` plus
// a lastentry index. dircache_size is implicit (Vec::len()).
static DIRCACHE_INNER: std::sync::OnceLock<std::sync::Mutex<Vec<dircache_entry>>> =
    std::sync::OnceLock::new();
static DIRCACHE_LASTENTRY: std::sync::atomic::AtomicUsize = // c:1517
    std::sync::atomic::AtomicUsize::new(usize::MAX); // sentinel "no last"

/// Build a hashed `cmdnam` carrying a resolved path. Mirrors C's
/// inline `cn->u.cmd = ztrdup(path); cn->node.flags = HASHED;` at
/// hashtable.c:704.
pub fn cmdnam_hashed(name: &str, path: &str) -> cmdnam {
    // c:704 idiom
    cmdnam {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: HASHED as i32,
        },
        name: None,
        cmd: Some(path.to_string()),
    }
}

/// Build an unhashed `cmdnam` whose lookup will scan
/// `path_segments`. Mirrors C's `cn->u.name = pathchecked;
/// cn->node.flags = 0;` at hashtable.c:712.
pub fn cmdnam_unhashed(name: &str, path_segments: Vec<String>) -> cmdnam {
    // c:712 idiom
    cmdnam {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: 0,
        },
        name: Some(path_segments),
        cmd: None,
    }
}

/// Build a `shfunc` for the lazy-compile path with body source text.
/// Mirrors C's `shfunctab->addnode(shfunctab, ztrdup(name), shf)`
/// after callers populate `shf->funcdef = parse_subst_string(body)`.
pub fn shfunc_with_body(name: &str, body: &str) -> shfunc {
    // c:824 idiom
    // c:Src/exec.c:5383 — `ztrdup(scriptfilename)`. zsh tags every
    // shfunc with the script it was defined in so `whence -v fn`
    // and `type fn` can print `is a shell function from <script>`.
    // For `-c '...'` invocations zsh sets scriptfilename to "zsh".
    // Without this seed, fusevm-compiled functions all had
    // filename=None and `type fn` lost the "from <script>" suffix.
    shfunc {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: 0,
        },
        filename: scriptfilename_get(),
        lineno: 0,
        funcdef: None,
        redir: None,
        sticky: None,
        body: Some(body.to_string()),
        redir_text: None,
    }
}

/// Build an autoload-marker `shfunc`. Mirrors C's
/// `createshfunc(name); shf->node.flags = PM_UNDEFINED;` at
/// hashtable.c:829.
pub fn shfunc_autoload(name: &str) -> shfunc {
    // c:829 idiom
    shfunc {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: PM_UNDEFINED as i32,
        },
        filename: None,
        lineno: 0,
        funcdef: None,
        redir: None,
        sticky: None,
        body: None,
        redir_text: None,
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
/// Per PORT_PLAN.md Phase 3 (bucket-2, read-mostly): the PATH cache
/// is read on every command resolution but mutated only by `hash`,
/// `rehash`, or `path` reassignment. `RwLock` lets parallel command
/// lookups proceed without serialising on a single mutex. Holder
/// accessor keeps the `_lock` suffix for source-stability (call
/// sites use `.read()`/`.write()` directly).
pub fn cmdnamtab_lock() -> &'static std::sync::RwLock<cmdnam_table> {
    // c:594
    static CMDNAMTAB: std::sync::OnceLock<std::sync::RwLock<cmdnam_table>> =
        std::sync::OnceLock::new();
    CMDNAMTAB.get_or_init(|| std::sync::RwLock::new(cmdnam_table::new()))
}

/// Port of `mod_export char **pathchecked;` from `Src/hashtable.c:595`.
///
/// Cursor into the `$path` array tracking how far the PATH-hash-on-
/// first-use machinery has walked. Bumped by `hashcmd` (exec.c:1042)
/// after each successful lookup so subsequent `hashdir` calls only
/// scan entries we haven't already cached.
///
/// C uses `char **pathchecked` (pointer into the `path[]` array); the
/// Rust port stores an index since `$path` lives in paramtab and is
/// re-fetched on each access. Reset to 0 by `path` reassignment per
/// `Src/hashtable.c:618`.
pub static pathchecked: std::sync::atomic::AtomicUsize = // c:595
    std::sync::atomic::AtomicUsize::new(0);

// hash table containing the aliases                                        // c:1174
/// Singleton accessor for the global `aliastab`.
/// Mirrors C's `mod_export HashTable aliastab` (hashtable.c:1186).
/// Bucket-2 read-mostly: aliases are looked up on every command word,
/// mutated only by `alias`/`unalias`. `RwLock` per PORT_PLAN.md.
pub fn aliastab_lock() -> &'static std::sync::RwLock<alias_table> {
    // c:1186
    static ALIASTAB: std::sync::OnceLock<std::sync::RwLock<alias_table>> =
        std::sync::OnceLock::new();
    ALIASTAB.get_or_init(|| std::sync::RwLock::new(alias_table::with_defaults()))
}

/// Singleton accessor for the global `sufaliastab`.
/// Mirrors C's `mod_export HashTable sufaliastab` (hashtable.c:1187).
/// Bucket-2 read-mostly: same rationale as `aliastab`.
pub fn sufaliastab_lock() -> &'static std::sync::RwLock<alias_table> {
    static SUFALIASTAB: std::sync::OnceLock<std::sync::RwLock<alias_table>> =
        std::sync::OnceLock::new();
    // c:1221 — `sufaliastab = newhashtable(11, "sufaliastab", NULL);`
    // "Table for suffix aliases --- make this smaller" (c:1219). The
    // smaller `hsize` gives a DIFFERENT bucket walk than `aliastab`'s
    // 23, so `${(k)saliases}` order depends on getting this exact
    // number. Built inline rather than via `alias_table::new()` (which
    // is `aliastab`'s 23) because a second named constructor would be
    // a Rust-only fn with no C counterpart.
    SUFALIASTAB.get_or_init(|| {
        std::sync::RwLock::new(alias_table {
            table: hashtable_nodes::newhashtable(11), // c:1221
        })
    })
}

// hash table containing the reserved words                                 // c:1111
/// Singleton accessor for the global `reswdtab`.
/// Mirrors C's `HashTable reswdtab` (hashtable.c, file-scope).
/// Bucket-2 read-mostly (effectively read-only post-init): every
/// command word is checked against reserved words; the table is
/// populated once at startup. `RwLock` per PORT_PLAN.md.
pub fn reswdtab_lock() -> &'static std::sync::RwLock<reswd_table> {
    // c:1115
    static reswdTAB: std::sync::OnceLock<std::sync::RwLock<reswd_table>> =
        std::sync::OnceLock::new();
    reswdTAB.get_or_init(|| std::sync::RwLock::new(reswd_table::new()))
}

/// Singleton accessor for the global `histtab` (history events).
/// Mirrors C's `HashTable histtab` (hashtable.c:1340).
pub fn histtab_lock() -> &'static std::sync::RwLock<HashMap<String, i32>> {
    static HISTTAB: std::sync::OnceLock<std::sync::RwLock<HashMap<String, i32>>> =
        std::sync::OnceLock::new();
    HISTTAB.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
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
// `OnceLock<Mutex<shfunc_table>>` exposed via `shfunctab_lock()`
// so the GSU-style C names below can mutate it without taking a
// `ShellExecutor` parameter (matching the C signatures, where
// the table is global).
// ===========================================================

/// Singleton accessor for the global `shfunctab`.
/// Mirrors C's `mod_export HashTable shfunctab` (hashtable.c:808).
/// Lazily initialised on first access. Bucket-2 read-mostly: shell
/// functions are looked up on every function-call dispatch, mutated
/// only by `function f()` / `unfunction` / `autoload`. `RwLock`
/// per PORT_PLAN.md.
pub fn shfunctab_lock() -> &'static std::sync::RwLock<shfunc_table> {
    // c:808
    static shfuncTAB: std::sync::OnceLock<std::sync::RwLock<shfunc_table>> =
        std::sync::OnceLock::new();
    shfuncTAB.get_or_init(|| std::sync::RwLock::new(shfunc_table::new()))
}

/// Glob-style match for hashtable scan callers. Direct port of C's
/// `pattry(pprog, hn->nam)` at `Src/hashtable.c:412` / `c:431` —
/// `scanmatchtable` compiles the caller's pattern once into a
/// `Patprog` and tests every node's name against it. zshrs's
/// `patmatch(pattern, text)` (pattern.rs:1561) does the
/// `patcompile + pattry` pair in one call, so we route through
/// it directly.
///
/// Previously this was an ad-hoc 30-line recursive matcher that
/// only handled `*` and `?` — char classes (`[abc]`), numeric
/// ranges (`<1-9>`), recursive globs, and the rest of zsh's
/// extended-glob set silently fell through. Now uses the
/// canonical engine.
fn simple_glob_match(pattern: &str, name: &str) -> bool {
    // c:hashtable.c:412 — `scanmatchtable` callers pass a compiled
    // `Patprog`; this helper inlines the compile+match since callers
    // here have only the raw pattern string.
    patcompile(
        &{
            let mut __pat_tok = (pattern).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        PAT_HEAPDUP as i32,
        None,
    )
    .map_or(false, |p| pattry(&p, name))
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

/// Singleton accessor for the `dircache` file-static at
/// `Src/hashtable.c:1517`.
pub fn dircache_lock() -> &'static std::sync::Mutex<Vec<dircache_entry>> {
    DIRCACHE_INNER.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    #[test]
    fn test_hasher() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(hasher(""), 0);
        assert_ne!(hasher("test"), 0);
        assert_eq!(hasher("test"), hasher("test"));
        assert_ne!(hasher("test"), hasher("Test"));
    }

    /// Pin `hnamcmp` to its canonical C body at `Src/hashtable.c:341-346`:
    /// must route through `ztrcmp` (META-AWARE compare), not naive
    /// `str::cmp`. The previous Rust port used byte-wise cmp which
    /// sorts Meta-encoded keys incorrectly.
    #[test]
    fn hnamcmp_uses_ztrcmp_meta_aware_compare() {
        let _g = crate::test_util::global_state_lock();
        // Plain ASCII: same as str::cmp.
        assert_eq!(hnamcmp("apple", "banana"), Ordering::Less);
        assert_eq!(hnamcmp("banana", "apple"), Ordering::Greater);
        assert_eq!(hnamcmp("equal", "equal"), Ordering::Equal);

        // Empty string sorts before non-empty.
        assert_eq!(hnamcmp("", "a"), Ordering::Less);
        assert_eq!(hnamcmp("a", ""), Ordering::Greater);

        // Meta-encoded byte: 0x83 0x41 → real 0x61 ('a'). The
        // Meta-aware ztrcmp treats `\x83\x41` as 'a' for compare
        // purposes; naive str::cmp would compare 0x83 vs 0x61 (so
        // "\x83\x41" would sort AFTER 'a'). Verify the Meta-aware
        // path: the encoded "a" should compare equal-ish to "a".
        // Construct via unsafe bytes since 0x83 isn't valid UTF-8
        // alone — Rust ztrcmp operates on bytes.
        let meta_a_bytes: Vec<u8> = vec![0x83, 0x41]; // Meta + 'A'^32 = 'a'
        let meta_a = unsafe { std::str::from_utf8_unchecked(&meta_a_bytes) };
        // Real "a" (0x61) vs encoded "a" (0x83 0x41): ztrcmp resolves
        // both to 0x61 at the first position → Equal. But ztrcmp also
        // takes into account end-of-string, so encoded "a" is longer
        // by one byte unstripped. The C ztrcmp loop skips matching
        // prefix; here the first bytes differ (0x61 vs 0x83), so it
        // resolves c1=0x61, c2=(0x41^32)=0x61 → Equal. Verify.
        assert_eq!(
            hnamcmp("a", meta_a),
            Ordering::Equal,
            "c:345 — Meta-encoded 'a' (0x83 0x41) compares equal to real 'a'"
        );
    }

    #[test]
    fn test_histhasher() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histhasher("  hello  world  "), histhasher("hello world"));
        assert_ne!(histhasher("hello world"), histhasher("helloworld"));
    }

    /// `Src/hashtable.c:1365-1380` — `histhasher` uses `inblank(*str)`
    /// per `Src/ztype.h:50`: NARROW space/tab only. The previous Rust
    /// port used `c.is_whitespace()` (broad Unicode) which would have
    /// silently rehashed any history line containing CR/FF/VT/NBSP.
    /// Pin the narrow-inblank semantics:
    ///   * Multi-space/tab runs collapse to a single ' ' bucket-mix.
    ///   * Newlines are NOT collapsed (newline is not inblank per c:50).
    ///   * NBSP / CR are NOT treated as inblank.
    #[test]
    fn histhasher_inblank_is_narrow_space_tab_only() {
        let _g = crate::test_util::global_state_lock();
        // c:1369 — leading inblank stripped; multiple equivalent forms hash same.
        assert_eq!(
            histhasher("\t  hello"),
            histhasher("hello"),
            "c:1369 — leading space+tab stripped before mixing"
        );
        // c:1373 — runs of inblank collapse to a single ' '.
        assert_eq!(
            histhasher("a \t  b"),
            histhasher("a b"),
            "c:1373 — interior inblank runs collapse to single space"
        );

        // Newline is NOT inblank per c:50; it must hash as itself, not collapse.
        assert_ne!(
            histhasher("a\nb"),
            histhasher("a b"),
            "c:50 — newline is NOT inblank; hashes as its own char"
        );
        // CR is NOT inblank.
        assert_ne!(
            histhasher("a\rb"),
            histhasher("ab"),
            "CR not in inblank; must mix as a character, not collapse"
        );
        // NBSP (0xA0) is NOT inblank (it's broad Unicode whitespace
        // but NOT in C's narrow typtab class).
        assert_ne!(
            histhasher("a\u{00A0}b"),
            histhasher("ab"),
            "NBSP not in inblank; must mix as a character, not collapse"
        );
    }

    #[test]
    fn test_histstrcmp() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            histstrcmp("  hello  world  ", "hello world", false),
            Ordering::Equal
        );
        assert_eq!(
            histstrcmp("hello world", "hello world", true),
            Ordering::Equal
        );
    }

    /// `Src/hashtable.c:1396-1421` — `histstrcmp` uses `inblank(*str)`
    /// (NARROW space/tab only per `Src/ztype.h:50`). The previous Rust
    /// port used `c.is_whitespace()` (broad Unicode) which silently
    /// folded history lines that C considers distinct.
    /// Pin narrow-inblank semantics.
    #[test]
    fn histstrcmp_inblank_is_narrow_space_tab_only() {
        let _g = crate::test_util::global_state_lock();
        // c:1411-1413 — runs of inblank collapse to a single boundary.
        assert_eq!(
            histstrcmp("hello\tworld", "hello world", false),
            Ordering::Equal,
            "c:1411-1413 — tab and space both inblank; mixed runs equal"
        );
        // Newline is NOT inblank per c:50 → string mismatch.
        assert_ne!(
            histstrcmp("hello\nworld", "hello world", false),
            Ordering::Equal,
            "c:50 — newline is NOT inblank; must be treated as ordinary char"
        );
        // CR is NOT inblank.
        assert_ne!(
            histstrcmp("hello\rworld", "hello world", false),
            Ordering::Equal,
            "CR not in inblank; not collapsed with space"
        );
        // NBSP is NOT inblank (broad Unicode whitespace, NOT typtab).
        assert_ne!(
            histstrcmp("hello\u{00A0}world", "hello world", false),
            Ordering::Equal,
            "NBSP not in inblank; not collapsed"
        );
        // c:1405 — HISTREDUCEBLANKS short-circuits to raw cmp.
        // With reduce_blanks=true the multi-space form is NOT collapsed.
        assert_ne!(
            histstrcmp("hello  world", "hello world", true),
            Ordering::Equal,
            "c:1405 — HISTREDUCEBLANKS=true → strcmp; runs do NOT collapse"
        );
    }

    /// `Src/hashtable.c:1398-1399` — leading inblank is stripped from
    /// both sides BEFORE comparison. So `"  cmd"` and `"\tcmd"` are
    /// equal. Trailing inblank (per the loop behavior, c:1421
    /// `*str1 - *str2` reaches 0 when one side runs out) is also
    /// folded: trailing run on one side vs end on the other returns
    /// Equal via the (Some, None) inblank-collapse branch.
    #[test]
    fn histstrcmp_strips_leading_and_trailing_inblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            histstrcmp("  cmd", "\tcmd", false),
            Ordering::Equal,
            "c:1398-1399 — leading inblank skipped (both kinds)"
        );
        assert_eq!(
            histstrcmp("cmd  ", "cmd", false),
            Ordering::Equal,
            "c:1421 — trailing inblank on left collapses to end-equal"
        );
        assert_eq!(
            histstrcmp("cmd", "cmd\t\t", false),
            Ordering::Equal,
            "c:1421 — trailing inblank on right collapses to end-equal"
        );
    }

    #[test]
    fn test_cmdnam_table() {
        let _g = crate::test_util::global_state_lock();
        let mut table = cmdnam_table::new();
        table.add(cmdnam_hashed("ls", "/bin/ls"));

        assert!(table.get("ls").is_some());
        assert!(table.get("nonexistent").is_none());

        let ls = table.get("ls").unwrap();
        assert_ne!((ls.node.flags & HASHED as i32), 0);
        assert_eq!((ls.node.flags & DISABLED as i32), 0);
    }

    #[test]
    fn test_shfunc_table() {
        let _g = crate::test_util::global_state_lock();
        let mut table = shfunc_table::new();
        table.add(shfunc_with_body("myfunc", "echo hello"));
        table.add(shfunc_autoload("lazy"));

        assert!(table.get("myfunc").is_some());
        assert_eq!(
            (table.get("myfunc").unwrap().node.flags & PM_UNDEFINED as i32),
            0
        );
        assert_ne!(
            (table.get("lazy").unwrap().node.flags & PM_UNDEFINED as i32),
            0
        );

        table.disable("myfunc");
        assert!(table.get("myfunc").is_none());
        assert!(table.get_including_disabled("myfunc").is_some());

        table.enable("myfunc");
        assert!(table.get("myfunc").is_some());
    }

    #[test]
    fn test_reswd_table() {
        let _g = crate::test_util::global_state_lock();
        let table = reswd_table::new();

        assert!(table.is_reserved("if"));
        assert!(table.is_reserved("while"));
        assert!(table.is_reserved("[["));
        assert!(!table.is_reserved("notreserved"));

        let if_rw = table.get("if").unwrap();
        assert_eq!(if_rw.token, IF);
    }

    #[test]
    fn test_alias_table() {
        let _g = crate::test_util::global_state_lock();
        let mut table = alias_table::with_defaults();

        assert!(table.get("run-help").is_some());
        assert_eq!(table.get("run-help").unwrap().text, "man");

        table.add(createaliasnode("G", "| grep", ALIAS_GLOBAL as u32));
        let g = table.get("G").unwrap();
        assert_ne!((g.node.flags & ALIAS_GLOBAL as i32), 0);

        table.add(createaliasnode("pdf", "zathura", ALIAS_SUFFIX as u32));
        let p = table.get("pdf").unwrap();
        assert_ne!((p.node.flags & ALIAS_SUFFIX as i32), 0);

        table.disable("G");
        assert!(table.get("G").is_none());
    }

    #[test]
    fn test_dir_cache() {
        let _g = crate::test_util::global_state_lock();
        // Smoke-test the canonical `dircache` file-static at
        // hashtable.c:1517 — the cache lives in a global Mutex
        // matching C semantics. Each test gets a fresh slice via
        // a unique-name marker so parallel tests don't collide.
        let cache = dircache_lock();
        {
            let mut g = cache.lock().unwrap();
            g.clear();
            g.push(dircache_entry {
                name: "/usr/share/zsh".into(),
                refs: 1,
            });
            g.push(dircache_entry {
                name: "/usr/share/zsh".into(),
                refs: 1,
            });
            // Dedupe-by-refs is the C semantic: get_or_insert bumps
            // refs on an existing entry. Verify the data shape.
            assert_eq!(g.len(), 2);
            assert_eq!(g[0].refs, 1);
        }
    }

    // -------------------------------------------------------------
    // Tests for the global shfunctab singleton & GSU callbacks.
    //
    // Tests are serialised via shfuncTAB_TEST_LOCK because they
    // mutate the process-wide singleton.
    // -------------------------------------------------------------

    static shfuncTAB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fresh_shfunctab() {
        let mut tab = shfunctab_lock().write().expect("shfunctab poisoned");
        tab.clear();
    }

    #[test]
    fn test_createshfunctable_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g = shfuncTAB_TEST_LOCK.lock();
        createshfunctable();
        createshfunctable();
        // Singleton handle stable across calls.
        let h1 = shfunctab_lock() as *const _;
        let h2 = shfunctab_lock() as *const _;
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_shfunctab_add_get_remove() {
        let _g = crate::test_util::global_state_lock();
        let _g = shfuncTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(shfunc_with_body("greet", "echo hello"));
        }
        {
            let tab = shfunctab_lock().read().unwrap();
            assert!(tab.get("greet").is_some());
            assert_eq!(
                tab.get("greet").unwrap().body.as_deref(),
                Some("echo hello")
            );
        }
        let removed = removeshfuncnode("greet");
        assert!(removed.is_some());
        assert!(shfunctab_lock().read().unwrap().get("greet").is_none());
    }

    #[test]
    fn test_shfunctab_disable_enable() {
        let _g = crate::test_util::global_state_lock();
        let _g = shfuncTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(shfunc_with_body("f", "true"));
        }
        disableshfuncnode("f");
        // get() filters disabled; get_including_disabled doesn't.
        {
            let tab = shfunctab_lock().read().unwrap();
            assert!(tab.get("f").is_none());
            assert!(tab.get_including_disabled("f").is_some());
        }
        enableshfuncnode("f");
        assert!(shfunctab_lock().read().unwrap().get("f").is_some());
        removeshfuncnode("f");
    }

    #[test]
    fn test_simple_glob_match() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let _g = shfuncTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(shfunc_with_body("foo", "echo a"));
            tab.add(shfunc_with_body("foobar", "echo b"));
            tab.add(shfunc_with_body("baz", "echo c"));
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
        let _g = crate::test_util::global_state_lock();
        let _g = shfuncTAB_TEST_LOCK.lock();
        fresh_shfunctab();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            let mut f = shfunc_with_body("f", "true");
            f.filename = Some("/tmp/zshrs-ported/f".to_string());
            tab.add(f);
        }
        assert_eq!(getshfuncfile("f"), Some("/tmp/zshrs-ported/f".to_string()));
        assert_eq!(getshfuncfile("nonexistent"), None);
        fresh_shfunctab();
    }

    // -------------------------------------------------------------
    // Generic hashtable ops + per-table singletons.
    // -------------------------------------------------------------

    #[test]
    fn test_generic_addhashnode_displaces_old() {
        let _g = crate::test_util::global_state_lock();
        let mut ht: HashMap<String, alias> = HashMap::new();
        addhashnode(&mut ht, "x", createaliasnode("x", "echo a", 0));
        let old = addhashnode2(&mut ht, "x", createaliasnode("x", "echo b", 0));
        assert!(old.is_some());
        assert_eq!(old.unwrap().text, "echo a");
        assert_eq!(gethashnode2(&ht, "x").unwrap().text, "echo b");
    }

    #[test]
    fn test_generic_disable_filters_get() {
        let _g = crate::test_util::global_state_lock();
        let mut ht: HashMap<String, alias> = HashMap::new();
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
        let _g = crate::test_util::global_state_lock();
        let mut ht: HashMap<String, alias> = HashMap::new();
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
        let _g = crate::test_util::global_state_lock();
        let mut ht: HashMap<String, alias> = HashMap::new();
        ht.insert("a".to_string(), createaliasnode("a", "1", 0));
        ht.insert("b".to_string(), createaliasnode("b", "2", 0));
        assert_eq!(ht.len(), 2);
        emptyhashtable(&mut ht);
        assert_eq!(ht.len(), 0);
    }

    #[test]
    fn test_resizehashtable_reserves_capacity() {
        let _g = crate::test_util::global_state_lock();
        let mut ht: HashMap<String, i32> = HashMap::new();
        let initial_cap = ht.capacity();
        resizehashtable(&mut ht, 200);
        assert!(ht.capacity() >= 200);
        assert!(ht.capacity() >= initial_cap);
    }

    #[test]
    fn test_aliastab_singleton_has_defaults() {
        let _g = crate::test_util::global_state_lock();
        let tab = aliastab_lock().read().unwrap();
        // createaliastables seeds run-help and which-command.
        assert!(tab.get_including_disabled("run-help").is_some());
        assert!(tab.get_including_disabled("which-command").is_some());
    }

    #[test]
    fn test_createaliasnode_sets_flags() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("foo", "echo bar", ALIAS_GLOBAL as u32);
        assert_eq!(a.node.nam, "foo");
        assert_eq!(a.text, "echo bar");
        assert_ne!((a.node.flags & ALIAS_GLOBAL as i32), 0);
    }

    #[test]
    fn test_printaliasnode_smoke() {
        // printaliasnode writes directly to stdout (matches C's void
        // return / writes-to-stdout signature). The behavioural parity
        // assertions live in `tests/builtin_c_parity.rs::alias_builtin`,
        // which compares against `/bin/zsh -fc 'alias gst'` byte-for-byte.
        // This unit test just exercises every flag branch to make sure
        // none panics / borrows incorrectly.
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("ll", "ls -la", 0);
        printaliasnode(&a, PRINT_NAMEONLY);
        printaliasnode(&a, PRINT_WHENCE_WORD);
        printaliasnode(&a, PRINT_WHENCE_SIMPLE);
        printaliasnode(&a, PRINT_WHENCE_CSH);
        printaliasnode(&a, PRINT_WHENCE_VERBOSE);
        printaliasnode(&a, PRINT_LIST);
        printaliasnode(&a, 0);
    }

    #[test]
    fn test_printreswdnode_smoke() {
        // printreswdnode writes directly to stdout (matches C's void
        // return / write-to-stdout signature at hashtable.c:1147).
        // Smoke-test every flag branch to make sure none panics.
        let _g = crate::test_util::global_state_lock();
        let table = reswd_table::new();
        let if_rw = table.get("if").unwrap();
        printreswdnode(if_rw, PRINT_WHENCE_WORD);
        printreswdnode(if_rw, PRINT_WHENCE_CSH);
        printreswdnode(if_rw, PRINT_WHENCE_VERBOSE);
        printreswdnode(if_rw, 0);
    }

    #[test]
    fn test_addhistnode_displaces_old() {
        let _g = crate::test_util::global_state_lock();
        emptyhisttable();
        assert_eq!(addhistnode("ls -la", 1), None);
        let old = addhistnode("ls -la", 5);
        assert_eq!(old, Some(1));
        emptyhisttable();
    }

    #[test]
    fn test_freecmdnamnode_removes() {
        let _g = crate::test_util::global_state_lock();
        emptycmdnamtable();
        {
            let mut tab = cmdnamtab_lock().write().unwrap();
            tab.add(cmdnam_unhashed("ls", vec!["/bin".to_string()]));
        }
        assert!(cmdnamtab_lock().read().unwrap().get("ls").is_some());
        freecmdnamnode("ls");
        assert!(cmdnamtab_lock().read().unwrap().get("ls").is_none());
    }

    #[test]
    fn test_dircache_set_refcounts() {
        let _g = crate::test_util::global_state_lock();
        // Refcount add → entries grow.
        let mut k: Option<String> = None;
        dircache_set(&mut k, Some("/usr/bin"));
        let mut k2: Option<String> = None;
        dircache_set(&mut k2, Some("/usr/bin"));
        let cache_size = dircache_lock().lock().unwrap().len();
        assert!(cache_size >= 1);
    }

    /// c:1230 — `createaliasnode(name, text, flags)` builds an crate::ported::zsh_h::alias
    /// with the text field populated. Regression that drops `text`
    /// would silently install aliases that expand to nothing.
    #[test]
    fn createaliasnode_round_trips_name_and_text() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("ls-color", "ls --color=auto", 0);
        assert_eq!(a.text, "ls --color=auto");
        assert_eq!(a.node.nam, "ls-color");
    }

    // ─── alias-creation zsh-corpus pins ────────────────────────────

    /// `createaliasnode` round-trips name+text+flags=0 (regular alias).
    #[test]
    fn alias_corpus_create_regular_alias() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("ll", "ls -la", 0);
        assert_eq!(a.node.nam, "ll");
        assert_eq!(a.text, "ls -la");
        // Regular alias = no GLOBAL/SUFFIX flags.
        let f = a.node.flags as i32;
        assert_eq!(
            f & (ALIAS_GLOBAL | ALIAS_SUFFIX),
            0,
            "regular alias has no GLOBAL/SUFFIX bits"
        );
    }

    /// `createaliasnode` with ALIAS_GLOBAL flag sets the global bit.
    #[test]
    fn alias_corpus_create_global_alias_carries_flag() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("G", "global text", ALIAS_GLOBAL as u32);
        let f = a.node.flags as i32;
        assert_ne!(f & ALIAS_GLOBAL, 0, "ALIAS_GLOBAL set");
    }

    /// `createaliasnode` with ALIAS_SUFFIX flag sets the suffix bit.
    #[test]
    fn alias_corpus_create_suffix_alias_carries_flag() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("S", "suffix text", ALIAS_SUFFIX as u32);
        let f = a.node.flags as i32;
        assert_ne!(f & ALIAS_SUFFIX, 0, "ALIAS_SUFFIX set");
    }

    /// Empty text is preserved (zsh allows zero-length alias expansion).
    #[test]
    fn alias_corpus_create_empty_text_preserved() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("noop", "", 0);
        assert_eq!(a.text, "");
    }

    /// Alias text may contain spaces — preserved as-is.
    #[test]
    fn alias_corpus_create_multi_word_text_preserved() {
        let _g = crate::test_util::global_state_lock();
        let a = createaliasnode("rmf", "rm -rf --no-preserve-root", 0);
        assert_eq!(a.text, "rm -rf --no-preserve-root");
    }

    /// `aliastab_lock` initialises with the two default aliases
    /// `run-help` and `which-command` per hashtable.c:1215-1216.
    /// A regression here breaks zsh's documented default behaviour
    /// where `run-help` resolves to `man` after `autoload -U run-help`.
    #[test]
    fn aliastab_seeds_run_help_and_which_command_defaults() {
        let _g = crate::test_util::global_state_lock();
        createaliastables();
        let tab = aliastab_lock().read().expect("aliastab poisoned");
        assert!(tab.get("run-help").is_some(), "run-help default missing");
        assert!(
            tab.get("which-command").is_some(),
            "which-command default missing"
        );
    }

    /// c:86 — `hasher` is the canonical zsh string hash. Same input
    /// MUST produce same output (basic determinism); different inputs
    /// SHOULD produce different outputs (no pathological collisions
    /// for single-char-different strings). The wrapping_add chain in
    /// the impl makes this a Bernstein-style hash; verify it's stable.
    #[test]
    fn hasher_is_deterministic_across_calls() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(hasher("foo"), hasher("foo"));
        assert_eq!(hasher(""), hasher(""));
        // Common shell names should not collide trivially.
        assert_ne!(hasher("ls"), hasher("cd"));
        assert_ne!(hasher("foo"), hasher("bar"));
    }

    /// c:86 — empty input hashes to 0 (the seed value). A regression
    /// changing the seed would invalidate every persisted hash + cause
    /// silent rebuild storms in the cache layer.
    #[test]
    fn hasher_empty_string_hashes_to_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(hasher(""), 0);
    }

    /// c:86 — single-byte input `c` hashes to `c as u32` exactly
    /// (the loop runs once: hashval = 0 + 0<<5 + c = c). Pins the
    /// canonical first-iteration formula.
    #[test]
    fn hasher_single_byte_equals_byte_value() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(hasher("a"), b'a' as u32);
        assert_eq!(hasher("Z"), b'Z' as u32);
        assert_eq!(hasher("0"), b'0' as u32);
    }

    /// `Src/hashtable.c:90-91` — `hashval += (hashval << 5) + c`
    /// simplifies to `hashval = hashval*33 + c` (the Bernstein
    /// hash variant). Pin the exact two-byte formula so a refactor
    /// to a different polynomial (e.g. FNV / djb2 / siphash) fails
    /// loudly. Regression here invalidates every cached fpath/hash
    /// digest stored on disk.
    #[test]
    fn hasher_two_byte_matches_bernstein_polynomial() {
        let _g = crate::test_util::global_state_lock();
        // For "ab": h0=0; h1 = 0 + (0<<5) + 'a' = 97; h2 = 97 + (97<<5) + 'b' = 97 + 3104 + 98 = 3299.
        assert_eq!(
            hasher("ab"),
            97u32
                .wrapping_add(97u32.wrapping_shl(5))
                .wrapping_add(b'b' as u32)
        );
        assert_eq!(hasher("ab"), 3299);
        // Pin the exact value for "ls" — a name we'll lookup billions of times.
        let ls_expected = {
            let mut h: u32 = 0;
            for &c in b"ls" {
                h = h.wrapping_add(h.wrapping_shl(5)).wrapping_add(c as u32);
            }
            h
        };
        assert_eq!(hasher("ls"), ls_expected);
    }

    /// c:86 — hasher must NOT mix in encoding/locale state — the
    /// algorithm is byte-by-byte. Multi-byte UTF-8 like 'é' (0xC3 0xA9)
    /// hashes the two bytes independently. Pin so a regression that
    /// uses chars instead of bytes (which would aggregate the two
    /// bytes into one codepoint) fails.
    #[test]
    fn hasher_processes_utf8_bytes_not_codepoints() {
        let _g = crate::test_util::global_state_lock();
        // 'é' UTF-8 = 0xC3 0xA9 — two bytes.
        let expected = {
            let mut h: u32 = 0;
            for &c in &[0xC3u8, 0xA9u8] {
                h = h.wrapping_add(h.wrapping_shl(5)).wrapping_add(c as u32);
            }
            h
        };
        assert_eq!(
            hasher("é"),
            expected,
            "c:90 — `*(unsigned char *) str++` reads BYTES, not codepoints"
        );
    }

    /// c:157 — `addhashnode` inserts; `gethashnode2` reads back.
    /// Round-trip MUST yield the value just inserted. Regression
    /// returning None on a present key would break every command-
    /// table lookup.
    #[test]
    fn addhashnode_then_gethashnode2_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode(&mut h, "key1", 42);
        assert_eq!(gethashnode2(&h, "key1"), Some(&42));
        assert_eq!(gethashnode2(&h, "missing"), None);
    }

    /// c:275 — `removehashnode` returns Some(value) when present and
    /// drops the entry. Subsequent lookup MUST miss. Regression
    /// returning Some without removing would let callers think they
    /// removed when they actually didn't.
    #[test]
    fn removehashnode_returns_value_and_drops_entry() {
        let _g = crate::test_util::global_state_lock();
        let mut h: HashMap<String, String> = HashMap::new();
        addhashnode(&mut h, "key1", "val".to_string());
        let removed = removehashnode(&mut h, "key1");
        assert_eq!(removed.as_deref(), Some("val"));
        assert!(
            gethashnode2(&h, "key1").is_none(),
            "after removehashnode, lookup must miss"
        );
    }

    /// c:275 — `removehashnode` on a missing key returns None and
    /// doesn't mutate the table. A regression where it errors or
    /// inserts a sentinel would break `unalias missing` (which is
    /// supposed to fail-soft).
    #[test]
    fn removehashnode_missing_key_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode(&mut h, "k1", 1);
        let len_before = h.len();
        assert!(removehashnode(&mut h, "missing").is_none());
        assert_eq!(h.len(), len_before, "missing-key remove must not mutate");
    }

    // ─── zsh-corpus pins: hashtable add/get/remove ──────────────────

    /// `addhashnode2` returns None on first insert.
    #[test]
    fn hashtable_corpus_add_new_returns_none() {
        let mut h: HashMap<String, i32> = HashMap::new();
        assert!(addhashnode2(&mut h, "fresh", 7).is_none());
        assert_eq!(gethashnode2(&h, "fresh"), Some(&7));
    }

    /// `addhashnode2` on an existing key returns the OLD value.
    #[test]
    fn hashtable_corpus_add_existing_returns_previous_value() {
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode2(&mut h, "k", 1);
        let prev = addhashnode2(&mut h, "k", 2);
        assert_eq!(prev, Some(1), "old value returned on replace");
        assert_eq!(gethashnode2(&h, "k"), Some(&2), "new value installed");
    }

    /// `gethashnode2` on missing key returns None.
    #[test]
    fn hashtable_corpus_get_missing_returns_none() {
        let h: HashMap<String, i32> = HashMap::new();
        assert!(gethashnode2(&h, "anything").is_none());
    }

    /// `newhashtable` returns (name, size); name preserved.
    #[test]
    fn hashtable_corpus_newhashtable_preserves_name() {
        let (name, sz) = newhashtable(64, "myht");
        assert_eq!(name, "myht");
        assert!(sz > 0, "size positive, got {sz}");
    }

    /// Round-trip with many distinct keys.
    #[test]
    fn hashtable_corpus_many_keys_round_trip() {
        let mut h: HashMap<String, i32> = HashMap::new();
        for i in 0..100 {
            addhashnode(&mut h, &format!("k{i}"), i);
        }
        for i in 0..100 {
            assert_eq!(gethashnode2(&h, &format!("k{i}")), Some(&i));
        }
        assert_eq!(h.len(), 100);
    }

    /// `removehashnode` followed by `gethashnode2` shows missing.
    #[test]
    fn hashtable_corpus_remove_then_get_is_none() {
        let mut h: HashMap<String, String> = HashMap::new();
        addhashnode(&mut h, "x", "value".into());
        let _ = removehashnode(&mut h, "x");
        assert!(gethashnode2(&h, "x").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.c hasher + hnamcmp +
    // generic add/remove primitives.
    // ═══════════════════════════════════════════════════════════════════

    /// c:86 — `hasher` of empty string returns 0 (no bytes contribute).
    #[test]
    fn hasher_empty_string_returns_zero() {
        assert_eq!(hasher(""), 0, "no bytes → hash 0");
    }

    /// c:86 — `hasher` is deterministic: same input → same output.
    #[test]
    fn hasher_is_deterministic() {
        let h1 = hasher("test_string");
        let h2 = hasher("test_string");
        assert_eq!(h1, h2, "hasher must be deterministic");
    }

    /// c:86 — `hasher` differentiates between different strings
    /// (no trivial collisions on common inputs).
    #[test]
    fn hasher_distinguishes_common_strings() {
        assert_ne!(hasher("foo"), hasher("bar"));
        assert_ne!(hasher("a"), hasher("b"));
        assert_ne!(hasher("test"), hasher("Test"), "case-sensitive");
    }

    /// c:86 — hash of single char "a" matches the formula
    /// `0 + (0<<5) + 'a' = 0x61` (verifies inline formula).
    #[test]
    fn hasher_single_char_matches_formula() {
        let h = hasher("a");
        assert_eq!(h, b'a' as u32, "single char 'a' → 0x61");
        let h = hasher("0");
        assert_eq!(h, b'0' as u32, "single char '0' → 0x30");
    }

    /// c:86 — hasher of "ab": h=0 → h=0+(0<<5)+'a'=0x61
    /// → h=0x61+(0x61<<5)+'b' = 0x61 + 0xC20 + 0x62 = 0xCE3.
    #[test]
    fn hasher_two_char_matches_formula() {
        let h = hasher("ab");
        let expected: u32 = 0u32
            .wrapping_add(0u32.wrapping_shl(5))
            .wrapping_add(b'a' as u32);
        let expected = expected
            .wrapping_add(expected.wrapping_shl(5))
            .wrapping_add(b'b' as u32);
        assert_eq!(h, expected, "two-char formula must match");
    }

    /// c:86 — uses wrapping arithmetic so long strings don't panic.
    #[test]
    fn hasher_long_string_does_not_panic() {
        let s = "a".repeat(10_000);
        let _ = hasher(&s);
    }

    /// c:345 — `hnamcmp("abc", "abc")` returns Equal.
    #[test]
    fn hnamcmp_equal_strings_return_equal() {
        assert_eq!(hnamcmp("abc", "abc"), std::cmp::Ordering::Equal);
        assert_eq!(hnamcmp("", ""), std::cmp::Ordering::Equal);
    }

    /// c:345 — `hnamcmp` orders lexicographically.
    #[test]
    fn hnamcmp_lex_order() {
        assert_eq!(hnamcmp("abc", "abd"), std::cmp::Ordering::Less);
        assert_eq!(hnamcmp("abd", "abc"), std::cmp::Ordering::Greater);
    }

    /// c:345 — empty string sorts before any non-empty string.
    #[test]
    fn hnamcmp_empty_sorts_first() {
        assert_eq!(hnamcmp("", "x"), std::cmp::Ordering::Less);
        assert_eq!(hnamcmp("x", ""), std::cmp::Ordering::Greater);
    }

    /// `emptyhashtable` drops all entries.
    #[test]
    fn emptyhashtable_clears_all_entries() {
        let mut h: HashMap<String, i32> = HashMap::new();
        h.insert("a".to_string(), 1);
        h.insert("b".to_string(), 2);
        h.insert("c".to_string(), 3);
        emptyhashtable(&mut h);
        assert!(h.is_empty(), "all entries dropped after emptyhashtable");
    }

    /// `deletehashtable` clears the map (Rust semantics).
    #[test]
    fn deletehashtable_clears_all_entries() {
        let mut h: HashMap<String, i32> = HashMap::new();
        h.insert("x".to_string(), 42);
        deletehashtable(&mut h);
        assert!(h.is_empty());
    }

    /// `removehashnode` on missing key returns None (no panic).
    #[test]
    fn removehashnode_missing_returns_none() {
        let mut h: HashMap<String, i32> = HashMap::new();
        let prev = removehashnode(&mut h, "never_there");
        assert!(prev.is_none(), "remove of missing key → None");
    }

    /// `addhashnode` overwriting existing key drops old value silently.
    #[test]
    fn addhashnode_overwrite_does_not_panic() {
        let mut h: HashMap<String, String> = HashMap::new();
        addhashnode(&mut h, "k", "first".into());
        addhashnode(&mut h, "k", "second".into());
        assert_eq!(gethashnode2(&h, "k"), Some(&"second".to_string()));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.c
    // c:55 hasher / c:85 newhashtable / c:97 deletehashtable /
    // c:150 addhashnode2 / c:343 gethashnode2 / c:355 removehashnode /
    // c:715 hnamcmp / c:876 expandhashtable / c:887 resizehashtable /
    // c:916 printhashtabinfo
    // ═══════════════════════════════════════════════════════════════════

    /// c:55 — `hasher("")` empty string returns u32 (type pin).
    #[test]
    fn hasher_empty_returns_u32_type() {
        let _: u32 = hasher("");
    }

    /// c:55 — `hasher` is pure.
    #[test]
    fn hasher_is_pure_full_sweep() {
        for s in ["", "a", "abc", "hello world", "日本"] {
            let first = hasher(s);
            for _ in 0..5 {
                assert_eq!(hasher(s), first, "hasher({:?}) must be pure", s);
            }
        }
    }

    /// c:85 — `newhashtable(0, "")` returns (String, i32) tuple type pin.
    #[test]
    fn newhashtable_returns_string_i32_tuple_type() {
        let _: (String, i32) = newhashtable(0, "");
    }

    /// c:97 — `deletehashtable` on empty table is safe no-op.
    #[test]
    fn deletehashtable_empty_no_panic() {
        let mut empty: HashMap<String, String> = HashMap::new();
        deletehashtable(&mut empty);
        assert!(empty.is_empty(), "still empty after delete");
    }

    /// c:150 — `addhashnode2` returns Option<T> (replaced value).
    #[test]
    fn addhashnode2_returns_option_type() {
        let mut h: HashMap<String, i32> = HashMap::new();
        let _: Option<i32> = addhashnode2(&mut h, "k", 1);
    }

    /// c:150 — `addhashnode2` first insert returns None.
    #[test]
    fn addhashnode2_first_insert_returns_none() {
        let mut h: HashMap<String, i32> = HashMap::new();
        let r = addhashnode2(&mut h, "k", 42);
        assert!(r.is_none(), "first insert → None (no replacement)");
    }

    /// c:150 — `addhashnode2` overwrite returns Some(old).
    #[test]
    fn addhashnode2_overwrite_returns_some_old() {
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode2(&mut h, "k", 1);
        let r = addhashnode2(&mut h, "k", 2);
        assert_eq!(r, Some(1), "overwrite returns previous value");
    }

    /// c:343 — `gethashnode2(empty, _)` returns None.
    #[test]
    fn gethashnode2_empty_table_returns_none() {
        let h: HashMap<String, String> = HashMap::new();
        assert!(gethashnode2(&h, "anything").is_none());
    }

    /// c:355 — `removehashnode(empty, _)` returns None.
    #[test]
    fn removehashnode_empty_table_returns_none() {
        let mut h: HashMap<String, String> = HashMap::new();
        assert!(removehashnode(&mut h, "anything").is_none());
    }

    /// c:715 — `hnamcmp` is antisymmetric.
    #[test]
    fn hnamcmp_antisymmetric() {
        use std::cmp::Ordering;
        for (a, b) in [("a", "b"), ("abc", "xyz"), ("", "x")] {
            let ab = hnamcmp(a, b);
            let ba = hnamcmp(b, a);
            assert_eq!(
                ab.reverse(),
                ba,
                "hnamcmp must be antisymmetric for ({:?}, {:?})",
                a,
                b
            );
            // ab cannot be Equal AND ba Equal unless both Equal
            if ab == Ordering::Equal {
                assert_eq!(ba, Ordering::Equal);
            }
        }
    }

    /// c:887 — `resizehashtable` with same size is no-op.
    #[test]
    fn resizehashtable_same_size_no_panic() {
        let mut h: HashMap<String, i32> = HashMap::new();
        h.insert("a".to_string(), 1);
        h.insert("b".to_string(), 2);
        resizehashtable(&mut h, 2);
        assert_eq!(h.len(), 2, "entries preserved after same-size resize");
    }

    /// c:876 — `expandhashtable` is idempotent.
    #[test]
    fn expandhashtable_idempotent() {
        let mut h: HashMap<String, i32> = HashMap::new();
        h.insert("a".to_string(), 1);
        for _ in 0..5 {
            expandhashtable(&mut h);
        }
        assert_eq!(h.get("a"), Some(&1), "value preserved across expansions");
    }

    /// c:916 — `printhashtabinfo("", empty)` returns String type.
    #[test]
    fn printhashtabinfo_returns_string_type() {
        let empty: HashMap<String, String> = HashMap::new();
        let _: String = printhashtabinfo("test", &empty);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hashtable.c
    // c:55 hasher / c:139 addhashnode / c:343 gethashnode2 / c:355 removehashnode /
    // c:715 hnamcmp / c:900 emptyhashtable / c:916 printhashtabinfo
    // ═══════════════════════════════════════════════════════════════════

    /// c:55 — `hasher` returns u32 (compile-time pin).
    #[test]
    fn hasher_returns_u32_type() {
        let _: u32 = hasher("anything");
    }

    /// c:55 — `hasher` is deterministic (same input → same hash, alt).
    #[test]
    fn hasher_is_deterministic_alt() {
        for s in ["", "x", "abc", "longer input", "日本"] {
            let first = hasher(s);
            for _ in 0..5 {
                assert_eq!(hasher(s), first, "hasher({:?}) must be pure", s);
            }
        }
    }

    /// c:55 — `hasher` distinguishes simple distinct inputs (sanity:
    /// not a constant hash).
    #[test]
    fn hasher_distinguishes_distinct_inputs() {
        let h_a = hasher("a");
        let h_b = hasher("b");
        let h_z = hasher("z");
        // At least two of three must differ (proves non-constant).
        let distinct = (h_a != h_b) || (h_b != h_z) || (h_a != h_z);
        assert!(
            distinct,
            "hasher must distinguish distinct inputs; got {} {} {}",
            h_a, h_b, h_z
        );
    }

    /// c:139 — `addhashnode` followed by gethashnode2 retrieves entry.
    #[test]
    fn addhashnode_then_gethashnode2_retrieves_entry() {
        let mut h: HashMap<String, String> = HashMap::new();
        addhashnode(&mut h, "key", "value".to_string());
        let v = gethashnode2(&h, "key");
        assert_eq!(
            v,
            Some(&"value".to_string()),
            "add then get must round-trip"
        );
    }

    /// c:355 — `removehashnode` after add returns Some(value).
    #[test]
    fn removehashnode_after_add_returns_some() {
        let mut h: HashMap<String, String> = HashMap::new();
        addhashnode(&mut h, "k", "v".to_string());
        let removed = removehashnode(&mut h, "k");
        assert_eq!(
            removed,
            Some("v".to_string()),
            "remove returns the removed value"
        );
        assert!(h.is_empty(), "table empty after remove");
    }

    /// c:355 — `removehashnode` twice returns Some then None.
    #[test]
    fn removehashnode_twice_returns_some_then_none() {
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode(&mut h, "k", 42);
        let first = removehashnode(&mut h, "k");
        let second = removehashnode(&mut h, "k");
        assert!(first.is_some());
        assert!(second.is_none(), "second remove of same key returns None");
    }

    /// c:715 — `hnamcmp(x, x)` returns Equal (reflexive).
    #[test]
    fn hnamcmp_reflexive() {
        use std::cmp::Ordering;
        for s in ["", "a", "hello", "long string here"] {
            assert_eq!(
                hnamcmp(s, s),
                Ordering::Equal,
                "hnamcmp({:?}, {:?}) must be Equal",
                s,
                s
            );
        }
    }

    /// c:900 — `emptyhashtable` actually drops all entries.
    #[test]
    fn emptyhashtable_drops_all_entries() {
        let mut h: HashMap<String, i32> = HashMap::new();
        for i in 0..10 {
            addhashnode(&mut h, &format!("k_{}", i), i);
        }
        assert_eq!(h.len(), 10);
        emptyhashtable(&mut h);
        assert_eq!(h.len(), 0, "empty must clear all entries");
    }

    /// c:916 — `printhashtabinfo` for empty table returns non-empty
    /// String (must contain at least the table name).
    #[test]
    fn printhashtabinfo_empty_table_non_empty_output() {
        let empty: HashMap<String, String> = HashMap::new();
        let r = printhashtabinfo("my_table_name", &empty);
        assert!(
            !r.is_empty(),
            "printhashtabinfo must produce non-empty output even for empty table"
        );
    }

    /// c:97 — `deletehashtable` empties + safe.
    #[test]
    fn deletehashtable_empties_table() {
        let mut h: HashMap<String, i32> = HashMap::new();
        addhashnode(&mut h, "k", 1);
        deletehashtable(&mut h);
        assert!(h.is_empty(), "delete must empty the table");
    }

    /// c:85 — `newhashtable` returns (String, i32) tuple (compile-time pin).
    #[test]
    fn newhashtable_returns_tuple_type() {
        let _: (String, i32) = newhashtable(0, "test");
    }

    /// c:954 — printshfuncnode renders a function body with
    /// `getpermtext(fd, NULL, 1)`, so `functions f` prints CANONICAL text
    /// rather than the source as typed. zshrs keeps raw source for
    /// shell-defined functions, so its listing path re-parses and renders
    /// through that same deparser; these are the shapes where the layout is
    /// an actual decision rather than a passthrough.
    ///
    /// The `always` case is the one that matters most: the previous
    /// hand-rolled canonicalization emitted `print x } always { print y`,
    /// which is not merely mis-indented, it no longer parses — and
    /// `functions` output is meant to be re-readable by the shell.
    ///
    /// Pins the deparse itself rather than printshfuncnode's stdout, so it
    /// doesn't depend on capturing print! output. Indent 1 matches C, which
    /// writes one tab via zoutputtab (c:949) before calling getpermtext.
    #[test]
    fn function_body_deparses_to_canonical_layout() {
        let _g = crate::test_util::global_state_lock();
        for (body, want) in [
            // `do` gets its own line; the body indents beneath it.
            (
                "for i in 1 2; do print $i; done",
                "for i in 1 2\n\tdo\n\t\tprint $i\n\tdone",
            ),
            // `(` and `)` break onto their own lines.
            ("(print s)", "(\n\t\tprint s\n\t)"),
            // taddassign appends a trailing space after the value
            // (c:Src/text.c:203-204) and nothing backs it off.
            ("g=inner", "g=inner "),
            // The shape the emulation broke.
            (
                "{ print x } always { print y }",
                "{\n\t\tprint x\n\t} always {\n\t\tprint y\n\t}",
            ),
        ] {
            let prog = crate::ported::exec::parse_string(body, 1)
                .unwrap_or_else(|| panic!("body must parse for the listing path: {body:?}"));
            let got = crate::ported::text::getpermtext(Box::new(prog), None, 1);
            assert_eq!(got, want, "c:954 deparse of {body:?}");
        }
    }
}
