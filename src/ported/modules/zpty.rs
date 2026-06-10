//! Pseudo-terminal module - port of Modules/zpty.c
//!
//! Provides zpty builtin for running sub-processes with pseudo terminals.

use crate::ported::zsh_h::{features, module, OPT_ARG, OPT_ISSET};
use std::collections::HashMap;
use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::unix::io::{IntoRawFd, RawFd};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// Port of `READ_MAX` from `Src/Modules/zpty.c:44`. Maximum bytes
/// to read at once from a pty's master end (1 MB).
pub const READ_MAX: usize = 1024 * 1024; // c:44

/// A pseudo-terminal command session.
/// Port of `struct ptycmd` from Src/Modules/zpty.c — the C
/// source threads it through `getptycmd()` (line 153),
/// `newptycmd()` (line 310), `deleteptycmd()` (line 490) etc.
/// Same fields (name, args, master fd, pid, echo, nonblock).
#[derive(Debug)]
pub struct ptycmd {
    /// `name` field (C: `char *name`).
    pub name: String,
    /// `args` field (C: `char **args`).
    pub args: Vec<String>,
    /// `master_fd` field (C: `int fd` — master end of the pty).
    pub master_fd: RawFd,
    /// `pid` field (C: `pid_t pid` — spawned child PID).
    pub pid: i32,
    /// `echo` field (C: `int echo`).
    pub echo: bool,
    /// `nonblock` field (C: `int nblock`).
    pub nonblock: bool,
    /// `finished` field (C: `int fin`).
    pub finished: bool,
    /// `read_buf` field — C: `int read;` initialised to -1; the
    /// single byte stashed by `checkptycmd` (c:543-544) for the next
    /// `ptyread` to consume at c:583-588. `None` ⇔ C's -1 sentinel.
    pub read_buf: Option<u8>,
    /// `old` field — C: `char *old; int olen;` populated when a
    /// pattern-bearing `ptyread` exits early with EWOULDBLOCK/EAGAIN
    /// (c:682-694) so the partial-match prefix carries to the next
    /// invocation (c:572-578).
    pub old: Option<Vec<u8>>,
}

impl ptycmd {
    /// `new` — see implementation.
    pub fn new(
        name: &str,
        args: Vec<String>,
        master_fd: RawFd,
        pid: i32,
        echo: bool,
        nonblock: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            args,
            master_fd,
            pid,
            echo,
            nonblock,
            finished: false,
            // c:455-458 — `cmd->read = -1; cmd->old = NULL; cmd->olen = 0;`
            read_buf: None,
            old: None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// WARNING: NOT IN ZPTY.C — OnceLock<Mutex> accessor for ptycmd registry; C uses static linked list `ptycmds`
/// (equivalent C logic at Src/Modules/zpty.c:48).
fn ptycmds() -> &'static Mutex<HashMap<String, ptycmd>> {
    PTYCMDS.get_or_init(|| Mutex::new(HashMap::<String, ptycmd>::new()))
}

/// Set non-blocking mode on a file descriptor.
/// Port of `ptynonblock(int fd)` from Src/Modules/zpty.c:65 — wraps
/// `fcntl(F_GETFL)` + `fcntl(F_SETFL, |O_NONBLOCK)`.
#[cfg(unix)]
pub fn ptynonblock(fd: RawFd) -> io::Result<()> {
    // c:65
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }

        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Port of `ptygettyinfo(int fd, struct ttyinfo *ti)` from `Src/Modules/zpty.c:97`. Calls
/// `tcgetattr(fd, &ti->tio)` to capture the pty's termios state.
/// Returns 0 on success, 1 on failure or when fd == -1.
///
/// C signature: `static int ptygettyinfo(int fd, struct ttyinfo *ti)`.
/// Rust port takes a mutable `&mut libc::termios` directly since
/// `struct ttyinfo` (zsh.h) wraps termios + a few legacy fields.
pub fn ptygettyinfo(fd: i32, ti: &mut libc::termios) -> i32 {
    // c:97
    if fd == -1 {
        // c:97 inverted
        return 1; // c:118
    }
    // c:101 — `tcgetattr(fd, &ti->tio)`.
    let r = unsafe { libc::tcgetattr(fd, ti as *mut libc::termios) };
    if r == -1 {
        // c:103
        return 1; // c:103
    }
    0 // c:124
}

/// Port of `ptysettyinfo(int fd, struct ttyinfo *ti)` from `Src/Modules/zpty.c:124`. Calls
/// `tcsetattr(fd, TCSADRAIN, &ti->tio)` to install the captured
/// termios state on the pty.
///
/// C signature: `static void ptysettyinfo(int fd, struct ttyinfo *ti)`.
pub fn ptysettyinfo(fd: i32, ti: &libc::termios) {
    // c:124
    if fd == -1 {
        // c:124 inverted
        return;
    }
    // c:128-132 — `tcsetattr(fd, TCSADRAIN, &ti->tio);`
    unsafe {
        libc::tcsetattr(fd, libc::TCSADRAIN, ti as *const libc::termios);
    }
}

// === auto-generated stubs ===
/// Port of `getptycmd(char *name)` from `Src/Modules/zpty.c:153`. Linear
/// scan over the `ptycmds` linked list looking for one matching
/// `name`.
///
/// C signature: `static Ptycmd getptycmd(char *name)`. Returns
/// the Ptycmd or NULL.
/// WARNING: param names don't match C — Rust=(cmds, name) vs C=(name)
pub fn getptycmd<'a>(cmds: &'a HashMap<String, ptycmd>, name: &str) -> Option<&'a ptycmd> {
    // c:153
    cmds.get(name) // c:153-160 strcmp loop
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ─── moved from src/ported/vm_helper (drift extraction) ───

// Note: dead `ZptyState` aggregate deleted per PORT_PLAN Phase 2.
// It was a duplicate of `ptycmd` (zpty.rs:19), which is the correct
// faithful port of C `struct ptycmd` (Src/Modules/zpty.c:48). The
// dead `ZptyState` was wired into ShellExecutor as
// `pub zptys: HashMap<String, ZptyState>` but never inserted or
// read. Use `ptycmd` + `HashMap<String, ptycmd>` (the port of the file-static
// `static Ptycmd ptycmds;` linked list at zpty.c:62) for any
// real wiring.

// =====================================================================
// static struct features module_features                            c:884 (zpty.c)
// =====================================================================

/// Open a pseudo-terminal master/slave pair.
/// Port of `get_pty(int master, int *retfd)` from Src/Modules/zpty.c:191 (or :255 for
/// the fallback path on systems without `posix_openpt`). Wraps
/// `posix_openpt` + `grantpt` + `unlockpt` + `ptsname` + `open`.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=() vs C=(master, retfd)
pub fn get_pty() -> io::Result<(RawFd, RawFd)> {
    // c:191
    let master_fd = unsafe {
        let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        fd
    };

    unsafe {
        if libc::grantpt(master_fd) < 0 {
            libc::close(master_fd);
            return Err(io::Error::last_os_error());
        }

        if libc::unlockpt(master_fd) < 0 {
            libc::close(master_fd);
            return Err(io::Error::last_os_error());
        }

        let slave_name = libc::ptsname(master_fd);
        if slave_name.is_null() {
            libc::close(master_fd);
            return Err(io::Error::other("ptsname failed"));
        }

        let slave_fd = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
        if slave_fd < 0 {
            libc::close(master_fd);
            return Err(io::Error::last_os_error());
        }

        Ok((master_fd, slave_fd))
    }
}

/// Port of `newptycmd(char *nam, char *pname, char **args, int echo, int nblock)` from `Src/Modules/zpty.c:310`. Forks a
/// new pty session, exec'ing `args` in the child. Allocates a
/// fresh `Ptycmd` record, configures the master fd, sets up the
/// echo / nonblock flags, and links it into `ptycmds`.
///
/// C signature: `static int newptycmd(char *nam, char *pname,
///                                     char **args, int echo, int nblock)`.
///
/// **Approximation:** the full pty-allocation path uses
/// `posix_openpt`/`grantpt`/`unlockpt`/`ptsname` (zpty.c:191-309)
/// and forks via `zfork()` with an extensive child-side reset
/// sequence. zshrs's port wires through `std::process::Command`
/// + a libc pty-spawn helper which doesn't preserve the full C
/// child-init contract. Returns 0 on success, 1 on failure.
pub fn newptycmd(
    cmds: &mut HashMap<String, ptycmd>,
    _nam: &str,
    pname: &str, // c:310
    args: &[String],
    echo: bool,
    nblock: bool,
) -> i32 {
    // c:310
    if args.is_empty() {
        return 1;
    }

    // c:438 — `get_pty(1, &master, &slave)` — allocate master/slave fds.
    // Approximation: full C path runs `parse_string(zjoin(args, ' ', 1))`
    // then `execode(prog, 1, 0, "zpty")` in the child, so the args are
    // interpreted as a shell command line (globs, redirs, builtins).
    // This port still uses execvp(3), matching the prior inline shape
    // — true execode integration is a separate iteration.
    #[cfg(unix)]
    {
        let (master, slave) = match get_pty() {
            Ok(p) => p,
            Err(_) => return 1, // c:438-440
        };
        let pid = unsafe { libc::fork() };
        match pid {
            -1 => {
                // c:441-446 — `if (pid == -1) { close(...); return 1; }`
                unsafe {
                    libc::close(master);
                    libc::close(slave);
                }
                1
            }
            0 => {
                // c:386-435 — child-side reset (setsid, dup2, etc.).
                unsafe {
                    libc::close(master);
                    libc::setsid();
                    libc::dup2(slave, 0);
                    libc::dup2(slave, 1);
                    libc::dup2(slave, 2);
                    if slave > 2 {
                        libc::close(slave);
                    }
                }
                if !echo {
                    // c:124 — `ptysettyinfo` — disable echo on slave-side termios.
                    unsafe {
                        let mut termios: libc::termios = std::mem::zeroed();
                        if libc::tcgetattr(0, &mut termios) >= 0 {
                            termios.c_lflag &= !libc::ECHO;
                            let _ = libc::tcsetattr(0, libc::TCSADRAIN, &termios);
                        }
                    }
                }
                let cmd = match CString::new(args[0].as_str()) {
                    Ok(c) => c,
                    Err(_) => unsafe { libc::_exit(1) },
                };
                let c_args: Vec<CString> = args
                    .iter()
                    .filter_map(|s| CString::new(s.as_str()).ok())
                    .collect();
                let c_args_ptrs: Vec<*const libc::c_char> = c_args
                    .iter()
                    .map(|s| s.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();
                unsafe {
                    libc::execvp(cmd.as_ptr(), c_args_ptrs.as_ptr());
                    libc::_exit(1); // c:436 — `zexit(lastval, ZEXIT_NORMAL)` on failure.
                }
            }
            pid => {
                unsafe {
                    libc::close(slave); // c:451
                }
                // c:466-467 — `if (nblock) ptynonblock(master);`
                if nblock {
                    let _ = ptynonblock(master);
                }
                // c:455-458 — `p = (Ptycmd) zalloc(...); p->name = ztrdup(pname);
                //              p->args = args; p->fd = master; p->pid = pid;
                //              p->echo = echo; p->nblock = nblock;`
                let new = ptycmd::new(pname, args.to_vec(), master, pid, echo, nblock);
                cmds.insert(new.name.clone(), new); // c:474 list-insert
                0
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (echo, nblock, pname, args, cmds);
        1
    }
}

/// Port of `deleteptycmd(Ptycmd cmd)` from `Src/Modules/zpty.c:490`. Removes
/// `cmd` from the `ptycmds` linked list, frees its name + args,
/// closes the master fd, and kills the process group via
/// `kill(-pid, SIGHUP)`.
///
/// C signature: `static void deleteptycmd(Ptycmd cmd)`.
/// WARNING: param names don't match C — Rust=(cmds, name) vs C=(cmd)
pub fn deleteptycmd(cmds: &mut HashMap<String, ptycmd>, name: &str) {
    // c:490
    if let Some(cmd) = cmds.remove(name) {
        // c:490-503 list-unlink
        // c:505 — `zsfree(p->name)` + c:506 `freearray(p->args)` —
        // Rust drops String/Vec automatically on `cmd` going out
        // of scope.
        // c:508 — `zclose(cmd->fd)`.
        unsafe {
            libc::close(cmd.master_fd);
        }
        // c:517 — `kill(-(p->pid), SIGHUP);` — kill the process group.
        unsafe {
            libc::kill(-cmd.pid, libc::SIGHUP);
        }
        // c:517 — `zfree(p, sizeof(*p))` — Rust drop handles.
    }
}

/// Port of `deleteallptycmds()` from `Src/Modules/zpty.c:517`.
/// Walks the `ptycmds` list and deletes every entry.
///
/// C signature: `static void deleteallptycmds(void)`.
/// WARNING: param names don't match C — Rust=(cmds) vs C=()
pub fn deleteallptycmds(cmds: &mut HashMap<String, ptycmd>) {
    // c:517
    let names: Vec<String> = cmds.keys().cloned().collect();
    for n in names {
        // c:530-525
        deleteptycmd(cmds, &n); // c:530
    }
}

/// Port of `checkptycmd(Ptycmd cmd)` from `Src/Modules/zpty.c:530`. Polls
/// the master fd with a 1-byte non-blocking read; if read fails
/// AND `kill(pid, 0)` confirms the process is gone, marks the
/// command as finished and closes the fd.
///
/// C signature: `static void checkptycmd(Ptycmd cmd)`.
pub fn checkptycmd(cmd: &mut ptycmd) {
    // c:530
    if cmd.finished {
        // c:530 cmd->fin
        return;
    }
    let mut c: u8 = 0;
    let r = unsafe { libc::read(cmd.master_fd, &mut c as *mut u8 as *mut _, 1) };
    if r <= 0 {
        // c:538
        // c:539 — `if (kill(cmd->pid, 0) < 0)` — process gone.
        if unsafe { libc::kill(cmd.pid, 0) } < 0 {
            cmd.finished = true; // c:540 cmd->fin = 1
            unsafe {
                libc::close(cmd.master_fd);
            } // c:541 zclose
        }
        return;
    }
    // c:543-544 — `cmd->read = (int) c;` — stash the probed byte
    // for the next `ptyread` call to consume at c:583-588.
    cmd.read_buf = Some(c);
}

/// Port of `ptyread(char *nam, Ptycmd cmd, char **args, int noblock, int mustmatch)`
/// from Src/Modules/zpty.c:548-710. Reads 1 byte at a time from the
/// pty master, metafies imeta bytes (c:642-646), pauses on each
/// iteration to honor `errflag/breaks/retflag/contflag` (c:678),
/// flushes to stdout in chunks when no setparam target is given
/// (c:649-653), and bails as soon as `pattern` matches via
/// `pattry` (c:680). Final disposition: setsparam(args[0], buf)
/// if args[0] is set; otherwise write the accumulated buffer to
/// stdout (c:696-701). Returns `0` on success-with-data,
/// `cmd->fin + 1` (1/2) otherwise (c:704).
///
/// Deferred (separate port iteration): the `cmd->old` / `cmd->read`
/// pre-buffer machinery at c:572-588, c:683-694 — these fields
/// don't exist on the Rust `ptycmd` struct yet; without them the
/// nblock-EWOULDBLOCK-stash path at c:682-694 falls through to
/// the standard exit (still correct return value, just slower
/// next-call because the unmatched prefix isn't carried).
pub fn ptyread(
    nam: &str,
    cmd: &mut ptycmd,
    args: &[&str],
    noblock: bool,
    mustmatch: bool,
) -> i32 {
    // c:550
    let mut used: usize = 0;
    let mut seen: bool = false;
    let mut matchok: bool = false;
    let mut ret: isize = 0;
    let mut prog: Option<crate::ported::pattern::Patprog> = None; // c:552

    // c:554-568 — pattern compile from args[1] if present.
    if !args.is_empty() && args.len() >= 2 {
        if args.len() > 2 {
            // c:557-559
            eprintln!("{}: too many arguments", nam);
            return 1;
        }
        let p = args[1];
        // c:565 — `patcompile(p, PAT_ZDUP, NULL)` — PAT_ZDUP = 0x100 per zsh.h.
        match crate::ported::pattern::patcompile(p, 0x100, None) {
            Some(pp) => prog = Some(pp),
            None => {
                // c:566
                eprintln!("{}: bad pattern: {}", nam, p);
                return 1;
            }
        }
    } else {
        // c:569-570 — `fflush(stdout)` before any unbuffered emit.
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    // c:572-582 — `if (cmd->old) { used=olen; buf=zhalloc(256+used+1); memcpy(buf, cmd->old, olen); ... } else { buf=zhalloc(256+1); }`
    let mut buf: Vec<u8> = if let Some(old) = cmd.old.take() {
        used = old.len();
        let mut v = Vec::with_capacity(256 + used + 1);
        v.extend_from_slice(&old);
        v
    } else {
        Vec::with_capacity(257)
    };
    // c:583-588 — `if (cmd->read != -1) { buf[used] = cmd->read; seen = used = 1; cmd->read = -1; }`
    if let Some(c) = cmd.read_buf.take() {
        buf.push(c);
        seen = true;
        used = 1;
    }

    // c:589 — `do { ... } while ();` loop.
    use std::sync::atomic::Ordering::Relaxed;
    let setparam_target = args.first().copied(); // None ⇔ C's `!*args` (stdout).

    loop {
        // c:590 — `if (noblock && cmd->read == -1)` — poll only when
        // we don't have a stashed read byte to consume; if read_buf
        // is set, fall through to the read-or-stash path below.
        if noblock && cmd.read_buf.is_none() {
            let mut pfd = libc::pollfd {
                fd: cmd.master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let pollret = unsafe { libc::poll(&mut pfd, 1, 0) };
            // c:626 — `if (pollret == 0) break;`
            if pollret == 0 {
                break;
            }
            // c:612-625 — when pollret < 0, C tries setblock_fd + 1-byte
            // read and stashes the byte to cmd->read. Skip: setblock_fd
            // isn't ported (mode-save/restore on fd 0); next iteration's
            // checkptycmd does the equivalent stash anyway via c:543-544.
        }
        // c:629-633 — refresh fin-state on every loop iteration when
        // we haven't read anything yet this round.
        if ret == 0 {
            checkptycmd(cmd);
            if cmd.finished {
                break; // c:632
            }
        }
        // c:634 — `if (cmd->read != -1 || (ret = read(cmd->fd, buf+used, 1)) == 1)`:
        // prefer the byte that checkptycmd's mid-loop pass at c:629-630
        // just stashed in `cmd.read_buf`; only fall through to a fresh
        // 1-byte read when none was stashed.
        let mut byte: u8 = 0;
        if let Some(c) = cmd.read_buf.take() {
            byte = c;
            ret = 1; // c:637
        } else {
            ret = unsafe { libc::read(cmd.master_fd, &mut byte as *mut u8 as *mut _, 1) } as isize;
        }
        if ret == 1 {
            // c:642-646 — `if (imeta(c)) { buf[used++] = Meta; buf[used++] = c ^ 32; } else buf[used++] = c;`
            if crate::ported::ztype_h::imeta(byte) {
                buf.push(crate::ported::zsh_h::Meta);
                buf.push(byte ^ 32);
                used += 2;
            } else {
                buf.push(byte);
                used += 1;
            }
            seen = true; // c:647
            // c:648-658 — when buf grows and no setparam target, flush
            // to stdout and reset; else grow the buffer.
            if used >= 255 && setparam_target.is_none() {
                let mut flush = std::mem::take(&mut buf);
                let len = crate::ported::utils::unmetafy(&mut flush);
                flush.truncate(len);
                use std::io::Write;
                let _ = std::io::stdout().write_all(&flush);
                used = 0;
            }
        }
        // c:662-677 — without a pattern: break on EOF or trailing-newline
        // when args[0] is set; with a pattern: break only on hard read error.
        if prog.is_none() {
            if ret <= 0 {
                break; // c:663 — EOF or hard error
            }
            // c:663-664 — `*args && buf[used-1] == '\n' && (used < 2 || buf[used-2] != Meta)`
            if setparam_target.is_some()
                && used > 0
                && buf[used - 1] == b'\n'
                && (used < 2 || buf[used - 2] != crate::ported::zsh_h::Meta)
            {
                break;
            }
        } else if ret < 0 {
            // c:667-676 — with pattern: break on read error UNLESS EWOULDBLOCK/EAGAIN.
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno != libc::EWOULDBLOCK && eno != libc::EAGAIN {
                break;
            }
        }
        // c:678 — `while (!(errflag || breaks || retflag || contflag) && used < READ_MAX && ...)`
        if crate::ported::utils::errflag.load(Relaxed) != 0
            || crate::ported::builtin::BREAKS.load(Relaxed) != 0
            || crate::ported::exec::retflag.load(Relaxed) != 0
            || crate::ported::builtin::CONTFLAG.load(Relaxed) != 0
        {
            break;
        }
        if used >= READ_MAX {
            break;
        }
        // c:680 — `prog && ret && (matchok = pattry(prog, buf))` early-exit.
        if let Some(ref pp) = prog {
            if ret > 0 {
                let s = String::from_utf8_lossy(&buf);
                if crate::ported::pattern::pattry(pp, &s) {
                    matchok = true;
                    break;
                }
            }
        }
    }

    // c:682-694 — pattern-pending + EWOULDBLOCK/EAGAIN partial-match
    // stash: copy `buf` into `cmd->old` for the next ptyread call to
    // pick up at c:572-578, then early-return 1 (alive, no match yet).
    if prog.is_some() && ret < 0 {
        let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if eno == libc::EWOULDBLOCK || eno == libc::EAGAIN {
            // c:691-692 — `cmd->old = zalloc(olen=used); memcpy(...)`.
            cmd.old = Some(buf);
            // c:694
            return 1;
        }
    }

    // c:696-701 — output disposition: setsparam(target, buf) or
    // write the unwritten tail to stdout.
    if let Some(target) = setparam_target {
        let s = String::from_utf8_lossy(&buf).to_string();
        let _ = crate::ported::params::setsparam(target, &s);
    } else if used > 0 {
        let mut flush = buf;
        let len = crate::ported::utils::unmetafy(&mut flush);
        flush.truncate(len);
        use std::io::Write;
        let _ = std::io::stdout().write_all(&flush);
    }

    // c:703-709 — final return.
    //   int r = cmd->fin + 1;
    //   if (seen && (!prog || matchok || !mustmatch)) r = 0;
    //   return r;
    let mut r: i32 = if cmd.finished { 2 } else { 1 };
    if seen && (prog.is_none() || matchok || !mustmatch) {
        r = 0;
    }
    r
}

/// Write a string to a pty's master end.
/// Port of `ptywritestr(Ptycmd cmd, char *s, int len)` from Src/Modules/zpty.c:713-740.
/// Walks the buffer with a control-flow-aware retry loop that bails
/// on `errflag`, `breaks`, `retflag`, `contflag`; in `cmd->nblock`
/// mode an `EWOULDBLOCK`/`EAGAIN` short-write returns `!all` so
/// the caller can stash the unwritten tail (mirrors C c:720-729).
/// On a non-nonblock write error, `checkptycmd` updates `cmd->fin`
/// (the pty died); if still alive, the byte count is set to 0 and
/// the loop continues (c:730-735). Returns `0` if any bytes
/// landed; otherwise `cmd->fin + 1` (c:739) — `1` for "child
/// alive, nothing written", `2` for "child dead".
pub fn ptywritestr(cmd: &mut ptycmd, s: &[u8]) -> i32 {
    // c:716
    use std::sync::atomic::Ordering::Relaxed;
    let mut all: usize = 0;
    let mut off: usize = 0;
    let mut len: usize = s.len();
    // c:718 — `for (; !errflag && !breaks && !retflag && !contflag && len; ...)`
    while crate::ported::utils::errflag.load(Relaxed) == 0
        && crate::ported::builtin::BREAKS.load(Relaxed) == 0
        && crate::ported::exec::retflag.load(Relaxed) == 0
        && crate::ported::builtin::CONTFLAG.load(Relaxed) == 0
        && len > 0
    {
        // c:720 — `written = write(cmd->fd, s, len)`.
        let written = unsafe {
            libc::write(
                cmd.master_fd,
                s.as_ptr().add(off) as *const libc::c_void,
                len,
            )
        };
        if written < 0 && cmd.nonblock {
            // c:720-729 — nblock + (EWOULDBLOCK || EAGAIN) → return `!all`.
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            #[allow(unused_mut)]
            let mut wouldblock = false;
            #[cfg(target_os = "linux")]
            {
                wouldblock = eno == libc::EWOULDBLOCK || eno == libc::EAGAIN;
            }
            #[cfg(not(target_os = "linux"))]
            {
                wouldblock = eno == libc::EWOULDBLOCK || eno == libc::EAGAIN;
            }
            if wouldblock {
                return if all == 0 { 1 } else { 0 }; // c:729 — `return !all`
            }
        }
        let written = if written < 0 {
            // c:730-735 — `checkptycmd(cmd); if (cmd->fin) break; written = 0;`
            checkptycmd(cmd);
            if cmd.finished {
                break;
            }
            0
        } else {
            written as usize
        };
        if written > 0 {
            // c:736-737 — `all += written;`
            all += written;
        }
        // c:719 — `len -= written, s += written`
        len = len.saturating_sub(written);
        off += written;
    }
    // c:739 — `return (all ? 0 : cmd->fin + 1);`
    if all > 0 {
        0
    } else if cmd.finished {
        2
    } else {
        1
    }
}

/// Port of `ptywrite(Ptycmd cmd, char **args, int nonl)` from
/// Src/Modules/zpty.c:742-769. Routes every write through
/// `ptywritestr` so partial-write retry + control-flow bail
/// (errflag/breaks/retflag/contflag) + nblock EWOULDBLOCK
/// stash all apply uniformly. Prior implementation used raw
/// `libc::write` four times inline and threw away all of those
/// guarantees.
///
/// C signature: `static int ptywrite(Ptycmd cmd, char **args, int nonl)`.
pub fn ptywrite(cmd: &mut ptycmd, args: &[&str], nonl: i32) -> i32 {
    // c:744
    if !args.is_empty() {
        // c:745
        // c:746 — `char sp = ' ', *tmp;` + `int len;`
        let sp = b' '; // c:746
        // c:749 — `while (*args)` — iterate argv with peek-ahead for
        // the inter-arg space write at c:751.
        for (i, a) in args.iter().enumerate() {
            // c:750 — `unmetafy((tmp = dupstring(*args)), &len);`
            let tmp = crate::ported::utils::unmeta(a);
            let bytes = tmp.as_bytes();
            // c:751-752 — `if (ptywritestr(cmd, tmp, len) ||
            //                  (*++args && ptywritestr(cmd, &sp, 1)))
            //                  return 1;`
            if ptywritestr(cmd, bytes) != 0 {
                return 1;
            }
            if i + 1 < args.len() && ptywritestr(cmd, &[sp]) != 0 {
                return 1;
            }
        }
        // c:755 — `if (!nonl) { sp = '\n'; if (ptywritestr(cmd, &sp, 1)) return 1; }`
        if nonl == 0 {
            let nl = b'\n'; // c:756
            if ptywritestr(cmd, &[nl]) != 0 {
                return 1; // c:758
            }
        }
    } else {
        // c:760-767 — `while ((n = read(0, buf, BUFSIZ)) > 0) if (ptywritestr(...)) return 1;`
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::read(0, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break; // c:764
            }
            // c:765 — `if (ptywritestr(cmd, buf, n)) return 1;`
            if ptywritestr(cmd, &buf[..n as usize]) != 0 {
                return 1;
            }
        }
    }
    // c:768 — `return 0;`
    0
}

/// Port of `bin_zpty(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/zpty.c:773`.
/// `zpty` builtin entry point — C-faithful signature matching
/// `static int bin_zpty(char *nam, char **args, Options ops, int func)`
/// from Src/Modules/zpty.c:773. Reads `-d/-L/-w/-r/-t/-b/-e/-T/-m`
/// flags via OPT_ISSET/OPT_ARG, dispatches by mode, emits output to
/// stdout/stderr based on status, returns the i32 status.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zpty(
    _nam: &str,
    args: &[String], // c:773
    ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // Per C: branches dispatch on OPT_ISSET(ops, 'X') directly. No
    // aggregator struct — Rule D forbids `*Options` bags.
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let args = &argv[..];
    let mut cmds_guard = ptycmds().lock().unwrap_or_else(|e| e.into_inner());
    let cmds: &mut HashMap<String, ptycmd> = &mut *cmds_guard;
    let (status, output): (i32, String) = (|| {
        let mut output = String::new();

        if OPT_ISSET(ops, b'd') {
            // c:809-824 — `-d` arm:
            //   Ptycmd p; int ret = 0;
            //   if (*args) {
            //       while (*args)
            //           if ((p = getptycmd(*args++))) deleteptycmd(p);
            //           else { zwarnnam(nam, "no such pty command: %s", args[-1]); ret = 1; }
            //   } else deleteallptycmds();
            //   return ret;
            let mut ret: i32 = 0; // c:811
            if !args.is_empty() {
                // c:813 — iterate; missing entries log + ret=1 but loop continues.
                for name in args {
                    if cmds.contains_key(*name) {
                        // c:815 — `deleteptycmd(p)` (SIGHUP to pgrp, not SIGTERM to leader).
                        deleteptycmd(cmds, name);
                    } else {
                        // c:818 — `zwarnnam(nam, "no such pty command: %s", args[-1])`.
                        output.push_str(&format!("zpty: no such pty command: {}\n", name));
                        ret = 1; // c:819
                    }
                }
            } else {
                // c:822 — `deleteallptycmds()`.
                deleteallptycmds(cmds);
            }
            return (ret, output); // c:824
        }

        if OPT_ISSET(ops, b'w') {
            // c:795 — `if (!*args) { zwarnnam(nam, "missing pty command name"); return 1; }`
            if args.is_empty() {
                return (1, "zpty: missing pty command name\n".to_string());
            }
            let name = args[0];
            // c:798 — `else if (!(p = getptycmd(*args))) { zwarnnam(...); return 1; }`
            let cmd = match cmds.get_mut(name) {
                Some(c) => c,
                None => return (1, format!("zpty: no such pty command: {}\n", name)),
            };
            // c:802-803 — `if (p->fin) return 2;`
            if cmd.finished {
                return (2, output);
            }
            // c:808 — `ptywrite(p, args + 1, OPT_ISSET(ops,'n'))`
            let tail: Vec<&str> = args[1..].to_vec();
            let nonl = if OPT_ISSET(ops, b'n') { 1 } else { 0 };
            let r = ptywrite(cmd, &tail, nonl);
            (r, output)
        } else if OPT_ISSET(ops, b'r') {
            // c:792-808 — symmetric `-r` arm: same prelude as `-w`,
            // then dispatch:
            //   ptyread(nam, p, args+1, OPT_ISSET(ops,'t'), OPT_ISSET(ops,'m'))
            // c:795
            if args.is_empty() {
                return (1, "zpty: missing pty command name\n".to_string());
            }
            let name = args[0];
            // c:798
            let cmd = match cmds.get_mut(name) {
                Some(c) => c,
                None => return (1, format!("zpty: no such pty command: {}\n", name)),
            };
            // c:802-803 — `if (p->fin) return 2;`
            if cmd.finished {
                return (2, output);
            }
            // c:805-807
            let tail: Vec<&str> = args[1..].to_vec();
            let noblock = OPT_ISSET(ops, b't');
            let mustmatch = OPT_ISSET(ops, b'm');
            let r = ptyread(_nam, cmd, &tail, noblock, mustmatch);
            (r, output)
        } else if OPT_ISSET(ops, b't') {
            // c:825-836 — `-t` arm: "is the child still running?"
            //   if (!*args) { zwarnnam(nam, "missing pty command name"); return 1; }
            //   else if (!(p = getptycmd(*args))) { zwarnnam(...); return 1; }
            //   checkptycmd(p);
            //   return p->fin;
            // Prior port used `poll(fd, POLLIN, 0)` to test fd-readability —
            // a different question (pending output) from the C one
            // (child-process liveness). The poll-based answer returned
            // 0 when output was queued and 1 otherwise, so `zpty -t` on
            // an alive-but-idle pty incorrectly reported "finished".
            // c:828
            if args.is_empty() {
                return (1, "zpty: missing pty command name\n".to_string());
            }
            let name = args[0];
            // c:831
            let cmd = match cmds.get_mut(name) {
                Some(c) => c,
                None => return (1, format!("zpty: no such pty command: {}\n", name)),
            };
            // c:835 — `checkptycmd(p)` (1-byte non-blocking probe + kill(pid, 0)).
            checkptycmd(cmd);
            // c:836 — `return p->fin;` — 1 if finished, 0 if alive.
            // `zpty -t name` is true iff child still running, so a
            // finished pty exits 1.
            let r = if cmd.finished { 1 } else { 0 };
            (r, output)
        } else if !args.is_empty() {
            // c:837-847 — positional arm: create a new pty command.
            //   if (!args[1]) { zwarnnam(nam, "missing command"); return 1; }
            //   if (getptycmd(*args)) { zwarnnam(...); return 1; }
            //   return newptycmd(nam, *args, args+1, OPT_ISSET(ops,'e'),
            //                    OPT_ISSET(ops,'b'));
            // c:838
            if args.len() < 2 {
                return (1, "zpty: missing command\n".to_string());
            }
            let name = args[0];
            // c:842
            if cmds.contains_key(name) {
                return (
                    1,
                    format!("zpty: pty command name already used: {}\n", name),
                );
            }
            // c:846 — single canonical entry point.
            let cmd_args: Vec<String> = args[1..].iter().map(|s| s.to_string()).collect();
            let r = newptycmd(
                cmds,
                _nam,
                name,
                &cmd_args,
                OPT_ISSET(ops, b'e'),
                OPT_ISSET(ops, b'b'),
            );
            (r, output)
        } else {
            // c:848-868 — no-flag (or -L) catch-all: list every live pty,
            // refreshing fin-state via checkptycmd. -L uses the
            // "nam [-e] [-b] name args..." format suitable for re-execution;
            // the default uses "(finished) name: args..." or
            // "(pid) name: args...".
            //   for (p = ptycmds; p; p = p->next) {
            //       checkptycmd(p);
            //       if (OPT_ISSET(ops,'L')) printf("%s %s%s%s ", nam, ...);
            //       else if (p->fin) printf("(finished) %s: ", p->name);
            //       else printf("(%d) %s: ", p->pid, p->name);
            //       for (a = p->args; *a; ) {
            //           quotedzputs(*a++, stdout);
            //           if (*a) putchar(' ');
            //       }
            //       putchar('\n');
            //   }
            //   return 0;
            // c:852 — iterate over snapshot of keys so checkptycmd can
            // mutate each ptycmd (and so we don't hold a borrow on cmds
            // while mutating).
            let names: Vec<String> = cmds.keys().cloned().collect();
            for n in &names {
                let cmd = match cmds.get_mut(n) {
                    Some(c) => c,
                    None => continue,
                };
                // c:853
                checkptycmd(cmd);
                if OPT_ISSET(ops, b'L') {
                    // c:854-856 — `printf("%s %s%s%s ", nam, (echo?-e:""), (nblock?-b:""), name)`
                    output.push_str(&format!(
                        "{} {}{}{} ",
                        _nam,
                        if cmd.echo { "-e " } else { "" },
                        if cmd.nonblock { "-b " } else { "" },
                        cmd.name
                    ));
                } else if cmd.finished {
                    // c:857-858
                    output.push_str(&format!("(finished) {}: ", cmd.name));
                } else {
                    // c:859-860
                    output.push_str(&format!("({}) {}: ", cmd.pid, cmd.name));
                }
                // c:861-865 — `for (a = p->args; *a; ) { quotedzputs(*a++, stdout);
                //               if (*a) putchar(' '); }`
                let n_args = cmd.args.len();
                for (i, a) in cmd.args.iter().enumerate() {
                    output.push_str(&crate::ported::utils::quotedzputs(a));
                    if i + 1 < n_args {
                        output.push(' ');
                    }
                }
                output.push('\n'); // c:866
            }
            (0, output) // c:868
        }
    })();
    drop(cmds_guard);
    if !output.is_empty() {
        if status == 0 {
            print!("{}", output);
        } else {
            eprint!("{}", output);
        }
    }
    status
}

/// Port of `ptyhook(UNUSED(Hookdef d), UNUSED(void *dummy))` from `Src/Modules/zpty.c:874`. The cleanup
/// hook installed at `boot_()` time — runs `deleteallptycmds()`
/// when the shell is exiting (via the `before_trap` hook).
///
/// C signature: `static int ptyhook(Hookdef d, void *dummy)`.
/// WARNING: param names don't match C — Rust=(cmds) vs C=(d, dummy)
pub fn ptyhook(cmds: &mut HashMap<String, ptycmd>) -> i32 {
    // c:874
    deleteallptycmds(cmds); // c:874
    0 // c:879
}

// `bintab` — port of `static struct builtin bintab[]` (zpty.c).

// `module_features` — port of `static struct features module_features`
// from zpty.c:884.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zpty.c:896`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:896
    // C body c:898-899 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zpty.c:903`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:903
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zpty.c:911`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:911
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zpty.c:918`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:918
    // C body c:921-922 — `ptycmds = NULL; addhookfunc("exit", ptyhook)`.
    *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
    let _ = ptyhook(&mut ptycmds().lock().unwrap()); // c:928 (hook handle)
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zpty.c:928`.
pub fn cleanup_(m: *const module) -> i32 {
    // c:928
    // c:937 — `deletehookfunc("exit", ptyhook)`. We have no live hook
    //          registry, so this is a no-op.
    // c:937 — `deleteallptycmds()`.
    deleteallptycmds(&mut ptycmds().lock().unwrap());
    // c:937 — `return setfeatureenables(m, &module_features, NULL)`.
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zpty.c:937`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:937
    // C body c:939-940 — `return 0`. Faithful empty-body port; the
    //                    pty session teardown happens in cleanup_.
    0
}

/// Global `ptycmds` linked-list from `Src/Modules/zpty.c:36`.
/// C declares `static Ptycmd ptycmds;` and mutates it through the
/// whole module. Rust uses OnceLock<Mutex<>> for thread-safe access.
pub static PTYCMDS: std::sync::OnceLock<Mutex<HashMap<String, ptycmd>>> =
    std::sync::OnceLock::new();

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZPTY.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:zpty".to_string()]
}

// WARNING: NOT IN ZPTY.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN ZPTY.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// WARNING: NOT IN ZPTY.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zsh_h::{options, MAX_OPS};

    #[test]
    fn test_pty_cmds_manager() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::<String, ptycmd>::new();
        assert!(cmds.is_empty());

        let cmd = ptycmd::new("test", vec!["echo".to_string()], 5, 1234, true, false);
        cmds.insert(cmd.name.clone(), cmd);

        assert_eq!(cmds.len(), 1);
        assert!(cmds.get("test").is_some());
        assert!(cmds.get("nonexistent").is_none());

        let names: Vec<String> = cmds.keys().cloned().collect();
        assert!(names.contains(&"test".to_string()));

        cmds.remove("test");
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_pty_cmd_fields() {
        let _g = crate::test_util::global_state_lock();
        let cmd = ptycmd::new(
            "mypty",
            vec!["bash".to_string(), "-c".to_string()],
            10,
            5678,
            false,
            true,
        );

        assert_eq!(cmd.name, "mypty");
        assert_eq!(cmd.args, vec!["bash", "-c"]);
        assert_eq!(cmd.master_fd, 10);
        assert_eq!(cmd.pid, 5678);
        assert!(!cmd.echo);
        assert!(cmd.nonblock);
        assert!(!cmd.finished);
    }

    fn ops_with_flag(c: u8) -> options {
        let mut o = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        o.ind[c as usize] = 1;
        o
    }

    /// Verifies `-L` (list) on an empty pty table returns 0.
    /// Mirrors Src/Modules/zpty.c:773 -L arm.
    #[test]
    fn test_builtin_zpty_list_empty() {
        let _g = crate::test_util::global_state_lock();
        // Reset global PTYCMDS for test isolation.
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'L'), 0);
        assert_eq!(status, 0);
    }

    /// Verifies `-d` with no positional args clears all sessions.
    /// Mirrors Src/Modules/zpty.c:773 -d arm.
    #[test]
    fn test_builtin_zpty_delete_all() {
        let _g = crate::test_util::global_state_lock();
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'd'), 0);
        assert_eq!(status, 0);
    }

    /// Verifies `-w` with no positional args returns 1 (needs name + data).
    #[test]
    fn test_builtin_zpty_write_no_args() {
        let _g = crate::test_util::global_state_lock();
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'w'), 0);
        assert_eq!(status, 1);
    }

    /// Verifies `-t` with no positional args returns 1 (needs name).
    #[test]
    fn test_builtin_zpty_test_no_args() {
        let _g = crate::test_util::global_state_lock();
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b't'), 0);
        assert_eq!(status, 1);
    }

    /// c:153 — `getptycmd` returns None for an unknown name. Pin
    /// the negative case so a regen that always returns Some (with
    /// a stale entry) gets caught.
    #[test]
    fn getptycmd_unknown_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let cmds: HashMap<String, ptycmd> = HashMap::new();
        assert!(getptycmd(&cmds, "never-created").is_none());
    }

    /// c:153 — `getptycmd` returns Some for a name in the map.
    #[test]
    fn getptycmd_returns_inserted_entry() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        let cmd = ptycmd::new("foo", vec!["x".to_string()], 3, 4, true, false);
        cmds.insert("foo".to_string(), cmd);
        let r = getptycmd(&cmds, "foo");
        assert!(r.is_some());
        assert_eq!(r.unwrap().name, "foo");
    }

    /// c:490 — `deleteptycmd` on a missing name is a safe no-op.
    #[test]
    fn deleteptycmd_missing_name_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        deleteptycmd(&mut cmds, "absent");
        assert!(cmds.is_empty());
    }

    /// c:490 — `deleteptycmd` on a present name removes exactly
    /// that entry, leaving siblings intact.
    ///
    /// IMPORTANT: deleteptycmd calls `libc::kill(-cmd.pid, SIGHUP)`
    /// at c:517 (kills the process group). Small pids would target
    /// real pgids (`kill(-1, ...)` = "all processes you can signal"
    /// — catastrophic in tests). Use pids well beyond any real
    /// pgid so the kill becomes ESRCH/EPERM no-op.
    #[test]
    fn deleteptycmd_removes_only_named_entry() {
        let _g = crate::test_util::global_state_lock();
        const SAFE_PID: i32 = i32::MAX - 1; // pgid that cannot exist
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        cmds.insert(
            "a".into(),
            ptycmd::new("a", vec![], -1, SAFE_PID, true, false),
        );
        cmds.insert(
            "b".into(),
            ptycmd::new("b", vec![], -1, SAFE_PID, true, false),
        );
        cmds.insert(
            "c".into(),
            ptycmd::new("c", vec![], -1, SAFE_PID, true, false),
        );
        deleteptycmd(&mut cmds, "b");
        assert!(cmds.contains_key("a"));
        assert!(!cmds.contains_key("b"));
        assert!(cmds.contains_key("c"));
    }

    /// c:517 — `deleteallptycmds` empties the map regardless of
    /// prior content. Pin the unconditional-clear contract.
    ///
    /// Uses SAFE_PID = i32::MAX-1 for the same `kill(-pid, SIGHUP)`
    /// safety reason as `deleteptycmd_removes_only_named_entry`.
    #[test]
    fn deleteallptycmds_clears_all() {
        let _g = crate::test_util::global_state_lock();
        const SAFE_PID: i32 = i32::MAX - 1;
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        for n in ["a", "b", "c", "d"] {
            cmds.insert(n.into(), ptycmd::new(n, vec![], -1, SAFE_PID, true, false));
        }
        assert_eq!(cmds.len(), 4);
        deleteallptycmds(&mut cmds);
        assert!(cmds.is_empty());
    }

    /// c:65 — `ptynonblock` on a closed/invalid fd must surface an
    /// error (not panic) because fcntl(F_GETFL) returns -1 on EBADF.
    #[test]
    fn ptynonblock_on_bad_fd_returns_error() {
        let _g = crate::test_util::global_state_lock();
        let r = ptynonblock(99999);
        assert!(r.is_err(), "ptynonblock on bad fd should be Err");
    }

    /// c:97 — `ptygettyinfo` on a bad fd returns 1 (error sentinel,
    /// per the c:103 `return 1` arm when `tcgetattr` fails). Pin the
    /// non-zero return so a regression that returns 0 (success
    /// sentinel) silently passes garbage termios up the call chain.
    #[test]
    fn ptygettyinfo_on_bad_fd_returns_error_sentinel() {
        let _g = crate::test_util::global_state_lock();
        let mut ti: libc::termios = unsafe { std::mem::zeroed() };
        let r = ptygettyinfo(99999, &mut ti);
        assert_ne!(r, 0, "ptygettyinfo on bad fd must NOT report success");
        assert_eq!(r, 1, "c:103 error path returns 1");
    }

    /// c:773 — `bin_zpty -r missing-name` (read from unknown
    /// session) returns nonzero. Pin missing-session lookup.
    #[test]
    fn bin_zpty_r_unknown_session_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let r = bin_zpty(
            "zpty",
            &["unknown-pty".to_string()],
            &ops_with_flag(b'r'),
            0,
        );
        assert_ne!(r, 0, "read from unknown pty must fail");
    }

    // ─── zsh-corpus pins for getptycmd / deleteptycmd ──────────────

    /// `getptycmd` on empty table returns None.
    #[test]
    fn zpty_corpus_getptycmd_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::<String, ptycmd>::new();
        assert!(getptycmd(&cmds, "anything").is_none());
    }

    /// `getptycmd` finds existing entry.
    #[test]
    fn zpty_corpus_getptycmd_finds_existing() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::<String, ptycmd>::new();
        let p = ptycmd::new("stub", Vec::new(), -1, 0, false, false);
        cmds.insert("my_session".to_string(), p);
        assert!(getptycmd(&cmds, "my_session").is_some());
    }

    /// `deleteptycmd` on missing is a no-op (avoid touching real fds).
    #[test]
    fn zpty_corpus_deleteptycmd_missing_no_op() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::<String, ptycmd>::new();
        deleteptycmd(&mut cmds, "never_was");
        assert!(cmds.is_empty(), "still empty");
    }

    // Note: deleteptycmd/deleteallptycmds with real entries can attempt
    // to close fd -1 (or kill pid 0), which blocks under the test harness.
    // Pin only the empty/no-op paths above.

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zpty.c — fd-error paths.
    // ═══════════════════════════════════════════════════════════════════

    /// c:159 — `getptycmd` on empty HashMap returns None.
    #[test]
    fn getptycmd_empty_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::<String, ptycmd>::new();
        assert!(getptycmd(&cmds, "any").is_none());
    }

    /// c:159 — `getptycmd` for absent name returns None even when
    /// table has other entries.
    #[test]
    fn getptycmd_absent_in_populated_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::<String, ptycmd>::new();
        cmds.insert(
            "session_a".to_string(),
            ptycmd::new("stub", Vec::new(), -1, 0, false, false),
        );
        assert!(getptycmd(&cmds, "session_b").is_none());
    }

    /// c:314 — `deleteallptycmds` on empty map is a safe no-op.
    #[test]
    fn deleteallptycmds_empty_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::<String, ptycmd>::new();
        deleteallptycmds(&mut cmds);
        assert!(cmds.is_empty());
    }

    /// c:97 — `ptynonblock(-1)` returns Err (invalid fd).
    #[test]
    fn ptynonblock_invalid_fd_returns_err() {
        let _g = crate::test_util::global_state_lock();
        let r = ptynonblock(-1);
        assert!(r.is_err(), "invalid fd → Err");
    }

    /// c:119 — `ptygettyinfo(-1, ...)` returns nonzero (libc error).
    #[test]
    fn ptygettyinfo_invalid_fd_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let mut ti: libc::termios = unsafe { std::mem::zeroed() };
        let r = ptygettyinfo(-1, &mut ti);
        assert_ne!(r, 0, "invalid fd → nonzero error");
    }

    /// c:713 — `ptywritestr(cmd, "x", 1)` with closed master_fd → write(2)
    /// returns -1 with EBADF; checkptycmd's `kill(pid, 0)` fails (pid=0
    /// is invalid signal target on most platforms), so `cmd->fin` flips
    /// true and the loop breaks. Final return: `all == 0 && fin == 1`
    /// → `cmd->fin + 1 == 2` per c:739.
    #[test]
    fn ptywritestr_invalid_fd_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let mut cmd = ptycmd::new("dummy", vec![], -1, 0, false, false);
        let r = ptywritestr(&mut cmd, b"data");
        assert_ne!(r, 0, "closed fd → nonzero per c:739");
    }

    /// c:548 — `ptyread(nam, cmd, [], noblock=true, mustmatch=false)`
    /// on a closed master_fd: noblock poll returns 0 (no readable fd
    /// → no data ready) so the loop breaks immediately with `seen=0`.
    /// Final return: `cmd->fin + 1` (1 if checkptycmd hasn't flagged
    /// the child yet, 2 if it has) per c:704. Pin nonzero + no panic.
    #[test]
    fn ptyread_invalid_fd_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut cmd = ptycmd::new("dummy", vec![], -1, 0, false, false);
        let r = ptyread("zpty", &mut cmd, &[], true, false);
        assert_ne!(r, 0, "no data + no pattern + noblock → nonzero per c:704");
    }

    /// c:761-782 — module setup_ / boot_ return 0.
    #[test]
    fn zpty_setup_boot_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:791-803 — cleanup_ / finish_ return 0.
    #[test]
    fn zpty_cleanup_finish_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zpty.c
    // c:97 ptynonblock / c:119 ptygettyinfo / c:159 getptycmd /
    // c:290 deleteptycmd / c:314 deleteallptycmds / c:748 ptyhook
    // ═══════════════════════════════════════════════════════════════════

    /// c:97 — `ptynonblock(-1)` returns Err (invalid fd).
    #[test]
    fn ptynonblock_negative_fd_returns_err() {
        let _g = crate::test_util::global_state_lock();
        assert!(ptynonblock(-1).is_err(), "negative fd must error");
    }

    /// c:97 — `ptynonblock(99999)` returns Err (way-out-of-range fd).
    #[test]
    fn ptynonblock_far_fd_returns_err() {
        let _g = crate::test_util::global_state_lock();
        assert!(ptynonblock(99999).is_err(), "out-of-range fd must error");
    }

    /// c:159 — `getptycmd` is pure (multiple calls same result).
    #[test]
    fn getptycmd_is_pure_function() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::new();
        let first = getptycmd(&cmds, "nothing").is_none();
        for _ in 0..5 {
            assert_eq!(getptycmd(&cmds, "nothing").is_none(), first);
        }
    }

    /// c:290 — `deleteptycmd` on empty table doesn't panic.
    #[test]
    fn deleteptycmd_on_empty_table_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::new();
        deleteptycmd(&mut cmds, "anything");
        deleteptycmd(&mut cmds, "");
    }

    /// c:314 — `deleteallptycmds` on already-empty table is no-op.
    #[test]
    fn deleteallptycmds_on_empty_table_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::new();
        deleteallptycmds(&mut cmds);
        assert!(cmds.is_empty(), "still empty");
    }

    /// c:748 — `ptyhook` on empty cmds map returns 0 (no jobs to clean).
    #[test]
    fn ptyhook_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::new();
        assert_eq!(ptyhook(&mut cmds), 0, "empty cmds map → 0");
    }

    /// c:761-803 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn zpty_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:761 — setup_ idempotent.
    #[test]
    fn zpty_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:803 — finish_ idempotent.
    #[test]
    fn zpty_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:159 — `getptycmd` empty name lookup returns None.
    #[test]
    fn getptycmd_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::new();
        assert!(getptycmd(&cmds, "").is_none());
    }

    /// c:119 — `ptygettyinfo(-1)` returns nonzero (error on bad fd).
    #[test]
    fn ptygettyinfo_negative_fd_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let mut ti: libc::termios = unsafe { std::mem::zeroed() };
        assert_ne!(ptygettyinfo(-1, &mut ti), 0, "negative fd → nonzero");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zpty.c
    // c:97 ptynonblock / c:119 ptygettyinfo / c:159 getptycmd /
    // c:748 ptyhook / c:761-803 lifecycle — type pins + edge cases
    // ═══════════════════════════════════════════════════════════════════

    /// c:97 — `ptynonblock` returns io::Result<()> (compile-time type pin).
    #[test]
    fn ptynonblock_returns_io_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: io::Result<()> = ptynonblock(-1);
    }

    /// c:119 — `ptygettyinfo` returns i32 (compile-time type pin).
    #[test]
    fn ptygettyinfo_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut ti: libc::termios = unsafe { std::mem::zeroed() };
        let _: i32 = ptygettyinfo(-1, &mut ti);
    }

    /// c:159 — `getptycmd` returns Option<&ptycmd> (compile-time type pin).
    #[test]
    fn getptycmd_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::new();
        let _: Option<&ptycmd> = getptycmd(&cmds, "anything");
    }

    /// c:748 — `ptyhook` returns i32 (compile-time type pin).
    #[test]
    fn ptyhook_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::new();
        let _: i32 = ptyhook(&mut cmds);
    }

    /// c:761 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn zpty_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:97 — `ptynonblock` is deterministic for bad fd.
    #[test]
    fn ptynonblock_negative_fd_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = ptynonblock(-1).is_err();
        for _ in 0..3 {
            assert_eq!(
                ptynonblock(-1).is_err(),
                first,
                "ptynonblock(-1) must be deterministic"
            );
        }
    }

    /// c:159 — `getptycmd` is deterministic.
    #[test]
    fn getptycmd_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let cmds = HashMap::new();
        for name in ["", "x", "any", "definitely_missing_xyz"] {
            let first = getptycmd(&cmds, name).is_none();
            for _ in 0..3 {
                assert_eq!(
                    getptycmd(&cmds, name).is_none(),
                    first,
                    "getptycmd({:?}) must be deterministic",
                    name
                );
            }
        }
    }

    /// c:768 — features list non-empty.
    #[test]
    fn zpty_features_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert!(!feats.is_empty(), "zpty must advertise ≥1 feature");
    }

    /// c:768 — every feature uses b:/p: prefix per zsh module spec.
    #[test]
    fn zpty_features_use_canonical_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        for f in &feats {
            assert!(
                f.starts_with("b:") || f.starts_with("p:"),
                "feature {:?} must use b:/p: prefix",
                f
            );
        }
    }

    /// c:791 — `cleanup_` idempotent.
    #[test]
    fn zpty_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:782 — `boot_` idempotent.
    #[test]
    fn zpty_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:748 — `ptyhook` is deterministic on empty cmds map.
    #[test]
    fn ptyhook_empty_cmds_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let mut cmds = HashMap::new();
        let first = ptyhook(&mut cmds);
        for _ in 0..3 {
            assert_eq!(
                ptyhook(&mut cmds),
                first,
                "ptyhook on empty cmds must be deterministic"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/zpty.c
    // c:159 getptycmd / c:290 deleteptycmd / c:314 deleteallptycmds /
    // c:501 bin_zpty / c:748 ptyhook / c:761-803 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:761 — `setup_` is idempotent.
    #[test]
    fn zpty_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:803 — `finish_` is idempotent.
    #[test]
    fn zpty_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:761 — `setup_` return type i32 (compile-time pin, alt).
    #[test]
    fn zpty_setup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:791 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn zpty_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:803 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn zpty_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:159 — `getptycmd` on empty map returns None.
    #[test]
    fn getptycmd_empty_map_returns_none() {
        let cmds: HashMap<String, ptycmd> = HashMap::new();
        assert!(
            getptycmd(&cmds, "anything").is_none(),
            "empty map must return None"
        );
    }

    /// c:159 — `getptycmd` with empty name returns None on empty map (alt).
    #[test]
    fn getptycmd_empty_name_returns_none_alt() {
        let cmds: HashMap<String, ptycmd> = HashMap::new();
        assert!(
            getptycmd(&cmds, "").is_none(),
            "empty name on empty map must return None"
        );
    }

    /// c:290 — `deleteptycmd` on empty map is safe (no panic).
    #[test]
    fn deleteptycmd_empty_map_no_panic() {
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        deleteptycmd(&mut cmds, "anything");
        deleteptycmd(&mut cmds, "");
    }

    /// c:314 — `deleteallptycmds` on empty map is safe and idempotent.
    #[test]
    fn deleteallptycmds_empty_map_idempotent_safe() {
        let mut cmds: HashMap<String, ptycmd> = HashMap::new();
        for _ in 0..10 {
            deleteallptycmds(&mut cmds);
            assert!(cmds.is_empty(), "must remain empty");
        }
    }

    /// c:501 — `bin_zpty` empty args non-negative.
    #[test]
    fn bin_zpty_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zpty("zpty", &[], &ops, 0);
        assert!(r >= 0, "bin_zpty empty must be ≥ 0, got {}", r);
    }

    /// c:501 — `bin_zpty` various func values don't panic.
    #[test]
    fn bin_zpty_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_zpty("zpty", &[], &ops, func);
        }
    }

    /// c:748 — `ptyhook` returns i32 (compile-time pin, alt).
    #[test]
    fn ptyhook_returns_i32_type_alt() {
        let mut cmds = HashMap::new();
        let _: i32 = ptyhook(&mut cmds);
    }

    /// c:768 — `features_` is deterministic.
    #[test]
    fn zpty_features_deterministic_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut v1: Vec<String> = Vec::new();
        let mut v2: Vec<String> = Vec::new();
        let _ = features_(std::ptr::null(), &mut v1);
        let _ = features_(std::ptr::null(), &mut v2);
        assert_eq!(v1, v2, "features_ must be deterministic");
    }
}
