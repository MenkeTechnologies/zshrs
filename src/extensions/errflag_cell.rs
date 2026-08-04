//! Per-thread storage for the shell's `errflag`.
//!
//! **zshrs-original infrastructure — no `Src/*.c` counterpart.** It exists
//! because zshrs replaced a `fork(2)` with a thread.
//!
//! In C zsh, `errflag` (`Src/init.c`, declared `zsh.h:2833`) is a plain
//! `int` owned by exactly one thread: the shell is single-threaded, and
//! every piece of background work — a completion run, a command
//! substitution, a `compinit` rebuild — is a forked CHILD. The child gets
//! a *copy* of `errflag`; whatever it sets there can never be observed by
//! the parent's line editor.
//!
//! zshrs runs that same work on the worker pool ([`crate::worker`]) instead
//! of forking. With one process-global `AtomicI32`, the copy semantics were
//! lost and `errflag` became shared mutable state between the line editor
//! and every background parse. That is a correctness bug, not a style one:
//! `errflag` is the ENTIRE mechanism by which `send-break` (^G,
//! `Src/Zle/zle_misc.c:1144`) aborts a line —
//!
//! ```c
//! int sendbreak(UNUSED(char **args)) { errflag |= ERRFLAG_ERROR|ERRFLAG_INT; return 1; }
//! ```
//!
//! — and by which `zleread` (`Src/Zle/zle_main.c:1387`) then decides to
//! throw the line away instead of running it:
//!
//! ```c
//! if (eofsent || errflag || exit_pending) { s = NULL; }
//! ```
//!
//! The `compinit` bytecode backfill (`ext_builtins.rs`) parses thousands of
//! autoload bodies on a pool thread, and each body is bracketed by C's own
//! `errflag &= ~ERRFLAG_ERROR; … errflag = saved;` save/clear/restore
//! pattern. Landing one of those stores inside the ^G window cleared
//! `ERRFLAG_ERROR` under the editor, and the aborted line EXECUTED — `ls
//! /us` + ^G ran `ls`. A late restore re-injected a stale flag into the
//! NEXT `zleread`, aborting a line the user never typed, and a body that
//! failed to parse on the worker surfaced as `zsh: parse error` against
//! whatever the main thread happened to be parsing.
//!
//! Restoring the C model is enough to fix all of that: the shell thread
//! keeps the process-wide flag, every other thread gets its own. A thread
//! that runs shell code off the main thread IS a child in the C design, so
//! its errors stay with it.
//!
//! The four accessors mirror the `AtomicI32` API this type replaced, so
//! every `errflag.load(…)` / `.store(…)` / `.fetch_or(…)` / `.fetch_and(…)`
//! call site reads exactly as it did before.

use std::cell::Cell;
use std::sync::atomic::{AtomicI32, Ordering};

thread_local! {
    /// This thread's private `errflag`, plus whether this thread is the
    /// shell thread (in which case the private copy is never consulted).
    /// The `is_shell` flag is computed once per thread, so the hot
    /// `errflag.load()` path is a TLS read and a branch.
    static SLOT: Slot = Slot {
        is_shell: std::thread::current().name() == Some("main"),
        val: Cell::new(0),
    };
}

struct Slot {
    is_shell: bool,
    val: Cell<i32>,
}

/// `errflag` storage: process-global on the shell thread, thread-local
/// everywhere else. See the module docs for why.
pub struct ErrflagCell {
    shell: AtomicI32,
}

impl ErrflagCell {
    /// A cleared flag. `const` so `errflag` stays a plain `static`.
    pub const fn new() -> Self {
        ErrflagCell {
            shell: AtomicI32::new(0),
        }
    }

    /// Mirrors `AtomicI32::load`.
    #[inline]
    pub fn load(&self, order: Ordering) -> i32 {
        SLOT.with(|s| {
            if s.is_shell {
                self.shell.load(order)
            } else {
                s.val.get()
            }
        })
    }

    /// Mirrors `AtomicI32::store`.
    #[inline]
    pub fn store(&self, val: i32, order: Ordering) {
        SLOT.with(|s| {
            if s.is_shell {
                self.shell.store(val, order);
            } else {
                s.val.set(val);
            }
        })
    }

    /// Mirrors `AtomicI32::fetch_or`, returning the previous value.
    #[inline]
    pub fn fetch_or(&self, val: i32, order: Ordering) -> i32 {
        SLOT.with(|s| {
            if s.is_shell {
                self.shell.fetch_or(val, order)
            } else {
                let prev = s.val.get();
                s.val.set(prev | val);
                prev
            }
        })
    }

    /// Mirrors `AtomicI32::fetch_and`, returning the previous value.
    #[inline]
    pub fn fetch_and(&self, val: i32, order: Ordering) -> i32 {
        SLOT.with(|s| {
            if s.is_shell {
                self.shell.fetch_and(val, order)
            } else {
                let prev = s.val.get();
                s.val.set(prev & val);
                prev
            }
        })
    }
}

impl Default for ErrflagCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::{ERRFLAG_ERROR, ERRFLAG_INT};

    /// The regression this type exists for: a background parse's
    /// `errflag &= ~ERRFLAG_ERROR; … errflag = saved;` bracket must not be
    /// able to clear the flag `sendbreak()` just raised on the shell
    /// thread. Before the split, the worker's restore landed on the same
    /// word and the aborted line executed.
    #[test]
    fn off_thread_writes_do_not_disturb_the_shell_flag() {
        static FLAG: ErrflagCell = ErrflagCell::new();
        FLAG.store(0, Ordering::Relaxed);

        // The shell thread aborts a line (zle_misc.c:1144 sendbreak).
        FLAG.fetch_or(ERRFLAG_ERROR | ERRFLAG_INT, Ordering::Relaxed);

        // A pool thread runs the backfill's save/clear/parse/restore.
        std::thread::spawn(|| {
            let saved = FLAG.load(Ordering::Relaxed);
            assert_eq!(saved, 0, "a non-shell thread starts with its own flag");
            FLAG.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
            FLAG.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // its parse failed
            FLAG.store(saved, Ordering::Relaxed);
        })
        .join()
        .unwrap();

        // zleread's epilogue (zle_main.c:1387) must still see the abort.
        assert_eq!(
            FLAG.load(Ordering::Relaxed) & ERRFLAG_ERROR,
            ERRFLAG_ERROR,
            "the worker's restore cleared the editor's abort flag"
        );
    }
}
