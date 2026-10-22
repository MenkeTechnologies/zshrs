//! The error indicator of C's `stdout` FILE for builtin output.
//!
//! `execcmd_exec` flushes stdout after every builtin and, when the command
//! did not redirect fd 1, reports `write error: %e` if `ferror(stdout)` is
//! set (c:Src/exec.c:4298-4305).
//!
//! !!! WARNING: RUST-ONLY !!! `std::io::Stdout` treats `EBADF` as a
//! successful write (std's `handle_ebadf`), so a write to a closed fd 1
//! never reports an error to the caller and `ferror(stdout)` cannot be
//! observed through it. print/echo/printf output goes through
//! [`fwrite_stdout`], which writes with write(2) and records the failure.

use std::io::Write as _;
use std::sync::atomic::{AtomicI32, Ordering};

/// errno of the failed stdout write; 0 = clear (C: `clearerr(stdout)`).
pub static STDOUT_FERROR: AtomicI32 = AtomicI32::new(0);

/// `fwrite(buf, 1, len, stdout)` for each of `bufs`, then the flush that ends
/// the builtin. Bytes already buffered in `lk` are flushed first to keep
/// output order.
///
/// The pieces go out in ONE write(2), as C's stdio buffer delivers them:
/// `print x` is `fwrite("x")` + `fputc('\n')` into the same buffer, and a
/// reader on the other end of a pipe — a multios tee, `| cat` — sees `x\n`
/// whole. Two writes let the tee read `x` alone and relay it to every target
/// before the newline arrived (`print x >&1 >f | cat` came out `xx\n\n`).
pub fn fwrite_stdout(lk: &mut std::io::StdoutLock<'_>, bufs: &[&[u8]]) {
    let _ = lk.flush();
    let joined: Vec<u8>;
    let buf: &[u8] = match bufs {
        [single] => single,
        _ => {
            joined = bufs.concat();
            &joined
        }
    };
    let mut off = 0;
    while off < buf.len() {
        let n = unsafe { libc::write(1, buf[off..].as_ptr() as *const libc::c_void, buf.len() - off) };
        if n < 0 {
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if e == libc::EINTR {
                continue;
            }
            STDOUT_FERROR.store(e, Ordering::Relaxed);
            return;
        }
        off += n as usize;
    }
}

/// `ferror(stdout)` + `clearerr(stdout)`: the recorded errno, cleared.
pub fn take_stdout_ferror() -> i32 {
    STDOUT_FERROR.swap(0, Ordering::Relaxed)
}
