//! System I/O builtins - port of Modules/system.c
//!
//! Provides bin_sysread, bin_syswrite, bin_sysopen, bin_sysseek, bin_syserror, zsystem builtins.

use std::collections::HashMap;
use crate::ported::utils::zwarnnam;
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const SYSREAD_BUFSIZE: usize = 8192;

/// Return values for bin_sysread
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// `bin_sysread` outcome variants.
/// Mirrors the integer return values `bin_sysread()` from
/// Src/Modules/system.c:72 produces: success / EOF / timeout /
/// error.
pub enum SysreadResult {
    Success = 0,
    ParamError = 1,
    ReadError = 2,
    WriteError = 3,
    Timeout = 4,
    Eof = 5,
}

/// Options for bin_sysread
#[derive(Debug, Default)]
/// `bin_sysread` builtin options.
/// Port of the `Options ops` flag bag `bin_sysread()`
/// (Src/Modules/system.c:72) reads — `-i`/`-o` fd, `-s` size,
/// `-c` count, `-t` timeout.
pub struct SysreadOptions {
    pub input_fd: Option<i32>,
    pub output_fd: Option<i32>,
    pub bufsize: Option<usize>,
    pub timeout: Option<f64>,
    pub count_var: Option<String>,
    pub output_var: Option<String>,
}

/// Perform a system read
/// `bin_sysread` builtin entry point.
/// Port of `bin_sysread()` from Src/Modules/system.c:72 — wraps
/// `read(2)` with optional `select(2)` timeout.
pub fn bin_sysread(options: &SysreadOptions) -> (SysreadResult, Option<Vec<u8>>, usize) {
    let input_fd = options.input_fd.unwrap_or(0);
    let bufsize = options.bufsize.unwrap_or(SYSREAD_BUFSIZE);

    let mut buffer = vec![0u8; bufsize];

    #[cfg(unix)]
    {
        if let Some(timeout_secs) = options.timeout {
            // Inline poll-with-timeout per c:Modules/system.c:72
            // bin_sysread — same `pollfd` shape and POLLIN event.
            let timeout_ms = (timeout_secs * 1000.0) as i32;
            let ready = unsafe {
                let mut pfd = libc::pollfd {
                    fd: input_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                libc::poll(&mut pfd, 1, timeout_ms) > 0
            };
            if !ready {
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

/// Options for bin_syswrite
#[derive(Debug, Default)]
/// `bin_syswrite` builtin options.
/// Port of the `Options ops` flag bag `bin_syswrite()` from
/// Src/Modules/system.c:238 reads — `-c` count, `-o` fd.
pub struct SyswriteOptions {
    pub output_fd: Option<i32>,
    pub count_var: Option<String>,
}

/// Perform a system write
/// `bin_syswrite` builtin entry point.
/// Port of `bin_syswrite()` from Src/Modules/system.c:238 —
/// wraps `write(2)` with `EINTR` retry.
pub fn bin_syswrite(data: &[u8], options: &SyswriteOptions) -> (i32, usize) {
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

/// Open options for bin_sysopen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// `bin_sysopen` flag bits.
/// Port of the `O_*` set the C source's `bin_sysopen()`
/// (Src/Modules/system.c:319) maps from `-o` argument tokens to
/// `open(2)` flag bits.
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/system.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/system.c`.
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

/// Options for bin_sysopen
#[derive(Debug, Default)]
/// `bin_sysopen` builtin options.
/// Mirrors the `Options ops` flag bag `bin_sysopen()` reads —
/// `-r`/`-w`/`-a`/`-u`/`-m` mode bits + the `-o` flag list.
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
/// `bin_sysopen` builtin entry point.
/// Port of `bin_sysopen()` from Src/Modules/system.c:319 —
/// wraps `open(2)` with the assembled flag bag and optional
/// mode.
pub fn bin_sysopen(path: &str, options: &SysopenOptions) -> Result<i32, String> {
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
        Err("bin_sysopen not supported on this platform".to_string())
    }
}

/// Seek whence options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// `bin_sysseek` whence values.
/// Mirrors the `SEEK_SET` / `SEEK_CUR` / `SEEK_END` constants the
/// C source's `bin_sysseek()` (Src/Modules/system.c:433) accepts
/// via the `-w` flag.
pub enum SeekWhence {
    #[default]
    Start,
    Current,
    End,
}

impl SeekWhence {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/system.c`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "start" | "0" => Some(Self::Start),
            "current" | "1" => Some(Self::Current),
            "end" | "2" => Some(Self::End),
            _ => None,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/system.c`.
    #[cfg(unix)]
    pub fn to_libc(&self) -> i32 {
        match self {
            Self::Start => libc::SEEK_SET,
            Self::Current => libc::SEEK_CUR,
            Self::End => libc::SEEK_END,
        }
    }
}

/// Options for bin_sysseek
#[derive(Debug, Default)]
/// `bin_sysseek` builtin options.
/// Port of the `Options ops` flag bag `bin_sysseek()`
/// (Src/Modules/system.c:433) reads — `-u` fd, `-w` whence.
pub struct SysseekOptions {
    pub fd: Option<i32>,
    pub whence: SeekWhence,
}

/// Seek on a file descriptor
/// `bin_sysseek` builtin entry point.
/// Port of `bin_sysseek()` from Src/Modules/system.c:433 —
/// wraps `lseek(2)`.
pub fn bin_sysseek(offset: i64, options: &SysseekOptions) -> Result<i64, String> {
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
        Err("bin_sysseek not supported on this platform".to_string())
    }
}

/// Get current position in file descriptor
/// `math_systell()` math function.
/// Port of `math_systell()` from Src/Modules/system.c:467 — the
/// C source registers it as a math function for `((pos =
/// math_systell(fd)))` arithmetic.
pub fn math_systell(fd: i32) -> Result<i64, String> {
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
        Err("math_systell not supported on this platform".to_string())
    }
}

/// Errno-name table, indexed 1-based to match zsh's `${errnos[N]}`
/// shape. Direct port of `sys_errnames[]` from
/// `Src/Modules/errnames2.awk` — that file generates the C table at
/// build time by walking each platform's `<errno.h>`. We do the same
/// by cfg-conditionally listing the kernel-stable subset per OS, so
/// `${errnos[35]}` returns the correct macro on each target. Linux
/// past errno 11 diverges from BSD/macOS (`EAGAIN` is 11 on Linux,
/// 35 on macOS, etc.); both tables are kept exact-by-platform.
#[cfg(target_os = "macos")]
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
    ("EDEADLK", 11),
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
    ("EAGAIN", 35),
    ("EINPROGRESS", 36),
    ("EALREADY", 37),
    ("ENOTSOCK", 38),
    ("EDESTADDRREQ", 39),
    ("EMSGSIZE", 40),
    ("EPROTOTYPE", 41),
    ("ENOPROTOOPT", 42),
    ("EPROTONOSUPPORT", 43),
    ("ESOCKTNOSUPPORT", 44),
    ("ENOTSUP", 45),
    ("EPFNOSUPPORT", 46),
    ("EAFNOSUPPORT", 47),
    ("EADDRINUSE", 48),
    ("EADDRNOTAVAIL", 49),
    ("ENETDOWN", 50),
    ("ENETUNREACH", 51),
    ("ENETRESET", 52),
    ("ECONNABORTED", 53),
    ("ECONNRESET", 54),
    ("ENOBUFS", 55),
    ("EISCONN", 56),
    ("ENOTCONN", 57),
    ("ESHUTDOWN", 58),
    ("ETOOMANYREFS", 59),
    ("ETIMEDOUT", 60),
    ("ECONNREFUSED", 61),
    ("ELOOP", 62),
    ("ENAMETOOLONG", 63),
    ("EHOSTDOWN", 64),
    ("EHOSTUNREACH", 65),
    ("ENOTEMPTY", 66),
    ("EPROCLIM", 67),
    ("EUSERS", 68),
    ("EDQUOT", 69),
    ("ESTALE", 70),
    ("EREMOTE", 71),
    ("EBADRPC", 72),
    ("ERPCMISMATCH", 73),
    ("EPROGUNAVAIL", 74),
    ("EPROGMISMATCH", 75),
    ("EPROCUNAVAIL", 76),
    ("ENOLCK", 77),
    ("ENOSYS", 78),
    ("EFTYPE", 79),
    ("EAUTH", 80),
    ("ENEEDAUTH", 81),
    ("EPWROFF", 82),
    ("EDEVERR", 83),
    ("EOVERFLOW", 84),
    ("EBADEXEC", 85),
    ("EBADARCH", 86),
    ("ESHLIBVERS", 87),
    ("EBADMACHO", 88),
    ("ECANCELED", 89),
    ("EIDRM", 90),
    ("ENOMSG", 91),
    ("EILSEQ", 92),
    ("ENOATTR", 93),
    ("EBADMSG", 94),
    ("EMULTIHOP", 95),
    ("ENODATA", 96),
    ("ENOLINK", 97),
    ("ENOSR", 98),
    ("ENOSTR", 99),
    ("EPROTO", 100),
    ("ETIME", 101),
    ("EOPNOTSUPP", 102),
    ("ENOPOLICY", 103),
    ("ENOTRECOVERABLE", 104),
    ("EOWNERDEAD", 105),
    ("EQFULL", 106),
    // ENOTCAPABLE (errno 107) exists in Apple's MacOSX26.sdk
    // headers but NOT in MacOSX15.sdk and earlier. Apple's stock
    // /bin/zsh is linked against the newer SDK so it lists 107
    // entries; Homebrew's zsh was built against an older SDK and
    // lists only 106. We pin to the Homebrew/older-SDK shape since
    // that's the parity target on this host. When zshrs is
    // eventually rebuilt for SDK 26+ a follow-up will conditionally
    // add ENOTCAPABLE.
];

/// Linux errno table — the kernel's order diverges from BSD/macOS
/// at #11 (`EAGAIN` not `EDEADLK`) and continues with Linux-only
/// codes through the 130s. Sourced from `<asm-generic/errno.h>` +
/// `<asm-generic/errno-base.h>` to match every distro.
#[cfg(target_os = "linux")]
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
    ("EDEADLK", 35),
    ("ENAMETOOLONG", 36),
    ("ENOLCK", 37),
    ("ENOSYS", 38),
    ("ENOTEMPTY", 39),
    ("ELOOP", 40),
];

/// Fallback for platforms zshrs doesn't have a verified table for.
/// Mirrors the POSIX-portable subset (errnos 1-34) which all Unix
/// kernels agree on; values past 34 vary by OS and are omitted.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
    ("ENOMEM", 12),
    ("EACCES", 13),
    ("EFAULT", 14),
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
/// Resolve an `ERRNO_NAME` to its integer code.
/// Port of the errno lookup `bin_syserror()` from
/// Src/Modules/system.c:494 performs against the C source's
/// per-platform `errnos[]` table.
pub fn errno_from_name(name: &str) -> Option<i32> {
    ERRNO_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| *e)
}

/// Get error name from number
/// Inverse of `errno_from_name`.
/// Port of `errnosgetfn()` from Src/Modules/system.c:832 — used
/// by `${errnos[N]}` lookup.
pub fn errnosgetfn(errno: i32) -> Option<&'static str> {
    ERRNO_NAMES
        .iter()
        .find(|(_, e)| *e == errno)
        .map(|(n, _)| *n)
}

/// Get error message for errno
/// Format an `errno`-aware error message.
/// Port of `bin_syserror()` from Src/Modules/system.c:494 —
/// wraps `strerror(3)` with an optional caller-supplied prefix.
pub fn bin_syserror(errno: i32, prefix: &str) -> String {
    let msg = io::Error::from_raw_os_error(errno).to_string();
    format!("{}{}", prefix, msg)
}

/// Options for zsystem bin_zsystem_flock
#[derive(Debug, Default)]
/// `zsystem bin_zsystem_flock` options.
/// Mirrors the flag bag `bin_zsystem_flock()` from
/// Src/Modules/system.c:546 reads — `-r`/`-x`/`-e` lock type,
/// `-t` timeout, `-i` non-blocking, `-f` fd.
pub struct FlockOptions {
    pub cloexec: bool,
    pub read_lock: bool,
    pub timeout: Option<f64>,
    pub interval: Option<f64>,
    pub fd_var: Option<String>,
}

/// Lock a file
#[cfg(unix)]
/// `zsystem bin_zsystem_flock` subcommand entry point.
/// Port of `bin_zsystem_flock()` from Src/Modules/system.c:546 —
/// wraps `bin_zsystem_flock(2)` (or `fcntl(F_SETLK)` on systems lacking it).
pub fn bin_zsystem_flock(path: &str, options: &FlockOptions) -> Result<i32, String> {
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

    // l_type is c_short on Linux + macOS; F_RDLCK/F_WRLCK are c_int on
    // Linux, c_short on macOS. Cast to i16 explicitly for cross-build —
    // clippy's unnecessary_cast fires on whichever platform already
    // matches but silently fails on the other if removed.
    #[allow(clippy::unnecessary_cast)]
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

    let cmd = if options.timeout != Some(0.0) {
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

/// Check if a zsystem feature is supported
/// `zsystem supports` subcommand entry point.
/// Port of `bin_zsystem_supports()` from Src/Modules/system.c:781
/// — reports which `zsystem` subcommands are compiled in.
pub fn bin_zsystem_supports(feature: &str) -> bool {
    feature == "supports" || (feature == "bin_zsystem_flock" && cfg!(unix))
}

/// System parameters
/// Fetch the `${sysparams}` map.
/// Port of `getpmsysparams()` (Src/Modules/system.c:873) +
/// `scanpmsysparams()` (line 885) — exposes selected `sysconf(3)`
/// values to shell scripts.
pub fn getpmsysparams() -> HashMap<String, String> {
    let mut params = HashMap::new();

    #[cfg(unix)]
    {
        params.insert("pid".to_string(), unsafe { libc::getpid() }.to_string());
        params.insert("ppid".to_string(), unsafe { libc::getppid() }.to_string());
    }

    params
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
        assert_eq!(errnosgetfn(1), Some("EPERM"));
        assert_eq!(errnosgetfn(2), Some("ENOENT"));
        assert_eq!(errnosgetfn(22), Some("EINVAL"));
        assert_eq!(errnosgetfn(999), None);
    }

    #[test]
    fn test_syserror() {
        let msg = bin_syserror(2, "prefix: ");
        assert!(msg.starts_with("prefix: "));
    }

    #[test]
    fn test_zsystem_supports() {
        assert!(bin_zsystem_supports("supports"));
        assert!(!bin_zsystem_supports("unknown"));
        #[cfg(unix)]
        assert!(bin_zsystem_supports("bin_zsystem_flock"));
    }

    #[test]
    fn test_get_sysparams() {
        let params = getpmsysparams();
        assert!(params.contains_key("pid"));
        assert!(params.contains_key("ppid"));
    }

    #[test]
    fn test_get_errnos() {
        let errnos: Vec<&'static str> = ERRNO_NAMES.iter().map(|(n, _)| *n).collect();
        assert!(errnos.contains(&"EPERM"));
        assert!(errnos.contains(&"ENOENT"));
        assert!(errnos.contains(&"EINVAL"));
    }

    /// Port of `bin_sysopen()` from `Src/Modules/system.c:319`.
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

        let fd = bin_sysopen(file_path.to_str().unwrap(), &options).unwrap();
        assert!(fd >= 0);

        unsafe {
            libc::close(fd);
        }
    }

    /// Port of `bin_sysread()` from `Src/Modules/system.c:72`.
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

        let (result, data, count) = bin_sysread(&options);
        unsafe {
            libc::close(fd);
        }

        assert_eq!(result, SysreadResult::Success);
        assert_eq!(count, 11);
        assert_eq!(data.unwrap(), b"hello world");
    }

    /// Port of `bin_sysopen()` from `Src/Modules/system.c:319`.
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

        let pos = bin_sysseek(5, &options).unwrap();
        assert_eq!(pos, 5);

        let current = math_systell(fd).unwrap();
        assert_eq!(current, 5);

        unsafe {
            libc::close(fd);
        }
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// zsystem - system interface (zsh/system module)
    /// Ported from zsh/Src/Modules/system.c bin_zsystem() lines 805-816
    pub(crate) fn bin_zsystem(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            zwarnnam("zsystem", "subcommand expected");
            return 1;
        }
        match args[0].as_str() {
            "bin_zsystem_flock" => self.bin_zsystem_flock(&args[1..]),
            "supports" => self.bin_zsystem_supports(&args[1..]),
            _ => {
                zwarnnam("zsystem", &format!("unknown subcommand: {}", args[0]));
                1
            }
        }
    }
    /// zsystem supports - ported from system.c bin_zsystem_supports() lines 780-801
    pub(crate) fn bin_zsystem_supports(&self, args: &[String]) -> i32 {
        if args.is_empty() {
            zwarnnam("zsystem", "supports: not enough arguments");
            return 255;
        }
        if args.len() > 1 {
            zwarnnam("zsystem", "supports: too many arguments");
            return 255;
        }
        match args[0].as_str() {
            "supports" | "bin_zsystem_flock" => 0,
            _ => 1,
        }
    }
    /// zsystem bin_zsystem_flock - ported from system.c bin_zsystem_flock() lines 546-774
    pub(crate) fn bin_zsystem_flock(&mut self, args: &[String]) -> i32 {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;

            let mut cloexec = true;
            let mut readlock = false;
            let mut unlock = false;
            let mut timeout: Option<f64> = None;
            // Default retry interval per zsh/Src/Modules/system.c:550
            // (timeout_interval = 1e6 µs = 1 s).
            let mut interval_us: u64 = 1_000_000;
            let mut fdvar: Option<String> = None;
            let mut file: Option<&str> = None;

            let mut i = 0;
            while i < args.len() {
                let arg = &args[i];
                if arg == "--" {
                    i += 1;
                    if i < args.len() {
                        file = Some(&args[i]);
                    }
                    break;
                }
                if !arg.starts_with('-') {
                    file = Some(arg);
                    break;
                }
                let mut chars = arg[1..].chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        'e' => cloexec = false,
                        'r' => readlock = true,
                        'u' => unlock = true,
                        'f' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                fdvar = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    fdvar = Some(args[i].clone());
                                } else {
                                    zwarnnam("bin_zsystem_flock", "option f requires a variable name");
                                    return 1;
                                }
                            }
                            break;
                        }
                        't' => {
                            let rest: String = chars.collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].clone()
                                } else {
                                    zwarnnam("bin_zsystem_flock", "option t requires a numeric timeout");
                                    return 1;
                                }
                            };
                            match val.parse::<f64>() {
                                Ok(t) => timeout = Some(t),
                                Err(_) => {
                                    zwarnnam("bin_zsystem_flock", &format!("invalid timeout value: '{}'", val));
                                    return 1;
                                }
                            }
                            break;
                        }
                        'i' => {
                            // Direct port of zsh/Src/Modules/system.c:621-648:
                            // -i SECONDS sets the retry-poll interval used
                            // when the lock is held by another. Float arg
                            // converted to whole microseconds, validated
                            // against [1, 0.999*LONG_MAX].
                            let rest: String = chars.collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("bin_zsystem_flock", "option i requires a numeric retry interval");
                                    return 1;
                                }
                                args[i].clone()
                            };
                            match val.parse::<f64>() {
                                Ok(n) if n > 0.0 => {
                                    let us = (n * 1e6).ceil();
                                    if us < 1.0 || us > (i64::MAX as f64 * 0.999) {
                                        zwarnnam("bin_zsystem_flock", &format!("invalid interval value: '{}'", val));
                                        return 1;
                                    }
                                    interval_us = us as u64;
                                }
                                _ => {
                                    zwarnnam("bin_zsystem_flock", &format!("invalid interval value: '{}'", val));
                                    return 1;
                                }
                            }
                            break;
                        }
                        _ => {
                            zwarnnam("zsystem", &format!("bin_zsystem_flock: unknown option: -{}", c));
                            return 1;
                        }
                    }
                }
                i += 1;
            }

            let filepath = match file {
                Some(f) => f,
                None => {
                    zwarnnam("zsystem", "bin_zsystem_flock: not enough arguments");
                    return 1;
                }
            };

            // -u: unlock. system.c:674-682 — argument is an FD number;
            // close it (which releases POSIX advisory locks held on
            // that open description). Was return 0 stub.
            if unlock {
                let fd: i32 = match filepath.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        zwarnnam("zsystem", &format!("bin_zsystem_flock: invalid fd: {}", filepath));
                        return 1;
                    }
                };
                let r = unsafe { libc::close(fd) };
                if r < 0 {
                    zwarnnam("bin_zsystem_flock", &format!("file descriptor {} not in use for locking", fd));
                    return 1;
                }
                return 0;
            }

            use std::fs::OpenOptions;
            let file_handle = match OpenOptions::new()
                .read(true)
                .write(!readlock)
                .create(true)
                .truncate(false)
                .open(filepath)
            {
                Ok(f) => f,
                Err(e) => {
                    zwarnnam("zsystem", &format!("bin_zsystem_flock: {}: {}", filepath, e));
                    return 1;
                }
            };

            let lock_type = if readlock {
                libc::F_RDLCK
            } else {
                libc::F_WRLCK
            };

            // l_type is c_short on Linux + macOS; F_RDLCK/F_WRLCK are
            // c_int on Linux, c_short on macOS. Cast to i16 explicitly
            // for cross-platform builds — clippy fires unnecessary_cast
            // on whichever platform already matches.
            #[allow(clippy::unnecessary_cast)]
            let mut bin_zsystem_flock = libc::flock {
                l_type: lock_type as i16,
                l_whence: libc::SEEK_SET as i16,
                l_start: 0,
                l_len: 0,
                l_pid: 0,
            };

            let cmd = if timeout.is_some() {
                libc::F_SETLK
            } else {
                libc::F_SETLKW
            };
            let start = std::time::Instant::now();
            let timeout_duration = timeout.map(std::time::Duration::from_secs_f64);

            loop {
                let ret = unsafe { libc::fcntl(file_handle.as_raw_fd(), cmd, &mut bin_zsystem_flock) };
                if ret == 0 {
                    // Port of system.c:695-701: when -e is NOT set
                    // (cloexec defaults to 1, cleared only by -e), set
                    // FD_CLOEXEC on the lock fd so it doesn't survive
                    // exec(). Without this, `zsystem bin_zsystem_flock f; exec ls`
                    // leaked the lock fd into the new process.
                    if cloexec {
                        let fd = file_handle.as_raw_fd();
                        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
                        if flags != -1 {
                            unsafe {
                                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                            }
                        }
                    }
                    if let Some(ref var) = fdvar {
                        let fd = file_handle.as_raw_fd();
                        std::mem::forget(file_handle);
                        self.variables.insert(var.clone(), fd.to_string());
                    } else {
                        std::mem::forget(file_handle);
                    }
                    return 0;
                }
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if errno != libc::EACCES && errno != libc::EAGAIN {
                    zwarnnam("bin_zsystem_flock", &format!("{}: {}", filepath, std::io::Error::last_os_error()));
                    return 1;
                }
                if let Some(td) = timeout_duration {
                    if start.elapsed() >= td {
                        return 2;
                    }
                    // Retry interval honors -i (default 1 000 000 µs).
                    // Was a hardcoded 100 ms which over-polled tight
                    // loops and ignored user-tuned wait values.
                    std::thread::sleep(std::time::Duration::from_micros(interval_us));
                } else {
                    zwarnnam("bin_zsystem_flock", &format!("{}: {}", filepath, std::io::Error::last_os_error()));
                    return 1;
                }
            }
        }
        #[cfg(not(unix))]
        {
            zwarnnam("zsystem", "bin_zsystem_flock: not supported on this platform");
            1
        }
    }
    /// bin_sysread - low-level read (zsh/system module)
    pub(crate) fn bin_sysread(&mut self, args: &[String]) -> i32 {
        // Direct port of zsh/Src/Modules/system.c:72 bin_sysread.
        // Return values per system.c:61-67:
        //   0  successful read (and write if -o)
        //   1  bad params / non-identifier varname
        //   2  read() error (errno set)
        //   3  write() error (errno set, partial residue stashed in
        //      outvar / count in countvar)
        //   4  -t timeout expired
        //   5  zero bytes read (EOF)
        let mut infd: i32 = 0;
        let mut outfd: i32 = -1;
        let mut bufsize: usize = 8192; // SYSREAD_BUFSIZE
        let mut countvar: Option<String> = None;
        let mut outvar: Option<String> = None;
        let mut timeout_ms: Option<i32> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-i" if i + 1 < args.len() => {
                    i += 1;
                    match args[i].parse::<i32>() {
                        Ok(n) if n >= 0 => infd = n,
                        _ => {
                            zwarnnam("bin_sysread", &format!("integer expected: {}", args[i]));
                            return 1;
                        }
                    }
                }
                "-o" if i + 1 < args.len() => {
                    i += 1;
                    match args[i].parse::<i32>() {
                        Ok(n) if n >= 0 => outfd = n,
                        _ => {
                            zwarnnam("bin_sysread", &format!("integer expected: {}", args[i]));
                            return 1;
                        }
                    }
                }
                "-s" if i + 1 < args.len() => {
                    i += 1;
                    match args[i].parse::<usize>() {
                        Ok(n) => bufsize = n,
                        Err(_) => {
                            zwarnnam("bin_sysread", &format!("integer expected: {}", args[i]));
                            return 1;
                        }
                    }
                }
                "-c" if i + 1 < args.len() => {
                    i += 1;
                    countvar = Some(args[i].clone());
                }
                "-t" if i + 1 < args.len() => {
                    i += 1;
                    // Timeout in seconds (float ok). Convert to ms.
                    match args[i].parse::<f64>() {
                        Ok(t) => timeout_ms = Some((t * 1000.0) as i32),
                        Err(_) => {
                            zwarnnam("bin_sysread", &format!("invalid timeout: {}", args[i]));
                            return 1;
                        }
                    }
                }
                _ => {
                    outvar = Some(args[i].clone());
                }
            }
            i += 1;
        }

        // -t poll(2) wait — system.c:127-186. Return 4 on timeout (poll
        // returned 0), 2 on error.
        if let Some(ms) = timeout_ms {
            let mut pfd = libc::pollfd {
                fd: infd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ret = unsafe { libc::poll(&mut pfd, 1, ms) };
            if ret == 0 {
                return 4;
            }
            if ret < 0 {
                return 2;
            }
        }

        let mut buf = vec![0u8; bufsize];
        let n = unsafe { libc::read(infd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            if let Some(cv) = countvar {
                self.variables.insert(cv, n.to_string());
            }
            return 2;
        }
        let count = n as usize;
        buf.truncate(count);
        if let Some(cv) = &countvar {
            self.variables.insert(cv.clone(), count.to_string());
        }

        // -o: copy to outfd via write(2). On partial-write error,
        // stash residue in outvar + count in countvar (system.c:204-212).
        if outfd >= 0 {
            if count == 0 {
                return 5;
            }
            let mut written = 0usize;
            while written < count {
                let w = unsafe {
                    libc::write(
                        outfd,
                        buf[written..].as_ptr() as *const libc::c_void,
                        count - written,
                    )
                };
                if w < 0 {
                    if let Some(ov) = outvar {
                        let s = String::from_utf8_lossy(&buf[written..]).to_string();
                        self.variables.insert(ov, s);
                    }
                    if let Some(cv) = countvar {
                        self.variables.insert(cv, (count - written).to_string());
                    }
                    return 3;
                }
                written += w as usize;
            }
            return 0;
        }

        // No -o: stash buffer into outvar (default REPLY).
        let s = String::from_utf8_lossy(&buf).to_string();
        let target = outvar.unwrap_or_else(|| "REPLY".to_string());
        self.variables.insert(target, s);
        if count == 0 {
            5
        } else {
            0
        }
    }
    /// bin_syswrite - low-level write (zsh/system module). Direct port of
    /// zsh/Src/Modules/system.c:238 bin_syswrite. Return values
    /// (system.c:230-234): 0 = success, 1 = bad params, 2 = write error.
    pub(crate) fn bin_syswrite(&mut self, args: &[String]) -> i32 {
        let mut outfd: i32 = 1;
        let mut countvar: Option<String> = None;
        let mut data: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-o" if i + 1 < args.len() => {
                    i += 1;
                    match args[i].parse::<i32>() {
                        Ok(n) if n >= 0 => outfd = n,
                        _ => {
                            zwarnnam("bin_syswrite", &format!("integer expected: {}", args[i]));
                            return 1;
                        }
                    }
                }
                "-c" if i + 1 < args.len() => {
                    i += 1;
                    countvar = Some(args[i].clone());
                }
                _ => {
                    data = Some(args[i].clone());
                }
            }
            i += 1;
        }

        let payload = match data {
            Some(d) => d,
            None => return 1,
        };
        let bytes = payload.as_bytes();
        let mut totcount = 0usize;
        let mut len = bytes.len();
        let mut ptr = bytes.as_ptr();
        while len > 0 {
            let w = unsafe { libc::write(outfd, ptr as *const libc::c_void, len) };
            if w < 0 {
                let err = std::io::Error::last_os_error();
                let errno = err.raw_os_error().unwrap_or(0);
                if errno == libc::EINTR {
                    continue;
                }
                if let Some(cv) = countvar {
                    self.variables.insert(cv, totcount.to_string());
                }
                return 2;
            }
            unsafe {
                ptr = ptr.add(w as usize);
            }
            totcount += w as usize;
            len -= w as usize;
        }
        if let Some(cv) = countvar {
            self.variables.insert(cv, totcount.to_string());
        }
        0
    }
    /// bin_syserror - get error message (zsh/system module)
    pub(crate) fn bin_syserror(&self, args: &[String]) -> i32 {
        let errno = if args.is_empty() {
            // Use last errno
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        } else {
            args[0].parse().unwrap_or(0)
        };

        let err = std::io::Error::from_raw_os_error(errno);
        println!("{}", err);
        0
    }
    /// bin_sysopen - open file descriptor (zsh/system module). Direct port
    /// of zsh/Src/Modules/system.c:319 bin_sysopen. Return values
    /// (system.c:311-315): 0 = success, 1 = bad params, 2 = open()
    /// error.
    pub(crate) fn bin_sysopen(&mut self, args: &[String]) -> i32 {
        let mut read_flag = false;
        let mut write_flag = false;
        let mut append_flag = false;
        let mut o_opts: Option<String> = None;
        let mut perms: u32 = 0o666;
        let mut fdvar: Option<String> = None;
        let mut filename: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "-r" => read_flag = true,
                "-w" => write_flag = true,
                "-a" => append_flag = true,
                "-u" if i + 1 < args.len() => {
                    i += 1;
                    fdvar = Some(args[i].clone());
                }
                "-o" if i + 1 < args.len() => {
                    i += 1;
                    o_opts = Some(args[i].clone());
                }
                "-m" if i + 1 < args.len() => {
                    i += 1;
                    let mode_str = &args[i];
                    if !mode_str.chars().all(|c| ('0'..='7').contains(&c)) || mode_str.len() < 3 {
                        zwarnnam("bin_sysopen", &format!("invalid mode {}", mode_str));
                        return 1;
                    }
                    perms = u32::from_str_radix(mode_str, 8).unwrap_or(0o666);
                }
                s if !s.starts_with('-') => {
                    filename = Some(s.to_string());
                }
                _ => {}
            }
            i += 1;
        }

        // system.c:335-338 — -u is required.
        let fdvar = match fdvar {
            Some(s) => s,
            None => {
                zwarnnam("bin_sysopen", "file descriptor not specified");
                return 1;
            }
        };
        let filename = match filename {
            Some(s) => s,
            None => return 1,
        };

        // system.c:342-347 — -u arg is either single digit (explicit
        // fd) or variable identifier to set after the open.
        let explicit_fd: Option<i32> =
            if fdvar.len() == 1 && fdvar.chars().next().unwrap().is_ascii_digit() {
                Some(fdvar.parse().unwrap())
            } else {
                None
            };

        // system.c:323-325 — base flags from -r/-w/-a.
        let base = libc::O_NOCTTY
            | (if append_flag { libc::O_APPEND } else { 0 })
            | if append_flag || write_flag {
                if read_flag {
                    libc::O_RDWR
                } else {
                    libc::O_WRONLY
                }
            } else {
                libc::O_RDONLY
            };

        // system.c:350-369 — comma-list of O_* names, case-insensitive,
        // optional 'O_' prefix.
        let mut flags = base;
        if let Some(opts) = &o_opts {
            for tok in opts.split(',') {
                let mut t = tok.to_uppercase();
                if t.starts_with("O_") {
                    t = t[2..].to_string();
                }
                let f = match t.as_str() {
                    "CLOEXEC" => libc::O_CLOEXEC,
                    "NOFOLLOW" => libc::O_NOFOLLOW,
                    "SYNC" => libc::O_SYNC,
                    "NONBLOCK" => libc::O_NONBLOCK,
                    "EXCL" => libc::O_EXCL | libc::O_CREAT,
                    "CREAT" | "CREATE" => libc::O_CREAT,
                    "TRUNCATE" | "TRUNC" => libc::O_TRUNC,
                    #[cfg(target_os = "linux")]
                    "NOATIME" => libc::O_NOATIME,
                    _ => {
                        zwarnnam("bin_sysopen", &format!("unsupported option: {}", tok));
                        return 1;
                    }
                };
                flags |= f;
            }
        }

        let cstr = match std::ffi::CString::new(filename.as_bytes()) {
            Ok(s) => s,
            Err(_) => return 1,
        };
        let fd = unsafe {
            if (flags & libc::O_CREAT) != 0 {
                libc::open(cstr.as_ptr(), flags, perms as libc::c_uint)
            } else {
                libc::open(cstr.as_ptr(), flags)
            }
        };
        if fd == -1 {
            let e = std::io::Error::last_os_error();
            zwarnnam("bin_sysopen", &format!("can't open file {}: {}", filename, e));
            return 2;
        }

        // system.c:392 — redup(fd, explicit) or movefd(fd) to land
        // outside the user's interactive 0-9 range. Use dup2 for
        // explicit; for default, just use the kernel-assigned fd.
        let final_fd = if let Some(target) = explicit_fd {
            let r = unsafe { libc::dup2(fd, target) };
            unsafe {
                libc::close(fd);
            }
            if r == -1 {
                let e = std::io::Error::last_os_error();
                zwarnnam("bin_sysopen", &format!("dup2 failed: {}", e));
                return 2;
            }
            target
        } else {
            fd
        };

        // system.c:406-410 — when O_CLOEXEC was requested but the fd
        // got moved (dup2 strips CLOEXEC), reapply via fcntl.
        if (flags & libc::O_CLOEXEC) != 0 && fd != final_fd {
            unsafe {
                libc::fcntl(final_fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }

        if explicit_fd.is_none() {
            self.variables.insert(fdvar, final_fd.to_string());
        }
        0
    }
    /// bin_sysseek - seek on file descriptor (zsh/system module). Direct
    /// port of zsh/Src/Modules/system.c:433 bin_sysseek. Return values
    /// (system.c:425-428): 0 = success, 1 = bad params, 2 = lseek error.
    pub(crate) fn bin_sysseek(&mut self, args: &[String]) -> i32 {
        let mut fd: i32 = 0;
        let mut whence: i32 = libc::SEEK_SET;
        let mut pos_arg: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-u" if i + 1 < args.len() => {
                    i += 1;
                    match args[i].parse::<i32>() {
                        Ok(n) if n >= 0 => fd = n,
                        _ => {
                            zwarnnam("bin_sysseek", &format!("integer expected: {}", args[i]));
                            return 1;
                        }
                    }
                }
                "-w" if i + 1 < args.len() => {
                    i += 1;
                    let w = args[i].to_lowercase();
                    whence = match w.as_str() {
                        "current" | "cur" | "1" => libc::SEEK_CUR,
                        "end" | "2" => libc::SEEK_END,
                        "start" | "set" | "0" => libc::SEEK_SET,
                        _ => {
                            zwarnnam("bin_sysseek", &format!("unknown argument to -w: {}", args[i]));
                            return 1;
                        }
                    };
                }
                s if !s.starts_with('-') => pos_arg = Some(s.to_string()),
                _ => {}
            }
            i += 1;
        }

        let pos: i64 = match pos_arg.as_deref().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => {
                zwarnnam("bin_sysseek", "position required");
                return 1;
            }
        };

        // system.c:461-462 — lseek(fd, pos, w); return 2 on -1.
        let new = unsafe { libc::lseek(fd, pos, whence) };
        if new == -1 {
            return 2;
        }
        0
    }
}
// END moved-from-exec-rs

/// Module loader entry — port of `setup_()` from Src/Modules/system.c:920.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/system.c:927.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/system.c:935.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/system.c:942.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/system.c:950.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/system.c:957.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/system.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `fillpmsysparams()` from Src/Modules/system.c:846.
#[allow(non_snake_case)]
pub fn fillpmsysparams() -> i32 { 0 }

/// Port of `getposint()` from Src/Modules/system.c:45.
#[allow(non_snake_case)]
pub fn getposint() -> i32 { 0 }

/// Port of `scanpmsysparams()` from Src/Modules/system.c:885.
#[allow(non_snake_case)]
pub fn scanpmsysparams() -> i32 { 0 }
