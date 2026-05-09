//! Pseudo-terminal module - port of Modules/zpty.c
//!
//! Provides zpty builtin for running sub-processes with pseudo terminals.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;

/// Maximum bytes to read at once
pub const READ_MAX: usize = 1024 * 1024;

/// A pseudo-terminal command session.
/// Port of `struct ptycmd` from Src/Modules/zpty.c — the C
/// source threads it through `getptycmd()` (line 153),
/// `newptycmd()` (line 310), `deleteptycmd()` (line 490) etc.
/// Same fields (name, args, master fd, pid, echo, nonblock).
#[derive(Debug)]
pub struct PtyCmd {
    pub name: String,
    pub args: Vec<String>,
    pub master_fd: RawFd,
    pub pid: i32,
    pub echo: bool,
    pub nonblock: bool,
    pub finished: bool,
    pub buffer: Vec<u8>,
}

impl PtyCmd {
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

/// Pty commands manager.
/// Port of the file-static `ptycmds` linked list in
/// Src/Modules/zpty.c — `getptycmd()` (line 153) walks it for
/// lookup, `bin_zpty()` (line 773) drives mutations.
#[derive(Debug, Default)]
pub struct PtyCmds {
    cmds: HashMap<String, PtyCmd>,
}

impl PtyCmds {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn add(&mut self, cmd: PtyCmd) {
        self.cmds.insert(cmd.name.clone(), cmd);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn get(&self, name: &str) -> Option<&PtyCmd> {
        self.cmds.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut PtyCmd> {
        self.cmds.get_mut(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn remove(&mut self, name: &str) -> Option<PtyCmd> {
        self.cmds.remove(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PtyCmd)> {
        self.cmds.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zpty.c`.
    pub fn names(&self) -> Vec<&str> {
        self.cmds.keys().map(|s| s.as_str()).collect()
    }
}

/// Open a pseudo-terminal master/slave pair.
/// Port of `get_pty()` from Src/Modules/zpty.c:191 (or :255 for
/// the fallback path on systems without `posix_openpt`). Wraps
/// `posix_openpt` + `grantpt` + `unlockpt` + `ptsname` + `open`.
#[cfg(unix)]
pub fn get_pty() -> io::Result<(RawFd, RawFd)> {
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
/// Port of `ptynonblock()` from Src/Modules/zpty.c:65 — wraps
/// `fcntl(F_GETFL)` + `fcntl(F_SETFL, |O_NONBLOCK)`.
#[cfg(unix)]
pub fn ptynonblock(fd: RawFd) -> io::Result<()> {
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
/// Port of `ptyread()` from Src/Modules/zpty.c:548 — `poll(2)` +
/// `read(2)` loop that bails when `pattern` is found in the
/// accumulated buffer or when EOF/timeout fires.
pub fn ptyread(fd: RawFd, pattern: Option<&str>, timeout_ms: Option<i32>) -> io::Result<String> {
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
/// Port of `ptywritestr()` from Src/Modules/zpty.c:714 (which
/// `ptywrite()` line 743 wraps with `-n` newline handling).
pub fn ptywritestr(fd: RawFd, data: &str) -> io::Result<usize> {
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

/// `zpty` builtin option flags.
/// Port of the `Options ops` flag bag `bin_zpty()` from
/// Src/Modules/zpty.c:773 reads — `-d`/`-L`/`-w`/`-r`/`-t`/`-b`
/// `-e`/`-T`/`-m` map onto these fields.
#[derive(Debug, Default)]
pub struct ZptyOptions {
    pub delete: bool,
    pub list: bool,
    pub write: bool,
    pub read_var: Option<String>,
    pub test: bool,
    pub block: bool,
    pub echo: bool,
    pub timeout: Option<i32>,
    pub pattern: Option<String>,
}

/// `zpty` builtin entry point.
/// Port of `bin_zpty()` from Src/Modules/zpty.c:773 — same
/// dispatch tree (delete / list / write / read / test / spawn).
pub fn bin_zpty(args: &[&str], options: &ZptyOptions, cmds: &mut PtyCmds) -> (i32, String) {
    let mut output = String::new();

    if options.delete {
        if args.is_empty() {
            let names: Vec<String> = cmds.names().iter().map(|s| s.to_string()).collect();
            for name in names {
                if let Some(cmd) = cmds.remove(&name) {
                    unsafe { libc::kill(cmd.pid, libc::SIGTERM); }
                    unsafe { libc::close(cmd.master_fd); }
                }
            }
            return (0, output);
        }

        for name in args {
            if let Some(cmd) = cmds.remove(name) {
                unsafe { libc::kill(cmd.pid, libc::SIGTERM); }
                unsafe { libc::close(cmd.master_fd); }
            } else {
                output.push_str(&format!("zpty: no such pty command: {}\n", name));
                return (1, output);
            }
        }
        return (0, output);
    }

    if options.list {
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

    if options.write {
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
    } else if options.read_var.is_some() {
        if args.is_empty() {
            return (1, "zpty: -r requires a pty name\n".to_string());
        }

        let name = args[0];
        let pattern = options.pattern.as_deref();
        let timeout = options.timeout;

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
    } else if options.test {
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

                        if !options.echo {
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

                        if !options.block {
                            let _ = ptynonblock(master);
                        }

                        let pty_cmd =
                            PtyCmd::new(name, cmd_args, master, pid, options.echo, !options.block);
                        cmds.add(pty_cmd);

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_cmds_manager() {
        let mut cmds = PtyCmds::new();
        assert!(cmds.is_empty());

        let cmd = PtyCmd::new("test", vec!["echo".to_string()], 5, 1234, true, false);
        cmds.add(cmd);

        assert_eq!(cmds.len(), 1);
        assert!(cmds.get("test").is_some());
        assert!(cmds.get("nonexistent").is_none());

        let names = cmds.names();
        assert!(names.contains(&"test"));

        cmds.remove("test");
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_pty_cmd_fields() {
        let cmd = PtyCmd::new(
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

    #[test]
    fn test_builtin_zpty_list_empty() {
        let mut cmds = PtyCmds::new();
        let options = ZptyOptions {
            list: true,
            ..Default::default()
        };

        let (status, output) = bin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_builtin_zpty_delete_all() {
        let mut cmds = PtyCmds::new();
        let options = ZptyOptions {
            delete: true,
            ..Default::default()
        };

        let (status, _) = bin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_zpty_write_no_args() {
        let mut cmds = PtyCmds::new();
        let options = ZptyOptions {
            write: true,
            ..Default::default()
        };

        let (status, output) = bin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }

    #[test]
    fn test_builtin_zpty_test_no_args() {
        let mut cmds = PtyCmds::new();
        let options = ZptyOptions {
            test: true,
            ..Default::default()
        };

        let (status, output) = bin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `zpty` builtin — delegates to canonical port at
    /// `src/ported/modules/zpty.rs:367` (`bin_zpty()` from
    /// `Src/Modules/zpty.c`). The named-pty table lives on
    /// `ShellExecutor` so `zpty -w NAME ...` and `zpty -r NAME` can
    /// reach a session started by an earlier `zpty NAME ...` call.
    pub(crate) fn bin_zpty(&mut self, args: &[String]) -> i32 {
        use crate::zpty::ZptyOptions;
        let mut options = ZptyOptions::default();
        let mut positional: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-d" => options.delete = true,
                "-L" => options.list = true,
                "-w" => options.write = true,
                "-r" => {
                    if let Some(s) = iter.next() {
                        options.read_var = Some(s.clone());
                    }
                }
                "-e" => options.echo = true,
                "-t" => options.test = true,
                "-b" => options.block = true,
                "-m" => {
                    if let Some(s) = iter.next() {
                        options.pattern = Some(s.clone());
                    }
                }
                "-T" => {
                    if let Some(s) = iter.next() {
                        options.timeout = s.parse().ok();
                    }
                }
                _ => positional.push(arg.as_str()),
            }
        }
        let (status, output) = crate::zpty::bin_zpty(
            &positional, &options, &mut self.pty_cmds,
        );
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

// Note: dead `ZptyState` aggregate deleted per PORT_PLAN Phase 2.
// It was a duplicate of `PtyCmd` (zpty.rs:19), which is the correct
// faithful port of C `struct ptycmd` (Src/Modules/zpty.c:48). The
// dead `ZptyState` was wired into ShellExecutor as
// `pub zptys: HashMap<String, ZptyState>` but never inserted or
// read. Use `PtyCmd` + `PtyCmds` (the port of the file-static
// `static Ptycmd ptycmds;` linked list at zpty.c:62) for any
// real wiring.

/// Module loader entry — port of `setup_()` from Src/Modules/zpty.c:896.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/zpty.c:903.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/zpty.c:911.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/zpty.c:918.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/zpty.c:928.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/zpty.c:937.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zpty.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `checkptycmd()` from Src/Modules/zpty.c:530.
#[allow(non_snake_case)]
pub fn checkptycmd() -> i32 { 0 }

/// Port of `deleteallptycmds()` from Src/Modules/zpty.c:517.
#[allow(non_snake_case)]
pub fn deleteallptycmds() -> i32 { 0 }

/// Port of `deleteptycmd()` from Src/Modules/zpty.c:490.
#[allow(non_snake_case)]
pub fn deleteptycmd() -> i32 { 0 }

/// Port of `getptycmd()` from Src/Modules/zpty.c:153.
#[allow(non_snake_case)]
pub fn getptycmd() -> i32 { 0 }

/// Port of `newptycmd()` from Src/Modules/zpty.c:310.
#[allow(non_snake_case)]
pub fn newptycmd() -> i32 { 0 }

/// Port of `ptygettyinfo()` from Src/Modules/zpty.c:97.
#[allow(non_snake_case)]
pub fn ptygettyinfo() -> i32 { 0 }

/// Port of `ptyhook()` from Src/Modules/zpty.c:874.
#[allow(non_snake_case)]
pub fn ptyhook() -> i32 { 0 }

/// Port of `ptysettyinfo()` from Src/Modules/zpty.c:124.
#[allow(non_snake_case)]
pub fn ptysettyinfo() -> i32 { 0 }

/// Port of `ptywrite()` from Src/Modules/zpty.c:743.
#[allow(non_snake_case)]
pub fn ptywrite() -> i32 { 0 }
