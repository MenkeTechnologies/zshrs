//! Keep the shell's own file descriptors out of the user's fd space.
//!
//! zsh routes every internal open through `movefd()` (Src/utils.c:1975), which
//! relocates the descriptor to >= 10 with `fcntl(fd, F_DUPFD, 10)` and records
//! it as `FDT_INTERNAL`. The point is that fds 0-9 belong to the SCRIPT: `exec
//! 3>out`, `read -u 4`, `print -u 3` and friends address them by number, and a
//! shell that parks its own state there is squatting on the user's namespace.
//!
//! zshrs was squatting. A `-c` run held:
//!
//!     fd 3 -> ~/.zshrs/zshrs.log
//!     fd 4 -> ~/.zshrs/zshrs_history.db          (sqlite)
//!     fd 5,6,7 -> ~/.zshrs/compsys.db + -wal + -shm
//!
//! so `print -u 3 -r -- x` appended `x` to the shell's own LOG and reported
//! success (zsh: `bad file number: 3`, status 1), and `exec 3>myfile` would
//! dup2 over the log handle — `exec 4>myfile` over the live sqlite handle.
//!
//! `movefd` only works for a descriptor we own and can close. sqlite caches the
//! fd number inside the connection, so it cannot be relocated after the fact.
//! The fix therefore has to make the open LAND high: hold every low fd for the
//! duration of the open, so the kernel's "lowest free descriptor" rule hands out
//! >= 10, then release. That covers lazily-opened descriptors (sqlite's WAL and
//! SHM appear on first use) as long as the open happens inside the guard.
//!
//! # Concurrency and fork
//!
//! Unlike C zsh, zshrs runs the guard off the main thread: `compinit` ships its
//! cache rebuild to the worker pool, and that task calls
//! [`crate::compsys::cache::CompsysCache::open`] while the main thread is still
//! sourcing the user's rc files. So the guard MUST be safe against concurrent
//! opens in other threads.
//!
//! It is made safe without any lock. Every descriptor the guard owns is obtained
//! from `fcntl(F_DUPFD_CLOEXEC)`, which allocates the lowest free descriptor as a
//! single atomic kernel operation — two threads racing get two distinct fds, and
//! the guard never names, and therefore never clobbers, a descriptor somebody
//! else owns.
//!
//! Lock-freedom is not an optimization here, it is a correctness requirement:
//! `fork(2)` is called directly in `fusevm_bridge.rs` and `ported/exec.rs`, and
//! only the forking thread survives into the child. A mutex held by any other
//! thread at that moment is inherited permanently locked, so a guard that
//! serialized on one would deadlock the first `LowFdGuard::new()` in the child.
//! Raw fd syscalls have no such hazard: the child inherits the fd table and the
//! guard behaves identically there.
//!
//! `F_DUPFD_CLOEXEC` also means the reserved slots do not leak through an
//! `exec(2)` that races the guard — the old `dup2` path cleared `FD_CLOEXEC` on
//! its targets, so a child could start life with `/dev/null` parked on fds 3-9.

use std::os::unix::io::RawFd;

/// The first descriptor the shell may use for itself — everything below belongs
/// to the script. Matches C's `movefd` threshold (Src/utils.c:1980, `F_DUPFD, 10`).
const FIRST_INTERNAL_FD: RawFd = 10;

/// The first descriptor the script can name. 0/1/2 are stdin/stdout/stderr and
/// are never part of the reservation — `exec 3>out` starts at 3.
const FIRST_SCRIPT_FD: RawFd = 3;

/// How many slots the guard can hold at most: the whole script range.
const SCRIPT_FD_SLOTS: usize = (FIRST_INTERNAL_FD - FIRST_SCRIPT_FD) as usize;

/// Upper bound of the sweep that registers freshly-landed internal
/// descriptors in the fdtable.
///
/// An internal open lands on the lowest descriptor free at the time, so
/// with the script range held it lands just above [`FIRST_INTERNAL_FD`];
/// SQLite adds at most two more per database (`-wal`, `-shm`). 64 is far
/// past anything the shell opens for itself. A descriptor that somehow
/// lands above it simply stays unregistered — the behaviour before this
/// sweep existed — rather than costing a longer scan on every open.
const FD_SCAN_LIMIT: RawFd = 64;

/// Is `fd` open in this process?
fn fd_is_open(fd: RawFd) -> bool {
    // SAFETY: F_GETFD only interrogates the descriptor table.
    unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
}

/// The descriptors at or above [`FIRST_INTERNAL_FD`] that this process
/// was STARTED with.
///
/// They belong to whoever spawned us — `zshrs 11>&1 -c …` is legal — and
/// c:Src/exec.c:3886-3891 is explicit that the shell must keep its hands
/// off a descriptor it did not open: "If the requested fd is >
/// max_zsh_fd, the shell doesn't know about it. Just assume the user
/// knows what they're doing." Recording them once lets the sweep below
/// register only descriptors that appeared afterwards, which are the
/// shell's own.
///
/// Captured on the first [`LowFdGuard::new`], which by construction runs
/// before the first internal open (that open is what the guard exists to
/// push upward).
fn inherited_fds() -> &'static [bool] {
    static INHERITED: std::sync::OnceLock<Vec<bool>> = std::sync::OnceLock::new();
    INHERITED.get_or_init(|| {
        (0..FD_SCAN_LIMIT)
            .map(|fd| fd >= FIRST_INTERNAL_FD && fd_is_open(fd))
            .collect()
    })
}

/// Occupies every currently-free descriptor below [`FIRST_INTERNAL_FD`] so that
/// opens performed while it is alive are pushed above the user's fd range.
/// Releases them on drop.
///
/// Descriptors that are ALREADY open are left alone — they may be the user's
/// (`zshrs 3>&1 -c '…'` is legal), and stealing them would be the very bug this
/// exists to prevent.
///
/// # Thread safety
///
/// Claiming is done exclusively through `fcntl(F_DUPFD_CLOEXEC)`, which asks the
/// kernel for the *lowest free* descriptor and allocates it in one atomic step.
/// The guard therefore only ever owns descriptors the kernel just handed it, and
/// never names a target descriptor.
///
/// The previous implementation probed each slot with `fcntl(fd, F_GETFD)` and
/// then `dup2`'d onto it. Those are two syscalls with a gap in between, and
/// `dup2` *silently closes* whatever occupies its target. Under the interactive
/// boot — where `compinit` ships its cache rebuild to a `zshrs-worker-N` thread
/// (`ext_builtins.rs`, `worker_pool.submit`) while the main thread is still
/// sourcing the user's rc files — a concurrent `File::open` could land on a slot
/// this guard had already probed as free. The `dup2` then closed a live
/// `OwnedFd` out from under its owner, and `drop` on that owner tripped std's
/// `debug_assert_fd_is_open`, aborting with
/// `IO Safety violation: owned file descriptor already closed`.
pub struct LowFdGuard {
    held: Vec<RawFd>,
}

impl LowFdGuard {
    /// Reserve the free script descriptors. Cheap: one `open` of `/dev/null`
    /// plus at most [`SCRIPT_FD_SLOTS`] `fcntl` calls, and only for slots that
    /// are actually free.
    pub fn new() -> Self {
        // Before anything is opened, so the snapshot is of the parent's
        // descriptors only. Cheap after the first call (a `OnceLock` read).
        let _ = inherited_fds();
        let mut held = Vec::with_capacity(SCRIPT_FD_SLOTS);
        // SAFETY: plain fd syscalls; every descriptor here came from this
        // thread's own `open`/`fcntl` and is closed exactly once — either at the
        // end of this function or in `drop`, never both.
        unsafe {
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if devnull < 0 {
                return Self { held };
            }
            // `open` also allocates the lowest free descriptor, so the /dev/null
            // handle may itself land inside the range being reserved. When it
            // does it counts as one of the reserved slots and is released with
            // the rest; otherwise it is a plain temporary closed below.
            let devnull_is_held = (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD).contains(&devnull);
            if devnull_is_held {
                held.push(devnull);
            }
            // Duplicate repeatedly. Each `F_DUPFD_CLOEXEC` returns the lowest
            // free descriptor at or above `FIRST_SCRIPT_FD`, so successive calls
            // walk up through the free slots; the first result at or above
            // `FIRST_INTERNAL_FD` proves the whole script range is now spoken for
            // — by this guard or by a pre-existing owner — and ends the loop.
            // `held.len()` bounds the iteration count even if another thread
            // frees a low slot mid-walk.
            //
            // This is `movefd` used as a claim rather than a move
            // (c:Src/utils.c:1980 `fcntl(fd, F_DUPFD, 10)`).
            while held.len() < SCRIPT_FD_SLOTS {
                let fd = libc::fcntl(devnull, libc::F_DUPFD_CLOEXEC, FIRST_SCRIPT_FD);
                if fd < 0 {
                    break;
                }
                if fd >= FIRST_INTERNAL_FD {
                    libc::close(fd);
                    break;
                }
                held.push(fd);
            }
            if !devnull_is_held {
                libc::close(devnull);
            }
        }
        // Off by default (`trace`), and the one call this module cares about —
        // "which thread reserved which slots" — is the only way to tell a
        // main-thread reservation from a worker-pool one after the fact.
        tracing::trace!(?held, "lowfd: reserved script descriptors");
        Self { held }
    }
}

impl Drop for LowFdGuard {
    fn drop(&mut self) {
        for &fd in &self.held {
            // SAFETY: only descriptors this guard installed are closed.
            unsafe {
                libc::close(fd);
            }
        }
    }
}

/// Mark every descriptor the shell has just opened for itself as
/// `FDT_INTERNAL`.
///
/// c:Src/utils.c:2007-2010 — `movefd` ends with `check_fd_table(fd);
/// fdtable[fd] = FDT_INTERNAL;`, so in C every descriptor the shell
/// relocates for its own use is registered at the moment it is
/// relocated. That registration is what the rest of the shell reads:
/// c:Src/exec.c:3830-3835 refuses `exec N>&-` on an internal descriptor
/// ("file descriptor %d used by shell, not closed"), and
/// c:Src/exec.c:3884-3897 fails `>&N` / `<&N` with `EBADF` when
/// `fdtable[N]` is neither `FDT_UNUSED` nor `FDT_EXTERNAL`. Both of
/// those gates are ported; both were dead for the shell's OWN
/// descriptors.
///
/// zshrs does not reach its internal descriptors through `movefd`. The
/// log file, the history database and the compsys database are opened by
/// `tracing-appender` and by SQLite, which hand back a `File` / a
/// `Connection` and never expose the descriptor — so there is no
/// `movefd` call site at which to register them, and they landed with
/// `fdtable` still reading `FDT_UNUSED`. The shell's own log sat at
/// fd 10 and its history database at fd 11, unregistered, and every gate
/// above waved them through: `exec 11>&-` closed the live SQLite handle
/// and `exec 3>&11` duplicated it.
///
/// The guard is the choke point every one of those opens passes through,
/// so the registration happens here instead: whatever is open above
/// [`FIRST_INTERNAL_FD`] that we did not inherit and that nothing has
/// claimed is, by elimination, a descriptor the shell opened for itself.
///
/// A slot that is already claimed is never overwritten — `FDT_EXTERNAL`
/// (a `{varid}>` descriptor, c:Src/exec.c:2409), `FDT_MODULE`,
/// `FDT_PROC_SUBST` and the rest keep their meaning. And a user
/// redirection racing this sweep from another thread self-corrects: the
/// redirection sets its slot to `FDT_EXTERNAL` unconditionally after the
/// open, so a momentary `FDT_INTERNAL` here is overwritten by the
/// classification that matters.
/// # Why this is not in `LowFdGuard::drop`
///
/// `fdtable_get`/`fdtable_set` take a `Mutex` (utils.rs:11483). This
/// module's header spells out why a lock must never be on the guard's
/// path: `fork(2)` is called directly in `fusevm_bridge.rs` and
/// `ported/exec.rs`, only the forking thread survives into the child,
/// and a mutex held by any other thread at that moment is inherited
/// permanently locked — so the child's first guard would deadlock. Doing
/// the sweep in `drop` did exactly that, and
/// `guard_still_works_in_a_fork_child_while_other_threads_hold_it`
/// caught it.
///
/// So the sweep is an explicit step on the open path instead
/// ([`with_high_fds`]), which only the shell's own database and log
/// opens take, and which no fork child runs. `LowFdGuard` itself stays
/// pure raw-syscall and fork-safe.
pub fn register_internal_fds() {
    let inherited = inherited_fds();
    for fd in FIRST_INTERNAL_FD..FD_SCAN_LIMIT {
        if inherited[fd as usize] || !fd_is_open(fd) {
            continue;
        }
        if crate::ported::utils::fdtable_get(fd) != crate::ported::zsh_h::FDT_UNUSED {
            continue; // already classified — EXTERNAL, MODULE, PROC_SUBST, …
        }
        crate::ported::utils::check_fd_table(fd); // c:2008
        crate::ported::utils::fdtable_set(fd, crate::ported::zsh_h::FDT_INTERNAL); // c:2009
        tracing::debug!(fd, "lowfd: registered shell-internal descriptor");
    }
}

impl Default for LowFdGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `f` with the low descriptors reserved, so anything it opens lands at or
/// above [`FIRST_INTERNAL_FD`].
pub fn with_high_fds<T>(f: impl FnOnce() -> T) -> T {
    let out = {
        let _guard = LowFdGuard::new();
        f()
    };
    // The guard is released above, so anything `f` opened is now at its
    // final descriptor and can be registered as the shell's own.
    register_internal_fds();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor the shell opens for itself is registered as
    /// `FDT_INTERNAL`, which is what makes c:Src/exec.c:3830-3835 refuse
    /// `exec N>&-` on it and c:Src/exec.c:3884-3897 fail `>&N` with
    /// `EBADF`. Both gates are ported; both read the fdtable, and the
    /// shell's own descriptors were never in it.
    #[test]
    fn an_internal_open_is_registered_in_the_fdtable() {
        // Same serialisation the other guard tests take: these tests share
        // one process, and a guard in flight moves descriptors 3-9 under
        // any test that is sampling them. `fd_test_lock` additionally keeps
        // out the rest of the binary's fd-moving tests — this assertion
        // reads the descriptor number the kernel handed back, so a
        // concurrently-freed low slot would land the open there.
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        use std::os::unix::io::AsRawFd;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("internal");
        let _file = with_high_fds(|| std::fs::File::create(&path).expect("create"));
        let fd = _file.as_raw_fd();
        assert!(
            fd >= FIRST_INTERNAL_FD,
            "the guard must push a shell-internal open above the script range; landed on {fd}"
        );
        assert!(
            fd < FD_SCAN_LIMIT,
            "test descriptor {fd} is past the sweep bound, the assertion below would be vacuous"
        );
        assert_eq!(
            crate::ported::utils::fdtable_get(fd),
            crate::ported::zsh_h::FDT_INTERNAL,
            "fd {fd} is the shell's own and must be marked FDT_INTERNAL (c:Src/utils.c:2009)"
        );
    }
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// The guard is process-global state. Two of these tests running on
    /// different harness threads would observe each other's reservations, so
    /// they take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// The lock the rest of the test binary uses to serialise work that
    /// touches process-global state, taken here for the descriptor TABLE.
    ///
    /// `SERIAL` only keeps the guard tests apart from each other; the fd table
    /// is shared with all ~10k tests, and several move low descriptors on
    /// purpose — `bin_sysread_no_args_returns_nonzero`
    /// (`ported/modules/system.rs:2595-2605`) dups a pipe onto fd 0 and closes
    /// pipe ends that can sit inside the script range. Those tests hold this
    /// lock, so taking it keeps them out of the window rather than racing
    /// them. It is not a complete fence — a test that touches fds without
    /// taking it still can interleave — which is why the assertions below pin
    /// the guard's own reservation rather than the descriptor number the
    /// kernel happens to return.
    ///
    /// Order is always this lock first, then `SERIAL`.
    fn fd_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_util::global_state_lock()
    }

    /// Which of the script descriptors are currently open.
    fn open_script_fds() -> Vec<RawFd> {
        (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD)
            .filter(|&fd| unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1)
            .collect()
    }

    /// The guard reserves the SCRIPT range, and that is what is pinned here:
    /// while it is alive every descriptor in `FIRST_SCRIPT_FD..FIRST_INTERNAL_FD`
    /// is occupied, so a fresh open cannot land on one.
    ///
    /// It does NOT, and must not, reserve 0/1/2 — those are stdin/stdout/stderr
    /// and belong to nobody the shell may park `/dev/null` on (see
    /// `FIRST_SCRIPT_FD`: the script's own descriptors start at 3, `exec 3>out`).
    /// Asserting the *consequence* (`landed >= FIRST_INTERNAL_FD`) instead of the
    /// reservation therefore made this test depend on a precondition the guard
    /// neither controls nor claims: that stdio is open. In a 10k-test binary it
    /// intermittently is not — another test has a low descriptor in flight — and
    /// the kernel's lowest-free rule then hands `open` fd 0 before the guard's
    /// range is ever reached. Observed in a full `cargo test --lib`:
    /// "open inside the guard landed on fd 0, inside the script range".
    #[test]
    fn guard_pushes_opens_above_the_script_range() {
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let guard = LowFdGuard::new();

        // The reservation itself — the guard's own invariant, independent of
        // which number the kernel hands back next. Every script slot is spoken
        // for: by this guard, or by whoever already owned it.
        for fd in FIRST_SCRIPT_FD..FIRST_INTERNAL_FD {
            assert!(
                fd_is_open(fd),
                "guard left script fd {fd} free; it reserved {:?}",
                guard.held
            );
        }

        let f = std::fs::File::open("/dev/null").expect("open /dev/null");
        let landed = f.as_raw_fd();
        // With every script slot proven occupied above, a landing anywhere in
        // that range is the guard's fault and nothing else's.
        assert!(
            !(FIRST_SCRIPT_FD..FIRST_INTERNAL_FD).contains(&landed),
            "open inside the guard landed on fd {landed}, inside the script range"
        );
        drop(f);
        drop(guard);
    }

    #[test]
    fn guard_restores_the_script_range_exactly() {
        // Samples the script range before and after, so any other test that
        // opens or closes a descriptor down there in between is a false
        // failure — see `fd_test_lock`.
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let before = open_script_fds();
        {
            let _g = LowFdGuard::new();
        }
        assert_eq!(
            before,
            open_script_fds(),
            "guard leaked or freed descriptors it did not take"
        );
    }

    /// The bug this module was rewritten for: the old probe-then-`dup2` claim
    /// could land on a descriptor another thread had just been handed, and
    /// `dup2` closes its target silently. Detect it by identity, not by
    /// liveness — a clobbered descriptor is still *open*, it just points at
    /// `/dev/null` instead of the file its owner opened.
    ///
    /// Deliberately uses raw `libc` descriptors rather than `std::fs::File`:
    /// under the old code the theft also tripped std's IO-safety assert, which
    /// aborts the whole test binary instead of failing one test.
    #[test]
    fn concurrent_guards_never_steal_a_descriptor_from_another_thread() {
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

        let path = std::env::temp_dir().join(format!("zshrs-lowfd-{}", std::process::id()));
        std::fs::write(&path, b"sentinel").expect("write probe file");
        let want_ino = std::fs::metadata(&path).expect("stat probe file").ino();
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let claimers: Vec<_> = (0..3)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        drop(LowFdGuard::new());
                    }
                })
            })
            .collect();

        // Meanwhile: open the probe file over and over. Whenever the kernel
        // hands back a descriptor in the script range, that descriptor is ours
        // and must still refer to the probe file.
        let mut stolen = None;
        for _ in 0..200_000 {
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
            if fd < 0 {
                continue;
            }
            // Only the SCRIPT range is watched. The guard can never affect
            // fds 0/1/2: it does not reserve them (`FIRST_SCRIPT_FD`), and the
            // only syscall it aims at them is `open`, which cannot return a
            // descriptor somebody else holds. A change of identity down there
            // is some other test re-pointing stdio — e.g.
            // `bin_sysread_no_args_returns_nonzero`
            // (`ported/modules/system.rs:2598-2604`) dups a pipe onto fd 0 and
            // back — which is exactly what this assertion used to report as
            // "a LowFdGuard on another thread took over live fd Some(0)".
            if (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD).contains(&fd) {
                std::thread::yield_now();
                let mut st: libc::stat = unsafe { std::mem::zeroed() };
                if unsafe { libc::fstat(fd, &mut st) } == 0 && st.st_ino != want_ino {
                    stolen = Some(fd);
                }
            }
            unsafe { libc::close(fd) };
            if stolen.is_some() {
                break;
            }
        }

        stop.store(true, Ordering::Relaxed);
        for t in claimers {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&path);

        assert!(
            stolen.is_none(),
            "a LowFdGuard on another thread took over live fd {:?}",
            stolen
        );
    }

    /// The guard must stay usable in a `fork(2)` child. zshrs forks raw in a
    /// dozen places (`fusevm_bridge.rs`, `ported/exec.rs`, `modules/zpty.rs`,
    /// `modules/clone.rs`, …) and only the forking thread survives, so any lock
    /// another thread happened to hold is inherited permanently locked. An
    /// earlier attempt to fix the descriptor race by serializing `new()` on a
    /// `Mutex` was reverted for exactly this reason; this test fails (by
    /// timeout) if anyone reintroduces one.
    #[test]
    fn guard_still_works_in_a_fork_child_while_other_threads_hold_it() {
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

        let stop = Arc::new(AtomicBool::new(false));
        let hammers: Vec<_> = (0..4)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        drop(LowFdGuard::new());
                    }
                })
            })
            .collect();

        let mut stuck = 0;
        // The descriptor each failing child landed on, so a real regression
        // names the slot instead of only counting.
        let mut failed: Vec<i32> = Vec::new();
        for _ in 0..20 {
            // SAFETY: the child does nothing but build a guard, open once, and
            // `_exit` — no unwinding, no atexit handlers, no stdio.
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let g = LowFdGuard::new();
                // The contract is "opens land high", not "we claimed
                // something": a child forked while another thread held the
                // whole range inherits those descriptors and correctly
                // reserves nothing.
                let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
                // The contract is the SCRIPT range. 0/1/2 are stdio and are
                // never reserved (`FIRST_SCRIPT_FD`), so a child forked while
                // the parent happened to have one of them closed legitimately
                // gets it back from the kernel's lowest-free rule — not a
                // guard failure. Report the offending descriptor as the exit
                // status so a real one is diagnosable.
                let bad = (FIRST_SCRIPT_FD..FIRST_INTERNAL_FD).contains(&fd);
                unsafe { libc::close(fd) };
                drop(g);
                unsafe { libc::_exit(if bad { fd } else { 0 }) };
            }
            assert!(pid > 0, "fork failed");

            // A guard that deadlocked in the child never reaches `_exit`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut status = 0;
            loop {
                let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if r == pid {
                    let code = libc::WEXITSTATUS(status);
                    if code != 0 {
                        failed.push(code);
                    }
                    break;
                }
                if std::time::Instant::now() > deadline {
                    stuck += 1;
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    unsafe { libc::waitpid(pid, &mut status, 0) };
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        stop.store(true, Ordering::Relaxed);
        for t in hammers {
            let _ = t.join();
        }
        assert_eq!(stuck, 0, "LowFdGuard::new() hung in a fork child");
        assert!(
            failed.is_empty(),
            "an open inside a fork child's guard landed in the script fd range: {failed:?}"
        );
    }

    /// The reservation must be `FD_CLOEXEC`. The old `dup2` claim cleared it, so
    /// an `exec` racing the guard started the child with `/dev/null` parked on
    /// the script's descriptors.
    #[test]
    fn reserved_descriptors_are_close_on_exec() {
        let _g = fd_test_lock();
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let before = open_script_fds();
        let guard = LowFdGuard::new();
        for &fd in &guard.held {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(flags != -1, "reserved fd {fd} is not open");
            assert!(
                flags & libc::FD_CLOEXEC != 0,
                "reserved fd {fd} would leak through exec"
            );
        }
        // Only free slots may be claimed — never one that was already live.
        for fd in &before {
            assert!(
                !guard.held.contains(fd),
                "guard claimed fd {fd}, which was already open"
            );
        }
        drop(guard);
    }
}
