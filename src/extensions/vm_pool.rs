//! Per-thread pool of recyclable fusevm `VM`s — a Rust-only optimization
//! that stops zshrs from rebuilding a VM (and re-registering its entire
//! builtin table) on every function call, command substitution, pipeline
//! stage, and subshell.
//!
//! # Why
//!
//! Handing a compiled `Chunk` to a [`fusevm::VM`] used to mean
//! `VM::new(chunk)` + `register_builtins(&mut vm)` at each of ~15 execution
//! sites. `register_builtins` installs the ~hundreds of `fn`-pointer
//! handlers that make up the shell builtin table — identical for every VM,
//! so redoing it per run was pure waste (the #2 hot spot after option
//! lookups in a function-call profile). Worse, throwing the VM away each
//! call meant the tracing JIT never stayed warm, so hot numeric loops never
//! got compiled.
//!
//! [`fusevm::VM::reset`] clears execution state (stack, frames, globals, ip,
//! status) but PRESERVES the `builtin_table`, shell host, and JIT wiring. So
//! a VM built once can be recycled for any later chunk with only a `reset` —
//! no re-registration — and its JIT trace state accumulates across runs.
//!
//! # Usage
//!
//! [`acquire`] returns a [`PooledVm`] RAII guard that derefs to the VM. It
//! pops a recycled VM (already registered — just `reset`) or builds and
//! registers a fresh one. On drop the VM returns to the pool. Runs nest
//! (a function body spawns a command substitution, …), so multiple guards
//! can be live at once; the pool grows to the maximum nesting depth.
//! Thread-local because a fusevm `VM` is not `Sync`.

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

thread_local! {
    /// Released, ready-to-reuse VMs for this thread. Each retains its
    /// registered builtin table and host from first construction.
    static POOL: RefCell<Vec<fusevm::VM>> = const { RefCell::new(Vec::new()) };
}

/// Cap on retained VMs per thread. Deep recursion can hold many guards at
/// once; we only bound what we *keep*, so a pathological one-off recursion
/// doesn't pin a large fleet forever. Excess returns are dropped.
const MAX_POOLED: usize = 64;

/// RAII handle to a pooled VM. Derefs to [`fusevm::VM`]; returns the VM to
/// the pool on drop.
pub struct PooledVm {
    vm: Option<fusevm::VM>,
}

/// Acquire a call-ready VM for `chunk`. Recycles a pooled VM (builtins
/// already registered — just `reset`) or builds and registers a fresh one.
pub fn acquire(chunk: fusevm::Chunk) -> PooledVm {
    let vm = match POOL.with(|p| p.borrow_mut().pop()) {
        Some(mut recycled) => {
            recycled.reset(chunk);
            recycled
        }
        None => {
            let mut fresh = fusevm::VM::new(chunk);
            crate::fusevm_bridge::register_builtins(&mut fresh);
            fresh
        }
    };
    PooledVm { vm: Some(vm) }
}

impl Deref for PooledVm {
    type Target = fusevm::VM;
    #[inline]
    fn deref(&self) -> &fusevm::VM {
        // Present for the whole lifetime; only `take`n in Drop.
        self.vm.as_ref().expect("PooledVm used after drop")
    }
}

impl DerefMut for PooledVm {
    #[inline]
    fn deref_mut(&mut self) -> &mut fusevm::VM {
        self.vm.as_mut().expect("PooledVm used after drop")
    }
}

impl Drop for PooledVm {
    fn drop(&mut self) {
        if let Some(vm) = self.vm.take() {
            POOL.with(|p| {
                let mut pool = p.borrow_mut();
                if pool.len() < MAX_POOLED {
                    pool.push(vm);
                }
            });
        }
    }
}
