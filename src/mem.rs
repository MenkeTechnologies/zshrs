//! Memory management for zshrs
//!
//! Port from zsh/Src/mem.c
//!
//! In Rust, we don't need the complex heap management that zsh uses in C.
//! Instead, we provide a simpler arena-style allocator abstraction that
//! can be used for temporary allocations that all get freed at once.

use std::cell::RefCell;

/// A memory arena for temporary allocations
///
/// This provides push/pop semantics similar to zsh's heap management,
/// but uses Rust's standard memory management under the hood.
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

    /// Push a new heap state (like zsh's pushheap)
    pub fn push(&mut self) {
        self.generations.push(Generation {
            strings: Vec::new(),
            buffers: Vec::new(),
        });
    }

    /// Pop and free all allocations since the last push (like zsh's popheap)
    pub fn pop(&mut self) {
        if self.generations.len() > 1 {
            self.generations.pop();
        }
    }

    /// Free allocations in current generation but keep generation marker (like zsh's freeheap)
    pub fn free_current(&mut self) {
        if let Some(gen) = self.generations.last_mut() {
            gen.strings.clear();
            gen.buffers.clear();
        }
    }

    /// Allocate a string in the current generation
    pub fn alloc_string(&mut self, s: String) -> &str {
        if let Some(gen) = self.generations.last_mut() {
            gen.strings.push(s);
            gen.strings.last().map(|s| s.as_str()).unwrap()
        } else {
            panic!("No generation available")
        }
    }

    /// Allocate bytes in the current generation
    pub fn alloc_bytes(&mut self, bytes: Vec<u8>) -> &[u8] {
        if let Some(gen) = self.generations.last_mut() {
            gen.buffers.push(bytes);
            gen.buffers.last().map(|b| b.as_slice()).unwrap()
        } else {
            panic!("No generation available")
        }
    }

    /// Get current stack depth
    pub fn depth(&self) -> usize {
        self.generations.len()
    }
}

thread_local! {
    static HEAP: RefCell<HeapArena> = RefCell::new(HeapArena::new());
}

/// Push heap state
pub fn pushheap() {
    HEAP.with(|h| h.borrow_mut().push());
}

/// Pop heap state and free allocations
pub fn popheap() {
    HEAP.with(|h| h.borrow_mut().pop());
}

/// Free current heap allocations but keep state
pub fn freeheap() {
    HEAP.with(|h| h.borrow_mut().free_current());
}

/// Allocate memory (in Rust, this just uses the normal allocator)
pub fn zalloc<T: Default>() -> Box<T> {
    Box::default()
}

/// Allocate zeroed memory
pub fn zshcalloc<T: Default>() -> Box<T> {
    Box::default()
}

/// Reallocate memory (Rust handles this automatically with Vec)
pub fn zrealloc<T>(v: &mut Vec<T>, new_size: usize)
where
    T: Default + Clone,
{
    v.resize(new_size, T::default());
}

/// Free memory (no-op in Rust, drop handles it)
pub fn zfree<T>(_ptr: Box<T>) {
    // Drop happens automatically
}

/// Free a string (no-op in Rust)
pub fn zsfree(_s: String) {
    // Drop happens automatically
}

/// Duplicate a string
pub fn dupstring(s: &str) -> String {
    s.to_string()
}

/// Duplicate a string with length
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Create a heap-allocated string (in Rust, just creates a String)
pub fn zhalloc_string(s: &str) -> String {
    s.to_string()
}

/// Check if a pointer is within a memory pool
/// In Rust, we don't need this - just returns true for any valid reference
pub fn zheapptr<T>(_ptr: &T) -> bool {
    true
}

/// Reallocate heap memory (from mem.c hrealloc)
pub fn hrealloc(old: Vec<u8>, new_size: usize) -> Vec<u8> {
    let mut v = old;
    v.resize(new_size, 0);
    v
}

/// Duplicate array of strings (from mem.c zarrdup)
pub fn zarrdup(arr: &[String]) -> Vec<String> {
    arr.to_vec()
}

/// Duplicate array with maximum length (from mem.c arrdup_max)
pub fn arrdup_max(arr: &[String], max: usize) -> Vec<String> {
    arr.iter().take(max).cloned().collect()
}

/// Get array length (from mem.c arrlen)
pub fn arrlen<T>(arr: &[T]) -> usize {
    arr.len()
}

/// Check if array length is less than n (from mem.c arrlen_lt)
pub fn arrlen_lt<T>(arr: &[T], n: usize) -> bool {
    arr.len() < n
}

/// Check if array length is less than or equal to n (from mem.c arrlen_le)
pub fn arrlen_le<T>(arr: &[T], n: usize) -> bool {
    arr.len() <= n
}

/// Check if array length equals n (from mem.c arrlen_eq)
pub fn arrlen_eq<T>(arr: &[T], n: usize) -> bool {
    arr.len() == n
}

/// Check if array length is greater than n (from mem.c arrlen_gt)
pub fn arrlen_gt<T>(arr: &[T], n: usize) -> bool {
    arr.len() > n
}

/// Concatenate strings with separator (from mem.c sepjoin)
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {
    arr.join(sep.unwrap_or(" "))
}

/// Split string by separator (from mem.c sepsplit)
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

/// Allocate zeroed buffer (from mem.c zshcalloc)
pub fn zshcalloc_buf(size: usize) -> Vec<u8> {
    vec![0u8; size]
}

/// Allocate buffer with size (from mem.c zalloc)
pub fn zalloc_buf(size: usize) -> Vec<u8> {
    Vec::with_capacity(size)
}

/// Duplicate string to permanent storage (from mem.c ztrdup)
pub fn ztrdup(s: &str) -> String {
    s.to_string()
}

/// Duplicate n characters (from mem.c ztrncpy / ztrduppfx)
pub fn ztrduppfx(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Concatenate two strings (from mem.c bicat)
pub fn bicat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Concatenate three strings (from mem.c tricat)
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    format!("{}{}{}", s1, s2, s3)
}

/// Dynamic concatenate on heap (from mem.c dyncat)
pub fn dyncat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Get last character of string (from mem.c strend)
pub fn strend(s: &str) -> Option<char> {
    s.chars().last()
}

/// Append string (from mem.c appstr)
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Memory statistics structure
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

/// Get memory statistics (stub - Rust manages memory automatically)
pub fn get_mem_stats() -> MemStats {
    MemStats::new()
}

/// Context save/restore for memory (from mem.c zcontext_save/restore)
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

/// Save memory context
pub fn zcontext_save() -> MemContext {
    MemContext::save()
}

/// Restore memory context
pub fn zcontext_restore(ctx: MemContext) {
    ctx.restore();
}

/// Queue signals during memory operations (stub in Rust - not needed)
pub fn queue_signals() {}

/// Unqueue signals after memory operations (stub in Rust - not needed)
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
