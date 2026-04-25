//! Select/poll builtin module - port of Modules/zselect.c
//!
//! Provides zselect builtin for select/poll system calls on file descriptors.

use std::collections::HashMap;
use std::os::unix::io::RawFd;

/// Which type of event to monitor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectMode {
    Read,
    Write,
    Error,
}

impl SelectMode {
    pub fn flag_char(&self) -> char {
        match self {
            SelectMode::Read => 'r',
            SelectMode::Write => 'w',
            SelectMode::Error => 'e',
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'r' => Some(SelectMode::Read),
            'w' => Some(SelectMode::Write),
            'e' => Some(SelectMode::Error),
            _ => None,
        }
    }
}

/// Options for zselect builtin
#[derive(Debug, Default)]
pub struct ZselectOptions {
    pub array_name: Option<String>,
    pub hash_name: Option<String>,
    pub timeout_hundredths: Option<i64>,
    pub fds: Vec<(RawFd, SelectMode)>,
}

/// Result of select operation
#[derive(Debug)]
pub struct SelectResult {
    pub ready_fds: Vec<(RawFd, SelectMode)>,
    pub as_array: Vec<String>,
    pub as_hash: HashMap<String, String>,
}

/// Perform select/poll on file descriptors
#[cfg(unix)]
pub fn zselect(options: &ZselectOptions) -> Result<SelectResult, String> {
    use std::collections::HashSet;

    if options.fds.is_empty() {
        return Ok(SelectResult {
            ready_fds: Vec::new(),
            as_array: Vec::new(),
            as_hash: HashMap::new(),
        });
    }

    let mut read_fds: HashSet<RawFd> = HashSet::new();
    let mut write_fds: HashSet<RawFd> = HashSet::new();
    let mut error_fds: HashSet<RawFd> = HashSet::new();

    let mut max_fd: RawFd = 0;

    for (fd, mode) in &options.fds {
        max_fd = max_fd.max(*fd);
        match mode {
            SelectMode::Read => {
                read_fds.insert(*fd);
            }
            SelectMode::Write => {
                write_fds.insert(*fd);
            }
            SelectMode::Error => {
                error_fds.insert(*fd);
            }
        }
    }

    let mut poll_fds: Vec<libc::pollfd> = Vec::new();

    for (fd, mode) in &options.fds {
        let events = match mode {
            SelectMode::Read => libc::POLLIN,
            SelectMode::Write => libc::POLLOUT,
            SelectMode::Error => libc::POLLERR | libc::POLLPRI,
        };

        if let Some(existing) = poll_fds.iter_mut().find(|p| p.fd == *fd) {
            existing.events |= events;
        } else {
            poll_fds.push(libc::pollfd {
                fd: *fd,
                events,
                revents: 0,
            });
        }
    }

    let timeout_ms = options
        .timeout_hundredths
        .map(|t| (t * 10) as libc::c_int)
        .unwrap_or(-1);

    let result = loop {
        let ret = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                timeout_ms,
            )
        };

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("error on select: {}", err));
        }

        break ret;
    };

    if result == 0 {
        return Ok(SelectResult {
            ready_fds: Vec::new(),
            as_array: Vec::new(),
            as_hash: HashMap::new(),
        });
    }

    let mut ready_fds = Vec::new();
    let mut fd_modes: HashMap<RawFd, String> = HashMap::new();

    for pfd in &poll_fds {
        if pfd.revents != 0 {
            if pfd.revents & libc::POLLIN != 0 && read_fds.contains(&pfd.fd) {
                ready_fds.push((pfd.fd, SelectMode::Read));
                fd_modes.entry(pfd.fd).or_insert_with(String::new).push('r');
            }
            if pfd.revents & libc::POLLOUT != 0 && write_fds.contains(&pfd.fd) {
                ready_fds.push((pfd.fd, SelectMode::Write));
                fd_modes.entry(pfd.fd).or_insert_with(String::new).push('w');
            }
            if (pfd.revents & (libc::POLLERR | libc::POLLPRI) != 0) && error_fds.contains(&pfd.fd) {
                ready_fds.push((pfd.fd, SelectMode::Error));
                fd_modes.entry(pfd.fd).or_insert_with(String::new).push('e');
            }
        }
    }

    let as_hash: HashMap<String, String> = fd_modes
        .iter()
        .map(|(fd, modes)| (fd.to_string(), modes.clone()))
        .collect();

    let mut as_array = Vec::new();
    let mut current_mode: Option<SelectMode> = None;

    for (fd, mode) in &ready_fds {
        if current_mode != Some(*mode) {
            as_array.push(format!("-{}", mode.flag_char()));
            current_mode = Some(*mode);
        }
        as_array.push(fd.to_string());
    }

    Ok(SelectResult {
        ready_fds,
        as_array,
        as_hash,
    })
}

#[cfg(not(unix))]
pub fn zselect(_options: &ZselectOptions) -> Result<SelectResult, String> {
    Err("your system does not implement the select system call".to_string())
}

/// Parse zselect arguments
pub fn parse_zselect_args(args: &[&str]) -> Result<ZselectOptions, String> {
    let mut options = ZselectOptions::default();
    let mut current_mode = SelectMode::Read;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i];

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            while j < chars.len() {
                match chars[j] {
                    'a' => {
                        let name = if j + 1 < chars.len() {
                            chars[j + 1..].iter().collect::<String>()
                        } else if i + 1 < args.len() {
                            i += 1;
                            args[i].to_string()
                        } else {
                            return Err("argument expected after -a".to_string());
                        };

                        if name
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            return Err(format!("invalid array name: {}", name));
                        }
                        options.array_name = Some(name);
                        break;
                    }
                    'A' => {
                        let name = if j + 1 < chars.len() {
                            chars[j + 1..].iter().collect::<String>()
                        } else if i + 1 < args.len() {
                            i += 1;
                            args[i].to_string()
                        } else {
                            return Err("argument expected after -A".to_string());
                        };

                        if name
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            return Err(format!("invalid array name: {}", name));
                        }
                        options.hash_name = Some(name);
                        break;
                    }
                    'r' => current_mode = SelectMode::Read,
                    'w' => current_mode = SelectMode::Write,
                    'e' => current_mode = SelectMode::Error,
                    't' => {
                        let timeout_str = if j + 1 < chars.len() {
                            chars[j + 1..].iter().collect::<String>()
                        } else if i + 1 < args.len() {
                            i += 1;
                            args[i].to_string()
                        } else {
                            return Err("argument expected after -t".to_string());
                        };

                        let timeout: i64 = timeout_str
                            .parse()
                            .map_err(|_| format!("number expected after -t: {}", timeout_str))?;
                        options.timeout_hundredths = Some(timeout);
                        break;
                    }
                    c if c.is_ascii_digit() => {
                        let fd_str: String = chars[j..].iter().collect();
                        let fd: RawFd = fd_str
                            .parse()
                            .map_err(|_| format!("expecting file descriptor: {}", fd_str))?;
                        options.fds.push((fd, current_mode));
                        break;
                    }
                    c => {
                        return Err(format!("unknown option: -{}", c));
                    }
                }
                j += 1;
            }
        } else if arg.chars().all(|c| c.is_ascii_digit()) {
            let fd: RawFd = arg
                .parse()
                .map_err(|_| format!("expecting file descriptor: {}", arg))?;
            options.fds.push((fd, current_mode));
        } else {
            return Err(format!("expecting file descriptor: {}", arg));
        }

        i += 1;
    }

    Ok(options)
}

/// Execute zselect builtin
pub fn builtin_zselect(args: &[&str]) -> (i32, Vec<String>, HashMap<String, String>) {
    let options = match parse_zselect_args(args) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("zselect: {}", e);
            return (1, Vec::new(), HashMap::new());
        }
    };

    match zselect(&options) {
        Ok(result) => {
            if result.ready_fds.is_empty() {
                (1, Vec::new(), HashMap::new())
            } else {
                (0, result.as_array, result.as_hash)
            }
        }
        Err(e) => {
            eprintln!("zselect: {}", e);
            (1, Vec::new(), HashMap::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_mode_char() {
        assert_eq!(SelectMode::Read.flag_char(), 'r');
        assert_eq!(SelectMode::Write.flag_char(), 'w');
        assert_eq!(SelectMode::Error.flag_char(), 'e');
    }

    #[test]
    fn test_select_mode_from_char() {
        assert_eq!(SelectMode::from_char('r'), Some(SelectMode::Read));
        assert_eq!(SelectMode::from_char('w'), Some(SelectMode::Write));
        assert_eq!(SelectMode::from_char('e'), Some(SelectMode::Error));
        assert_eq!(SelectMode::from_char('x'), None);
    }

    #[test]
    fn test_parse_basic_args() {
        let args = vec!["-r", "0", "-w", "1"];
        let options = parse_zselect_args(&args).unwrap();

        assert_eq!(options.fds.len(), 2);
        assert!(options.fds.contains(&(0, SelectMode::Read)));
        assert!(options.fds.contains(&(1, SelectMode::Write)));
    }

    #[test]
    fn test_parse_timeout() {
        let args = vec!["-t", "100", "-r", "0"];
        let options = parse_zselect_args(&args).unwrap();

        assert_eq!(options.timeout_hundredths, Some(100));
    }

    #[test]
    fn test_parse_combined_args() {
        let args = vec!["-r0", "-w1"];
        let options = parse_zselect_args(&args).unwrap();

        assert_eq!(options.fds.len(), 2);
    }

    #[test]
    fn test_parse_array_name() {
        let args = vec!["-a", "myarray", "-r", "0"];
        let options = parse_zselect_args(&args).unwrap();

        assert_eq!(options.array_name, Some("myarray".to_string()));
    }

    #[test]
    fn test_parse_hash_name() {
        let args = vec!["-A", "myhash", "-r", "0"];
        let options = parse_zselect_args(&args).unwrap();

        assert_eq!(options.hash_name, Some("myhash".to_string()));
    }

    #[test]
    fn test_parse_invalid_fd() {
        let args = vec!["-r", "abc"];
        let result = parse_zselect_args(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_zselect_empty() {
        let options = ZselectOptions::default();
        let result = zselect(&options).unwrap();
        assert!(result.ready_fds.is_empty());
    }
}
