//! File descriptor utilities for zshrs
//!
//! Based on fish-shell's fds.rs, providing safe fd management.

use std::fs::File;
use std::io;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

/// The first "high fd", outside the user-specifiable range (>&5).
pub const FIRST_HIGH_FD: RawFd = 10;

/// A pair of connected pipe file descriptors.
pub struct AutoClosePipes {
    pub read: OwnedFd,
    pub write: OwnedFd,
}

/// Create a pair of connected pipes with CLOEXEC set.
/// Returns None on failure.
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
pub fn dup_fd(fd: RawFd) -> io::Result<RawFd> {
    let new_fd = unsafe { libc::dup(fd) };
    if new_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(new_fd)
    }
}

/// Duplicate fd to a specific target fd.
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

/// A File wrapper that doesn't close on drop (borrows the fd).
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

/// File descriptor type constants (from zsh.h FDT_*)
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

/// Move file descriptor to >= 10 to keep low fds free for user redirection
/// Port from zsh/Src/utils.c movefd() lines 1980-2012
pub fn movefd(fd: RawFd) -> RawFd {
    if fd < 0 || fd >= FIRST_HIGH_FD {
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

/// Duplicate fd x to y. If x == -1, fd y is closed.
/// Port from zsh/Src/utils.c redup() lines 2019-2068
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

/// Close a file descriptor
/// Port from zsh/Src/utils.c zclose() lines 2126-2148
pub fn zclose(fd: RawFd) -> i32 {
    if fd >= 0 {
        unsafe { libc::close(fd) }
    } else {
        -1
    }
}

/// Duplicate file descriptor
pub fn zdup(fd: RawFd) -> RawFd {
    if fd < 0 {
        return -1;
    }
    unsafe { libc::dup(fd) }
}

/// Check if file descriptor is open
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

        // Create borrowed file and drop it
        let borrowed = unsafe { BorrowedFdFile::from_raw_fd(fd) };
        drop(borrowed);

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
