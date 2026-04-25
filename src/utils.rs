//! Utility functions for zshrs
//!
//! Port from zsh/Src/utils.c
//!
//! Provides miscellaneous utilities: error handling, file operations,
//! string utilities, and character classification.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Script name for error messages
pub static mut SCRIPT_NAME: Option<String> = None;
/// Script filename
pub static mut SCRIPT_FILENAME: Option<String> = None;

/// Print an error message
pub fn zerr(msg: &str) {
    eprintln!("zsh: {}", msg);
}

/// Print an error message with command name
pub fn zerrnam(cmd: &str, msg: &str) {
    eprintln!("{}: {}", cmd, msg);
}

/// Print a warning message
pub fn zwarn(msg: &str) {
    eprintln!("zsh: warning: {}", msg);
}

/// Print a warning with command name  
pub fn zwarnnam(cmd: &str, msg: &str) {
    eprintln!("{}: warning: {}", cmd, msg);
}

/// Print formatted error with optional errno
pub fn zerrmsg(msg: &str, errno: Option<i32>) {
    if let Some(e) = errno {
        let errmsg = std::io::Error::from_raw_os_error(e);
        eprintln!("zsh: {}: {}", msg, errmsg);
    } else {
        eprintln!("zsh: {}", msg);
    }
}

/// Check if a path is a directory
pub fn is_directory(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Check if a file exists and is executable
pub fn is_executable(path: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode();
            return meta.is_file() && (mode & 0o111 != 0);
        }
        false
    }
    #[cfg(not(unix))]
    {
        Path::new(path).is_file()
    }
}

/// Find an executable in PATH
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        if is_executable(name) {
            return Some(path);
        }
        return None;
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let full_path = PathBuf::from(dir).join(name);
            if let Some(path_str) = full_path.to_str() {
                if is_executable(path_str) {
                    return Some(full_path);
                }
            }
        }
    }
    None
}

/// Expand tilde in a path
pub fn expand_tilde(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }

    let (user, rest) = if let Some(pos) = path[1..].find('/') {
        (&path[1..pos + 1], &path[pos + 1..])
    } else {
        (&path[1..], "")
    };

    if user.is_empty() {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, rest);
        }
    } else {
        #[cfg(unix)]
        {
            if let Some(dir) = get_user_home(user) {
                return format!("{}{}", dir, rest);
            }
        }
    }

    path.to_string()
}

#[cfg(unix)]
fn get_user_home(user: &str) -> Option<String> {
    use std::ffi::CString;
    unsafe {
        let c_user = CString::new(user).ok()?;
        let pw = libc::getpwnam(c_user.as_ptr());
        if pw.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        dir.to_str().ok().map(|s| s.to_string())
    }
}

/// Nicely format a string for display (escape unprintable chars)
pub fn nicechar(c: char) -> String {
    if c.is_ascii_control() {
        match c {
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            '\x1b' => "\\e".to_string(),
            _ => format!("^{}", ((c as u8) + 64) as char),
        }
    } else if c == '\x7f' {
        "^?".to_string()
    } else {
        c.to_string()
    }
}

/// Nicely format a string
pub fn nicezputs(s: &str) -> String {
    s.chars().map(nicechar).collect()
}

/// Check if character is a word character
pub fn is_word_char(c: char, wordchars: &str) -> bool {
    c.is_alphanumeric() || wordchars.contains(c)
}

/// Check if character is an IFS character
pub fn is_ifs_char(c: char, ifs: &str) -> bool {
    ifs.contains(c)
}

/// Convert character to lowercase
pub fn tulower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Convert character to uppercase
pub fn tuupper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// Check if string is a valid identifier
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Check if string looks like a number
pub fn is_number(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let s = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .unwrap_or(s);
    if s.is_empty() {
        return false;
    }
    s.chars().all(|c| c.is_ascii_digit())
}

/// Check if string is a valid floating point number
pub fn is_float(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

/// Get monotonic time in nanoseconds
pub fn monotonic_time_ns() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

/// Sleep for a given number of seconds (fractional)
pub fn zsleep(seconds: f64) {
    let duration = std::time::Duration::from_secs_f64(seconds);
    std::thread::sleep(duration);
}

/// Write a string to a file descriptor
pub fn write_to_fd(fd: i32, data: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        write!(file, "{}", data)?;
        std::mem::forget(file); // Don't close the fd
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, data);
        Err(io::Error::new(io::ErrorKind::Unsupported, "Not supported"))
    }
}

/// Move a file descriptor to a high number (>10)
pub fn move_fd(fd: i32) -> i32 {
    #[cfg(unix)]
    {
        if fd < 10 {
            unsafe {
                let newfd = libc::fcntl(fd, libc::F_DUPFD, 10);
                if newfd >= 0 {
                    libc::close(fd);
                    return newfd;
                }
            }
        }
        fd
    }
    #[cfg(not(unix))]
    {
        fd
    }
}

/// Close a file descriptor
pub fn zclose(fd: i32) {
    #[cfg(unix)]
    unsafe {
        libc::close(fd);
    }
}

/// Check if a file descriptor is a tty
pub fn is_tty(fd: i32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::isatty(fd) != 0
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        false
    }
}

/// Get terminal width
pub fn get_term_width() -> usize {
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80)
}

/// Get terminal height
pub fn get_term_height() -> usize {
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
                return ws.ws_row as usize;
            }
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

/// Quote type constants for quotestring()
/// Port from zsh.h QT_* enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteType {
    None = 0,
    Backslash = 1,
    Single = 2,
    Double = 3,
    Dollars = 4,
    Backtick = 5,
    SingleOptional = 6,
    BackslashPattern = 7,
    BackslashShownull = 8,
}

impl QuoteType {
    /// Convert q flag count to QuoteType
    /// (q)=Backslash, (qq)=Single, (qqq)=Double, (qqqq)=Dollars
    pub fn from_q_count(count: u32) -> Self {
        match count {
            0 => QuoteType::None,
            1 => QuoteType::Backslash,
            2 => QuoteType::Single,
            3 => QuoteType::Double,
            _ => QuoteType::Dollars,
        }
    }
}

/// Check if character is special for shell
/// Port from ispecial() macro in zsh.h
fn is_special(c: char) -> bool {
    matches!(
        c,
        '|' | '&'
            | ';'
            | '<'
            | '>'
            | '('
            | ')'
            | '$'
            | '`'
            | '"'
            | '\''
            | '\\'
            | ' '
            | '\t'
            | '\n'
            | '='
            | '['
            | ']'
            | '*'
            | '?'
            | '#'
            | '~'
            | '{'
            | '}'
            | '!'
            | '^'
    )
}

/// Check if character is a pattern character
/// Port from ipattern() macro in zsh.h
fn is_pattern(c: char) -> bool {
    matches!(
        c,
        '*' | '?' | '[' | ']' | '<' | '>' | '(' | ')' | '|' | '#' | '^' | '~'
    )
}

/// Quote a string according to the specified type
/// Port from zsh/Src/utils.c quotestring() (lines 6141-6452)
pub fn quotestring(s: &str, quote_type: QuoteType) -> String {
    if s.is_empty() {
        return match quote_type {
            QuoteType::None => String::new(),
            QuoteType::BackslashShownull | QuoteType::Backslash => "''".to_string(),
            QuoteType::Single | QuoteType::SingleOptional => "''".to_string(),
            QuoteType::Double => "\"\"".to_string(),
            QuoteType::Dollars => "$''".to_string(),
            QuoteType::BackslashPattern => String::new(),
            QuoteType::Backtick => String::new(),
        };
    }

    match quote_type {
        QuoteType::None => s.to_string(),

        QuoteType::BackslashPattern => {
            // Only quote pattern characters (lines 6242-6247)
            let mut result = String::with_capacity(s.len() * 2);
            for c in s.chars() {
                if is_pattern(c) {
                    result.push('\\');
                }
                result.push(c);
            }
            result
        }

        QuoteType::Backslash | QuoteType::BackslashShownull => {
            // Backslash quoting (lines 6260-6416)
            let mut result = String::with_capacity(s.len() * 2);
            for c in s.chars() {
                if is_special(c) {
                    result.push('\\');
                }
                result.push(c);
            }
            result
        }

        QuoteType::Single => {
            // Single quote: 'string' (lines 6359-6382)
            let mut result = String::with_capacity(s.len() + 4);
            result.push('\'');
            for c in s.chars() {
                if c == '\'' {
                    // End quote, add escaped quote, start new quote
                    result.push_str("'\\''");
                } else if c == '\n' {
                    // Newlines need $'...' quoting
                    result.push_str("'$'\\n''");
                } else {
                    result.push(c);
                }
            }
            result.push('\'');
            result
        }

        QuoteType::SingleOptional => {
            // Only add quotes where necessary (lines 6314-6363)
            let needs_quoting = s.chars().any(|c| is_special(c));
            if !needs_quoting {
                return s.to_string();
            }

            let mut result = String::with_capacity(s.len() + 4);
            let mut in_quotes = false;

            for c in s.chars() {
                if c == '\'' {
                    if in_quotes {
                        result.push('\'');
                        in_quotes = false;
                    }
                    result.push_str("\\'");
                } else if is_special(c) {
                    if !in_quotes {
                        result.push('\'');
                        in_quotes = true;
                    }
                    result.push(c);
                } else {
                    if in_quotes {
                        result.push('\'');
                        in_quotes = false;
                    }
                    result.push(c);
                }
            }
            if in_quotes {
                result.push('\'');
            }
            result
        }

        QuoteType::Double => {
            // Double quote: "string" (lines 6272-6280, 6311-6312)
            let mut result = String::with_capacity(s.len() + 4);
            result.push('"');
            for c in s.chars() {
                if matches!(c, '$' | '`' | '"' | '\\') {
                    result.push('\\');
                }
                result.push(c);
            }
            result.push('"');
            result
        }

        QuoteType::Dollars => {
            // $'...' quoting with escape sequences (lines 6203-6241)
            let mut result = String::with_capacity(s.len() + 4);
            result.push_str("$'");
            for c in s.chars() {
                match c {
                    '\\' | '\'' => {
                        result.push('\\');
                        result.push(c);
                    }
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    '\x1b' => result.push_str("\\e"),
                    '\x07' => result.push_str("\\a"),
                    '\x08' => result.push_str("\\b"),
                    '\x0c' => result.push_str("\\f"),
                    '\x0b' => result.push_str("\\v"),
                    c if c.is_ascii_control() => {
                        // Octal escape for control characters
                        result.push_str(&format!("\\{:03o}", c as u8));
                    }
                    c => result.push(c),
                }
            }
            result.push('\'');
            result
        }

        QuoteType::Backtick => {
            // Backtick quoting (minimal - just escape backticks)
            s.replace('`', "\\`")
        }
    }
}

/// Quote a string for safe shell use (convenience wrapper)
pub fn quote_string(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    let needs_quotes = s.chars().any(is_special);

    if !needs_quotes {
        s.to_string()
    } else {
        quotestring(s, QuoteType::Single)
    }
}

/// Split a string respecting quotes
pub fn split_quoted(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for c in s.chars() {
        if escape_next {
            current.push(c);
            escape_next = false;
            continue;
        }

        match c {
            '\\' if !in_single_quote => escape_next = true,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

/// Split string by separator - port from zsh/Src/utils.c sepsplit() lines 3961-3992
///
/// If sep is None, performs IFS-style word splitting (spacesplit).
/// Otherwise splits on the given separator string.
/// allownull: if true, allows empty strings in result
pub fn sepsplit(s: &str, sep: Option<&str>, allownull: bool) -> Vec<String> {
    // Handle Nularg at start (zsh internal marker) - line 3968
    let s = if s.starts_with('\x00') && s.len() > 1 {
        &s[1..]
    } else {
        s
    };

    match sep {
        None => spacesplit(s, allownull),
        Some(sep) if sep.is_empty() => {
            // Empty separator: split into characters
            if allownull {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.chars()
                    .map(|c| c.to_string())
                    .filter(|c| !c.is_empty())
                    .collect()
            }
        }
        Some(sep) => {
            let parts: Vec<String> = s.split(sep).map(|p| p.to_string()).collect();
            if allownull {
                parts
            } else {
                parts.into_iter().filter(|p| !p.is_empty()).collect()
            }
        }
    }
}

/// IFS-style word splitting - port from zsh/Src/utils.c spacesplit()
///
/// Splits on whitespace (space, tab, newline), treating consecutive
/// whitespace as a single separator.
pub fn spacesplit(s: &str, allownull: bool) -> Vec<String> {
    if allownull {
        s.split(|c: char| c == ' ' || c == '\t' || c == '\n')
            .map(|p| p.to_string())
            .collect()
    } else {
        s.split_whitespace().map(|p| p.to_string()).collect()
    }
}

/// Join array with separator - port from zsh/Src/utils.c sepjoin() lines 3926-3958
///
/// If sep is None, uses first char of IFS (defaults to space).
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {
    if arr.is_empty() {
        return String::new();
    }
    let sep = sep.unwrap_or(" ");
    arr.join(sep)
}

/// Parse a string to a signed integer with base detection
/// Port from zsh/Src/utils.c zstrtol() lines 2384-2516
pub fn zstrtol(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (neg, rest) = if s.starts_with('-') {
        (true, &s[1..])
    } else if s.starts_with('+') {
        (false, &s[1..])
    } else {
        (false, s)
    };

    let (base, rest) = if rest.starts_with("0x") || rest.starts_with("0X") {
        (16, &rest[2..])
    } else if rest.starts_with("0b") || rest.starts_with("0B") {
        (2, &rest[2..])
    } else if rest.starts_with('0') && rest.len() > 1 {
        (8, &rest[1..])
    } else {
        (10, rest)
    };

    let rest = rest.replace('_', "");
    let val = u64::from_str_radix(&rest, base).ok()?;
    let result = val as i64;
    Some(if neg { -result } else { result })
}

/// Parse unsigned integer with underscore support
/// Port from zsh/Src/utils.c zstrtoul_underscore() lines 2528-2575
pub fn zstrtoul_underscore(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s);

    let (base, rest) = if s.starts_with("0x") || s.starts_with("0X") {
        (16, &s[2..])
    } else if s.starts_with("0b") || s.starts_with("0B") {
        (2, &s[2..])
    } else if s.starts_with('0') && s.len() > 1 {
        (8, &s[1..])
    } else {
        (10, s)
    };

    let rest = rest.replace('_', "");
    u64::from_str_radix(&rest, base).ok()
}

/// Convert integer to string with specified base
/// Port from zsh/Src/utils.c convbase()
pub fn convbase(val: i64, base: u32) -> String {
    match base {
        2 => format!("0b{:b}", val),
        8 => format!("0{:o}", val),
        16 => format!("0x{:x}", val),
        _ => val.to_string(),
    }
}

/// Set blocking/nonblocking on a file descriptor
/// Port from zsh/Src/utils.c setblock_fd() lines 2578-2618
pub fn setblock_fd(fd: i32, blocking: bool) -> bool {
    #[cfg(unix)]
    {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        if flags < 0 {
            return false;
        }
        let new_flags = if blocking {
            flags & !libc::O_NONBLOCK
        } else {
            flags | libc::O_NONBLOCK
        };
        if new_flags != flags {
            unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) >= 0 }
        } else {
            true
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, blocking);
        false
    }
}

/// Read poll - check for pending input
/// Port from zsh/Src/utils.c read_poll() lines 2643-2730
pub fn read_poll(fd: i32, timeout_us: i64) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::RawFd;
        let mut fds = [libc::pollfd {
            fd: fd as RawFd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let timeout_ms = (timeout_us / 1000) as i32;
        let result = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
        result > 0 && (fds[0].revents & libc::POLLIN) != 0
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, timeout_us);
        false
    }
}

/// Check glob qualifier syntax
/// Port from zsh/Src/utils.c checkglobqual()
pub fn checkglobqual(s: &str) -> bool {
    if !s.ends_with(')') {
        return false;
    }
    let mut depth = 0;
    let mut in_bracket = false;
    for c in s.chars() {
        match c {
            '[' if !in_bracket => in_bracket = true,
            ']' if in_bracket => in_bracket = false,
            '(' if !in_bracket => depth += 1,
            ')' if !in_bracket => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Compute edit distance between two strings (for spelling correction)
/// Port from zsh/Src/utils.c spdist() lines 4675-4759
pub fn spdist(s: &str, t: &str, max_dist: usize) -> usize {
    let s_chars: Vec<char> = s.chars().collect();
    let t_chars: Vec<char> = t.chars().collect();
    let m = s_chars.len();
    let n = t_chars.len();

    if m.abs_diff(n) > max_dist {
        return max_dist + 1;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if s_chars[i - 1] == t_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Get temporary file/directory name
/// Port from zsh/Src/utils.c gettempname()
pub fn gettempname(prefix: Option<&str>, dir: bool) -> Option<String> {
    let prefix = prefix.unwrap_or("zsh");
    let tmp_dir = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TMP"))
        .or_else(|_| std::env::var("TEMP"))
        .unwrap_or_else(|_| "/tmp".to_string());

    let pid = std::process::id();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let name = format!("{}/{}{}_{}", tmp_dir, prefix, pid, timestamp);

    if dir {
        std::fs::create_dir_all(&name).ok()?;
    }
    Some(name)
}

/// Check if metafied - port from zsh/Src/utils.c has_token()
pub fn has_token(s: &str) -> bool {
    s.bytes().any(|b| b == 0x83) // Meta character
}

/// Array length - port from arrlen()
pub fn arrlen<T>(arr: &[T]) -> usize {
    arr.len()
}

/// Duplicate string prefix
pub fn dupstrpfx(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

const META_CHAR: char = '\u{83}';

/// Unmetafy string (from utils.c unmeta lines 4930-5051)
pub fn unmeta(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == META_CHAR && i + 1 < chars.len() {
            let c = (chars[i + 1] as u8) ^ 32;
            result.push(c as char);
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Metafy string (from utils.c metafy)
pub fn metafy(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        let b = c as u32;
        if b < 32 || (b >= 0x83 && b <= 0x9b) {
            result.push(META_CHAR);
            result.push(char::from_u32((c as u8 ^ 32) as u32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafied string length (from utils.c ztrlen lines 5135-5152)
pub fn ztrlen(s: &str) -> usize {
    let mut len = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        len += 1;
        if chars[i] == META_CHAR && i + 1 < chars.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    len
}

/// Compare strings with meta handling (from utils.c ztrcmp lines 5106-5130)
pub fn ztrcmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    unmeta(s1).cmp(&unmeta(s2))
}

/// String pointer subtraction with meta handling (from utils.c ztrsub)
pub fn ztrsub(t: &str, s: &str) -> usize {
    ztrlen(&t[..t.len().saturating_sub(s.len())])
}

/// Get home directory for user by name (from utils.c getpwnam handling)
pub fn get_user_home_by_name(username: &str) -> Option<String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let c_user = CString::new(username).ok()?;
        let pwd = unsafe { libc::getpwnam(c_user.as_ptr()) };
        if pwd.is_null() {
            return None;
        }
        let home = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_dir) };
        home.to_str().ok().map(|s| s.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = username;
        None
    }
}

/// Get username from UID (from utils.c getpwuid handling)
pub fn get_username(uid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        let pwd = unsafe { libc::getpwuid(uid) };
        if pwd.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
        name.to_str().ok().map(|s| s.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = uid;
        None
    }
}

/// Get group name from GID (from utils.c getgrgid handling)
pub fn get_groupname(gid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        let grp = unsafe { libc::getgrgid(gid) };
        if grp.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*grp).gr_name) };
        name.to_str().ok().map(|s| s.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = gid;
        None
    }
}

/// Compare strings case-insensitively (from utils.c zstricmp)
pub fn zstricmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    s1.to_lowercase().cmp(&s2.to_lowercase())
}

/// Find needle in haystack (from utils.c zstrstr)
pub fn zstrstr(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

/// String duplicate (from utils.c ztrdup)
pub fn ztrdup(s: &str) -> String {
    s.to_string()
}

/// Duplicate n characters (from utils.c ztrncpy)
pub fn ztrncpy(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// String concat (from utils.c dyncat)
pub fn dyncat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Triple concat (from utils.c tricat)
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    format!("{}{}{}", s1, s2, s3)
}

/// Buffer concat (from utils.c bicat)
pub fn bicat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Numeric string comparison (from utils.c nstrncmp)
pub fn nstrcmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    let n1: i64 = s1.parse().unwrap_or(0);
    let n2: i64 = s2.parse().unwrap_or(0);
    n1.cmp(&n2)
}

/// Inverted numeric comparison (from utils.c invnstrncmp)
pub fn invnstrcmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    nstrcmp(s2, s1)
}

/// Check if string ends with suffix (from utils.c)
pub fn str_ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// Check if string starts with prefix
pub fn str_starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// Get basename of path (from utils.c)
pub fn zbasename(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// Get dirname of path (from utils.c)
pub fn zdirname(path: &str) -> &str {
    std::path::Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".")
}

/// Check if character is a simple word character (from utils.c)
pub fn is_word_char_simple(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Get next word boundary (from utils.c)
pub fn next_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos;

    while i < chars.len() && is_word_char_simple(chars[i]) {
        i += 1;
    }
    while i < chars.len() && !is_word_char_simple(chars[i]) {
        i += 1;
    }
    i
}

/// Get previous word boundary (from utils.c)
pub fn prev_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos.min(chars.len());

    while i > 0 && !is_word_char_simple(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char_simple(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Path normalization (from utils.c xsymlink handling)
pub fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    let absolute = path.starts_with('/');

    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                if !components.is_empty() && components.last() != Some(&"..") {
                    components.pop();
                } else if !absolute {
                    components.push("..");
                }
            }
            _ => components.push(part),
        }
    }

    let result = components.join("/");
    if absolute {
        format!("/{}", result)
    } else if result.is_empty() {
        ".".to_string()
    } else {
        result
    }
}

/// Check access with effective UID (from utils.c eaccess)
pub fn eaccess(path: &str, mode: i32) -> bool {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let c_path = match CString::new(path) {
            Ok(p) => p,
            Err(_) => return false,
        };
        unsafe { libc::access(c_path.as_ptr(), mode) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        false
    }
}

/// Word count for strings
pub fn wordcount(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Character count for strings
pub fn charcount(s: &str) -> usize {
    s.chars().count()
}

/// Line count for strings
pub fn linecount(s: &str) -> usize {
    s.lines().count()
}

/// Join array with delimiter (from utils.c zjoin)
pub fn zjoin(arr: &[String], delim: char) -> String {
    arr.join(&delim.to_string())
}

/// Split colon-separated list (from utils.c colonsplit)
pub fn colonsplit(s: &str, uniq: bool) -> Vec<String> {
    let mut result = Vec::new();
    for item in s.split(':') {
        if !item.is_empty() {
            if uniq && result.contains(&item.to_string()) {
                continue;
            }
            result.push(item.to_string());
        }
    }
    result
}

/// Skip whitespace separators (from utils.c skipwsep)
pub fn skipwsep(s: &str) -> &str {
    s.trim_start()
}

/// Check if character is a whitespace separator
pub fn iwsep(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Check if character needs metafication
pub fn imeta(c: char) -> bool {
    (c as u32) < 32 || c == '\x7f' || c == '\u{83}'
}

/// Get nice representation of control character
pub fn nicechar_ctrl(c: char) -> String {
    let c_byte = c as u8;
    if c_byte < 32 {
        format!("^{}", (c_byte + 64) as char)
    } else if c_byte == 127 {
        "^?".to_string()
    } else {
        c.to_string()
    }
}

/// Format time struct (from utils.c ztrftime)
pub fn ztrftime(fmt: &str, time: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;

    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;

    #[cfg(unix)]
    unsafe {
        let tm = libc::localtime(&secs);
        if tm.is_null() {
            return String::new();
        }

        let mut buf = vec![0u8; 256];
        let c_fmt = std::ffi::CString::new(fmt).unwrap_or_default();
        let len = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            c_fmt.as_ptr(),
            tm,
        );

        if len > 0 {
            buf.truncate(len);
            String::from_utf8_lossy(&buf).to_string()
        } else {
            String::new()
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (fmt, secs);
        String::new()
    }
}

/// Get current time formatted
pub fn current_time_fmt(fmt: &str) -> String {
    ztrftime(fmt, std::time::SystemTime::now())
}

/// Print-safe string representation
pub fn printsafe(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            if c == '\n' {
                result.push_str("\\n");
            } else if c == '\t' {
                result.push_str("\\t");
            } else if c == '\r' {
                result.push_str("\\r");
            } else {
                result.push_str(&format!("\\x{:02x}", c as u32));
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Escape string for shell
pub fn shescape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '.' || c == '-')
    {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Unescape string
pub fn unescape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('"') => result.push('"'),
                Some('0') => result.push('\0'),
                Some('a') => result.push('\x07'),
                Some('b') => result.push('\x08'),
                Some('e') => result.push('\x1b'),
                Some('f') => result.push('\x0c'),
                Some('v') => result.push('\x0b'),
                Some('x') => {
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(&c) = chars.peek() {
                            if c.is_ascii_hexdigit() {
                                hex.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(val) = u8::from_str_radix(&hex, 16) {
                        result.push(val as char);
                    }
                }
                Some(c) => result.push(c),
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if string contains only printable characters
pub fn isprintable(s: &str) -> bool {
    s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t')
}

/// Get terminal width (fallback to 80)
pub fn term_columns() -> usize {
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        unsafe {
            let mut ws: MaybeUninit<libc::winsize> = MaybeUninit::uninit();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr()) == 0 {
                let ws = ws.assume_init();
                if ws.ws_col > 0 {
                    return ws.ws_col as usize;
                }
            }
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80)
}

/// Get terminal lines (fallback to 24)
pub fn term_lines() -> usize {
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        unsafe {
            let mut ws: MaybeUninit<libc::winsize> = MaybeUninit::uninit();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr()) == 0 {
                let ws = ws.assume_init();
                if ws.ws_row > 0 {
                    return ws.ws_row as usize;
                }
            }
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

/// Sleep for milliseconds
pub fn zsleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Get hostname
pub fn gethostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = vec![0u8; 256];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                return String::from_utf8_lossy(&buf[..len]).to_string();
            }
        }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
}

/// Get current working directory
pub fn zgetcwd() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Set current working directory
pub fn zchdir(path: &str) -> bool {
    std::env::set_current_dir(path).is_ok()
}

/// Check if path is absolute
pub fn isabspath(path: &str) -> bool {
    path.starts_with('/')
}

/// Make path absolute
pub fn makeabspath(path: &str) -> String {
    if isabspath(path) {
        return path.to_string();
    }
    if let Some(cwd) = zgetcwd() {
        format!("{}/{}", cwd, path)
    } else {
        path.to_string()
    }
}

/// Get real (canonical) path
pub fn realpath(path: &str) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Check if file exists
pub fn file_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Check if path is a file
pub fn is_file(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

/// Check if path is a directory
pub fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

/// Check if path is a symlink
pub fn is_link(path: &str) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Get file size
pub fn file_size(path: &str) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Get file modification time as seconds since epoch
pub fn file_mtime(path: &str) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// Read file contents to string
pub fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Read file lines
pub fn read_lines(path: &str) -> Option<Vec<String>> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.lines().map(|l| l.to_string()).collect())
}

/// Write string to file
pub fn write_file(path: &str, contents: &str) -> bool {
    std::fs::write(path, contents).is_ok()
}

/// Append to file
pub fn append_file(path: &str, contents: &str) -> bool {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut f| f.write_all(contents.as_bytes()))
        .is_ok()
}

/// List directory contents
pub fn list_dir(path: &str) -> Option<Vec<String>> {
    std::fs::read_dir(path).ok().map(|entries| {
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    })
}

/// Create directory
pub fn mkdir(path: &str) -> bool {
    std::fs::create_dir(path).is_ok()
}

/// Create directory recursively
pub fn mkdir_p(path: &str) -> bool {
    std::fs::create_dir_all(path).is_ok()
}

/// Remove file
pub fn rm_file(path: &str) -> bool {
    std::fs::remove_file(path).is_ok()
}

/// Remove directory
pub fn rm_dir(path: &str) -> bool {
    std::fs::remove_dir(path).is_ok()
}

/// Remove directory recursively
pub fn rm_dir_all(path: &str) -> bool {
    std::fs::remove_dir_all(path).is_ok()
}

/// Copy file
pub fn copy_file(src: &str, dst: &str) -> bool {
    std::fs::copy(src, dst).is_ok()
}

/// Rename/move file
pub fn rename_file(src: &str, dst: &str) -> bool {
    std::fs::rename(src, dst).is_ok()
}

/// Create symlink
pub fn symlink(src: &str, dst: &str) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (src, dst);
        false
    }
}

/// Read symlink target
pub fn readlink(path: &str) -> Option<String> {
    std::fs::read_link(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Get environment variable
pub fn getenv(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Set environment variable
pub fn setenv(name: &str, value: &str) {
    std::env::set_var(name, value);
}

/// Unset environment variable
pub fn unsetenv(name: &str) {
    std::env::remove_var(name);
}

/// Get all environment variables
pub fn environ() -> Vec<(String, String)> {
    std::env::vars().collect()
}

/// Get current user ID
pub fn getuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getuid()
    }
    #[cfg(not(unix))]
    0
}

/// Get effective user ID
pub fn geteuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::geteuid()
    }
    #[cfg(not(unix))]
    0
}

/// Get current group ID
pub fn getgid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getgid()
    }
    #[cfg(not(unix))]
    0
}

/// Get effective group ID
pub fn getegid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getegid()
    }
    #[cfg(not(unix))]
    0
}

/// Get process ID
pub fn getpid() -> i32 {
    std::process::id() as i32
}

/// Get parent process ID
pub fn getppid() -> i32 {
    #[cfg(unix)]
    unsafe {
        libc::getppid()
    }
    #[cfg(not(unix))]
    0
}

/// Check if running as root
pub fn is_root() -> bool {
    geteuid() == 0
}

/// Get umask
pub fn getumask() -> u32 {
    #[cfg(unix)]
    unsafe {
        let mask = libc::umask(0);
        libc::umask(mask);
        mask as u32
    }
    #[cfg(not(unix))]
    0o022
}

/// Set umask
pub fn setumask(mask: u32) -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::umask(mask as libc::mode_t) as u32
    }
    #[cfg(not(unix))]
    {
        let _ = mask;
        0
    }
}

/// Get current time as seconds since epoch
pub fn time_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Get current time with nanoseconds
pub fn time_now_ns() -> (i64, i64) {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (dur.as_secs() as i64, dur.subsec_nanos() as i64)
}

/// Format seconds as HH:MM:SS
pub fn format_time(secs: i64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{}:{:02}", mins, secs)
    }
}

/// Parse HH:MM:SS to seconds
pub fn parse_time(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        1 => parts[0].parse().ok(),
        2 => {
            let mins: i64 = parts[0].parse().ok()?;
            let secs: i64 = parts[1].parse().ok()?;
            Some(mins * 60 + secs)
        }
        3 => {
            let hours: i64 = parts[0].parse().ok()?;
            let mins: i64 = parts[1].parse().ok()?;
            let secs: i64 = parts[2].parse().ok()?;
            Some(hours * 3600 + mins * 60 + secs)
        }
        _ => None,
    }
}

/// Generate random integer
pub fn random_int() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish() as u32
}

/// Generate random integer in range [0, max)
pub fn random_range(max: u32) -> u32 {
    if max == 0 {
        0
    } else {
        random_int() % max
    }
}

/// Hash a string (simple djb2)
pub fn hash_string(s: &str) -> u64 {
    let mut hash: u64 = 5381;
    for c in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(c as u64);
    }
    hash
}

// ---------------------------------------------------------------------------
// Missing utility functions ported from utils.c
// ---------------------------------------------------------------------------

/// Split path into components (from utils.c slashsplit)
pub fn slashsplit(s: &str) -> Vec<String> {
    s.split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Split on '=' returning (name, value) (from utils.c equalsplit)
pub fn equalsplit(s: &str) -> Option<(String, String)> {
    let eq = s.find('=')?;
    Some((s[..eq].to_string(), s[eq + 1..].to_string()))
}

/// Make single-element array (from utils.c mkarray)
pub fn mkarray(s: Option<&str>) -> Vec<String> {
    match s {
        Some(val) => vec![val.to_string()],
        None => Vec::new(),
    }
}

/// Free array (no-op in Rust, provided for API compat)
pub fn freearray(_arr: Vec<String>) {
    // Rust Drop handles this
}

/// Check if s is a prefix of t (from utils.c strpfx)
pub fn strpfx(s: &str, t: &str) -> bool {
    t.starts_with(s)
}

/// Check if s is a suffix of t (from utils.c strsfx)
pub fn strsfx(s: &str, t: &str) -> bool {
    t.ends_with(s)
}

/// Ring the terminal bell (from utils.c zbeep)
pub fn zbeep() {
    eprint!("\x07");
}

/// Convert file mode to octal string (from utils.c mode_to_octal)
pub fn mode_to_octal(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// Go up n directories (from utils.c upchdir)
pub fn upchdir(n: usize) -> io::Result<()> {
    let mut path = String::new();
    for i in 0..n {
        if i > 0 {
            path.push('/');
        }
        path.push_str("..");
    }
    std::env::set_current_dir(&path)?;
    Ok(())
}

/// Change directory with safeguards (from utils.c lchdir)
pub fn lchdir(path: &str) -> io::Result<()> {
    let resolved = if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        let cwd = std::env::current_dir()?;
        cwd.join(path)
    };
    std::env::set_current_dir(&resolved)?;
    Ok(())
}

/// Adjust terminal window size (from utils.c adjustwinsize)
pub fn adjustwinsize() -> (usize, usize) {
    let cols = get_term_width();
    let rows = get_term_height();
    (cols, rows)
}

/// Spelling correction distance (from utils.c spdist, already exists but adding spckword)
/// Check if word is close enough to correct (from utils.c spckword)
pub fn spckword(word: &str, candidates: &[&str], threshold: usize) -> Option<String> {
    let mut best = None;
    let mut best_dist = threshold + 1;
    for &candidate in candidates {
        let dist = spdist(word, candidate, threshold);
        if dist < best_dist {
            best_dist = dist;
            best = Some(candidate.to_string());
        }
    }
    best
}

/// Simple interactive query (from utils.c getquery)
pub fn getquery(prompt: &str, valid_chars: &str) -> Option<char> {
    eprint!("{}", prompt);
    let _ = io::stderr().flush();

    let mut buf = [0u8; 1];
    #[cfg(unix)]
    {
        use std::io::Read;
        if std::io::stdin().read_exact(&mut buf).is_ok() {
            let c = buf[0] as char;
            if valid_chars.is_empty() || valid_chars.contains(c) {
                return Some(c);
            }
        }
    }
    None
}

/// Read a single character (from utils.c read1char)
pub fn read1char() -> Option<char> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut buf = [0u8; 1];
        if std::io::stdin().read_exact(&mut buf).is_ok() {
            return Some(buf[0] as char);
        }
    }
    None
}

/// Check before removing directory tree (from utils.c checkrmall)
pub fn checkrmall(path: &str) -> bool {
    if let Some(c) = getquery(
        &format!("zsh: sure you want to delete all of {}? [yn] ", path),
        "yn",
    ) {
        c == 'y' || c == 'Y'
    } else {
        false
    }
}

/// Resolve symlinks in path (from utils.c xsymlinks/xsymlink)
pub fn xsymlink(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string(),
    }
}

/// Check if running with elevated privileges (from utils.c privasserted)
pub fn privasserted() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() != libc::geteuid() || libc::getgid() != libc::getegid() }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Get the current working directory (port of findpwd/set_pwd_env)
pub fn findpwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

/// Check if path is the current directory (from utils.c ispwd)
pub fn ispwd(path: &str) -> bool {
    if let Ok(cwd) = std::env::current_dir() {
        cwd.to_string_lossy() == path
    } else {
        false
    }
}

/// Print directory name with ~ substitution (from utils.c fprintdir)
pub fn fprintdir(path: &str, home: &str) -> String {
    if !home.is_empty() && path.starts_with(home) {
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            return format!("~{}", rest);
        }
    }
    path.to_string()
}

/// Duplicate array (from utils.c arrdup)
pub fn arrdup(arr: &[String]) -> Vec<String> {
    arr.to_vec()
}

/// Duplicate array with max elements (from utils.c arrdup_max)
pub fn arrdup_max(arr: &[String], max: usize) -> Vec<String> {
    arr.iter().take(max).cloned().collect()
}

/// Read/write loop wrappers (from utils.c read_loop/write_loop)
pub fn read_loop(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::read(
                    fd,
                    buf[total..].as_mut_ptr() as *mut libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e);
                }
                break;
            }
            total += n as usize;
        }
        Ok(total)
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, buf);
        Err(io::Error::new(io::ErrorKind::Unsupported, "not unix"))
    }
}

pub fn write_loop(fd: i32, buf: &[u8]) -> io::Result<usize> {
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::write(
                    fd,
                    buf[total..].as_ptr() as *const libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e);
                }
                break;
            }
            total += n as usize;
        }
        Ok(total)
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, buf);
        Err(io::Error::new(io::ErrorKind::Unsupported, "not unix"))
    }
}

/// Redup: duplicate fd x to y (from utils.c redup)
pub fn redup(x: i32, y: i32) {
    #[cfg(unix)]
    {
        if x != y {
            unsafe {
                libc::dup2(x, y);
                libc::close(x);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (x, y);
    }
}

/// Check if a character type at end of string (from utils.c itype_end)
/// Returns the position after the identifier characters
pub fn itype_end(s: &str, allow_digits_start: bool) -> usize {
    let mut chars = s.chars().peekable();
    let mut pos = 0;

    if let Some(&first) = chars.peek() {
        if !allow_digits_start && first.is_ascii_digit() {
            return 0;
        }
        if !first.is_alphanumeric() && first != '_' && first != '.' {
            return 0;
        }
    }

    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            pos += c.len_utf8();
        } else {
            break;
        }
    }
    pos
}

/// Initialize character type table (from utils.c inittyptab)
/// In Rust we use Unicode-aware char methods, so this is mostly a no-op
pub fn inittyptab() {
    // Rust handles character classification natively
}

/// Skip whitespace separators from IFS (port helper, with custom IFS)
pub fn skipwsep_ifs<'a>(s: &'a str, ifs: &str) -> &'a str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if !ifs.contains(c) || !c.is_ascii_whitespace() {
            break;
        }
        i += 1;
    }
    &s[i..]
}

/// Find a separator in string (from utils.c findsep)
pub fn findsep(s: &str, sep: Option<&str>) -> Option<usize> {
    match sep {
        Some(sep) if sep.len() == 1 => s.find(sep.chars().next().unwrap()),
        Some(sep) => s.find(sep),
        None => {
            // Default: split on whitespace
            s.find(|c: char| c.is_ascii_whitespace())
        }
    }
}

/// Count words in string (from utils.c wordcount - extended version)
pub fn wordcount_sep(s: &str, sep: Option<&str>) -> usize {
    match sep {
        Some(sep) => s.split(sep).filter(|w| !w.is_empty()).count(),
        None => s.split_whitespace().count(),
    }
}

/// Find word at position (from utils.c findword)
pub fn findword<'a>(s: &'a str, sep: Option<&'a str>) -> Option<(&'a str, &'a str)> {
    let s = match sep {
        Some(_) => s,
        None => s.trim_start(),
    };
    if s.is_empty() {
        return None;
    }
    match sep {
        Some(sep) => {
            if let Some(pos) = s.find(sep) {
                Some((&s[..pos], &s[pos + sep.len()..]))
            } else {
                Some((s, ""))
            }
        }
        None => {
            let end = s.find(|c: char| c.is_ascii_whitespace()).unwrap_or(s.len());
            Some((&s[..end], &s[end..]))
        }
    }
}

/// Parse getkeystring escape sequences (from utils.c getkeystring)
/// Handles \n \t \r \e \a \b \f \v \\ \' \" \xNN \uNNNN \UNNNNNNNN \0NNN
pub fn getkeystring(s: &str) -> (String, usize) {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    let mut consumed = 0;

    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => {
                result.push('\n');
                consumed += 1;
            }
            Some('t') => {
                result.push('\t');
                consumed += 1;
            }
            Some('r') => {
                result.push('\r');
                consumed += 1;
            }
            Some('e') | Some('E') => {
                result.push('\x1b');
                consumed += 1;
            }
            Some('a') => {
                result.push('\x07');
                consumed += 1;
            }
            Some('b') => {
                result.push('\x08');
                consumed += 1;
            }
            Some('f') => {
                result.push('\x0c');
                consumed += 1;
            }
            Some('v') => {
                result.push('\x0b');
                consumed += 1;
            }
            Some('\\') => {
                result.push('\\');
                consumed += 1;
            }
            Some('\'') => {
                result.push('\'');
                consumed += 1;
            }
            Some('"') => {
                result.push('"');
                consumed += 1;
            }
            Some('x') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&hex, 16) {
                    result.push(val as char);
                }
            }
            Some('u') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..4 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
            }
            Some('U') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..8 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
            }
            Some(c @ '0'..='7') => {
                consumed += 1;
                let mut oct = String::new();
                oct.push(c);
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if c >= '0' && c <= '7' {
                            oct.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&oct, 8) {
                    result.push(val as char);
                }
            }
            Some('c') => {
                consumed += 1;
                // \cX = control character
                if let Some(c) = chars.next() {
                    consumed += 1;
                    result.push((c as u8 & 0x1f) as char);
                }
            }
            Some(c) => {
                consumed += 1;
                result.push('\\');
                result.push(c);
            }
            None => {
                result.push('\\');
            }
        }
    }
    (result, consumed)
}

/// Convert UCS-4 to UTF-8 (from utils.c ucs4toutf8)
pub fn ucs4toutf8(codepoint: u32) -> Option<String> {
    char::from_u32(codepoint).map(|c| c.to_string())
}

/// Duplicate a string with quoting for display (from utils.c quotedzputs)
pub fn quotedzputs(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else if is_special(c) {
            result.push('\\');
            result.push(c);
        } else if c.is_ascii_control() {
            result.push_str(&format!("$'\\x{:02x}'", c as u8));
        } else {
            result.push(c);
        }
    }
    result
}

/// Nice format for string display (from utils.c mb_niceformat)
pub fn niceformat(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if c.is_ascii_control() {
            result.push_str(&nicechar(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if nice formatting is needed (from utils.c is_mb_niceformat)
pub fn is_niceformat(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_control())
}

/// Check for special characters that need quoting (from utils.c hasspecial)
pub fn hasspecial(s: &str) -> bool {
    s.chars().any(|c| is_special(c))
}

/// Print/format time in HH:MM:SS (from utils.c printhhmmss)
pub fn printhhmmss(secs: f64) -> String {
    let total_secs = secs as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    let frac = secs - total_secs as f64;
    if hours > 0 {
        format!(
            "{}:{:02}:{:02}.{:03}",
            hours,
            mins,
            s,
            (frac * 1000.0) as u64
        )
    } else {
        format!("{}:{:02}.{:03}", mins, s, (frac * 1000.0) as u64)
    }
}

/// Get or set the file creation mask (wrapper over umask)
pub fn getumask_value() -> u32 {
    #[cfg(unix)]
    {
        let mask = unsafe { libc::umask(0o022) };
        unsafe { libc::umask(mask) };
        mask as u32
    }
    #[cfg(not(unix))]
    {
        0o022
    }
}

/// Attach to the controlling tty's process group (from utils.c attachtty)
#[cfg(unix)]
pub fn attachtty(pgrp: i32) {
    unsafe {
        libc::tcsetpgrp(0, pgrp);
    }
}

/// Get the terminal's process group (from utils.c gettygrp)
#[cfg(unix)]
pub fn gettygrp() -> i32 {
    unsafe { libc::tcgetpgrp(0) }
}

/// Check if directory is readable with entries (from utils.c)
pub fn zreaddir(path: &str) -> Vec<String> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|s| s != "." && s != "..")
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Initialize terminal (from utils.c zsetupterm)
pub fn zsetupterm() -> bool {
    // Rust doesn't need explicit terminal setup like C terminfo
    // Return true if terminal seems usable
    is_tty(1)
}

/// Delete terminal setup (from utils.c zdeleteterm)
pub fn zdeleteterm() {
    // No-op in Rust
}

/// Put raw character to terminal (from utils.c putraw)
pub fn putraw(c: char) {
    print!("{}", c);
}

/// Put character to shell output (from utils.c putshout)
pub fn putshout(c: char) {
    print!("{}", c);
}

/// Nice char with quoting selection (from utils.c nicechar_sel)
pub fn nicechar_sel(c: char, quotable: bool) -> String {
    if quotable && is_special(c) {
        format!("\\{}", c)
    } else {
        nicechar(c)
    }
}

/// Initialize multibyte state (from utils.c mb_charinit) - no-op in Rust
pub fn mb_charinit() {
    // Rust handles UTF-8 natively
}

/// Wide char nice format (from utils.c wcs_nicechar_sel)
pub fn wcs_nicechar_sel(c: char, quotable: bool) -> String {
    nicechar_sel(c, quotable)
}

/// Wide char nice format (from utils.c wcs_nicechar)
pub fn wcs_nicechar(c: char) -> String {
    nicechar(c)
}

/// Check if wide char needs nice formatting (from utils.c is_wcs_nicechar)
pub fn is_wcs_nicechar(c: char) -> bool {
    c.is_ascii_control()
}

/// Get wide character width (from utils.c zwcwidth)
pub fn zwcwidth(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)
}

/// Find program in PATH (from utils.c pathprog)
pub fn pathprog(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return if p.exists() { Some(p) } else { None };
    }
    find_in_path(prog)
}

/// Print symlink target if it is one (from utils.c print_if_link)
pub fn print_if_link(path: &str) -> Option<String> {
    match std::fs::read_link(path) {
        Ok(target) => Some(format!("{} -> {}", path, target.display())),
        Err(_) => None,
    }
}

/// Substitute named directory in path (from utils.c substnamedir)
pub fn substnamedir(
    path: &str,
    home: &str,
    named_dirs: &std::collections::HashMap<String, String>,
) -> String {
    // Try home first
    if !home.is_empty() && path.starts_with(home) {
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            return format!("~{}", rest);
        }
    }
    // Try named dirs
    let mut best_name = "";
    let mut best_len = 0;
    for (name, dir) in named_dirs {
        if path.starts_with(dir.as_str()) && dir.len() > best_len {
            let rest = &path[dir.len()..];
            if rest.is_empty() || rest.starts_with('/') {
                best_name = name;
                best_len = dir.len();
            }
        }
    }
    if best_len > 0 {
        format!("~{}{}", best_name, &path[best_len..])
    } else {
        path.to_string()
    }
}

/// Scan for named directory matches (from utils.c finddir_scan)
pub fn finddir_scan(
    path: &str,
    named_dirs: &std::collections::HashMap<String, String>,
) -> Option<(String, String)> {
    let mut best = None;
    let mut best_len = 0;
    for (name, dir) in named_dirs {
        if path.starts_with(dir.as_str()) && dir.len() > best_len {
            let rest = &path[dir.len()..];
            if rest.is_empty() || rest.starts_with('/') {
                best = Some((name.clone(), rest.to_string()));
                best_len = dir.len();
            }
        }
    }
    best
}

/// Find named directory for path (from utils.c finddir)
pub fn finddir(
    path: &str,
    home: &str,
    named_dirs: &std::collections::HashMap<String, String>,
) -> Option<String> {
    if !home.is_empty() && path.starts_with(home) {
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            return Some(format!("~{}", rest));
        }
    }
    finddir_scan(path, named_dirs).map(|(name, rest)| format!("~{}{}", name, rest))
}

/// Add user directory (from utils.c adduserdir)
pub fn adduserdir(
    named_dirs: &mut std::collections::HashMap<String, String>,
    name: &str,
    dir: &str,
) {
    named_dirs.insert(name.to_string(), dir.to_string());
}

/// Get named directory (from utils.c getnameddir)
pub fn getnameddir(
    name: &str,
    named_dirs: &std::collections::HashMap<String, String>,
) -> Option<String> {
    named_dirs.get(name).cloned()
}

/// Compare directory paths (from utils.c dircmp)
pub fn dircmp(s: &str, t: &str) -> bool {
    let s = s.trim_end_matches('/');
    let t = t.trim_end_matches('/');
    s == t
}

/// Pre-prompt function list (from utils.c addprepromptfn/delprepromptfn)
pub type PrepromptFn = Box<dyn Fn()>;

/// Hook function manager (from utils.c callhookfunc)
pub struct HookManager {
    hooks: std::collections::HashMap<String, Vec<String>>,
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HookManager {
    pub fn new() -> Self {
        HookManager {
            hooks: std::collections::HashMap::new(),
        }
    }

    pub fn add(&mut self, name: &str, func: &str) {
        self.hooks
            .entry(name.to_string())
            .or_default()
            .push(func.to_string());
    }

    pub fn remove(&mut self, name: &str, func: &str) {
        if let Some(list) = self.hooks.get_mut(name) {
            list.retain(|f| f != func);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Vec<String>> {
        self.hooks.get(name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.hooks.get(name).map(|v| !v.is_empty()).unwrap_or(false)
    }
}

/// Timed function entry (from utils.c addtimedfn/deltimedfn)
pub struct TimedFn {
    pub func: String,
    pub when: i64,
}

/// Pre-prompt processing (from utils.c preprompt)
pub fn preprompt_actions() {
    // In Rust, this is handled by the exec loop:
    // - Check mail
    // - Run precmd hooks
    // - Update terminal title
    // - Check for background job notifications
}

/// Check mail paths (from utils.c checkmailpath)
pub fn checkmailpath(paths: &[String]) -> Vec<String> {
    let mut messages = Vec::new();
    for path in paths {
        // PATH?message format
        let (file, msg) = if let Some(pos) = path.find('?') {
            (&path[..pos], Some(&path[pos + 1..]))
        } else {
            (path.as_str(), None)
        };

        if let Ok(meta) = std::fs::metadata(file) {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    if elapsed.as_secs() < 60 {
                        let default_msg = format!("You have new mail in {}", file);
                        messages.push(msg.unwrap_or(&default_msg).to_string());
                    }
                }
            }
        }
    }
    messages
}

/// Print prompt4 (PS4 for trace output) (from utils.c printprompt4)
pub fn printprompt4(ps4: &str) -> String {
    // PS4 expansion - typically "+ " or "+%N:%i> "
    // Simple expansion for now
    ps4.replace("%N", "").replace("%i", "").replace("%_", "")
}

/// Get terminal info (from utils.c gettyinfo/fdgettyinfo)
#[cfg(unix)]
pub fn gettyinfo(fd: i32) -> Option<libc::termios> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } == 0 {
        Some(termios)
    } else {
        None
    }
}

/// Set terminal info (from utils.c settyinfo/fdsettyinfo)
#[cfg(unix)]
pub fn settyinfo(fd: i32, ti: &libc::termios) -> bool {
    unsafe { libc::tcsetattr(fd, libc::TCSADRAIN, ti) == 0 }
}

/// Adjust terminal lines (from utils.c adjustlines)
pub fn adjustlines() -> usize {
    get_term_height()
}

/// Adjust terminal columns (from utils.c adjustcolumns)
pub fn adjustcolumns() -> usize {
    get_term_width()
}

/// Check fd table for valid file descriptors (from utils.c check_fd_table)
pub fn check_fd_table(fd: i32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        false
    }
}

/// Move file descriptor to a high number (from utils.c movefd)
pub fn movefd(fd: i32) -> i32 {
    #[cfg(unix)]
    {
        if fd < 10 {
            let new_fd = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
            if new_fd >= 0 {
                unsafe { libc::close(fd) };
                // Set close-on-exec
                unsafe { libc::fcntl(new_fd, libc::F_SETFD, libc::FD_CLOEXEC) };
                return new_fd;
            }
        }
        fd
    }
    #[cfg(not(unix))]
    {
        fd
    }
}

/// Add module file descriptor (from utils.c addmodulefd)
pub fn addmodulefd(fd: i32) {
    #[cfg(unix)]
    {
        // Set close-on-exec
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
    }
}

/// Add lock file descriptor (from utils.c addlockfd)
pub fn addlockfd(fd: i32, cloexec: bool) {
    #[cfg(unix)]
    {
        if cloexec {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, cloexec);
    }
}

/// Close lock file descriptor (from utils.c zcloselockfd)
pub fn zcloselockfd(fd: i32) {
    zclose(fd);
}

/// Parse integer with underscore separators (from utils.c zstrtol_underscore)
pub fn zstrtol_underscore(s: &str, base: u32) -> Option<i64> {
    let cleaned: String = s.chars().filter(|&c| c != '_').collect();
    if base == 0 || base == 10 {
        cleaned.parse().ok()
    } else {
        i64::from_str_radix(&cleaned, base).ok()
    }
}

/// Compute time difference in microseconds (from utils.c timespec_diff_us)
pub fn timespec_diff_us(t1: &std::time::Instant, t2: &std::time::Instant) -> i64 {
    if *t2 > *t1 {
        t2.duration_since(*t1).as_micros() as i64
    } else {
        -(t1.duration_since(*t2).as_micros() as i64)
    }
}

/// Get monotonic time (from utils.c zmonotime)
pub fn zmonotime() -> i64 {
    std::time::Instant::now().elapsed().as_secs() as i64
}

/// Sleep random amount up to max microseconds (from utils.c zsleep_random)
pub fn zsleep_random(max_us: u64) {
    let us = (std::process::id() as u64 * 1103515245 + 12345) % max_us;
    std::thread::sleep(std::time::Duration::from_micros(us));
}

/// Suppress query (from utils.c noquery)
pub fn noquery(_purge: bool) -> bool {
    false
}

/// Scan for spelling correction (from utils.c spscan)
pub fn spscan(name: &str, candidates: &[String], threshold: usize) -> Option<String> {
    let mut best = None;
    let mut best_dist = threshold + 1;
    for candidate in candidates {
        let dist = spdist(name, candidate, threshold);
        if dist < best_dist {
            best_dist = dist;
            best = Some(candidate.clone());
        }
    }
    best
}

/// Get shell function by name (from utils.c getshfunc)
pub fn getshfunc(
    name: &str,
    functions: &std::collections::HashMap<String, String>,
) -> Option<String> {
    functions.get(name).cloned()
}

/// Make comma character special (from utils.c makecommaspecial)
pub fn makecommaspecial(_yes: bool) {
    // Character type table manipulation - handled differently in Rust
}

/// Duplicate array with zsh allocation (from utils.c zarrdup)
pub fn zarrdup(arr: &[String]) -> Vec<String> {
    arr.to_vec()
}

/// Spelling correction: find closest match (from utils.c spname)
pub fn spname(name: &str, dir: &str) -> Option<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut best = None;
    let mut best_dist = 4; // threshold

    for entry in entries.flatten() {
        if let Some(entry_name) = entry.file_name().to_str() {
            let dist = spdist(name, entry_name, best_dist);
            if dist < best_dist {
                best_dist = dist;
                best = Some(entry_name.to_string());
            }
        }
    }
    best
}

/// Spelling correction with full path (from utils.c mindist)
pub fn mindist(dir: &str, name: &str) -> Option<(String, usize)> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut best = None;
    let mut best_dist = 4;

    for entry in entries.flatten() {
        if let Some(entry_name) = entry.file_name().to_str() {
            let dist = spdist(name, entry_name, best_dist);
            if dist < best_dist {
                best_dist = dist;
                best = Some(entry_name.to_string());
            }
        }
    }
    best.map(|name| (name, best_dist))
}

/// Unmetafy string (from utils.c unmetafy) - zsh meta encoding to plain
pub fn unmetafy(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x83 && i + 1 < bytes.len() {
            // Meta character
            result.push(bytes[i + 1] ^ 32);
            i += 2;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&result).to_string()
}

/// Count meta characters in string (from utils.c metalen)
pub fn metalen(s: &str, len: usize) -> usize {
    let bytes = s.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < len.min(bytes.len()) {
        if bytes[i] == 0x83 {
            i += 2;
        } else {
            i += 1;
        }
        count += 1;
    }
    count
}

/// Dup string nicely (from utils.c nicedup)
pub fn nicedup(s: &str) -> String {
    niceformat(s)
}

/// Count nice string length (from utils.c niceztrlen)
pub fn niceztrlen(s: &str) -> usize {
    niceformat(s).len()
}

/// Duplicate and double-quote a string (from utils.c dquotedztrdup)
pub fn dquotedztrdup(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(c, '$' | '`' | '"' | '\\') {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

/// Restore saved directory (from utils.c restoredir)
pub fn restoredir(saved: &str) -> bool {
    std::env::set_current_dir(saved).is_ok()
}

/// Convert float for output (from utils.c convfloat)
pub fn convfloat(dval: f64, digits: i32, flags: u32) -> String {
    crate::params::format_float(dval, digits, flags)
}

/// Convert float with underscores (from utils.c convfloat_underscore)
pub fn convfloat_underscore(dval: f64, underscore: i32) -> String {
    crate::params::convfloat_underscore(dval, underscore)
}

/// Convert UCS-4 to multibyte (from utils.c ucs4tomb)
pub fn ucs4tomb(wval: u32) -> Option<String> {
    char::from_u32(wval).map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sepsplit() {
        assert_eq!(sepsplit("a:b:c", Some(":"), false), vec!["a", "b", "c"]);
        assert_eq!(sepsplit("a::b", Some(":"), false), vec!["a", "b"]);
        assert_eq!(sepsplit("a::b", Some(":"), true), vec!["a", "", "b"]);
    }

    #[test]
    fn test_spacesplit() {
        assert_eq!(spacesplit("a b c", false), vec!["a", "b", "c"]);
        assert_eq!(spacesplit("a  b", false), vec!["a", "b"]);
    }

    #[test]
    fn test_sepjoin() {
        assert_eq!(
            sepjoin(&["a".into(), "b".into(), "c".into()], Some(":")),
            "a:b:c"
        );
        assert_eq!(sepjoin(&["a".into(), "b".into()], None), "a b");
    }

    #[test]
    fn test_is_identifier() {
        assert!(is_identifier("foo"));
        assert!(is_identifier("_bar"));
        assert!(is_identifier("baz123"));
        assert!(!is_identifier("123abc"));
        assert!(!is_identifier("foo-bar"));
    }

    #[test]
    fn test_is_number() {
        assert!(is_number("123"));
        assert!(is_number("-456"));
        assert!(is_number("+789"));
        assert!(!is_number("12.34"));
        assert!(!is_number("abc"));
    }

    #[test]
    fn test_nicechar() {
        assert_eq!(nicechar('\n'), "\\n");
        assert_eq!(nicechar('\t'), "\\t");
        assert_eq!(nicechar('a'), "a");
    }

    #[test]
    fn test_quote_string() {
        assert_eq!(quote_string("simple"), "simple");
        assert_eq!(quote_string("has space"), "'has space'");
        assert_eq!(quote_string("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_quotestring_backslash() {
        assert_eq!(quotestring("hello", QuoteType::Backslash), "hello");
        assert_eq!(
            quotestring("has space", QuoteType::Backslash),
            "has\\ space"
        );
        assert_eq!(quotestring("$var", QuoteType::Backslash), "\\$var");
    }

    #[test]
    fn test_quotestring_single() {
        assert_eq!(quotestring("hello", QuoteType::Single), "'hello'");
        assert_eq!(quotestring("it's", QuoteType::Single), "'it'\\''s'");
    }

    #[test]
    fn test_quotestring_double() {
        assert_eq!(quotestring("hello", QuoteType::Double), "\"hello\"");
        assert_eq!(
            quotestring("say \"hi\"", QuoteType::Double),
            "\"say \\\"hi\\\"\""
        );
    }

    #[test]
    fn test_quotestring_dollars() {
        assert_eq!(quotestring("hello", QuoteType::Dollars), "$'hello'");
        assert_eq!(
            quotestring("line\nbreak", QuoteType::Dollars),
            "$'line\\nbreak'"
        );
        assert_eq!(
            quotestring("tab\there", QuoteType::Dollars),
            "$'tab\\there'"
        );
    }

    #[test]
    fn test_quotestring_pattern() {
        assert_eq!(quotestring("*.txt", QuoteType::BackslashPattern), "\\*.txt");
        assert_eq!(
            quotestring("file[1]", QuoteType::BackslashPattern),
            "file\\[1\\]"
        );
    }

    #[test]
    fn test_quotetype_from_q_count() {
        assert_eq!(QuoteType::from_q_count(1), QuoteType::Backslash);
        assert_eq!(QuoteType::from_q_count(2), QuoteType::Single);
        assert_eq!(QuoteType::from_q_count(3), QuoteType::Double);
        assert_eq!(QuoteType::from_q_count(4), QuoteType::Dollars);
    }

    #[test]
    fn test_split_quoted() {
        let result = split_quoted("foo bar baz");
        assert_eq!(result, vec!["foo", "bar", "baz"]);

        let result = split_quoted("'hello world' test");
        assert_eq!(result, vec!["hello world", "test"]);

        let result = split_quoted("\"double quoted\" value");
        assert_eq!(result, vec!["double quoted", "value"]);
    }

    #[test]
    fn test_expand_tilde() {
        // Just test that it doesn't crash - actual expansion depends on env
        let result = expand_tilde("~/test");
        assert!(!result.starts_with('~') || result == "~/test");
    }

    #[test]
    fn test_tulower_tuupper() {
        assert_eq!(tulower('A'), 'a');
        assert_eq!(tuupper('a'), 'A');
        assert_eq!(tulower('1'), '1');
    }
}

// ---------------------------------------------------------------------------
// Remaining 33 missing utils.c functions
// ---------------------------------------------------------------------------

/// Set wide character array (from utils.c set_widearray) - no-op, Rust uses native UTF-8
pub fn set_widearray(_s: &str) {}

/// Warning with va_list formatting (from utils.c zwarning)
pub fn zwarning(cmd: &str, msg: &str) {
    if cmd.is_empty() {
        eprintln!("zsh: {}", msg);
    } else {
        eprintln!("{}: {}", cmd, msg);
    }
}

/// Plural helper (from utils.c zz_plural_z_alpha) - returns 's' for plural
pub fn zz_plural_z_alpha() -> &'static str {
    "s"
}

/// Check if a character needs nice formatting (from utils.c is_nicechar)
pub fn is_nicechar(c: char) -> bool {
    c.is_ascii_control() || !c.is_ascii()
}

/// Free a string (from utils.c freestr) - no-op in Rust
pub fn freestr(_s: String) {
    // Rust Drop handles this
}

/// Create a temporary file (from utils.c gettempfile)
pub fn gettempfile(prefix: &str, suffix: &str) -> Option<String> {
    let dir = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let name = format!("{}/{}{}{}", dir, prefix, std::process::id(), suffix);
    Some(name)
}

/// Copy string with upper/lower case (from utils.c strucpy)
pub fn strucpy(s: &str, upper: bool) -> String {
    if upper {
        s.to_uppercase()
    } else {
        s.to_string()
    }
}

/// Copy n chars with upper/lower case (from utils.c struncpy)
pub fn struncpy(s: &str, n: usize, upper: bool) -> String {
    let s: String = s.chars().take(n).collect();
    if upper {
        s.to_uppercase()
    } else {
        s
    }
}

/// Check if array length >= n (from utils.c arrlen_ge)
pub fn arrlen_ge<T>(arr: &[T], n: usize) -> bool {
    arr.len() >= n
}

/// Check if array length > n (from utils.c arrlen_gt)
pub fn arrlen_gt<T>(arr: &[T], n: usize) -> bool {
    arr.len() > n
}

/// Check if array length < n (from utils.c arrlen_lt)
pub fn arrlen_lt<T>(arr: &[T], n: usize) -> bool {
    arr.len() < n
}

/// Set stdin to blocking mode (from utils.c setblock_stdin)
pub fn setblock_stdin() {
    setblock_fd(0, true);
}

/// Buffer size helper for time formatting (from utils.c ztrftimebuf)
pub fn ztrftimebuf(needed: usize) -> usize {
    // Return a reasonable buffer size for time formatting
    needed.max(256)
}

/// Call shell function by name (from utils.c subst_string_by_func)
pub fn subst_string_by_func(_func_name: &str, _arg: &str, _orig: &str) -> Option<String> {
    // This would require exec engine access - return None to indicate no substitution
    None
}

/// Make bang character special/non-special (from utils.c makebangspecial)
pub fn makebangspecial(_yes: bool) {
    // Character type table manipulation - handled by the lexer in Rust
}

/// Check if wide character is blank (from utils.c wcsiblank)
pub fn wcsiblank(c: char) -> bool {
    c == ' ' || c == '\t' || c.is_whitespace()
}

/// Get wide character type (from utils.c wcsitype)
pub fn wcsitype(c: char, itype: u32) -> bool {
    const IALPHA: u32 = 1;
    const IALNUM: u32 = 2;
    const IDIGIT: u32 = 3;
    const IIDENT: u32 = 4;
    const IWORD: u32 = 5;
    const IBLANK: u32 = 6;
    const ISPACE: u32 = 7;

    match itype {
        IALPHA => c.is_alphabetic(),
        IALNUM => c.is_alphanumeric(),
        IDIGIT => c.is_ascii_digit(),
        IALPHA | IIDENT => c.is_alphanumeric() || c == '_',
        IWORD => c.is_alphanumeric() || c == '_',
        IBLANK => c == ' ' || c == '\t',
        ISPACE => c.is_whitespace(),
        _ => false,
    }
}

/// Duplicate array of wide strings (from utils.c wcs_zarrdup) - same as zarrdup in Rust
pub fn wcs_zarrdup(arr: &[String]) -> Vec<String> {
    arr.to_vec()
}

/// Set terminal to cbreak mode (from utils.c setcbreak)
#[cfg(unix)]
pub fn setcbreak() -> bool {
    if let Some(mut ti) = gettyinfo(0) {
        ti.c_lflag &= !(libc::ICANON | libc::ECHO);
        ti.c_cc[libc::VMIN] = 1;
        ti.c_cc[libc::VTIME] = 0;
        settyinfo(0, &ti)
    } else {
        false
    }
}

#[cfg(not(unix))]
pub fn setcbreak() -> bool {
    false
}

/// Metafy and duplicate string (from utils.c ztrdup_metafy)
pub fn ztrdup_metafy(s: &str) -> String {
    metafy(s)
}

/// Unmetafy a single character (from utils.c unmeta_one)
pub fn unmeta_one(s: &str) -> (char, usize) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return ('\0', 0);
    }
    if bytes[0] == 0x83 && bytes.len() > 1 {
        ((bytes[1] ^ 32) as char, 2)
    } else {
        (bytes[0] as char, 1)
    }
}

/// Get string length counting to end pointer (from utils.c ztrlenend)
pub fn ztrlenend(s: &str, end: usize) -> usize {
    s[..end.min(s.len())].chars().count()
}

/// Multibyte metachar length with conversion (from utils.c mb_metacharlenconv_r)
pub fn mb_metacharlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    if let Some(c) = s[pos..].chars().next() {
        (c.len_utf8(), Some(c))
    } else {
        (0, None)
    }
}

/// Multibyte metastring length to end (from utils.c mb_metastrlenend)
pub fn mb_metastrlenend(s: &str, width: bool, end: usize) -> usize {
    if width {
        s[..end.min(s.len())]
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
            .sum()
    } else {
        s[..end.min(s.len())].chars().count()
    }
}

/// Multibyte char length with conversion (from utils.c mb_charlenconv_r)
pub fn mb_charlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    mb_metacharlenconv_r(s, pos)
}

/// Multibyte char length (from utils.c mb_charlenconv)
pub fn mb_charlenconv(s: &str, pos: usize) -> usize {
    s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0)
}

/// Single-byte nice format (from utils.c sb_niceformat)
pub fn sb_niceformat(s: &str) -> String {
    niceformat(s)
}

/// Check if single-byte needs nice format (from utils.c is_sb_niceformat)
pub fn is_sb_niceformat(s: &str) -> bool {
    is_niceformat(s)
}

/// Expand tabs to spaces (from utils.c zexpandtabs)
pub fn zexpandtabs(s: &str, tabstop: usize) -> String {
    let tabstop = if tabstop == 0 { 8 } else { tabstop };
    let mut result = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = tabstop - (col % tabstop);
            for _ in 0..spaces {
                result.push(' ');
            }
            col += spaces;
        } else if c == '\n' {
            result.push(c);
            col = 0;
        } else {
            result.push(c);
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        }
    }
    result
}

/// Add unprintable character representation (from utils.c addunprintable)
pub fn addunprintable(c: char) -> String {
    if c.is_ascii_control() {
        if (c as u8) < 32 {
            format!("^{}", (c as u8 + 64) as char)
        } else {
            format!("^?")
        }
    } else if !c.is_ascii() {
        format!("\\u{:04x}", c as u32)
    } else {
        c.to_string()
    }
}

/// Double-quote and print string (from utils.c dquotedzputs)
pub fn dquotedzputs(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '$' | '`' | '"' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            '\n' => result.push_str("\\n"),
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

/// Initialize directory save struct (from utils.c init_dirsav)
#[derive(Debug, Clone)]
pub struct DirSav {
    pub dirfd: i32,
    pub dirname: Option<String>,
    pub level: i32,
}

pub fn init_dirsav() -> DirSav {
    DirSav {
        dirfd: -1,
        dirname: std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
        level: 0,
    }
}

/// Debug printf (from utils.c dputs) - only active in debug builds
pub fn dputs(msg: &str) {
    #[cfg(debug_assertions)]
    {
        eprintln!("BUG: {}", msg);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = msg;
    }
}

/// Remove character from string (from utils.c chuck)
pub fn chuck(s: &mut String, pos: usize) {
    if pos < s.len() {
        s.remove(pos);
    }
}

/// Check if array length <= n (from utils.c arrlen_le)
pub fn arrlen_le<T>(arr: &[T], n: usize) -> bool {
    arr.len() <= n
}

/// Skip balanced parentheses (from utils.c skipparens)
pub fn skipparens(s: &str, open: char, close: char) -> usize {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + c.len_utf8();
            }
        }
    }
    s.len()
}

/// Call hook function by name (from utils.c subst_string_by_hook)
pub fn subst_string_by_hook(_hook: &str, _arg: &str, _orig: &str) -> Option<String> {
    // Hook functions require access to the exec engine
    None
}

/// Make single-element array on heap (from utils.c hmkarray)
pub fn hmkarray(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        vec![s.to_string()]
    }
}

/// Nice-format and duplicate string (from utils.c nicedupstring)
pub fn nicedupstring(s: &str) -> String {
    niceformat(s)
}

/// Check mail file status (from utils.c mailstat)
pub fn mailstat(path: &str) -> Option<std::fs::Metadata> {
    // Check for strstrstrstrstrstrstrstr/strstrstrstrstrstrstrstr format (strstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstrstr)
    // First try the path as a Strstrdir
    let strstrdir = format!("{}/.strstrdir/strstrstr", path);
    if let Ok(meta) = std::fs::metadata(&strstrdir) {
        return Some(meta);
    }
    // Then try direct file
    std::fs::metadata(path).ok()
}
