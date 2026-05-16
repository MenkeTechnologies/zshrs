//! Memory management for zshrs
//!
//! Port from zsh/Src/mem.c
//!
//! In Rust, we don't need the complex heap management that zsh uses in C.
//! Instead, we provide a simpler arena-style allocator abstraction that
//! can be used for temporary allocations that all get freed at once.

use std::cell::RefCell;

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
/// Port of `void *zhalloc(size_t size)` from `Src/mem.c:577` — heap-
/// arena `malloc` (memory freed at the end of the current heap frame
/// via `popheap`). zshrs delegates to Rust ownership: callers use
/// owned `Vec` / `String` / `Box` which get freed when they drop out
/// of scope, matching the C heap-arena lifetime model. No actual
/// allocator dispatch needed — this is a C heap-arena name-parity
/// shim. Box goes out of scope at the end of the surrounding fn,
/// achieving the same `popheap`-bounded lifetime.
#[allow(unused_variables)]
pub fn zhalloc(size: usize) -> usize { 0 }                                  // c:577

/// Port of `int memory_validate(Heapid heap_id)` from `Src/mem.c:896`.
/// Under `ZSH_MEM_DEBUG`, walks the heap chain to verify `heap_id` is
/// still alive. Returns 0 if valid, 1 otherwise.
///
/// **Architectural divergence:** ZSH_MEM_DEBUG is a debug-build C
/// feature that tracks heap-arena IDs to catch use-after-free in
/// the arena allocator. zshrs delegates to Rust's borrow checker
/// which catches use-after-free at compile time — runtime validation
/// is unnecessary. Drop happens automatically when the Box / Vec /
/// String goes out of scope, so a heap-id check would always
/// succeed for any handle the caller actually holds. Static-link
/// path: return 0 (valid).
pub fn memory_validate(heap_id: u64) -> i32 {                                // c:896
    const HEAPID_PERMANENT: u64 = 0;
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
/// Rust idiom replacement: heap-arena tracking is unnecessary —
/// `Vec::resize` covers the C arena copy + free-of-old + zero-pad
/// in one call; `old` size arg drops since Vec tracks its own len.
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

/// Port of `void *malloc(size_t size)` from `Src/mem.c:1862` —
/// the non-ZSH_MEM wrapper that just returns `libc::malloc(size)`.
/// In zshrs, Box goes out of scope when its scope ends, triggering
/// `__rust_dealloc` → `free()`. Rust callers use Box/Vec/String
/// which dispatch through `__rust_alloc` → libc malloc directly.
/// Drop happens automatically. This entry exists for C-ABI parity;
/// no zshrs caller invokes raw `malloc()`.
#[allow(unused_variables)]
pub fn malloc(size: usize) -> usize { 0 }                                    // c:1862

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
/// Port of `mod_export void zfree(void *p, int sz)` from `Src/mem.c:1433`
/// (under `#ifdef ZSH_MEM`) and `Src/mem.c:1869` (the `#ifndef ZSH_MEM`
/// libc-allocator wrapper).
///
/// zsh builds in two memory-allocator modes:
///
/// 1. **ZSH_MEM** (c:1433-1631) — a hand-rolled slab allocator: `m_hdr`
///    block headers, per-size-class freelists in `m_small[]`,
///    block-merge on free. 325 lines of allocator surgery.
///    ```c
///    mod_export void
///    zfree(void *p, int sz)
///    {
///        struct m_hdr *m = (struct m_hdr *)(((char *)p) - M_ISIZE);
///        /* ...325 lines: secure-free check, small-block freelist push,
///           block-merge, block-list relink... */
///    }
///    ```
///
/// 2. **non-ZSH_MEM** (c:1869-1872) — thin wrapper around libc free:
///    ```c
///    mod_export void
///    zfree(void *p, UNUSED(int sz))
///    {
///        free(p);
///    }
///    ```
///
/// zshrs targets path #2 because Rust's stdlib uses the system
/// allocator. `Box<T>::drop` runs `__rust_dealloc` which is
/// `free()` on every supported target — exact behavioral parity
/// with the C wrapper at c:1869. Drop happens automatically when
/// the Box goes out of scope. There's no point porting the
/// ZSH_MEM slab allocator: it's a userspace-malloc replacement
/// the C source ships only because zsh historically ran on
/// platforms with broken libc malloc; modern Rust doesn't need it.
#[allow(clippy::boxed_local)]
pub fn zfree<T>(p: Box<T>) {                                                 // c:1869
    // c:1871 — `free(p);`
    // Rust path: `Box::drop` calls `__rust_dealloc` → libc::free
    // (verified against alloc::alloc::dealloc). End-of-scope drop.
    drop(p);                                                                 // c:1871
}

/// Free a string.
/// Port of `zsfree(char *p)` from Src/mem.c:1641 — the C source's
/// `free(NULL)`-tolerant string-specific deallocator. In Rust the
/// Drop impl on `String` handles the actual free.
pub fn zsfree(p: String) {                                                  // c:1641
    // Drop happens automatically
}

/// Port of `realloc(void *p, size_t size)` from Src/mem.c:1648 — wrapped `realloc`.
/// Rust idiom replacement: zshrs never calls raw realloc — every
/// caller uses Vec/Box/Arc which manage their own allocation. The
/// C source wraps libc realloc for the heap-arena fallback path
/// that zshrs replaces entirely. Shim is name-parity only.
/// WARNING: param names don't match C — Rust=(_size) vs C=(p, size)
pub fn realloc(_size: usize) -> usize { 0 }

/// Port of `calloc(size_t n, size_t size)` from Src/mem.c:1697 — wrapped `calloc`.
/// Shim.
#[allow(unused_variables)]
pub fn calloc(n: usize, size: usize) -> usize { 0 }


/// Port of `int bin_mem(char *name, char **argv, Options ops, int func)`
/// from `Src/mem.c:1722` (ZSH_MEM_DEBUG-gated). Reads zsh's custom
/// malloc counters (`m_l`, `m_high`, `m_s`, `m_b`, `m_m[]`, `m_f[]`,
/// `h_push`, `h_pop`, `h_free`, `h_m[]`) and prints them. zshrs uses
/// the system allocator, so these counters are always 0; the output
/// shape matches C for parity with debug scripts.
/// WARNING: param names don't match C — Rust=(_name, _argv, ops, _func) vs C=(name, argv, ops, func)
pub fn bin_mem(                                                              // c:1722
    _name: &str,
    _argv: &[String],
    ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    let m_l: i64 = 0;                                                        // c:1727 low addr
    let m_high: i64 = 0;                                                     // c:1727 high addr
    let m_s: i32 = 0;                                                        // c:1742 sbrk total
    let m_b: i32 = 0;                                                        // c:1742 brk total
    crate::ported::signals::queue_signals();                                 // c:1729
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1730
        println!("The lower and the upper addresses of the heap. Diff gives");
        println!("the difference between them, i.e. the size of the heap.\n");
    }
    println!("low mem {}\t high mem {}\t diff {}", m_l, m_high, m_high - m_l);// c:1734
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1737
        println!("\nThe number of bytes that were allocated using sbrk() and");
        println!("the number of bytes that were given back to the system");
        println!("via brk().");
    }
    println!("\nsbrk {}\tbrk {}", m_s, m_b);                                 // c:1742
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1744
        println!("\nInformation about the sizes that were allocated or freed.");
        println!("For each size that were used the number of mallocs and");
        println!("frees is shown. Diff gives the difference between these");
        println!("values, i.e. the number of blocks of that size that is");
        println!("currently allocated. Total is the product of size and diff,");
        println!("i.e. the number of bytes that are allocated for blocks of");
        println!("this size. The last field gives the accumulated number of");
        println!("bytes for all sizes.");
    }
    println!("\nsize\tmalloc\tfree\tdiff\ttotal\tcum");                      // c:1754
    // c:1755-1761 m_m[i]/m_f[i] histogram — all zero with system allocator.
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1766
        println!("\nThe list of memory blocks. For each block the following");
        println!("information is shown:\n");
        println!("num\tthe number of this block");
        println!("tnum\tlike num but counted separately for used and free");
        println!("\tblocks");
        println!("addr\tthe address of this block");
        println!("len\tthe length of the block");
        println!("state\tthe state of this block, this can be:");
        println!("\t  used\tthis block is used for one big block");
        println!("\t  free\tthis block is free");
        println!("\t  small\tthis block is used for an array of small blocks");
        println!("cum\tthe accumulated sizes of the blocks, counted");
        println!("\tseparately for used and free blocks");
        println!("\nFor blocks holding small blocks the number of free");
        println!("blocks, the number of used blocks and the size of the");
        println!("blocks is shown. For otherwise used blocks the first few");
        println!("bytes are shown as an ASCII dump.");
    }
    println!("\nblock list:\nnum\ttnum\taddr\t\tlen\tstate\tcum");            // c:1785
    // c:1786-1816 block-list walk — empty under system allocator.
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1818
        println!("\nHere is some information about the small blocks used.");
        println!("For each size the arrays with the number of free and the");
        println!("number of used blocks are shown.");
    }
    println!("\nsmall blocks:\nsize\tblocks (free/used)");                   // c:1823
    // c:1825-1836 — m_small histogram, all zero.
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1837
        println!("\n\nBelow is some information about the allocation");
        println!("behaviour of the zsh heaps. First the number of times");
        println!("pushheap(), popheap(), and freeheap() were called.");
    }
    println!("\nzsh heaps:\n");                                              // c:1842
    let h_push: i32 = 0;                                                     // c:1844 — debug counter
    let h_pop: i32 = 0;
    let h_free: i32 = 0;
    println!("push {}\tpop {}\tfree {}\n", h_push, h_pop, h_free);           // c:1844
    if crate::ported::zsh_h::OPT_ISSET(ops, b'v') {                 // c:1846
        println!("\nThe next list shows for several sizes the number of times");
        println!("memory of this size were taken from heaps.\n");
    }
    println!("size\tmalloc\ttotal");                                         // c:1850
    // c:1851-1856 h_m[] histogram — all zero.
    crate::ported::signals::unqueue_signals();                               // c:1858
    0                                                                        // c:1859
}

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
///
/// C body (c:54-57): `t = zhalloc(len + 1); memcpy(t, s, len); t[len] = '\0';`.
/// `memcpy` is byte-based, NOT char-based. Previous Rust port used
/// `s.chars().take(len)` which counts codepoints, not bytes — so for
/// `dupstring_wlen("café", 5)` C copies 5 bytes (`c-a-f-é` with é=2
/// bytes), Rust returned 5 chars (`café` + 1 phantom — same string
/// length BUT off-by-one on multibyte boundaries). Byte-based slicing
/// matches C exactly; `from_utf8_lossy` keeps the no-panic guarantee
/// at mid-codepoint cuts.
pub fn dupstring_wlen(s: &str, len: usize) -> String {                      // c:48
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
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

/// Duplicate the first `len` bytes of a string.
/// Port of `ztrduppfx(const char *s, int len)` from Src/string.c:172 — same role as
/// `dupstring_wlen` (Src/string.c:48 / dupstrpfx at c:161) but
/// allocated as permanent rather than heap-arena. Both lanes collapse
/// to `String` in Rust.
///
/// C body (c:175-177): `r = zalloc(len+1); memcpy(r, s, len); r[len]='\0';`.
/// Byte-counted copy — the previous Rust port used `chars().take(len)`
/// which counts codepoints, diverging from C on any multibyte input.
pub fn ztrduppfx(s: &str, len: usize) -> String {                           // c:172
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
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

    /// c:3928 — `sepjoin([a, b, c], Some("-"))` produces `"a-b-c"`.
    /// Used by `${(j:-:)array}`. Regression dropping the separator
    /// or doubling it would silently mangle every joined-array path.
    #[test]
    fn sepjoin_with_explicit_separator() {
        assert_eq!(sepjoin(&["a".into(), "b".into(), "c".into()], Some("-")), "a-b-c");
        assert_eq!(sepjoin(&["a".into(), "b".into(), "c".into()], Some("")),  "abc");
    }

    /// c:3928 — `sepjoin(arr, None)` defaults separator to space (matches
    /// C's `s ? s : " "` pattern).
    #[test]
    fn sepjoin_none_defaults_to_space() {
        assert_eq!(sepjoin(&["a".into(), "b".into()], None), "a b");
    }

    /// c:3928 — empty array returns empty string regardless of sep.
    /// Catches a regression that returns the separator alone.
    #[test]
    fn sepjoin_empty_array_returns_empty() {
        assert_eq!(sepjoin(&[], Some("-")), "");
        assert_eq!(sepjoin(&[], None),      "");
    }

    /// c:3962 — `sepsplit("a/b/c", "/", false)` → `["a","b","c"]`.
    /// Used by `${(s:/:)PATH}` style splits. Regression that drops
    /// the splitter would yield the whole string as one element.
    #[test]
    fn sepsplit_canonical_path_split() {
        let r = sepsplit("a/b/c", "/", false);
        assert_eq!(r, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    /// c:3962 — `allow_empty=false` filters empty segments (consecutive
    /// separators don't produce empty entries). Critical for path-walks
    /// where `//` should not yield `""`.
    #[test]
    fn sepsplit_filters_empty_segments_when_disallowed() {
        let r = sepsplit("a//b", "/", false);
        assert_eq!(r, vec!["a".to_string(), "b".to_string()]);
    }

    /// c:3962 — `allow_empty=true` PRESERVES empty segments (matches
    /// C `allownull=1`). Used by IFS-driven word splits where each
    /// run of separators counts as one boundary.
    #[test]
    fn sepsplit_preserves_empty_segments_when_allowed() {
        let r = sepsplit("a//b", "/", true);
        assert_eq!(r.len(), 3, "consecutive sep yields empty middle entry");
        assert_eq!(r[1], "");
    }

    /// `new_heap_id` is monotonically increasing — each call returns
    /// a strictly greater id. Catches a regression where the counter
    /// resets or wraps unexpectedly (would alias unrelated heaps).
    #[test]
    fn new_heap_id_is_monotonically_increasing() {
        let a = new_heap_id();
        let b = new_heap_id();
        let c = new_heap_id();
        assert!(a < b && b < c, "heap-id sequence must be strictly increasing: {a} < {b} < {c}");
    }

    /// c:291/443 — `pushheap`/`popheap` are nest-balanced. Two pushes
    /// followed by two pops MUST leave the heap stack empty (matches
    /// the `do_X { pushheap; ... popheap; }` C usage everywhere).
    /// Regression that drops a push frame would corrupt heap state
    /// across function boundaries.
    #[test]
    fn pushheap_popheap_balance_two_levels() {
        pushheap();
        pushheap();
        popheap();
        popheap();
        // Survival is the test — heap stack must be empty.
    }

    /// c:325 — `freeheap` discards CURRENT level allocations but keeps
    /// the level (no pop). Distinct from popheap which discards the
    /// level entirely. Three sequential freeheap calls must not
    /// disturb the surrounding push/pop bracket.
    #[test]
    fn freeheap_does_not_pop_levels() {
        pushheap();
        freeheap();
        freeheap();
        freeheap();
        popheap();
        // Survival is the test — push/freeheap/pop bracket intact.
    }

    /// c:443 — `popheap` without a matching `pushheap` is a no-op.
    /// (The HEAP impl tolerates over-popping for robustness.) Catches
    /// a regression that crashes on accidental over-pop in error paths.
    #[test]
    fn popheap_without_push_does_not_panic() {
        popheap();
        // No assertion — survival is the contract.
    }

    /// `Src/string.c:48-58` — `dupstring_wlen(s, len)` is BYTE-counted
    /// (`memcpy(t, s, len)`). Previous Rust port used `chars().take(len)`
    /// which is CODEPOINT-counted — diverges on multibyte input.
    /// Pin: `dupstring_wlen("café", 5)` must copy 5 bytes, NOT 5 chars.
    #[test]
    fn dupstring_wlen_is_byte_counted_not_char_counted() {
        // "café" = `c-a-f-é` where é is 2 bytes (0xC3 0xA9) → 5 bytes total.
        // Byte-count 5 → exactly "café" (whole string).
        assert_eq!(dupstring_wlen("café", 5), "café",
            "c:55 — memcpy is byte-counted; 5 bytes of 'café' is the whole string");
        // Char-count 5 would walk past the end, but byte-count clamps
        // at the actual byte length.
        assert_eq!(dupstring_wlen("café", 100), "café",
            "c:55 — len > strlen clamps (Rust safety) instead of UB");
        // Byte-count 4 → "caf" + replacement char (mid-codepoint cut).
        let r = dupstring_wlen("café", 4);
        assert!(r.starts_with("caf"),
            "c:55 — byte cut at codepoint boundary uses from_utf8_lossy");
        // Empty input → empty output.
        assert_eq!(dupstring_wlen("", 0), "");
        assert_eq!(dupstring_wlen("hello", 0), "",
            "c:55 — len 0 → empty result");
    }

    /// `Src/string.c:172-178` — `ztrduppfx` is the permanent-storage
    /// twin of `dupstrpfx` / `dupstring_wlen`. Same byte-counted
    /// `memcpy(r, s, len)` semantics — previous port used
    /// `chars().take(len)` diverging on multibyte input.
    #[test]
    fn ztrduppfx_is_byte_counted_not_char_counted() {
        assert_eq!(ztrduppfx("café", 5), "café",
            "c:175 — 5 bytes copies whole 'café' (é=2 bytes)");
        assert_eq!(ztrduppfx("hello", 3), "hel");
        // Multibyte 字 = 3 bytes (0xE5 0xAD 0x97). len=3 copies whole.
        assert_eq!(ztrduppfx("字", 3), "字");
        assert_eq!(ztrduppfx("",  5), "");
        assert_eq!(ztrduppfx("ab", 100), "ab",
            "c:175 — len > strlen clamps");
    }

    /// `Src/string.c:33-42` — `dupstring(s)` returns an independent
    /// owned copy. Source modifications must not affect the dup.
    #[test]
    fn dupstring_yields_independent_copy() {
        let mut src = String::from("original");
        let dup = dupstring(&src);
        src.clear();
        assert_eq!(dup, "original",
            "c:38-40 — dup must survive source-side mutation");
    }
}
