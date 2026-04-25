//! Terminal feature probing for ZLE
//!
//! Port from zsh/Src/Zle/termquery.c (968 lines)
//!
//! Probes the terminal for capabilities using escape sequence queries:
//! device attributes, color support, bracketed paste, clipboard,
//! cursor shape, URL encoding, and OSC sequences.

use std::io::{self, Read, Write};
use std::time::Duration;

/// Terminal capabilities discovered by probing
#[derive(Debug, Clone, Default)]
pub struct TermCapabilities {
    pub truecolor: bool,
    pub bracketed_paste: bool,
    pub clipboard_osc52: bool,
    pub cursor_shape: bool,
    pub osc7_cwd: bool,
    pub osc133_prompt: bool,
    pub sixel_graphics: bool,
    pub kitty_keyboard: bool,
    pub synchronized_output: bool,
    pub unicode_version: Option<String>,
}

/// Default probe timeout (from termquery.c TIMEOUT)
const PROBE_TIMEOUT_MS: u64 = 500;

/// Query the terminal for supported features (from termquery.c query_terminal)
pub fn query_terminal() -> TermCapabilities {
    let mut caps = TermCapabilities::default();

    // Only probe if stdout is a tty
    #[cfg(unix)]
    {
        if unsafe { libc::isatty(1) } != 1 {
            return caps;
        }
    }

    // Send Device Attributes query (DA1): ESC [ c
    if let Ok(response) = send_query("\x1b[c", PROBE_TIMEOUT_MS) {
        parse_device_attributes(&response, &mut caps);
    }

    // Check COLORTERM for truecolor
    if let Ok(ct) = std::env::var("COLORTERM") {
        if ct == "truecolor" || ct == "24bit" {
            caps.truecolor = true;
        }
    }

    // Check for known terminal emulators
    if let Ok(term_program) = std::env::var("TERM_PROGRAM") {
        match term_program.as_str() {
            "iTerm.app" | "WezTerm" | "Alacritty" | "kitty" => {
                caps.truecolor = true;
                caps.bracketed_paste = true;
                caps.osc7_cwd = true;
            }
            _ => {}
        }
    }

    if std::env::var("KITTY_WINDOW_ID").is_ok() {
        caps.kitty_keyboard = true;
        caps.truecolor = true;
    }

    caps
}

/// Send an escape sequence query and read the response
fn send_query(query: &str, timeout_ms: u64) -> io::Result<String> {
    #[cfg(unix)]
    {
        // Set terminal to raw mode for reading response
        let mut old_termios: libc::termios = unsafe { std::mem::zeroed() };
        let has_old = unsafe { libc::tcgetattr(0, &mut old_termios) } == 0;

        if has_old {
            let mut raw = old_termios;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = (timeout_ms / 100).min(255) as u8;
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) };
        }

        // Write query
        let _ = io::stdout().write_all(query.as_bytes());
        let _ = io::stdout().flush();

        // Read response
        let mut response = Vec::new();
        let mut buf = [0u8; 1];
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);

        while std::time::Instant::now() < deadline {
            match io::stdin().read(&mut buf) {
                Ok(1) => {
                    response.push(buf[0]);
                    // Check for terminal response ending characters
                    if buf[0] == b'c'
                        || buf[0] == b'n'
                        || buf[0] == b't'
                        || buf[0] == b'\\'
                        || buf[0] == 0x07
                    {
                        break;
                    }
                }
                Ok(0) => break,
                _ => break,
            }
        }

        // Restore terminal
        if has_old {
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &old_termios) };
        }

        Ok(String::from_utf8_lossy(&response).to_string())
    }

    #[cfg(not(unix))]
    {
        let _ = (query, timeout_ms);
        Ok(String::new())
    }
}

/// Parse DA1 response (from termquery.c handle_query)
fn parse_device_attributes(response: &str, caps: &mut TermCapabilities) {
    // DA1 response format: ESC [ ? Ps ; Ps ; ... c
    // Common parameter values:
    // 4 = sixel graphics
    // 22 = ANSI color
    // 28 = rectangular editing
    if response.contains("?") {
        let params: Vec<&str> = response
            .trim_start_matches("\x1b[?")
            .trim_end_matches('c')
            .split(';')
            .collect();

        for param in params {
            match param.trim() {
                "4" => caps.sixel_graphics = true,
                _ => {}
            }
        }
    }
}

/// Probe for bracketed paste support (from termquery.c)
pub fn probe_bracketed_paste() -> bool {
    // Most modern terminals support this
    if let Ok(term) = std::env::var("TERM") {
        !term.starts_with("dumb") && !term.starts_with("cons")
    } else {
        false
    }
}

/// Handle paste mode (from termquery.c handle_paste)
pub fn enable_bracketed_paste() -> String {
    "\x1b[?2004h".to_string()
}

pub fn disable_bracketed_paste() -> String {
    "\x1b[?2004l".to_string()
}

/// URL encode a string (from termquery.c url_encode)
pub fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Get clipboard via OSC 52 (from termquery.c system_clipget)
pub fn system_clipget() -> Option<String> {
    // OSC 52: ESC ] 52 ; c ; <base64-data> ST
    // This is read asynchronously from the terminal
    None // Requires terminal response handling
}

/// Set clipboard via OSC 52 (from termquery.c system_clipput)
pub fn system_clipput(data: &str) -> String {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let encoder = base64_encode(data.as_bytes());
        buf.extend_from_slice(b"\x1b]52;c;");
        buf.extend_from_slice(encoder.as_bytes());
        buf.extend_from_slice(b"\x1b\\");
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Check if extension is enabled (from termquery.c extension_enabled)
pub fn extension_enabled(name: &str) -> bool {
    match name {
        "bracketed-paste" => probe_bracketed_paste(),
        "truecolor" => std::env::var("COLORTERM")
            .map(|v| v == "truecolor" || v == "24bit")
            .unwrap_or(false),
        "osc7" | "osc133" => std::env::var("TERM_PROGRAM")
            .map(|v| matches!(v.as_str(), "iTerm.app" | "WezTerm" | "kitty"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Set cursor shape (from termquery.c zle_set_cursorform)
pub fn set_cursor_shape(shape: CursorShape) -> String {
    match shape {
        CursorShape::Block => "\x1b[2 q".to_string(),
        CursorShape::Underline => "\x1b[4 q".to_string(),
        CursorShape::Bar => "\x1b[6 q".to_string(),
        CursorShape::BlinkingBlock => "\x1b[1 q".to_string(),
        CursorShape::BlinkingUnderline => "\x1b[3 q".to_string(),
        CursorShape::BlinkingBar => "\x1b[5 q".to_string(),
        CursorShape::Default => "\x1b[0 q".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Default,
    BlinkingBlock,
    Block,
    BlinkingUnderline,
    Underline,
    BlinkingBar,
    Bar,
}

/// Notify terminal of current working directory (from termquery.c notify_pwd)
pub fn notify_pwd(path: &str) -> String {
    // OSC 7: file://hostname/path
    let hostname = crate::utils::gethostname();
    format!("\x1b]7;file://{}{}\x1b\\", hostname, url_encode(path))
}

/// Prompt markers for shell integration (from termquery.c prompt_markers/mark_output)
pub fn prompt_marker_start() -> &'static str {
    "\x1b]133;A\x1b\\" // OSC 133;A = prompt start
}

pub fn prompt_marker_end() -> &'static str {
    "\x1b]133;B\x1b\\" // OSC 133;B = command start
}

pub fn output_marker_start() -> &'static str {
    "\x1b]133;C\x1b\\" // OSC 133;C = command output start
}

pub fn output_marker_end(exit_code: i32) -> String {
    format!("\x1b]133;D;{}\x1b\\", exit_code) // OSC 133;D = command end
}

/// Enable/disable synchronized output (from termquery.c)
pub fn sync_output_start() -> &'static str {
    "\x1b[?2026h"
}

pub fn sync_output_end() -> &'static str {
    "\x1b[?2026l"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("/home/user"), "/home/user");
        assert_eq!(url_encode("/path with spaces"), "/path%20with%20spaces");
        assert_eq!(url_encode("hello&world"), "hello%26world");
    }

    #[test]
    fn test_cursor_shape() {
        assert_eq!(set_cursor_shape(CursorShape::Bar), "\x1b[6 q");
        assert_eq!(set_cursor_shape(CursorShape::Block), "\x1b[2 q");
    }

    #[test]
    fn test_bracketed_paste() {
        assert_eq!(enable_bracketed_paste(), "\x1b[?2004h");
        assert_eq!(disable_bracketed_paste(), "\x1b[?2004l");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
    }

    #[test]
    fn test_prompt_markers() {
        assert!(prompt_marker_start().contains("133;A"));
        assert!(prompt_marker_end().contains("133;B"));
    }
}
