//! Pseudo-terminal module - port of Modules/zpty.c
//!
//! Provides zpty builtin for running sub-processes with pseudo terminals.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
use std::os::unix::io::IntoRawFd;
use std::process::Command;

/// Port of `READ_MAX` from `Src/Modules/zpty.c:44`. Maximum bytes
/// to read at once from a pty's master end (1 MB).
pub const READ_MAX: usize = 1024 * 1024;                                     // c:44

/// A pseudo-terminal command session.
/// Port of `struct ptycmd` from Src/Modules/zpty.c — the C
/// source threads it through `getptycmd()` (line 153),
/// `newptycmd()` (line 310), `deleteptycmd()` (line 490) etc.
/// Same fields (name, args, master fd, pid, echo, nonblock).
#[derive(Debug)]
pub struct ptycmd {
    pub name: String,
    pub args: Vec<String>,
    pub master_fd: RawFd,
    pub pid: i32,
    pub echo: bool,
    pub nonblock: bool,
    pub finished: bool,
    pub buffer: Vec<u8>,
}

impl ptycmd {
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
            buffer: Vec::new(),
        }
    }
}


/// Open a pseudo-terminal master/slave pair.
/// Port of `get_pty(int master, int *retfd)` from Src/Modules/zpty.c:191 (or :255 for
/// the fallback path on systems without `posix_openpt`). Wraps
/// `posix_openpt` + `grantpt` + `unlockpt` + `ptsname` + `open`.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=() vs C=(master, retfd)
pub fn get_pty() -> io::Result<(RawFd, RawFd)> {                            // c:191
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

/// Set non-blocking mode on a file descriptor.
/// Port of `ptynonblock(int fd)` from Src/Modules/zpty.c:65 — wraps
/// `fcntl(F_GETFL)` + `fcntl(F_SETFL, |O_NONBLOCK)`.
#[cfg(unix)]
pub fn ptynonblock(fd: RawFd) -> io::Result<()> {                           // c:65
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

/// Read from a pty, optionally matching a pattern.
/// Port of `ptyread(char *nam, Ptycmd cmd, char **args, int noblock, int mustmatch)` from Src/Modules/zpty.c:548 — `poll(2)` +
/// `read(2)` loop that bails when `pattern` is found in the
/// accumulated buffer or when EOF/timeout fires.
/// WARNING: param names don't match C — Rust=(fd, pattern, timeout_ms) vs C=(nam, cmd, args, noblock, mustmatch)
pub fn ptyread(fd: RawFd, pattern: Option<&str>, timeout_ms: Option<i32>) -> io::Result<String> { // c:548
    let mut buffer = vec![0u8; 4096];
    let mut result = Vec::new();

    #[cfg(unix)]
    {
        if let Some(timeout) = timeout_ms {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };

            let ret = unsafe { libc::poll(&mut pfd, 1, timeout) };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
            if ret == 0 {
                return Ok(String::new());
            }
        }

        loop {
            let n =
                unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };

            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                return Err(err);
            }

            if n == 0 {
                break;
            }

            result.extend_from_slice(&buffer[..n as usize]);

            if result.len() >= READ_MAX {
                break;
            }

            if let Some(pat) = pattern {
                if let Ok(s) = String::from_utf8(result.clone()) {
                    if s.contains(pat) {
                        break;
                    }
                }
            }
        }
    }

    String::from_utf8(result).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write a string to a pty's master end.
/// Port of `ptywritestr(Ptycmd cmd, char *s, int len)` from Src/Modules/zpty.c:714 (which
/// `ptywrite()` line 743 wraps with `-n` newline handling).
/// WARNING: param names don't match C — Rust=(fd, data) vs C=(cmd, s, len)
pub fn ptywritestr(fd: RawFd, data: &str) -> io::Result<usize> {            // c:714
    #[cfg(unix)]
    {
        let bytes = data.as_bytes();
        let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };

        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    #[cfg(not(unix))]
    {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not supported"))
    }
}

/// Port of `bin_zpty(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/zpty.c:773`.
/// `zpty` builtin entry point — C-faithful signature matching
/// `static int bin_zpty(char *nam, char **args, Options ops, int func)`
/// from Src/Modules/zpty.c:773. Reads `-d/-L/-w/-r/-t/-b/-e/-T/-m`
/// flags via OPT_ISSET/OPT_ARG, dispatches by mode, emits output to
/// stdout/stderr based on status, returns the i32 status.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zpty(_nam: &str, args: &[String],                                 // c:773
                ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    // Per C: branches dispatch on OPT_ISSET(ops, 'X') directly. No
    // aggregator struct — Rule D forbids `*Options` bags.
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let args = &argv[..];
    let mut cmds_guard = ptycmds().lock()
        .unwrap_or_else(|e| { e.into_inner() });
    let cmds: &mut HashMap<String, ptycmd> = &mut *cmds_guard;
    let (status, output): (i32, String) = (|| {
    let mut output = String::new();

    if OPT_ISSET(ops, b'd') {
        if args.is_empty() {
            let names: Vec<String> = cmds.keys().cloned().collect();
            for name in names {
                if let Some(cmd) = cmds.remove(&name) {
                    unsafe { libc::kill(cmd.pid, libc::SIGTERM); }
                    unsafe { libc::close(cmd.master_fd); }
                }
            }
            return (0, output);
        }

        for name in args {
            if let Some(cmd) = cmds.remove(*name) {
                unsafe { libc::kill(cmd.pid, libc::SIGTERM); }
                unsafe { libc::close(cmd.master_fd); }
            } else {
                output.push_str(&format!("zpty: no such pty command: {}\n", name));
                return (1, output);
            }
        }
        return (0, output);
    }

    if OPT_ISSET(ops, b'L') {
        for (name, cmd) in cmds.iter() {
            let status = if cmd.finished {
                "(finished)"
            } else {
                "(running)"
            };
            output.push_str(&format!("{}: {} {}\n", name, cmd.args.join(" "), status));
        }
        return (0, output);
    }

    if OPT_ISSET(ops, b'w') {
        if args.len() < 2 {
            return (1, "zpty: -w requires a pty name and data\n".to_string());
        }

        let name = args[0];
        let data: String = args[1..].join(" ");

        if let Some(cmd) = cmds.get(name) {
            match ptywritestr(cmd.master_fd, &data) {
                Ok(_) => (0, output),
                Err(e) => (1, format!("zpty: write failed: {}\n", e)),
            }
        } else {
            (1, format!("zpty: no such pty command: {}\n", name))
        }
    } else if OPT_ISSET(ops, b'r') {
        if args.is_empty() {
            return (1, "zpty: -r requires a pty name\n".to_string());
        }

        let name = args[0];
        let pattern = OPT_ARG(ops, b'm');
        let timeout: Option<i32> = OPT_ARG(ops, b'T').and_then(|s| s.parse().ok());

        if let Some(cmd) = cmds.get(name) {
            match ptyread(cmd.master_fd, pattern, timeout) {
                Ok(data) => {
                    output.push_str(&data);
                    (0, output)
                }
                Err(e) => (1, format!("zpty: read failed: {}\n", e)),
            }
        } else {
            (1, format!("zpty: no such pty command: {}\n", name))
        }
    } else if OPT_ISSET(ops, b't') {
        if args.is_empty() {
            return (1, "zpty: -t requires a pty name\n".to_string());
        }

        let name = args[0];
        if let Some(cmd) = cmds.get(name) {
            // Inline of the deleted pty_test helper: poll(2) with zero
            // timeout (Src/Modules/zpty.c:773 -t branch).
            #[cfg(unix)]
            {
                let mut pfd = libc::pollfd {
                    fd: cmd.master_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ret = unsafe { libc::poll(&mut pfd, 1, 0) };
                if ret < 0 {
                    (1, format!("zpty: test failed: {}\n", io::Error::last_os_error()))
                } else if ret > 0 {
                    (0, output)
                } else {
                    (1, output)
                }
            }
            #[cfg(not(unix))]
            (0, output)
        } else {
            (1, format!("zpty: no such pty command: {}\n", name))
        }
    } else {
        if args.len() < 2 {
            return (1, "zpty: requires a name and command\n".to_string());
        }

        let name = args[0];
        if cmds.get(name).is_some() {
            return (1, format!("zpty: pty command {} already exists\n", name));
        }

        let cmd_args: Vec<String> = args[1..].iter().map(|s| s.to_string()).collect();

        #[cfg(unix)]
        {
            match get_pty() {
                Ok((master, slave)) => match unsafe { libc::fork() } {
                    -1 => {
                        unsafe { libc::close(master); }
                        unsafe { libc::close(slave); }
                        (
                            1,
                            format!("zpty: fork failed: {}\n", io::Error::last_os_error()),
                        )
                    }
                    0 => {
                        unsafe { libc::close(master); }
                        unsafe {
                            libc::setsid();
                            libc::dup2(slave, 0);
                            libc::dup2(slave, 1);
                            libc::dup2(slave, 2);
                            if slave > 2 {
                                libc::close(slave);
                            }
                        }

                        if !OPT_ISSET(ops, b'e') {
                            // Inline of the deleted disable_echo helper
                            // (Src/Modules/zpty.c:124 ptysettyinfo).
                            unsafe {
                                let mut termios: libc::termios = std::mem::zeroed();
                                if libc::tcgetattr(0, &mut termios) >= 0 {
                                    termios.c_lflag &= !libc::ECHO;
                                    let _ = libc::tcsetattr(0, libc::TCSADRAIN, &termios);
                                }
                            }
                        }

                        let cmd = CString::new(cmd_args[0].clone()).unwrap();
                        let c_args: Vec<CString> = cmd_args
                            .iter()
                            .map(|s| CString::new(s.as_str()).unwrap())
                            .collect();
                        let c_args_ptrs: Vec<*const libc::c_char> = c_args
                            .iter()
                            .map(|s| s.as_ptr())
                            .chain(std::iter::once(std::ptr::null()))
                            .collect();

                        unsafe {
                            libc::execvp(cmd.as_ptr(), c_args_ptrs.as_ptr());
                            libc::_exit(1);
                        }
                    }
                    pid => {
                        unsafe { libc::close(slave); }

                        if !OPT_ISSET(ops, b'b') {
                            let _ = ptynonblock(master);
                        }

                        let pty_cmd =
                            ptycmd::new(name, cmd_args, master, pid,
                                        OPT_ISSET(ops, b'e'),
                                        !OPT_ISSET(ops, b'b'));
                        cmds.insert(pty_cmd.name.clone(), pty_cmd);

                        (0, output)
                    }
                },
                Err(e) => (1, format!("zpty: can't open pty: {}\n", e)),
            }
        }

        #[cfg(not(unix))]
        {
            (1, "zpty: not supported on this platform\n".to_string())
        }
    }
    })();
    drop(cmds_guard);
    if !output.is_empty() {
        if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    #[test]
    fn test_pty_cmds_manager() {
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    #[test]
    fn test_pty_cmd_fields() {
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    fn ops_with_flag(c: u8) -> crate::ported::zsh_h::options {
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut o = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                              argscount: 0, argsalloc: 0 };
        o.ind[c as usize] = 1;
        o
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    /// Verifies `-L` (list) on an empty pty table returns 0.
    /// Mirrors Src/Modules/zpty.c:773 -L arm.
    #[test]
    fn test_builtin_zpty_list_empty() {
        // Reset global PTYCMDS for test isolation.
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'L'), 0);
        assert_eq!(status, 0);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    /// Verifies `-d` with no positional args clears all sessions.
    /// Mirrors Src/Modules/zpty.c:773 -d arm.
    #[test]
    fn test_builtin_zpty_delete_all() {
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'd'), 0);
        assert_eq!(status, 0);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    /// Verifies `-w` with no positional args returns 1 (needs name + data).
    #[test]
    fn test_builtin_zpty_write_no_args() {
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b'w'), 0);
        assert_eq!(status, 1);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    /// Verifies `-t` with no positional args returns 1 (needs name).
    #[test]
    fn test_builtin_zpty_test_no_args() {
        *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
        let status = bin_zpty("zpty", &[], &ops_with_flag(b't'), 0);
        assert_eq!(status, 1);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

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

use std::sync::Mutex;
use std::sync::OnceLock;
use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (zpty.c).


// `module_features` — port of `static struct features module_features`
// from zpty.c:884.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zpty.c:896`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:896
    // C body c:898-899 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zpty.c:903`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:903
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zpty.c:911`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:911
    handlefeatures(m, module_features(), enables)
}

/// Global `ptycmds` linked-list from `Src/Modules/zpty.c:36`.
/// C declares `static Ptycmd ptycmds;` and mutates it through the
/// whole module. Rust uses OnceLock<Mutex<>> for thread-safe access.
pub static PTYCMDS: std::sync::OnceLock<Mutex<HashMap<String, ptycmd>>> = std::sync::OnceLock::new();

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zpty.c`.
fn ptycmds() -> &'static Mutex<HashMap<String, ptycmd>> {
    PTYCMDS.get_or_init(|| Mutex::new(HashMap::<String, ptycmd>::new()))
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zpty.c:918`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                 // c:918
    // C body c:921-922 — `ptycmds = NULL; addhookfunc("exit", ptyhook)`.
    *ptycmds().lock().unwrap() = HashMap::<String, ptycmd>::new();
    let _ = ptyhook(&mut ptycmds().lock().unwrap());                     // c:928 (hook handle)
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zpty.c:928`.
pub fn cleanup_(m: *const module) -> i32 {                              // c:928
    // c:937 — `deletehookfunc("exit", ptyhook)`. We have no live hook
    //          registry, so this is a no-op.
    // c:937 — `deleteallptycmds()`.
    deleteallptycmds(&mut ptycmds().lock().unwrap());
    // c:937 — `return setfeatureenables(m, &module_features, NULL)`.
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zpty.c:937`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:937
    // C body c:939-940 — `return 0`. Faithful empty-body port; the
    //                    pty session teardown happens in cleanup_.
    0
}

// === auto-generated stubs ===
/// Port of `getptycmd(char *name)` from `Src/Modules/zpty.c:153`. Linear
/// scan over the `ptycmds` linked list looking for one matching
/// `name`.
///
/// C signature: `static Ptycmd getptycmd(char *name)`. Returns
/// the Ptycmd or NULL.
/// WARNING: param names don't match C — Rust=(cmds, name) vs C=(name)
pub fn getptycmd<'a>(cmds: &'a HashMap<String, ptycmd>, name: &str) -> Option<&'a ptycmd> {  // c:153
    cmds.get(name)                                                       // c:153-160 strcmp loop
}

/// Port of `deleteptycmd(Ptycmd cmd)` from `Src/Modules/zpty.c:490`. Removes
/// `cmd` from the `ptycmds` linked list, frees its name + args,
/// closes the master fd, and kills the process group via
/// `kill(-pid, SIGHUP)`.
///
/// C signature: `static void deleteptycmd(Ptycmd cmd)`.
/// WARNING: param names don't match C — Rust=(cmds, name) vs C=(cmd)
pub fn deleteptycmd(cmds: &mut HashMap<String, ptycmd>, name: &str) {                    // c:490
    if let Some(cmd) = cmds.remove(name) {                               // c:490-503 list-unlink
        // c:505 — `zsfree(p->name)` + c:506 `freearray(p->args)` —
        // Rust drops String/Vec automatically on `cmd` going out
        // of scope.
        // c:508 — `zclose(cmd->fd)`.
        unsafe { libc::close(cmd.master_fd); }
        // c:517 — `kill(-(p->pid), SIGHUP);` — kill the process group.
        unsafe { libc::kill(-cmd.pid, libc::SIGHUP); }
        // c:517 — `zfree(p, sizeof(*p))` — Rust drop handles.
    }
}

/// Port of `deleteallptycmds()` from `Src/Modules/zpty.c:517`.
/// Walks the `ptycmds` list and deletes every entry.
///
/// C signature: `static void deleteallptycmds(void)`.
/// WARNING: param names don't match C — Rust=(cmds) vs C=()
pub fn deleteallptycmds(cmds: &mut HashMap<String, ptycmd>) {                            // c:517
    let names: Vec<String> = cmds.keys().cloned().collect();
    for n in names {                                                     // c:530-525
        deleteptycmd(cmds, &n);                                          // c:530
    }
}

/// Port of `checkptycmd(Ptycmd cmd)` from `Src/Modules/zpty.c:530`. Polls
/// the master fd with a 1-byte non-blocking read; if read fails
/// AND `kill(pid, 0)` confirms the process is gone, marks the
/// command as finished and closes the fd.
///
/// C signature: `static void checkptycmd(Ptycmd cmd)`.
pub fn checkptycmd(cmd: &mut ptycmd) {                                   // c:530
    if cmd.finished {                                                    // c:530 cmd->fin
        return;
    }
    let mut c: u8 = 0;
    let r = unsafe { libc::read(cmd.master_fd, &mut c as *mut u8 as *mut _, 1) };
    if r <= 0 {                                                          // c:538
        // c:539 — `if (kill(cmd->pid, 0) < 0)` — process gone.
        if unsafe { libc::kill(cmd.pid, 0) } < 0 {
            cmd.finished = true;                                         // c:540 cmd->fin = 1
            unsafe { libc::close(cmd.master_fd); }                       // c:541 zclose
        }
        return;
    }
    // c:544 — `cmd->read = (int) c;` — buffer the read byte for
    // the next read-builtin call. zshrs's ptycmd uses Vec<u8>.
    cmd.buffer.push(c);
}

/// Port of `ptygettyinfo(int fd, struct ttyinfo *ti)` from `Src/Modules/zpty.c:97`. Calls
/// `tcgetattr(fd, &ti->tio)` to capture the pty's termios state.
/// Returns 0 on success, 1 on failure or when fd == -1.
///
/// C signature: `static int ptygettyinfo(int fd, struct ttyinfo *ti)`.
/// Rust port takes a mutable `&mut libc::termios` directly since
/// `struct ttyinfo` (zsh.h) wraps termios + a few legacy fields.
pub fn ptygettyinfo(fd: i32, ti: &mut libc::termios) -> i32 {            // c:97
    if fd == -1 {                                                        // c:97 inverted
        return 1;                                                        // c:118
    }
    // c:101 — `tcgetattr(fd, &ti->tio)`.
    let r = unsafe { libc::tcgetattr(fd, ti as *mut libc::termios) };
    if r == -1 {                                                         // c:103
        return 1;                                                        // c:103
    }
    0                                                                    // c:124
}

/// Port of `ptysettyinfo(int fd, struct ttyinfo *ti)` from `Src/Modules/zpty.c:124`. Calls
/// `tcsetattr(fd, TCSADRAIN, &ti->tio)` to install the captured
/// termios state on the pty.
///
/// C signature: `static void ptysettyinfo(int fd, struct ttyinfo *ti)`.
pub fn ptysettyinfo(fd: i32, ti: &libc::termios) {                       // c:124
    if fd == -1 {                                                        // c:124 inverted
        return;
    }
    // c:128-132 — `tcsetattr(fd, TCSADRAIN, &ti->tio);`
    unsafe { libc::tcsetattr(fd, libc::TCSADRAIN, ti as *const libc::termios); }
}

/// Port of `ptywrite(Ptycmd cmd, char **args, int nonl)` from `Src/Modules/zpty.c:743`. Writes
/// the joined argv to the pty master fd (or copies stdin to the
/// pty when argv is empty). `nonl` suppresses the trailing
/// newline.
///
/// C signature: `static int ptywrite(Ptycmd cmd, char **args, int nonl)`.
pub fn ptywrite(cmd: &ptycmd, args: &[&str], nonl: i32) -> i32 {         // c:743
    if !args.is_empty() {                                                // c:743
        for (i, a) in args.iter().enumerate() {                          // c:751
            // c:752 — unmetafy + ptywritestr.
            let unmeta = crate::ported::utils::unmeta(a);
            let bytes = unmeta.as_bytes();
            let r = unsafe { libc::write(cmd.master_fd, bytes.as_ptr() as *const _, bytes.len()) };
            if r < 0 { return 1; }                                       // c:753
            if i + 1 < args.len() {                                      // c:754 sp = ' '
                let sp = b' ';
                let r = unsafe { libc::write(cmd.master_fd, &sp as *const u8 as *const _, 1) };
                if r < 0 { return 1; }
            }
        }
        if nonl == 0 {                                                   // c:757
            let nl = b'\n';                                              // c:758
            let r = unsafe { libc::write(cmd.master_fd, &nl as *const u8 as *const _, 1) };
            if r < 0 { return 1; }                                       // c:760
        }
    } else {                                                             // c:763
        // c:764-768 — `while ((n = read(0, buf, BUFSIZ)) > 0)` copy stdin.
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::read(0, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 { break; }
            let r = unsafe { libc::write(cmd.master_fd, buf.as_ptr() as *const _, n as usize) };
            if r < 0 { return 1; }                                       // c:768
        }
    }
    0                                                                    // c:771
}

/// Port of `ptyhook(UNUSED(Hookdef d), UNUSED(void *dummy))` from `Src/Modules/zpty.c:874`. The cleanup
/// hook installed at `boot_()` time — runs `deleteallptycmds()`
/// when the shell is exiting (via the `before_trap` hook).
///
/// C signature: `static int ptyhook(Hookdef d, void *dummy)`.
/// WARNING: param names don't match C — Rust=(cmds) vs C=(d, dummy)
pub fn ptyhook(cmds: &mut HashMap<String, ptycmd>) -> i32 {                              // c:874
    deleteallptycmds(cmds);                                              // c:874
    0                                                                    // c:879
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
pub fn newptycmd(cmds: &mut HashMap<String, ptycmd>, _nam: &str, pname: &str,                // c:310
                 args: &[String], echo: bool, nblock: bool) -> i32 {     // c:310
    if args.is_empty() { return 1; }
    let cmd_path = &args[0];
    let cmd_args = &args[1..];

    // Spawn under a forkpty wrapper. Approximation: use
    // openpty + fork via std::process::Command stdin/stdout
    // redirection. A faithful port needs forkpty(3) integration.
    let child = match Command::new(cmd_path)
        .args(cmd_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let pid = child.id() as i32;
    let stdin = child.stdin.expect("piped").into_raw_fd();

    let new = ptycmd::new(pname, args.to_vec(), stdin, pid, echo, nblock);
    cmds.insert(new.name.clone(), new);
    0
}

use crate::ported::zsh_h::features as features_t;

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zpty.c`.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zpty.c`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:zpty".to_string()]
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zpty.c`.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zpty.c`.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

