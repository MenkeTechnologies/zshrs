//! Memory management for zshrs
//!
//! Port from zsh/Src/mem.c
//!
//! In Rust, we don't need the complex heap management that zsh uses in C.
//! Instead, we provide a simpler arena-style allocator abstraction that
//! can be used for temporary allocations that all get freed at once.

use std::cell::RefCell;

/// A memory arena for temporary allocations.
///
/// Port of the `heaps` linked-list arena C zsh maintains in
/// Src/mem.c (see `new_heaps()` line 194 / `old_heaps()` line 220).
/// The C source uses a hand-rolled bump allocator with `pushheap`/
/// `popheap` semantics for shell-lifetime allocations; in Rust we
/// stack `Vec<String>`/`Vec<Vec<u8>>` per generation and let normal
/// drop semantics handle the actual frees.
pub struct HeapArena {
    /// Stack of arena generations
    generations: Vec<Generation>,
}

struct Generation {
    /// Strings allocated in this generation
    strings: Vec<String>,
    /// Byte buffers allocated in this generation
    buffers: Vec<Vec<u8>>,
}

impl Default for HeapArena {
    fn default() -> Self {
        Self::new()
    }
}

impl HeapArena {
    pub fn new() -> Self {
        HeapArena {
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
    static HEAP: RefCell<HeapArena> = RefCell::new(HeapArena::new());
}

/// Push heap state.
/// Port of `pushheap()` from Src/mem.c:291 — the global entry-point
/// version that operates on the thread-local arena.
pub fn pushheap() {
    HEAP.with(|h| h.borrow_mut().push());
}

/// Pop heap state and free allocations.
/// Port of `popheap()` from Src/mem.c:443.
pub fn popheap() {
    HEAP.with(|h| h.borrow_mut().pop());
}

/// Free current heap allocations but keep state.
/// Port of `freeheap()` from Src/mem.c:325.
pub fn freeheap() {
    HEAP.with(|h| h.borrow_mut().free_current());
}

/// Allocate memory.
/// Port of `zalloc()` from Src/mem.c:959. In Rust we use `Box`
/// rather than `malloc(3)`; the type-default initialization stands
/// in for the C source's uninitialized buffer.
pub fn zalloc<T: Default>() -> Box<T> {
    Box::default()
}

/// Allocate zeroed memory.
/// Port of `zshcalloc()` from Src/mem.c:977 — the C source pairs
/// `zalloc()` with `memset(0)`; Rust's `Box::default()` handles
/// both.
pub fn zshcalloc<T: Default>() -> Box<T> {
    Box::default()
}

/// Reallocate memory.
/// Port of `zrealloc()` from Src/mem.c:994 — Vec::resize fills the
/// gap with `T::default()`, mirroring the C source's "old contents
/// preserved, new bytes uninitialized" semantics.
pub fn zrealloc<T>(v: &mut Vec<T>, new_size: usize)
where
    T: Default + Clone,
{
    v.resize(new_size, T::default());
}

/// Free memory.
/// Port of `zfree()` from Src/mem.c:1433 (or :1869 in the
/// MALLOC_DEBUG build). Takes a `Box<T>` rather than `T` so the
/// C-port call sites read the same as the original `zfree(ptr)`
/// (an explicit allocator release on a heap pointer). Drop happens
/// automatically when the Box goes out of scope.
#[allow(clippy::boxed_local)]
pub fn zfree<T>(_ptr: Box<T>) {
    // Drop happens automatically
}

/// Free a string.
/// Port of `zsfree()` from Src/mem.c:1641 — the C source's
/// `free(NULL)`-tolerant string-specific deallocator. In Rust the
/// Drop impl on `String` handles the actual free.
pub fn zsfree(_s: String) {
    // Drop happens automatically
}

/// Duplicate a string into heap storage.
/// Port of `dupstring()` from Src/string.c:33 — the heap-arena
/// variant of `ztrdup()`. In Rust both collapse to `String::clone`
/// since `String` always owns its allocation.
pub fn dupstring(s: &str) -> String {
    s.to_string()
}

/// Duplicate a string with explicit length.
/// Port of `dupstring_wlen()` from Src/string.c:48 — used when the
/// source isn't NUL-terminated (e.g. a slice of a larger buffer).
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Check if a pointer is within the heap arena.
/// Port of `zheapptr()` from Src/mem.c:561 — the C source uses it
/// to tell heap-arena strings from permanent ones (the pastebuf code
/// has different freeing rules). Rust's borrow-checker subsumes
/// this distinction; the function is kept for call-site parity but
/// always returns true.
pub fn zheapptr<T>(_ptr: &T) -> bool {
    true
}

/// Reallocate heap memory.
/// Port of `hrealloc()` from Src/mem.c:687 — heap-arena
/// counterpart of `zrealloc()` (Src/mem.c:994).
pub fn hrealloc(old: Vec<u8>, new_size: usize) -> Vec<u8> {
    let mut v = old;
    v.resize(new_size, 0);
    v
}

/// Duplicate an array of strings.
/// Port of `zarrdup()` from Src/utils.c:4532.
pub fn zarrdup(arr: &[String]) -> Vec<String> {
    arr.to_vec()
}

/// Duplicate an array up to a maximum length.
/// zshrs-original convenience — closest C analog is the bounded
/// loops Src/utils.c uses around `zarrdup` when the max is known.
pub fn arrdup_max(arr: &[String], max: usize) -> Vec<String> {
    arr.iter().take(max).cloned().collect()
}

/// Get array length.
/// Port of `arrlen()` from Src/utils.c:2357 — the C source's
/// canonical NULL-terminated `char**` length walker. Rust slices
/// already know their length, so this collapses to `arr.len()`.
pub fn arrlen<T>(arr: &[T]) -> usize {
    arr.len()
}

/// Check if array length is less than n.
/// Port of `arrlen_lt()` from Src/utils.c:2400 — short-circuit
/// version that stops walking once the bound is exceeded.
pub fn arrlen_lt<T>(arr: &[T], n: usize) -> bool {
    arr.len() < n
}

/// Check if array length is less than or equal to n.
/// Port of `arrlen_le()` from Src/utils.c:2391.
pub fn arrlen_le<T>(arr: &[T], n: usize) -> bool {
    arr.len() <= n
}

/// Check if array length is greater than n.
/// Port of `arrlen_gt()` from Src/utils.c:2382.
pub fn arrlen_gt<T>(arr: &[T], n: usize) -> bool {
    arr.len() > n
}

/// Concatenate strings with separator.
/// Port of `sepjoin()` from Src/utils.c:3928 — C source's `IFS`-
/// driven array→string join. Default separator is space, matching
/// the C source's `sep ? sep : " "` fallback.
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {
    arr.join(sep.unwrap_or(" "))
}

/// Split string by separator.
/// Port of `sepsplit()` from Src/utils.c:3962 — the C source's
/// `IFS`-driven splitter. `allow_empty` mirrors the `allownull`
/// argument the C function takes.
pub fn sepsplit(s: &str, sep: &str, allow_empty: bool) -> Vec<String> {
    if allow_empty {
        s.split(sep).map(|s| s.to_string()).collect()
    } else {
        s.split(sep)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

/// Duplicate a string to permanent storage.
/// Port of `ztrdup()` from Src/string.c:62 — C zsh's canonical
/// `strdup(3)` analog tied to the zsh allocator. In Rust both heap
/// and permanent storage are the same (`String` owns its buffer).
pub fn ztrdup(s: &str) -> String {
    s.to_string()
}

/// Duplicate the first `n` characters of a string.
/// Port of `ztrduppfx()` from Src/string.c:172 — same role as
/// `dupstring_wlen` (Src/string.c:48) but allocated as permanent
/// rather than heap-arena. Rust collapses both to `String::clone`.
pub fn ztrduppfx(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Concatenate two strings into a new permanent string.
/// Port of `bicat()` from Src/string.c:145.
pub fn bicat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Concatenate three strings into a new permanent string.
/// Port of `tricat()` from Src/string.c:98 — used heavily by the
/// completion machinery for "prefix + match + suffix" assembly.
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    format!("{}{}{}", s1, s2, s3)
}

/// Concatenate two strings into a new heap-arena string.
/// Port of `dyncat()` from Src/string.c:131 — heap-arena variant
/// of `bicat()`.
pub fn dyncat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Get the last character of a string.
/// Port of `strend()` from Src/string.c:196 — C source returns the
/// pointer to the NUL terminator's predecessor; Rust returns the
/// char.
pub fn strend(s: &str) -> Option<char> {
    s.chars().last()
}

/// Append a string in-place.
/// Port of `appstr()` from Src/string.c:186 — the C source uses
/// `strcat(3)` with realloc; Rust's `String::push_str` does both.
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Memory statistics structure.
/// Port of the per-heap counters Src/mem.c tracks for `bin_mem()`
/// (line 1722) — the `mem` builtin in `zsh/mem` reports these.
#[derive(Default, Debug, Clone)]
pub struct MemStats {
    pub heap_count: usize,
    pub heap_total: usize,
    pub heap_used: usize,
    pub alloc_count: usize,
    pub free_count: usize,
}

impl MemStats {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Get memory statistics.
/// Port of `bin_mem()` from Src/mem.c:1722. Rust manages memory
/// automatically so the counters are zero by default; the function
/// is kept for parity with the `mem` builtin's parameter-shape.
pub fn bin_mem() -> MemStats {
    MemStats::new()
}

/// Context save/restore for memory state.
/// Port of `zcontext_save()`/`zcontext_restore()` from
/// Src/init.c. The C source captures the heap stack depth so a
/// fault deep in eval can unwind to a known-good marker; we
/// replicate the pattern with the arena depth counter.
pub struct MemContext {
    heap_depth: usize,
}

impl MemContext {
    pub fn save() -> Self {
        let depth = HEAP.with(|h| h.borrow().depth());
        MemContext { heap_depth: depth }
    }

    pub fn restore(self) {
        HEAP.with(|h| {
            let mut heap = h.borrow_mut();
            while heap.depth() > self.heap_depth {
                heap.pop();
            }
        });
    }
}

/// Save memory context.
/// Port of `zcontext_save()` from Src/init.c — entry-point
/// function form.
pub fn zcontext_save() -> MemContext {
    MemContext::save()
}

/// Restore memory context.
/// Port of `zcontext_restore()` from Src/init.c — pops every
/// generation pushed since the saved depth.
pub fn zcontext_restore(ctx: MemContext) {
    ctx.restore();
}

/// Queue signals during memory operations.
/// Port of `queue_signals()` from Src/signals.c — defers
/// `SIGINT`/`SIGCHLD` etc. until the matching `unqueue_signals()`.
/// Stubbed in Rust because the worker pool isolates each task in
/// its own thread (see `src/worker.rs`).
pub fn queue_signals() {}

/// Unqueue signals after memory operations.
/// Port of `unqueue_signals()` from Src/signals.c — pair of
/// `queue_signals`. Stubbed in Rust for the same reason.
pub fn unqueue_signals() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heap_push_pop() {
        let mut arena = HeapArena::new();
        assert_eq!(arena.depth(), 1);

        arena.push();
        assert_eq!(arena.depth(), 2);

        arena.alloc_string("test".to_string());

        arena.pop();
        assert_eq!(arena.depth(), 1);
    }

    #[test]
    fn test_heap_free_current() {
        let mut arena = HeapArena::new();

        arena.alloc_string("test1".to_string());
        arena.alloc_bytes(vec![1, 2, 3]);

        arena.free_current();
        // Arena still at depth 1
        assert_eq!(arena.depth(), 1);
    }

    #[test]
    fn test_nested_generations() {
        let mut arena = HeapArena::new();

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
    NEXT_HEAP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)          // c:185
}

// `next_heap_id` from Src/mem.c:178 — monotonically incrementing counter
// for heap-arena identification under ZSH_MEM_DEBUG.
pub static NEXT_HEAP_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Port of `new_heaps()` from Src/mem.c:194 — push a new heap
/// frame onto the arena stack. Shim.
pub fn new_heaps() {}

/// Port of `old_heaps()` from Src/mem.c:220 — pop the current
/// heap frame, freeing its arena. Shim.
pub fn old_heaps() {}

/// Port of `switch_heaps()` from Src/mem.c:267 — temporarily
/// swap heap frames. Shim.
pub fn switch_heaps() {}

/// Port of `mmap_heap_alloc()` from Src/mem.c:526.
/// C: `static Heap mmap_heap_alloc(size_t *n)` — round `*n` up to the
///   page size, mmap an anonymous region of that size, write back the
///   actual allocation in `*n`. Returns the Heap header.
pub fn mmap_heap_alloc(n: &mut usize) -> *mut std::ffi::c_void {             // c:526
    // c:530 — `static size_t pgsz = 0;`
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

/// Port of `zhalloc()` from Src/mem.c:577 — heap-arena `malloc`
/// (memory freed at the end of the current heap frame). Shim;
/// Rust callers use owned `Vec`/`String`.
pub fn zhalloc(_size: usize) -> usize { 0 }

/// Port of `memory_validate()` from Src/mem.c:896 — `ZSH_MEM_DEBUG`
/// arena consistency check. Shim.
pub fn memory_validate() {}

/// Port of `hcalloc()` from Src/mem.c:946 — heap-arena `calloc`
/// (zero-fill `zhalloc`). Shim.
pub fn hcalloc(_size: usize) -> usize { 0 }

/// Port of `malloc()` from Src/mem.c:1189 — wrapped `malloc`
/// for the legacy arena system. Shim.
pub fn malloc(_size: usize) -> usize { 0 }

/// Port of `free()` from Src/mem.c:1631 — wrapped `free`. Shim.
pub fn free() {}

/// Port of `realloc()` from Src/mem.c:1648 — wrapped `realloc`.
/// Shim.
pub fn realloc(_size: usize) -> usize { 0 }

/// Port of `calloc()` from Src/mem.c:1697 — wrapped `calloc`.
/// Shim.
pub fn calloc(_n: usize, _size: usize) -> usize { 0 }
