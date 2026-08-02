//! `shout` — buffered terminal-output stream for the ZLE display.
//!
//! Rust-only infrastructure, not a port: it supplies the stdio buffering
//! that zsh gets for free from libc. C's display code writes every escape,
//! pad byte and glyph through `FILE *shout` (`putc`/`fputs`/
//! `tputs(..., putshout)`) and flushes ONCE per frame — `fflush(shout)` at
//! `Src/Zle/zle_refresh.c:1488` and `Src/Zle/zle_main.c:2090` — so a repaint
//! reaches the terminal as a single write.
//!
//! zshrs had no such stream: every fragment went straight to
//! `write_loop(SHTTY, …)`, one `write(2)` per escape or cell. Measured on a
//! 20-match `menu select` list (100x40 pty): zsh delivered the frame in 2 tty
//! writes, zshrs in 22+. The terminal renders each fragment as it lands, so
//! the repaint is visible as the cursor stepping across the menu.
//!
//! [`begin`]/[`end`] bracket a frame; writes inside accumulate and go out as
//! one `write_loop` when the outermost region closes. Outside a region
//! [`write`] passes through immediately, so no path that never opened a
//! region can strand output in the buffer. [`flush`] empties the buffer
//! without closing the region — the equivalent of C's `fflush(shout)`, to be
//! called before anything that blocks on the user (a key read).

use crate::ported::init::SHTTY;
use crate::ported::utils::write_loop;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

/// Bytes accumulated by the currently open region.
static BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());
/// Nesting depth of open [`begin`] regions; 0 = write through.
static DEPTH: AtomicI32 = AtomicI32::new(0);

/// Open a buffered region. Writes accumulate until the matching [`end`].
pub fn begin() {
    DEPTH.fetch_add(1, Ordering::SeqCst);
}

/// Close a buffered region; the outermost close flushes the frame.
pub fn end() {
    if DEPTH.fetch_sub(1, Ordering::SeqCst) <= 1 {
        DEPTH.store(0, Ordering::SeqCst);
        flush();
    }
}

/// Write out what the region has accumulated, leaving it open
/// (C: `fflush(shout)`).
pub fn flush() {
    let pending = {
        let mut buf = match BUF.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };
        if buf.is_empty() {
            return;
        }
        std::mem::take(&mut *buf)
    };
    let fd = SHTTY.load(Ordering::Relaxed);
    let _ = write_loop(if fd >= 0 { fd } else { 1 }, &pending);
}

/// Write to the terminal through `shout`: buffered inside a region,
/// immediate outside one.
pub fn write(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if DEPTH.load(Ordering::SeqCst) > 0 {
        if let Ok(mut buf) = BUF.lock() {
            buf.extend_from_slice(bytes);
            return;
        }
    }
    let fd = SHTTY.load(Ordering::Relaxed);
    let _ = write_loop(if fd >= 0 { fd } else { 1 }, bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A region defers its writes and emits them in order on close; the
    /// depth counter must survive nesting so an inner `end` does not flush
    /// a frame the outer region is still building.
    #[test]
    fn nested_region_defers_until_outermost_end() {
        let _g = crate::test_util::global_state_lock();
        let mut fds: [libc::c_int; 2] = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe(2) ok");
        let saved = SHTTY.load(Ordering::Relaxed);
        SHTTY.store(fds[1], Ordering::Relaxed);

        begin();
        write(b"outer-");
        begin();
        write(b"inner");
        end(); // inner close: still buffered
        let mut probe = [0u8; 16];
        let ready = unsafe {
            let mut set: libc::fd_set = std::mem::zeroed();
            libc::FD_SET(fds[0], &mut set);
            let mut tv = libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            };
            libc::select(
                fds[0] + 1,
                &mut set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            )
        };
        assert_eq!(ready, 0, "inner end must not flush the outer frame");
        end(); // outer close: one write, in order

        let n = unsafe {
            libc::read(
                fds[0],
                probe.as_mut_ptr() as *mut libc::c_void,
                probe.len(),
            )
        };
        SHTTY.store(saved, Ordering::Relaxed);
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        assert!(n > 0, "frame reached the fd");
        assert_eq!(&probe[..n as usize], b"outer-inner");
    }

    /// Outside a region every write goes straight to the fd, so code paths
    /// that never call `begin` keep working unchanged.
    #[test]
    fn write_outside_region_is_immediate() {
        let _g = crate::test_util::global_state_lock();
        let mut fds: [libc::c_int; 2] = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe(2) ok");
        let saved = SHTTY.load(Ordering::Relaxed);
        SHTTY.store(fds[1], Ordering::Relaxed);

        write(b"direct");

        let mut probe = [0u8; 16];
        let n = unsafe {
            libc::read(
                fds[0],
                probe.as_mut_ptr() as *mut libc::c_void,
                probe.len(),
            )
        };
        SHTTY.store(saved, Ordering::Relaxed);
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        assert_eq!(&probe[..n.max(0) as usize], b"direct");
    }
}
