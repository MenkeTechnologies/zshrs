//! Pseudo-terminal module - port of Modules/zpty.c
//!
//! Provides zpty builtin for running sub-processes with pseudo terminals.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;

/// Maximum bytes to read at once
pub const READ_MAX: usize = 1024 * 1024;

/// A pseudo-terminal command session
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

/// Pty commands manager
#[derive(Debug, Default)]
pub struct PtyCmds {
    cmds: HashMap<String, PtyCmd>,
}

impl PtyCmds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, cmd: PtyCmd) {
        self.cmds.insert(cmd.name.clone(), cmd);
    }

    pub fn get(&self, name: &str) -> Option<&PtyCmd> {
        self.cmds.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut PtyCmd> {
        self.cmds.get_mut(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<PtyCmd> {
        self.cmds.remove(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PtyCmd)> {
        self.cmds.iter()
    }

    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.cmds.keys().map(|s| s.as_str()).collect()
    }
}

/// Open a pseudo-terminal pair
#[cfg(unix)]
pub fn open_pty() -> io::Result<(RawFd, RawFd)> {
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
            return Err(io::Error::new(io::ErrorKind::Other, "ptsname failed"));
        }

        let slave_fd = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
        if slave_fd < 0 {
            libc::close(master_fd);
            return Err(io::Error::last_os_error());
        }

        Ok((master_fd, slave_fd))
    }
}

/// Set non-blocking mode on a file descriptor
#[cfg(unix)]
pub fn set_nonblock(fd: RawFd) -> io::Result<()> {
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

/// Disable echo on a terminal
#[cfg(unix)]
pub fn disable_echo(fd: RawFd) -> io::Result<()> {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) < 0 {
            return Err(io::Error::last_os_error());
        }

        termios.c_lflag &= !libc::ECHO;

        if libc::tcsetattr(fd, libc::TCSADRAIN, &termios) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Read from a pty, optionally matching a pattern
pub fn pty_read(fd: RawFd, pattern: Option<&str>, timeout_ms: Option<i32>) -> io::Result<String> {
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

/// Write to a pty
pub fn pty_write(fd: RawFd, data: &str) -> io::Result<usize> {
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

/// Send EOF to pty
pub fn pty_send_eof(fd: RawFd) -> io::Result<()> {
    #[cfg(unix)]
    {
        let eof = [4u8];
        let n = unsafe { libc::write(fd, eof.as_ptr() as *const libc::c_void, 1) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Check if a pty has data available
pub fn pty_test(fd: RawFd) -> io::Result<bool> {
    #[cfg(unix)]
    {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };

        let ret = unsafe { libc::poll(&mut pfd, 1, 0) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ret > 0)
    }

    #[cfg(not(unix))]
    {
        Ok(true)
    }
}

/// Kill a pty process
pub fn pty_kill(pid: i32, signal: i32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid, signal) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Close a pty
pub fn pty_close(fd: RawFd) -> io::Result<()> {
    #[cfg(unix)]
    {
        let ret = unsafe { libc::close(fd) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Options for zpty builtin
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

/// Execute zpty builtin
pub fn builtin_zpty(args: &[&str], options: &ZptyOptions, cmds: &mut PtyCmds) -> (i32, String) {
    let mut output = String::new();

    if options.delete {
        if args.is_empty() {
            let names: Vec<String> = cmds.names().iter().map(|s| s.to_string()).collect();
            for name in names {
                if let Some(cmd) = cmds.remove(&name) {
                    let _ = pty_kill(cmd.pid, libc::SIGTERM);
                    let _ = pty_close(cmd.master_fd);
                }
            }
            return (0, output);
        }

        for name in args {
            if let Some(cmd) = cmds.remove(*name) {
                let _ = pty_kill(cmd.pid, libc::SIGTERM);
                let _ = pty_close(cmd.master_fd);
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
            match pty_write(cmd.master_fd, &data) {
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
            match pty_read(cmd.master_fd, pattern, timeout) {
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
            match pty_test(cmd.master_fd) {
                Ok(true) => (0, output),
                Ok(false) => (1, output),
                Err(e) => (1, format!("zpty: test failed: {}\n", e)),
            }
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
            match open_pty() {
                Ok((master, slave)) => match unsafe { libc::fork() } {
                    -1 => {
                        let _ = pty_close(master);
                        let _ = pty_close(slave);
                        (
                            1,
                            format!("zpty: fork failed: {}\n", io::Error::last_os_error()),
                        )
                    }
                    0 => {
                        let _ = pty_close(master);
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
                            let _ = disable_echo(0);
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
                        let _ = pty_close(slave);

                        if !options.block {
                            let _ = set_nonblock(master);
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

        let (status, output) = builtin_zpty(&[], &options, &mut cmds);
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

        let (status, _) = builtin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_zpty_write_no_args() {
        let mut cmds = PtyCmds::new();
        let options = ZptyOptions {
            write: true,
            ..Default::default()
        };

        let (status, output) = builtin_zpty(&[], &options, &mut cmds);
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

        let (status, output) = builtin_zpty(&[], &options, &mut cmds);
        assert_eq!(status, 1);
        assert!(output.contains("requires"));
    }
}
