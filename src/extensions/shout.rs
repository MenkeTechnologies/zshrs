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

thread_local! {
    /// Byte sink for [`tputs`]'s `int (*putc)(int)` callback. `tputs(3)`
    /// takes a plain fn pointer with no user data, so the collector has to
    /// be reachable from a free function; it is thread-local because the
    /// callback always runs on the calling thread, inside the `tputs` call.
    static TPUTS_SINK: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `tputs(3)`'s per-byte output callback — the shape of `putshout`
/// (`Src/utils.c:434`), collecting into [`TPUTS_SINK`] instead of writing
/// straight to the stream so the caller can route the expanded bytes
/// through [`write`] (the buffered frame) or a raw fd.
extern "C" fn tputs_collect(c: libc::c_int) -> libc::c_int {
    TPUTS_SINK.with(|s| s.borrow_mut().push(c as u8));
    0
}

/// Expand a termcap/terminfo capability string for output — the
/// `tputs(str, 1, putshout)` that C's display code wraps around EVERY
/// capability emission (`Src/prompt.c:1088,1092,1099` `tsetcap`,
/// `Src/Zle/zle_refresh.c:2345` `tcout`, `:2417` `tcoutarg`,
/// `Src/Zle/zle_keymap.c:1272`, `Src/Zle/complist.c` `tputs(buf, 1,
/// putshout)`).
///
/// Rust-only infrastructure, not a port: `tputs(3)` is the termcap
/// library routine zsh links against, and this is the binding zshrs needs
/// because it collects into a buffer rather than a `FILE *`. It is not a
/// reimplementation — the real ncurses `tputs` does the work (ncurses is
/// already linked; `src/ported/init.rs:699` externs `tgetent`/`tgetstr`
/// from it).
///
/// What it fixes: capability strings carry `$<n>` / `$<n*>` / `$<n/>`
/// delay specifications. `tputs` strips those and emits the matching pad
/// characters for the tty's baud rate. zshrs wrote `tcstr[cap]` to the
/// terminal verbatim, so under any TERM whose entry has padding — vt100,
/// which is what `Test/comptest`'s `comptesteval` sets for every ZLE
/// test — the literal text `$<2>` appeared on screen where zsh emitted
/// two NUL pad bytes:
///
/// ```text
/// zsh   : \x1b[1m\x00\x00\x1b[7m\x00\x00%\x1b[m\x00\x00
/// zshrs : \x1b[1m$<2>\x1b[7m$<2>%\x1b[m$<2>
/// ```
///
/// A capability with no `$<` in it is returned unchanged without calling
/// into ncurses, which also keeps the path safe when `init_term` bailed
/// before `tgetent` (TERM unset / TERM_BAD) and `cur_term` is NULL.
pub fn tputs(s: &str) -> Vec<u8> {
    if !s.contains("$<") {
        return s.as_bytes().to_vec();
    }
    // `affcnt` = 1 — the same constant every zsh call site passes.
    // `crate::tparm::tputs` replaced an `extern "C"` call into ncurses; the
    // pad-rate inputs it needs come from the loaded terminfo entry and the
    // tty's own output speed instead of from `cur_term` and `ospeed`.
    crate::tparm::tputs(s.as_bytes(), 1, &pad_info())
}

/// Gather the terminal properties `crate::tparm::tputs` needs. Reads `ospeed`
/// straight off the tty and `xon` / `pb` / `npc` / `pad` from the entry
/// `init_term` loaded. When stdout is not a tty the speed is 0, which is
/// ncurses' own "emit no pad bytes" case.
fn pad_info() -> crate::tparm::PadInfo {
    use crate::terminfo_db;
    let baud = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDOUT_FILENO, &mut t) == 0 {
            libc::cfgetospeed(&t) as i32
        } else {
            0
        }
    };
    crate::tparm::PadInfo {
        baud,
        xon: terminfo_db::tigetflag("xon") == 1,
        padding_baud_rate: terminfo_db::tigetnum("pb").max(0),
        no_pad_char: terminfo_db::tigetflag("npc") == 1,
        pad_char: terminfo_db::tigetstr("pad")
            .ok()
            .flatten()
            .and_then(|v| v.first().copied())
            .unwrap_or(0),
    }
}

/// [`tputs`] + [`write`] — the exact `tputs(cap, 1, putshout)` pair.
pub fn tputs_write(s: &str) {
    write(&tputs(s));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capability with no delay spec must come back byte-identical and
    /// must not enter ncurses (the fast path also covers `cur_term ==
    /// NULL`, i.e. a shell that never reached `tgetent`).
    #[test]
    fn tputs_passes_through_unpadded_capability() {
        assert_eq!(tputs("\x1b[1m"), b"\x1b[1m".to_vec());
        assert_eq!(tputs(""), Vec::<u8>::new());
    }

    /// vt100's `md` is `\e[1m$<2>`: `tputs` must strip the `$<2>` delay
    /// spec. zshrs used to write it to the terminal verbatim, so the
    /// literal characters `$<2>` appeared in every ZLE frame under a
    /// padded TERM.
    #[test]
    fn tputs_strips_delay_specification() {
        let out = tputs("\x1b[1m$<2>");
        assert!(
            out.starts_with(b"\x1b[1m"),
            "capability text must survive: {:?}",
            out
        );
        assert!(
            !out.windows(2).any(|w| w == b"$<"),
            "delay spec must not reach the terminal: {:?}",
            out
        );
    }

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

        let n = unsafe { libc::read(fds[0], probe.as_mut_ptr() as *mut libc::c_void, probe.len()) };
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
        let n = unsafe { libc::read(fds[0], probe.as_mut_ptr() as *mut libc::c_void, probe.len()) };
        SHTTY.store(saved, Ordering::Relaxed);
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        assert_eq!(&probe[..n.max(0) as usize], b"direct");
    }
}
