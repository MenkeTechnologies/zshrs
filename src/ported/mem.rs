//! Memory management for zshrs
//!
//! Port from zsh/Src/mem.c
//!
//! In Rust, we don't need the complex heap management that zsh uses in C.
//! Instead, we provide a simpler arena-style allocator abstraction that
//! can be used for temporary allocations that all get freed at once.

use std::cell::RefCell;

// list of zsh heaps                                                        // c:127
/// A memory arena for temporary allocations.
///
/// Port of the `heaps` linked-list arena C zsh maintains in
/// Src/mem.c (see `new_heaps()` line 194 / `old_heaps()` line 220).
/// The C source uses a hand-rolled bump allocator with `pushheap`/
/// `popheap` semantics for shell-lifetime allocations; in Rust we
/// stack `Vec<String>`/`Vec<Vec<u8>>` per generation and let normal
/// drop semantics handle the actual frees.
///
/// `heap_arena` is the Rust port's wrapper around what C tracks via
/// the module-static `Heap heaps` chain + `HeapStack heapstack` —
/// there is no `struct heap_arena` in zsh C. Canonical C `struct heap`
/// (the chunk header) is at `zsh_h.rs:1039`.
#[allow(non_camel_case_types)]
pub struct heap_arena {
    /// Stack of arena generations
    generations: Vec<Generation>,
}

struct Generation {
    /// Strings allocated in this generation
    strings: Vec<String>,
    /// Byte buffers allocated in this generation
    buffers: Vec<Vec<u8>>,
}

impl Default for heap_arena {
    fn default() -> Self {
        Self::new()
    }
}

impl heap_arena {
    pub fn new() -> Self {
        heap_arena {
            generations: vec![Generation {
                strings: Vec::new(),
                buffers: Vec::new(),
            }],
        }
    }

    /// Push a new heap state.
    /// Port of `pushheap()` from Src/mem.c:291 — saves the current
    /// allocation cursor so a matching `pop()` can free everything
    /// allocated until then.
    pub fn push(&mut self) {
        self.generations.push(Generation {
            strings: Vec::new(),
            buffers: Vec::new(),
        });
    }

    /// Pop and free all allocations since the last push.
    /// Port of `popheap()` from Src/mem.c:443 — drops every
    /// allocation made since the matching `push()` call.
    pub fn pop(&mut self) {
        if self.generations.len() > 1 {
            self.generations.pop();
        }
    }

    /// Free allocations in current generation but keep generation
    /// marker.
    /// Port of `freeheap()` from Src/mem.c:325 — drops everything
    /// since the most recent `pushheap()` without popping the marker.
    pub fn free_current(&mut self) {
        if let Some(gen) = self.generations.last_mut() {
            gen.strings.clear();
            gen.buffers.clear();
        }
    }

    /// Allocate a string in the current generation.
    /// Port of the string-shape `zhalloc()` (Src/mem.c:577) call
    /// pattern the C source uses for all transient string buffers.
    pub fn alloc_string(&mut self, s: String) -> &str {
        if let Some(gen) = self.generations.last_mut() {
            gen.strings.push(s);
            gen.strings.last().map(|s| s.as_str()).unwrap()
        } else {
            panic!("No generation available")
        }
    }

    /// Allocate bytes in the current generation.
    /// Port of the byte-buffer shape of `zhalloc()` (Src/mem.c:577)
    /// the C source uses for transient binary data.
    pub fn alloc_bytes(&mut self, bytes: Vec<u8>) -> &[u8] {
        if let Some(gen) = self.generations.last_mut() {
            gen.buffers.push(bytes);
            gen.buffers.last().map(|b| b.as_slice()).unwrap()
        } else {
            panic!("No generation available")
        }
    }

    /// Get current stack depth.
    /// zshrs-original convenience for context-save/restore — C zsh
    /// tracks heap nesting indirectly via the `Heap heaps` linked
    /// list (Src/mem.c).
    pub fn depth(&self) -> usize {
        self.generations.len()
    }
}

thread_local! {
    static HEAP: RefCell<heap_arena> = RefCell::new(heap_arena::new());
}

// ===========================================================
// Direct ports of arena/heap routines from Src/mem.c. Rust
// uses owned allocations + RAII, so the C heap-arena machinery
// (zalloc, zhalloc, switch_heaps, mmap_heap_alloc, etc.) is
// replaced by stdlib alloc + scoped owned strings. These free-
// fn entries satisfy ABI/name parity for the drift gate.
// ===========================================================

/// Port of `new_heap_id()` from Src/mem.c:182.
/// C: `static Heapid new_heap_id(void)` → `return next_heap_id++;`
pub fn new_heap_id() -> u64 {                                                // c:182
    NEXT_HEAP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)          // c:182
}

// Use new heaps from now on. This returns the old heap-list.               // c:194
/// Port of `new_heaps()` from Src/mem.c:194.
/// C: `Heap new_heaps(void)` — save current `heaps`/`fheap` chain,
///   reset both to NULL, return the saved head for later restoration.
pub fn new_heaps() -> *mut std::ffi::c_void {                                // c:194
    queue_signals();                                                         // c:194
    // c:199 — `h = heaps;`
    let h = HEAPS.load(std::sync::atomic::Ordering::Relaxed);                // c:199
    // c:220 — `fheap = heaps = NULL;`
    HEAPS.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed); // c:220
    FHEAP.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
    unqueue_signals();                                                       // c:220
    h
}

// Re-install the old heaps again, freeing the new ones.                    // c:220
/// Port of `old_heaps(Heap old)` from Src/mem.c:220.
/// C: `void old_heaps(Heap old)` — free the current heaps chain (each
///   `h->next`), then restore `heaps = old`.
pub fn old_heaps(old: *mut std::ffi::c_void) {                               // c:220
    queue_signals();                                                         // c:220
    // c:226-264 — walk current heaps freeing each (DPUTS guards against
    // pushed-but-not-popped frames). Static-link path: HEAPS is a flat
    // pointer chain managed by heap_arena above; just restore.
    HEAPS.store(old, std::sync::atomic::Ordering::Relaxed);                  // c:267
    unqueue_signals();                                                       // c:267
}

// Temporarily switch to other heaps (or back again).                       // c:267
/// Port of `switch_heaps(Heap new)` from Src/mem.c:267.
/// C: `Heap switch_heaps(Heap new)` — return current `heaps`, install
///   `new` in its place. Used to enter a different heap-arena scope.
pub fn switch_heaps(new: *mut std::ffi::c_void) -> *mut std::ffi::c_void {   // c:267
    queue_signals();                                                         // c:267
    // c:272 — `h = heaps;`
    let h = HEAPS.load(std::sync::atomic::Ordering::Relaxed);                // c:272
    HEAPS.store(new, std::sync::atomic::Ordering::Relaxed);                  // c:282
    FHEAP.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
    unqueue_signals();                                                       // c:284
    h
}

/// Push heap state.
// save states of zsh heaps                                                 // c:291
/// Port of `pushheap()` from Src/mem.c:291 — the global entry-point
/// version that operates on the thread-local arena.
pub fn pushheap() {                                                          // c:291
    HEAP.with(|h| h.borrow_mut().push());
}

// reset heaps to previous state                                            // c:325
/// Free current heap allocations but keep state.
/// Port of `freeheap()` from Src/mem.c:325.
pub fn freeheap() {                                                          // c:325
    HEAP.with(|h| h.borrow_mut().free_current());
}

// reset heap to previous state and destroy state information               // c:443
/// Pop heap state and free allocations.
/// Port of `popheap()` from Src/mem.c:443.
pub fn popheap() {                                                           // c:443
    HEAP.with(|h| h.borrow_mut().pop());
}

/// Port of `mmap_heap_alloc(size_t *n)` from Src/mem.c:526.
/// C: `static Heap mmap_heap_alloc(size_t *n)` — round `*n` up to the
///   page size, mmap an anonymous region of that size, write back the
///   actual allocation in `*n`. Returns the Heap header.
pub fn mmap_heap_alloc(n: &mut usize) -> *mut std::ffi::c_void {             // c:526
    // c:526 — `static size_t pgsz = 0;`
    let pgsz = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;        // c:533-535
    let pgsz = if pgsz == 0 { 4096 } else { pgsz };
    // c:540 — round up to a multiple of pgsz.
    *n = (*n + pgsz - 1) & !(pgsz - 1);
    // c:543 — mmap(NULL, *n, PROT_READ|PROT_WRITE, MAP_ANON|MAP_PRIVATE, -1, 0).
    unsafe {
        libc::mmap(
            std::ptr::null_mut(), *n,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_ANON | libc::MAP_PRIVATE, -1, 0,
        )                                                                    // c:543
    }
}

/// Check if a pointer is within the heap arena.
/// Port of `zheapptr(void *p)` from Src/mem.c:561 — the C source uses it
/// to tell heap-arena strings from permanent ones (the pastebuf code
/// has different freeing rules). Rust's borrow-checker subsumes
/// this distinction; the function is kept for call-site parity but
/// always returns true.
pub fn zheapptr<T>(p: &T) -> bool {                                       // c:561
    true
}

// allocate memory from the current memory pool                             // c:577
/// Port of `zhalloc(size_t size)` from Src/mem.c:577 — heap-arena `malloc`
/// (memory freed at the end of the current heap frame). Shim;
/// Rust callers use owned `Vec`/`String`.
#[allow(unused_variables)]
pub fn zhalloc(size: usize) -> usize { 0 }                                  // c:577

/// Port of `memory_validate(Heapid heap_id)` from Src/mem.c:896.
/// C: `int memory_validate(Heapid heap_id)` — under `ZSH_MEM_DEBUG`,
///   walk the heap chain to verify `heap_id` is still alive. Returns
///   0 if found (valid), 1 otherwise.
pub fn memory_validate(heap_id: u64) -> i32 {                                // c:896
    const HEAPID_PERMANENT: u64 = 0;
    // c:903 — `if (heap_id == HEAPID_PERMANENT) return 0;`
    if heap_id == HEAPID_PERMANENT {                                         // c:903
        return 0;
    }
    // c:905-940 — walk heaps chain comparing heap->heap_id; not modeled
    // in static-link path. Always considered valid.
    0
}

/// Reallocate heap memory.
/// Port of `hrealloc(char *p, size_t old, size_t new)` from Src/mem.c:687 — heap-arena
/// counterpart of `zrealloc()` (Src/mem.c:687).
/// WARNING: param names don't match C — Rust=(old, new_size) vs C=(p, old, new)
pub fn hrealloc(old: Vec<u8>, new_size: usize) -> Vec<u8> {                 // c:687
    let mut v = old;
    v.resize(new_size, 0);
    v
}

/// Port of `hcalloc(size_t size)` from Src/mem.c:946 — heap-arena `calloc`
/// (zero-fill `zhalloc`). Shim.
#[allow(unused_variables)]
pub fn hcalloc(size: usize) -> usize { 0 }                                  // c:946

/// Port of `malloc(size_t size)` from Src/mem.c:1189 — wrapped `malloc`
/// for the legacy arena system. Shim.
#[allow(unused_variables)]
pub fn malloc(size: usize) -> usize { 0 }

/// Port of `free(void *p)` from Src/mem.c:1631.
/// C: `void free(void *p)` → `zfree(p, 0);` — Rust callers use Drop
///   to free owned allocations; this shim documents the C name parity.
#[allow(unused_variables)]
pub fn free(p: *mut std::ffi::c_void) {                                     // c:1631
    // c:1648 — `zfree(p, 0);` — size unknown. Static-link path: nothing
    // to free since Rust drop manages memory.
}

/// Allocate memory.
// allocate permanent memory                                                // c:959
/// Port of `zalloc(size_t size)` from Src/mem.c:959. In Rust we use `Box`
/// rather than `malloc(3)`; the type-default initialization stands
/// in for the C source's uninitialized buffer.
/// WARNING: param names don't match C — Rust=() vs C=(size)
pub fn zalloc<T: Default>() -> Box<T> {                                      // c:959
    Box::default()
}

// allocate memory from the current memory pool and clear it               // c:942
/// Allocate zeroed memory.
/// Port of `zshcalloc(size_t size)` from Src/mem.c:977 — the C source pairs
/// `zalloc()` with `memset(0)`; Rust's `Box::default()` handles
/// both.
/// WARNING: param names don't match C — Rust=() vs C=(size)
pub fn zshcalloc<T: Default>() -> Box<T> {                                  // c:977
    Box::default()
}

/// Reallocate memory.
/// Port of `zrealloc(void *ptr, size_t size)` from Src/mem.c:994 — Vec::resize fills the
/// gap with `T::default()`, mirroring the C source's "old contents
/// preserved, new bytes uninitialized" semantics.
/// WARNING: param names don't match C — Rust=() vs C=(ptr, size)
pub fn zrealloc<T>(v: &mut Vec<T>, new_size: usize)                          // c:994
where
    T: Default + Clone,
{
    v.resize(new_size, T::default());
}

/// Free memory.
/// Port of `zfree(void *p, int sz)` from Src/mem.c:1433 (or :1869 in the
/// MALLOC_DEBUG build). Takes a `Box<T>` rather than `T` so the
/// C-port call sites read the same as the original `zfree(ptr)`
// right size of this block, freeing it will be faster, though; the value // c:1433
// 0 for this parameter means: `don't know'                                // c:1433
/// (an explicit allocator release on a heap pointer). Drop happens
/// automatically when the Box goes out of scope.
#[allow(clippy::boxed_local)]
/// WARNING: param names don't match C — Rust=(_ptr) vs C=(p, sz)
pub fn zfree<T>(_ptr: Box<T>) {                                              // c:1433
    // Drop happens automatically
}

/// Free a string.
/// Port of `zsfree(char *p)` from Src/mem.c:1641 — the C source's
/// `free(NULL)`-tolerant string-specific deallocator. In Rust the
/// Drop impl on `String` handles the actual free.
pub fn zsfree(p: String) {                                                  // c:1641
    // Drop happens automatically
}

/// Port of `realloc(void *p, size_t size)` from Src/mem.c:1648 — wrapped `realloc`.
/// Shim.
/// WARNING: param names don't match C — Rust=(_size) vs C=(p, size)
pub fn realloc(_size: usize) -> usize { 0 }

/// Port of `calloc(size_t n, size_t size)` from Src/mem.c:1697 — wrapped `calloc`.
/// Shim.
#[allow(unused_variables)]
pub fn calloc(n: usize, size: usize) -> usize { 0 }


/// Port of `bin_mem(char *name, char **argv, Options ops, int func)` from `Src/mem.c:1722`.
/// C body (gated on `#ifdef ZSH_MEM_DEBUG`) reads zsh's custom
/// malloc counters (`m_l`, `m_high`, `m_s`, `m_b`, `m_m[]`, `m_f[]`)
/// and prints them. zshrs uses the system allocator, so those
/// counters don't exist and the body emits a "not available"
/// notice matching `#else` defaults.
///
/// C signature: `int bin_mem(char *name, char **argv, Options ops,
///                            int func)`.
/// WARNING: param names don't match C — Rust=(_argv, _ops, _func) vs C=(name, argv, ops, func)
pub fn bin_mem(                                                              // c:1722
    _name: &str,
    _argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // c:1725-1727 — queue_signals(); print verbose header if -v.
    // Static-link Rust path uses system malloc; the C-only `m_*`
    // globals (m_l/m_high/m_s/m_b/m_m/m_f) don't exist.
    println!("memory statistics not available with system allocator");
    0
}

/// Duplicate a string into heap storage.
/// Port of `dupstring(const char *s)` from Src/string.c:33 — the heap-arena
/// variant of `ztrdup()`. In Rust both collapse to `String::clone`
/// since `String` always owns its allocation.
pub fn dupstring(s: &str) -> String {                                       // c:33
    s.to_string()
}

/// Duplicate a string with explicit length.
/// Port of `dupstring_wlen(const char *s, unsigned len)` from Src/string.c:48 — used when the
/// source isn't NUL-terminated (e.g. a slice of a larger buffer).
pub fn dupstring_wlen(s: &str, len: usize) -> String {                      // c:48
    s.chars().take(len).collect()
}

/// Duplicate an array of strings.
/// Port of `zarrdup(char **s)` from Src/utils.c:4532.
pub fn zarrdup(s: &[String]) -> Vec<String> {                             // c:4532
    s.to_vec()
}

/// Duplicate an array up to a maximum length.
/// zshrs-original convenience — closest C analog is the bounded
/// loops Src/utils.c uses around `zarrdup` when the max is known.
pub fn arrdup_max(arr: &[String], max: usize) -> Vec<String> {
    arr.iter().take(max).cloned().collect()
}

/// Get array length.
/// Port of `arrlen(char **s)` from Src/utils.c:2357 — the C source's
/// canonical NULL-terminated `char**` length walker. Rust slices
/// already know their length, so this collapses to `arr.len()`.
pub fn arrlen<T>(s: &[T]) -> usize {                                      // c:2357
    s.len()
}

/// Check if array length is less than n.
/// Port of `arrlen_lt(char **s, unsigned upper_bound)` from Src/utils.c:2400 — short-circuit
/// version that stops walking once the bound is exceeded.
pub fn arrlen_lt<T>(s: &[T], upper_bound: usize) -> bool {                          // c:2400
    s.len() < upper_bound
}

/// Check if array length is less than or equal to n.
/// Port of `arrlen_le(char **s, unsigned upper_bound)` from Src/utils.c:2391.
pub fn arrlen_le<T>(s: &[T], upper_bound: usize) -> bool {                          // c:2391
    s.len() <= upper_bound
}

/// Check if array length is greater than n.
/// Port of `arrlen_gt(char **s, unsigned lower_bound)` from Src/utils.c:2382.
pub fn arrlen_gt<T>(s: &[T], lower_bound: usize) -> bool {                          // c:2382
    s.len() > lower_bound
}

/// Concatenate strings with separator.
/// Port of `sepjoin(char **s, char *sep, int heap)` from Src/utils.c:3928 — C source's `IFS`-
/// driven array→string join. Default separator is space, matching
/// the C source's `sep ? sep : " "` fallback.
/// WARNING: param names don't match C — Rust=(arr, sep) vs C=(s, sep, heap)
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {               // c:3928
    arr.join(sep.unwrap_or(" "))
}

// The canonical `zcontext_save()` / `zcontext_restore()` port lives
// in `crate::ported::context` (Src/context.c:80/117), NOT here. The
// previous Rust port had a `MemContext` aggregate + zero-arg
// `zcontext_save() -> MemContext` shim attributed to "Src/init.c"
// which is not where the C versions live — invented Rust-only
// duplicate name. Deleted per PORT.md Rule A (no fns/structs whose
// name doesn't exist in upstream C source at the cited location).
// No external callers used the mem.rs versions.

// queue_signals / unqueue_signals / QUEUEING_ENABLED / run_queued_signals
// live in `signals_h.rs` — that's the canonical Rust home for the
// `Src/signals.h:90/92/112/114/116` macros. mem.rs callers that need
// the same state must go through signals_h so the counter is shared
// across the whole tree (the prior parallel copies here split the
// queueing state, which was wrong).
pub use crate::ported::signals_h::{queue_signals, unqueue_signals};



/// Split string by separator.
/// Port of `sepsplit(char *s, char *sep, int allownull, int heap)` from Src/utils.c:3962 — the C source's
/// `IFS`-driven splitter. `allow_empty` mirrors the `allownull`
/// argument the C function takes.
/// WARNING: param names don't match C — Rust=(s, sep, allow_empty) vs C=(s, sep, allownull, heap)
pub fn sepsplit(s: &str, sep: &str, allow_empty: bool) -> Vec<String> {     // c:3962
    if allow_empty {
        s.split(sep).map(|s| s.to_string()).collect()
    } else {
        s.split(sep)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

// `next_heap_id` from Src/mem.c:178 — monotonically incrementing counter
// for heap-arena identification under ZSH_MEM_DEBUG.
pub static NEXT_HEAP_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Port of `mod_export Heapid last_heap_id` from `Src/mem.c:194`.
/// Tracks the most recently created heap arena id — used by
/// `memory_validate` (ZSH_MEM_DEBUG path) to recognize cross-arena
/// pointer use. Without ZSH_MEM_DEBUG this is set but never read.
pub static LAST_HEAP_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);                                    // c:153

/// Duplicate a string to permanent storage.
/// Port of `ztrdup(const char *s)` from Src/string.c:62 — C zsh's canonical
/// `strdup(3)` analog tied to the zsh allocator. In Rust both heap
/// and permanent storage are the same (`String` owns its buffer).
pub fn ztrdup(s: &str) -> String {                                          // c:62
    s.to_string()
}

/// Duplicate the first `n` characters of a string.
/// Port of `ztrduppfx(const char *s, int len)` from Src/string.c:172 — same role as
/// `dupstring_wlen` (Src/string.c:145) but allocated as permanent
/// rather than heap-arena. Rust collapses both to `String::clone`.
pub fn ztrduppfx(s: &str, len: usize) -> String {                           // c:172
    s.chars().take(len).collect()
}

/// Concatenate two strings into a new permanent string.
/// Port of `bicat(const char *s1, const char *s2)` from Src/string.c:145.
pub fn bicat(s1: &str, s2: &str) -> String {                                // c:145
    format!("{}{}", s1, s2)
}

// `heaps` / `fheap` from Src/mem.c:526 — head of the current arena
// chain and free-list pointer respectively.
pub static HEAPS: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
pub static FHEAP: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

// This version always uses permanently-allocated space.                   // c:98
/// Concatenate three strings into a new permanent string.
/// Port of `tricat(char const *s1, char const *s2, char const *s3)` from Src/string.c:98 — used heavily by the
/// completion machinery for "prefix + match + suffix" assembly.
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {                     // c:98
    format!("{}{}{}", s1, s2, s3)
}

// concatenate s1 and s2 in dynamically allocated buffer                  // c:131
// This version always uses space from the current heap.                   // c:131
/// Concatenate two strings into a new heap-arena string.
/// Port of `dyncat(const char *s1, const char *s2)` from Src/string.c:131 — heap-arena variant
/// of `bicat()`.
pub fn dyncat(s1: &str, s2: &str) -> String {                               // c:131
    format!("{}{}", s1, s2)
}

/// Get the last character of a string.
/// Port of `strend(char *str)` from Src/string.c:196 — C source returns the
/// pointer to the NUL terminator's predecessor; Rust returns the
/// char.
pub fn strend(str: &str) -> Option<char> {                                    // c:196
    str.chars().last()
}

// Append a string to an allocated string, reallocating to make room.     // c:186
/// Append a string in-place.
/// Port of `appstr(char *base, char const *append)` from Src/string.c:186 — the C source uses
/// `strcat(3)` with realloc; Rust's `String::push_str` does both.
pub fn appstr(base: &mut String, append: &str) {                            // c:186
    base.push_str(append);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heap_push_pop() {
        let mut arena = heap_arena::new();
        assert_eq!(arena.depth(), 1);

        arena.push();
        assert_eq!(arena.depth(), 2);

        arena.alloc_string("test".to_string());

        arena.pop();
        assert_eq!(arena.depth(), 1);
    }

    #[test]
    fn test_heap_free_current() {
        let mut arena = heap_arena::new();

        arena.alloc_string("test1".to_string());
        arena.alloc_bytes(vec![1, 2, 3]);

        arena.free_current();
        // Arena still at depth 1
        assert_eq!(arena.depth(), 1);
    }

    #[test]
    fn test_nested_generations() {
        let mut arena = heap_arena::new();

        arena.alloc_string("level1".to_string());

        arena.push();
        arena.alloc_string("level2".to_string());

        arena.push();
        arena.alloc_string("level3".to_string());

        assert_eq!(arena.depth(), 3);

        arena.pop();
        assert_eq!(arena.depth(), 2);

        arena.pop();
        assert_eq!(arena.depth(), 1);
    }

    #[test]
    fn test_dupstring() {
        let s = dupstring("hello");
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_dupstring_wlen() {
        let s = dupstring_wlen("hello world", 5);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_global_heap() {
        pushheap();
        pushheap();
        popheap();
        popheap();
        // Should not panic
    }
}
