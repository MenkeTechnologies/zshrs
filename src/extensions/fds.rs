//! File descriptor utilities for zshrs.
//!
//! The top half (`AutoClosePipes`, `heightenize_fd`, `BorrowedFdFile`)
//! is borrowed from fish-shell's `fds.rs` — these are zshrs-original
//! safe-wrapper infrastructure with no direct C zsh counterpart.
//!
//! The bottom half (`movefd`, `redup`, `zclose`, etc.) is a direct
//! port of the fd helpers C zsh keeps in `Src/utils.c` for redirection
//! and pipe management. Each function below is annotated with its
//! exact C origin.

use std::fs::File;
use std::io;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

/// The first "high fd", outside the user-specifiable range (>&5).
/// Mirrors the `FDT_HIGHFD` constant Src/exec.c uses when calling
/// `movefd()` to keep low fds free for `>&N` user redirection.
pub const FIRST_HIGH_FD: RawFd = 10;

/// A pair of connected pipe file descriptors.
/// zshrs-original RAII wrapper (borrowed from fish-shell). C zsh
/// uses bare `int fds[2]` from `pipe(2)` and `mpipe()`
/// (Src/exec.c) which gets manual `close(2)` calls on every error
/// path; the OwnedFd pair makes that automatic.
pub struct AutoClosePipes {
    pub read: OwnedFd,
    pub write: OwnedFd,
}

/// Create a pair of connected pipes with CLOEXEC set.
/// zshrs-original wrapper around `pipe(2)` (borrowed from fish-
/// shell). C zsh's equivalent is `mpipe()` in Src/exec.c which
/// calls `pipe(2)` then `movefd(fd, 1)` on each end. The Rust path
/// also sets `FD_CLOEXEC` so descriptors don't leak into child
/// processes.
pub fn make_autoclose_pipes() -> io::Result<AutoClosePipes> {
    let (read_fd, write_fd) =
        nix::unistd::pipe().map_err(|e| io::Error::from_raw_os_error(e as i32))?;

    // Move fds to high range and set CLOEXEC
    let read_fd = heightenize_fd(read_fd)?;
    let write_fd = heightenize_fd(write_fd)?;

    Ok(AutoClosePipes {
        read: read_fd,
        write: write_fd,
    })
}

/// Move an fd to the high range (>= FIRST_HIGH_FD) and set CLOEXEC.
/// Equivalent to `movefd()` from Src/utils.c plus an additional
/// `FD_CLOEXEC` set step. The C source sets `FD_CLOEXEC` indirectly
/// by registering the fd in `fdtable[]` with `FDT_INTERNAL`.
fn heightenize_fd(fd: OwnedFd) -> io::Result<OwnedFd> {
    let raw_fd = fd.as_raw_fd();

    if raw_fd >= FIRST_HIGH_FD {
        set_cloexec(raw_fd, true)?;
        return Ok(fd);
    }

    // Dup to high range with CLOEXEC
    let new_fd = nix::fcntl::fcntl(raw_fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(FIRST_HIGH_FD))
        .map_err(|e| io::Error::from_raw_os_error(e as i32))?;

    Ok(unsafe { OwnedFd::from_raw_fd(new_fd) })
}

/// Set or clear CLOEXEC on a file descriptor.
/// Port of the `fdtable_flocks` close-on-exec set/clear logic
/// Src/utils.c uses on internal fds — `fcntl(F_GETFD)` + `F_SETFD`
/// with the bit toggled. The C source doesn't expose this as a
/// dedicated function; we factor it out for the RAII pipe path.
pub fn set_cloexec(fd: RawFd, should_set: bool) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }

    let new_flags = if should_set {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };

    if flags != new_flags {
        let result = unsafe { libc::fcntl(fd, libc::F_SETFD, new_flags) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

/// Make an fd nonblocking.
/// Port of the `fcntl(fd, F_SETFL, O_NONBLOCK)` idiom Src/utils.c
/// uses for completion's coroutine fds and for `read -t`.
pub fn make_fd_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }

    if (flags & libc::O_NONBLOCK) == 0 {
        let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

/// Make an fd blocking.
/// Counterpart of `make_fd_nonblocking`. C zsh restores blocking
/// mode after `read -t` (Src/utils.c) by clearing `O_NONBLOCK`.
pub fn make_fd_blocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }

    if (flags & libc::O_NONBLOCK) != 0 {
        let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

/// Close a file descriptor, retrying on EINTR.
/// Port of `zclose()` from Src/utils.c — same `EINTR` retry loop;
/// negative fds are treated as already-closed (no-op).
pub fn close_fd(fd: RawFd) {
    if fd < 0 {
        return;
    }
    loop {
        let result = unsafe { libc::close(fd) };
        if result == 0 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            break;
        }
    }
}

/// Duplicate a file descriptor.
/// Port of the `dup(2)` call sites in Src/exec.c that copy a saved
/// fd back into place during `exec >` / `exec <` redirection
/// teardown.
pub fn dup_fd(fd: RawFd) -> io::Result<RawFd> {
    let new_fd = unsafe { libc::dup(fd) };
    if new_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(new_fd)
    }
}

/// Duplicate fd to a specific target fd.
/// Port of the `dup2(2)` calls Src/exec.c uses to install the
/// shell's pipeline ends on fd 0/1/2 before `execve(2)`.
pub fn dup2_fd(src: RawFd, dst: RawFd) -> io::Result<()> {
    if src == dst {
        return Ok(());
    }
    let result = unsafe { libc::dup2(src, dst) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A `File` wrapper that doesn't close on drop (borrows the fd).
/// zshrs-original (borrowed from fish-shell). C zsh shares fds
/// across builtins with bare `int fd` since lifetime is implicit;
/// Rust needs an explicit RAII handle that dups the fd without
/// transferring ownership.
pub struct BorrowedFdFile(ManuallyDrop<File>);

impl Deref for BorrowedFdFile {
    type Target = File;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for BorrowedFdFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromRawFd for BorrowedFdFile {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(ManuallyDrop::new(unsafe { File::from_raw_fd(fd) }))
    }
}

impl AsRawFd for BorrowedFdFile {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl IntoRawFd for BorrowedFdFile {
    fn into_raw_fd(self) -> RawFd {
        ManuallyDrop::into_inner(self.0).into_raw_fd()
    }
}

impl Clone for BorrowedFdFile {
    fn clone(&self) -> Self {
        unsafe { Self::from_raw_fd(self.as_raw_fd()) }
    }
}

impl BorrowedFdFile {
    pub fn stdin() -> Self {
        unsafe { Self::from_raw_fd(libc::STDIN_FILENO) }
    }

    pub fn stdout() -> Self {
        unsafe { Self::from_raw_fd(libc::STDOUT_FILENO) }
    }

    pub fn stderr() -> Self {
        unsafe { Self::from_raw_fd(libc::STDERR_FILENO) }
    }
}

impl io::Read for BorrowedFdFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.deref_mut().read(buf)
    }
}

impl io::Write for BorrowedFdFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.deref_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.deref_mut().flush()
    }
}

// ============================================================================
// Port from zsh/Src/utils.c: File descriptor table and management
// ============================================================================

/// File descriptor type constants.
/// Port of the `FDT_*` enum from `Src/zsh.h` — the C source uses
/// these to tag every fd in `fdtable[]` so `closem()` (Src/utils.c)
/// can decide which fds to leave open across `exec(2)` / `fork(2)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdType {
    Unused = 0,
    External = 1,
    Internal = 2,
    Module = 3,
    Flock = 4,
    FlockExec = 5,
    Proc = 6,
}

/// Move a file descriptor to >= `FIRST_HIGH_FD` so the low fds 0-9
/// stay free for user redirection.
/// Port of `movefd()` from Src/utils.c (~lines 1980-2012). Uses
/// `fcntl(F_DUPFD_CLOEXEC)` to get the new fd in the high range
/// then closes the original.
pub fn movefd(fd: RawFd) -> RawFd {
    if !(0..FIRST_HIGH_FD).contains(&fd) {
        return fd;
    }

    unsafe {
        let new_fd = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, FIRST_HIGH_FD);
        if new_fd != -1 {
            libc::close(fd);
            return new_fd;
        }
    }
    fd
}

/// Duplicate fd `x` to `y`. If `x == -1`, fd `y` is closed.
/// Port of `redup()` from Src/utils.c (~lines 2019-2068) — the C
/// source's "move fd x onto y, closing x" primitive that the
/// pipeline-builder calls for every redirection.
pub fn redup(x: RawFd, y: RawFd) -> RawFd {
    if x < 0 {
        unsafe { libc::close(y) };
        return y;
    }

    if x == y {
        return y;
    }

    let result = unsafe { libc::dup2(x, y) };
    if result == -1 {
        return -1;
    }

    unsafe { libc::close(x) };
    y
}

/// Close a file descriptor.
/// Port of `zclose()` from Src/utils.c (~lines 2126-2148) — the
/// safe-close shim that no-ops on negative fds and clears the
/// fdtable entry the C source maintains for `closem()` accounting.
pub fn zclose(fd: RawFd) -> i32 {
    if fd >= 0 {
        unsafe { libc::close(fd) }
    } else {
        -1
    }
}

/// Duplicate a file descriptor.
/// Port of the bare `dup(2)` calls Src/utils.c sprinkles around
/// the redirection-save/restore code. Negative input returns -1
/// (matches the C source's no-op-on-bad-fd convention).
pub fn zdup(fd: RawFd) -> RawFd {
    if fd < 0 {
        return -1;
    }
    unsafe { libc::dup(fd) }
}

/// Check if a file descriptor is open.
/// zshrs-original convenience — equivalent to the
/// `fcntl(fd, F_GETFD)` probe Src/utils.c uses inline before
/// touching an fd that may have been closed by a parallel branch.
pub fn fd_is_open(fd: RawFd) -> bool {
    if fd < 0 {
        return false;
    }
    unsafe { libc::fcntl(fd, libc::F_GETFD, 0) >= 0 }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_autoclose_pipes() {
        let pipes = make_autoclose_pipes().expect("Failed to create pipes");

        // Both fds should be in the high range
        assert!(pipes.read.as_raw_fd() >= FIRST_HIGH_FD);
        assert!(pipes.write.as_raw_fd() >= FIRST_HIGH_FD);

        // Both should have CLOEXEC set
        let read_flags = unsafe { libc::fcntl(pipes.read.as_raw_fd(), libc::F_GETFD, 0) };
        let write_flags = unsafe { libc::fcntl(pipes.write.as_raw_fd(), libc::F_GETFD, 0) };

        assert!(read_flags >= 0);
        assert!(write_flags >= 0);
        assert_ne!(read_flags & libc::FD_CLOEXEC, 0);
        assert_ne!(write_flags & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn test_set_cloexec() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = file.as_raw_fd();

        // Set CLOEXEC
        set_cloexec(fd, true).unwrap();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);

        // Clear CLOEXEC
        set_cloexec(fd, false).unwrap();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
        assert_eq!(flags & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn test_nonblocking() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = file.as_raw_fd();

        // Make nonblocking
        make_fd_nonblocking(fd).unwrap();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        assert_ne!(flags & libc::O_NONBLOCK, 0);

        // Make blocking again
        make_fd_blocking(fd).unwrap();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        assert_eq!(flags & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn test_borrowed_fd_file_does_not_close() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = file.as_raw_fd();

        // Create borrowed file and let it fall out of scope. Direct
        // `drop()` would tickle clippy::drop_non_drop because
        // BorrowedFdFile has no Drop impl by design — the whole
        // point of this type is that scope-end is a no-op for the
        // underlying fd. The inner block makes the lifetime
        // boundary explicit.
        {
            let _borrowed = unsafe { BorrowedFdFile::from_raw_fd(fd) };
        }

        // fd should still be valid
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
        assert!(
            flags >= 0,
            "fd should still be valid after dropping BorrowedFdFile"
        );

        // Now drop the original file
        drop(file);

        // fd should now be invalid
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
        assert!(
            flags < 0,
            "fd should be invalid after dropping original File"
        );
    }
}
