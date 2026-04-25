//! System I/O builtins - port of Modules/system.c
//!
//! Provides sysread, syswrite, sysopen, sysseek, syserror, zsystem builtins.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const SYSREAD_BUFSIZE: usize = 8192;

/// Return values for sysread
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysreadResult {
    Success = 0,
    ParamError = 1,
    ReadError = 2,
    WriteError = 3,
    Timeout = 4,
    Eof = 5,
}

/// Options for sysread
#[derive(Debug, Default)]
pub struct SysreadOptions {
    pub input_fd: Option<i32>,
    pub output_fd: Option<i32>,
    pub bufsize: Option<usize>,
    pub timeout: Option<f64>,
    pub count_var: Option<String>,
    pub output_var: Option<String>,
}

/// Perform a system read
pub fn sysread(options: &SysreadOptions) -> (SysreadResult, Option<Vec<u8>>, usize) {
    let input_fd = options.input_fd.unwrap_or(0);
    let bufsize = options.bufsize.unwrap_or(SYSREAD_BUFSIZE);

    let mut buffer = vec![0u8; bufsize];

    #[cfg(unix)]
    {
        if let Some(timeout_secs) = options.timeout {
            if !wait_for_read(input_fd, timeout_secs) {
                return (SysreadResult::Timeout, None, 0);
            }
        }

        let count =
            unsafe { libc::read(input_fd, buffer.as_mut_ptr() as *mut libc::c_void, bufsize) };

        if count < 0 {
            return (SysreadResult::ReadError, None, 0);
        }

        let count = count as usize;
        buffer.truncate(count);

        if let Some(output_fd) = options.output_fd {
            if count == 0 {
                return (SysreadResult::Eof, None, 0);
            }

            let mut written = 0;
            while written < count {
                let ret = unsafe {
                    libc::write(
                        output_fd,
                        buffer[written..].as_ptr() as *const libc::c_void,
                        count - written,
                    )
                };
                if ret < 0 {
                    return (
                        SysreadResult::WriteError,
                        Some(buffer[written..].to_vec()),
                        written,
                    );
                }
                written += ret as usize;
            }
            return (SysreadResult::Success, None, count);
        }

        if count == 0 {
            (SysreadResult::Eof, Some(buffer), 0)
        } else {
            (SysreadResult::Success, Some(buffer), count)
        }
    }

    #[cfg(not(unix))]
    {
        (SysreadResult::ParamError, None, 0)
    }
}

#[cfg(unix)]
fn wait_for_read(fd: i32, timeout_secs: f64) -> bool {
    let timeout_ms = (timeout_secs * 1000.0) as i32;

    unsafe {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };

        let ret = libc::poll(&mut pfd, 1, timeout_ms);
        ret > 0
    }
}

/// Options for syswrite
#[derive(Debug, Default)]
pub struct SyswriteOptions {
    pub output_fd: Option<i32>,
    pub count_var: Option<String>,
}

/// Perform a system write
pub fn syswrite(data: &[u8], options: &SyswriteOptions) -> (i32, usize) {
    let output_fd = options.output_fd.unwrap_or(1);

    #[cfg(unix)]
    {
        let mut written = 0;
        let mut remaining = data;

        while !remaining.is_empty() {
            let ret = unsafe {
                libc::write(
                    output_fd,
                    remaining.as_ptr() as *const libc::c_void,
                    remaining.len(),
                )
            };

            if ret < 0 {
                return (2, written);
            }

            let count = ret as usize;
            written += count;
            remaining = &remaining[count..];
        }

        (0, written)
    }

    #[cfg(not(unix))]
    {
        (1, 0)
    }
}

/// Open options for sysopen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenOpt {
    Cloexec,
    Nofollow,
    Sync,
    Noatime,
    Nonblock,
    Excl,
    Creat,
    Truncate,
}

impl OpenOpt {
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("O_").unwrap_or(name);
        let name_lower = name.to_lowercase();
        match name_lower.as_str() {
            "cloexec" => Some(Self::Cloexec),
            "nofollow" => Some(Self::Nofollow),
            "sync" => Some(Self::Sync),
            "noatime" => Some(Self::Noatime),
            "nonblock" => Some(Self::Nonblock),
            "excl" => Some(Self::Excl),
            "creat" | "create" => Some(Self::Creat),
            "truncate" | "trunc" => Some(Self::Truncate),
            _ => None,
        }
    }

    #[cfg(unix)]
    pub fn to_flags(&self) -> i32 {
        match self {
            Self::Cloexec => libc::O_CLOEXEC,
            Self::Nofollow => libc::O_NOFOLLOW,
            Self::Sync => libc::O_SYNC,
            Self::Noatime => 0, // Not all systems support O_NOATIME
            Self::Nonblock => libc::O_NONBLOCK,
            Self::Excl => libc::O_EXCL | libc::O_CREAT,
            Self::Creat => libc::O_CREAT,
            Self::Truncate => libc::O_TRUNC,
        }
    }
}

/// Options for sysopen
#[derive(Debug, Default)]
pub struct SysopenOptions {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub options: Vec<OpenOpt>,
    pub mode: Option<u32>,
    pub fd_var: Option<String>,
    pub explicit_fd: Option<i32>,
}

/// Open a file with system call
pub fn sysopen(path: &str, options: &SysopenOptions) -> Result<i32, String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;

        let mut flags = libc::O_NOCTTY;

        if options.append {
            flags |= libc::O_APPEND;
        }

        if options.append || options.write {
            if options.read {
                flags |= libc::O_RDWR;
            } else {
                flags |= libc::O_WRONLY;
            }
        } else {
            flags |= libc::O_RDONLY;
        }

        for opt in &options.options {
            flags |= opt.to_flags();
        }

        let mode = options.mode.unwrap_or(0o666);
        let path_c = CString::new(path).map_err(|e| e.to_string())?;

        let fd = unsafe {
            if flags & libc::O_CREAT != 0 {
                libc::open(path_c.as_ptr(), flags, mode)
            } else {
                libc::open(path_c.as_ptr(), flags)
            }
        };

        if fd < 0 {
            return Err(format!(
                "can't open file {}: {}",
                path,
                io::Error::last_os_error()
            ));
        }

        if let Some(explicit) = options.explicit_fd {
            let new_fd = unsafe { libc::dup2(fd, explicit) };
            unsafe {
                libc::close(fd);
            }
            if new_fd < 0 {
                return Err(format!("can't dup fd to {}", explicit));
            }
            Ok(new_fd)
        } else {
            Ok(fd)
        }
    }

    #[cfg(not(unix))]
    {
        Err("sysopen not supported on this platform".to_string())
    }
}

/// Seek whence options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeekWhence {
    #[default]
    Start,
    Current,
    End,
}

impl SeekWhence {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "start" | "0" => Some(Self::Start),
            "current" | "1" => Some(Self::Current),
            "end" | "2" => Some(Self::End),
            _ => None,
        }
    }

    #[cfg(unix)]
    pub fn to_libc(&self) -> i32 {
        match self {
            Self::Start => libc::SEEK_SET,
            Self::Current => libc::SEEK_CUR,
            Self::End => libc::SEEK_END,
        }
    }
}

/// Options for sysseek
#[derive(Debug, Default)]
pub struct SysseekOptions {
    pub fd: Option<i32>,
    pub whence: SeekWhence,
}

/// Seek on a file descriptor
pub fn sysseek(offset: i64, options: &SysseekOptions) -> Result<i64, String> {
    let fd = options.fd.unwrap_or(0);

    #[cfg(unix)]
    {
        let result = unsafe { libc::lseek(fd, offset, options.whence.to_libc()) };
        if result < 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(result)
        }
    }

    #[cfg(not(unix))]
    {
        Err("sysseek not supported on this platform".to_string())
    }
}

/// Get current position in file descriptor
pub fn systell(fd: i32) -> Result<i64, String> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) };
        if result < 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(result)
        }
    }

    #[cfg(not(unix))]
    {
        Err("systell not supported on this platform".to_string())
    }
}

/// Well-known errno names
pub const ERRNO_NAMES: &[(&str, i32)] = &[
    ("EPERM", 1),
    ("ENOENT", 2),
    ("ESRCH", 3),
    ("EINTR", 4),
    ("EIO", 5),
    ("ENXIO", 6),
    ("E2BIG", 7),
    ("ENOEXEC", 8),
    ("EBADF", 9),
    ("ECHILD", 10),
    ("EAGAIN", 11),
    ("ENOMEM", 12),
    ("EACCES", 13),
    ("EFAULT", 14),
    ("ENOTBLK", 15),
    ("EBUSY", 16),
    ("EEXIST", 17),
    ("EXDEV", 18),
    ("ENODEV", 19),
    ("ENOTDIR", 20),
    ("EISDIR", 21),
    ("EINVAL", 22),
    ("ENFILE", 23),
    ("EMFILE", 24),
    ("ENOTTY", 25),
    ("ETXTBSY", 26),
    ("EFBIG", 27),
    ("ENOSPC", 28),
    ("ESPIPE", 29),
    ("EROFS", 30),
    ("EMLINK", 31),
    ("EPIPE", 32),
    ("EDOM", 33),
    ("ERANGE", 34),
];

/// Get error number from name
pub fn errno_from_name(name: &str) -> Option<i32> {
    ERRNO_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| *e)
}

/// Get error name from number
pub fn errno_to_name(errno: i32) -> Option<&'static str> {
    ERRNO_NAMES
        .iter()
        .find(|(_, e)| *e == errno)
        .map(|(n, _)| *n)
}

/// Get error message for errno
pub fn syserror(errno: i32, prefix: &str) -> String {
    let msg = io::Error::from_raw_os_error(errno).to_string();
    format!("{}{}", prefix, msg)
}

/// Options for zsystem flock
#[derive(Debug, Default)]
pub struct FlockOptions {
    pub cloexec: bool,
    pub read_lock: bool,
    pub timeout: Option<f64>,
    pub interval: Option<f64>,
    pub fd_var: Option<String>,
}

/// Lock a file
#[cfg(unix)]
pub fn flock(path: &str, options: &FlockOptions) -> Result<i32, String> {
    use std::ffi::CString;

    let flags = if options.read_lock {
        libc::O_RDONLY | libc::O_NOCTTY
    } else {
        libc::O_RDWR | libc::O_NOCTTY
    };

    let path_c = CString::new(path).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::open(path_c.as_ptr(), flags) };

    if fd < 0 {
        return Err(format!(
            "failed to open {}: {}",
            path,
            io::Error::last_os_error()
        ));
    }

    if options.cloexec {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }

    let lock_type = if options.read_lock {
        libc::F_RDLCK
    } else {
        libc::F_WRLCK
    };

    let lck = libc::flock {
        l_type: lock_type as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };

    if let Some(timeout) = options.timeout {
        if timeout > 0.0 {
            let start = Instant::now();
            let timeout_duration = Duration::from_secs_f64(timeout);
            let interval = Duration::from_secs_f64(options.interval.unwrap_or(1.0));

            loop {
                let result = unsafe { libc::fcntl(fd, libc::F_SETLK, &lck) };
                if result >= 0 {
                    return Ok(fd);
                }

                let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if errno != libc::EINTR && errno != libc::EACCES && errno != libc::EAGAIN {
                    unsafe {
                        libc::close(fd);
                    }
                    return Err(format!(
                        "failed to lock {}: {}",
                        path,
                        io::Error::last_os_error()
                    ));
                }

                if start.elapsed() >= timeout_duration {
                    unsafe {
                        libc::close(fd);
                    }
                    return Err("timeout waiting for lock".to_string());
                }

                std::thread::sleep(interval.min(timeout_duration - start.elapsed()));
            }
        }
    }

    let cmd = if options.timeout.map_or(true, |t| t != 0.0) {
        libc::F_SETLKW
    } else {
        libc::F_SETLK
    };

    loop {
        let result = unsafe { libc::fcntl(fd, cmd, &lck) };
        if result >= 0 {
            return Ok(fd);
        }

        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EINTR {
            continue;
        }

        unsafe {
            libc::close(fd);
        }
        return Err(format!(
            "failed to lock {}: {}",
            path,
            io::Error::last_os_error()
        ));
    }
}

/// Unlock a file descriptor
#[cfg(unix)]
pub fn funlock(fd: i32) -> Result<(), String> {
    let lck = libc::flock {
        l_type: libc::F_UNLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };

    let result = unsafe { libc::fcntl(fd, libc::F_SETLK, &lck) };
    if result < 0 {
        Err(io::Error::last_os_error().to_string())
    } else {
        unsafe {
            libc::close(fd);
        }
        Ok(())
    }
}

/// Check if a zsystem feature is supported
pub fn zsystem_supports(feature: &str) -> bool {
    match feature {
        "supports" => true,
        "flock" => cfg!(unix),
        _ => false,
    }
}

/// System parameters
pub fn get_sysparams() -> HashMap<String, String> {
    let mut params = HashMap::new();

    #[cfg(unix)]
    {
        params.insert("pid".to_string(), unsafe { libc::getpid() }.to_string());
        params.insert("ppid".to_string(), unsafe { libc::getppid() }.to_string());
    }

    params
}

/// Get list of errno names
pub fn get_errnos() -> Vec<&'static str> {
    ERRNO_NAMES.iter().map(|(n, _)| *n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_open_opt_from_name() {
        assert_eq!(OpenOpt::from_name("cloexec"), Some(OpenOpt::Cloexec));
        assert_eq!(OpenOpt::from_name("O_CREAT"), Some(OpenOpt::Creat));
        assert_eq!(OpenOpt::from_name("truncate"), Some(OpenOpt::Truncate));
        assert_eq!(OpenOpt::from_name("trunc"), Some(OpenOpt::Truncate));
        assert_eq!(OpenOpt::from_name("invalid"), None);
    }

    #[test]
    fn test_seek_whence_from_str() {
        assert_eq!(SeekWhence::from_str("start"), Some(SeekWhence::Start));
        assert_eq!(SeekWhence::from_str("0"), Some(SeekWhence::Start));
        assert_eq!(SeekWhence::from_str("current"), Some(SeekWhence::Current));
        assert_eq!(SeekWhence::from_str("1"), Some(SeekWhence::Current));
        assert_eq!(SeekWhence::from_str("end"), Some(SeekWhence::End));
        assert_eq!(SeekWhence::from_str("2"), Some(SeekWhence::End));
        assert_eq!(SeekWhence::from_str("invalid"), None);
    }

    #[test]
    fn test_errno_from_name() {
        assert_eq!(errno_from_name("EPERM"), Some(1));
        assert_eq!(errno_from_name("ENOENT"), Some(2));
        assert_eq!(errno_from_name("EINVAL"), Some(22));
        assert_eq!(errno_from_name("INVALID"), None);
    }

    #[test]
    fn test_errno_to_name() {
        assert_eq!(errno_to_name(1), Some("EPERM"));
        assert_eq!(errno_to_name(2), Some("ENOENT"));
        assert_eq!(errno_to_name(22), Some("EINVAL"));
        assert_eq!(errno_to_name(999), None);
    }

    #[test]
    fn test_syserror() {
        let msg = syserror(2, "prefix: ");
        assert!(msg.starts_with("prefix: "));
    }

    #[test]
    fn test_zsystem_supports() {
        assert!(zsystem_supports("supports"));
        assert!(!zsystem_supports("unknown"));
        #[cfg(unix)]
        assert!(zsystem_supports("flock"));
    }

    #[test]
    fn test_get_sysparams() {
        let params = get_sysparams();
        assert!(params.contains_key("pid"));
        assert!(params.contains_key("ppid"));
    }

    #[test]
    fn test_get_errnos() {
        let errnos = get_errnos();
        assert!(errnos.contains(&"EPERM"));
        assert!(errnos.contains(&"ENOENT"));
        assert!(errnos.contains(&"EINVAL"));
    }

    #[test]
    #[cfg(unix)]
    fn test_sysopen_and_close() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        let options = SysopenOptions {
            write: true,
            options: vec![OpenOpt::Creat],
            mode: Some(0o644),
            ..Default::default()
        };

        let fd = sysopen(file_path.to_str().unwrap(), &options).unwrap();
        assert!(fd >= 0);

        unsafe {
            libc::close(fd);
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_syswrite_sysread() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let fd = {
            use std::ffi::CString;
            let path_c = CString::new(file_path.to_str().unwrap()).unwrap();
            unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY) }
        };

        let options = SysreadOptions {
            input_fd: Some(fd),
            bufsize: Some(100),
            ..Default::default()
        };

        let (result, data, count) = sysread(&options);
        unsafe {
            libc::close(fd);
        }

        assert_eq!(result, SysreadResult::Success);
        assert_eq!(count, 11);
        assert_eq!(data.unwrap(), b"hello world");
    }

    #[test]
    #[cfg(unix)]
    fn test_sysseek_systell() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let fd = {
            use std::ffi::CString;
            let path_c = CString::new(file_path.to_str().unwrap()).unwrap();
            unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY) }
        };

        let options = SysseekOptions {
            fd: Some(fd),
            whence: SeekWhence::Start,
        };

        let pos = sysseek(5, &options).unwrap();
        assert_eq!(pos, 5);

        let current = systell(fd).unwrap();
        assert_eq!(current, 5);

        unsafe {
            libc::close(fd);
        }
    }
}
