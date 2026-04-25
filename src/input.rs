//! Input buffering and stack management for zshrs
//!
//! Direct port from zsh/Src/input.c
//!
//! This module handles:
//! - Reading input from files, strings, and the line editor
//! - Input stack for alias expansion and history substitution
//! - Character-by-character input with push-back support
//! - Meta-character encoding for internal tokens

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read};

/// Size of the shell input buffer
const SHIN_BUF_SIZE: usize = 8192;

/// Initial input stack size
const INSTACK_INITIAL: usize = 4;

/// Input flags
pub mod flags {
    pub const INP_FREE: u32 = 0x01; // Free input string when done
    pub const INP_CONT: u32 = 0x02; // Continue to next stack element
    pub const INP_ALIAS: u32 = 0x04; // Input is alias expansion
    pub const INP_HIST: u32 = 0x08; // Input is history expansion
    pub const INP_LINENO: u32 = 0x10; // Increment line number on newline
    pub const INP_APPEND: u32 = 0x20; // Append to existing input
    pub const INP_ALCONT: u32 = 0x40; // Alias continuation marker
    pub const INP_HISTCONT: u32 = 0x80; // History continuation marker
    pub const INP_RAW_KEEP: u32 = 0x100; // Keep raw input for history
}

/// An entry on the input stack
#[derive(Debug, Clone)]
struct InputStackEntry {
    /// The input buffer
    buf: String,
    /// Current position in buffer
    pos: usize,
    /// Flags for this input level
    flags: u32,
    /// Associated alias name (if any)
    alias: Option<String>,
}

impl Default for InputStackEntry {
    fn default() -> Self {
        InputStackEntry {
            buf: String::new(),
            pos: 0,
            flags: 0,
            alias: None,
        }
    }
}

/// Input buffer state
pub struct InputBuffer {
    /// Stack of input sources
    stack: Vec<InputStackEntry>,
    /// Current input buffer
    buf: String,
    /// Position in current buffer
    pos: usize,
    /// Current flags
    flags: u32,
    /// Total characters available
    pub buf_ct: usize,
    /// Whether we're reading from a string
    pub strin: bool,
    /// Current line number
    pub lineno: usize,
    /// Stop lexing flag
    pub lexstop: bool,
    /// Shell input file descriptor buffer
    shin_buffer: String,
    /// Position in SHIN buffer
    shin_pos: usize,
    /// Stack of saved SHIN buffers
    shin_save_stack: Vec<(String, usize)>,
    /// Push-back buffer for characters
    pushback: VecDeque<char>,
    /// Raw input accumulator for history
    raw_input: String,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBuffer {
    pub fn new() -> Self {
        InputBuffer {
            stack: Vec::with_capacity(INSTACK_INITIAL),
            buf: String::new(),
            pos: 0,
            flags: 0,
            buf_ct: 0,
            strin: false,
            lineno: 1,
            lexstop: false,
            shin_buffer: String::new(),
            shin_pos: 0,
            shin_save_stack: Vec::new(),
            pushback: VecDeque::new(),
            raw_input: String::new(),
        }
    }

    /// Reset the SHIN buffer
    pub fn shin_buf_reset(&mut self) {
        self.shin_buffer.clear();
        self.shin_pos = 0;
    }

    /// Allocate a new SHIN buffer
    pub fn shin_buf_alloc(&mut self) {
        self.shin_buffer = String::with_capacity(SHIN_BUF_SIZE);
        self.shin_buf_reset();
    }

    /// Save current SHIN buffer state
    pub fn shin_buf_save(&mut self) {
        self.shin_save_stack
            .push((std::mem::take(&mut self.shin_buffer), self.shin_pos));
        self.shin_buf_alloc();
    }

    /// Restore saved SHIN buffer state
    pub fn shin_buf_restore(&mut self) {
        if let Some((buffer, pos)) = self.shin_save_stack.pop() {
            self.shin_buffer = buffer;
            self.shin_pos = pos;
        }
    }

    /// Get next character from a reader
    pub fn shin_getchar<R: Read>(&mut self, reader: &mut BufReader<R>) -> Option<char> {
        // First check if we have buffered data
        if self.shin_pos < self.shin_buffer.len() {
            let ch = self.shin_buffer.chars().nth(self.shin_pos)?;
            self.shin_pos += 1;
            return Some(ch);
        }

        // Need to read more data
        self.shin_buf_reset();
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => {
                self.shin_buffer = line;
                self.shin_pos = 1;
                self.shin_buffer.chars().next()
            }
            Err(_) => None,
        }
    }

    /// Read a line from shell input, encoding meta characters
    pub fn shin_getline<R: Read>(&mut self, reader: &mut BufReader<R>) -> Option<String> {
        let mut result = String::new();

        loop {
            match self.shin_getchar(reader) {
                None => {
                    if result.is_empty() {
                        return None;
                    }
                    return Some(result);
                }
                Some('\n') => {
                    result.push('\n');
                    return Some(result);
                }
                Some(c) => {
                    if is_meta(c) {
                        result.push(META);
                        result.push(meta_encode(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        }
    }

    /// Get the next character from input
    pub fn ingetc(&mut self) -> Option<char> {
        if self.lexstop {
            return Some(' ');
        }

        // Check pushback buffer first
        if let Some(c) = self.pushback.pop_front() {
            self.raw_add(c);
            return Some(c);
        }

        loop {
            // Try to get from current buffer
            if self.pos < self.buf.len() {
                let c = self.buf.chars().nth(self.pos)?;
                self.pos += 1;
                self.buf_ct = self.buf_ct.saturating_sub(1);

                // Skip internal tokens
                if is_tok(c) {
                    continue;
                }

                // Track line numbers
                if ((self.flags & flags::INP_LINENO != 0) || !self.strin) && c == '\n' {
                    self.lineno += 1;
                }

                self.raw_add(c);
                return Some(c);
            }

            // Check if we've reached end of input
            if self.buf_ct == 0 && (self.strin || self.lexstop) {
                self.lexstop = true;
                return None;
            }

            // If continuation, pop the stack
            if self.flags & flags::INP_CONT != 0 {
                self.inpop_top();
                continue;
            }

            // No more input available
            self.lexstop = true;
            return None;
        }
    }

    /// Push a character back into input
    pub fn inungetc(&mut self, c: char) {
        if self.lexstop {
            return;
        }

        if self.pos > 0 {
            self.pos -= 1;
            self.buf_ct += 1;
            if ((self.flags & flags::INP_LINENO != 0) || !self.strin) && c == '\n' {
                self.lineno = self.lineno.saturating_sub(1);
            }
            self.raw_back();
        } else if self.flags & flags::INP_CONT == 0 {
            // Can't back up at start - push as new input
            self.pushback.push_front(c);
        } else {
            // Push onto pushback for continuation
            self.pushback.push_front(c);
        }
    }

    /// Push a string onto the input stack
    pub fn inpush(&mut self, s: &str, flags: u32, alias: Option<String>) {
        // Save current state
        let entry = InputStackEntry {
            buf: std::mem::take(&mut self.buf),
            pos: self.pos,
            flags: self.flags,
            alias: None,
        };
        self.stack.push(entry);

        // Set up new input
        self.buf = s.to_string();
        self.pos = 0;

        // Handle alias/history flags
        let mut new_flags = flags;
        if flags & (flags::INP_ALIAS | flags::INP_HIST) != 0 {
            new_flags |= flags::INP_CONT | flags::INP_ALIAS;
            if let Some(ref a) = alias {
                // Mark alias as in use
                if let Some(last) = self.stack.last_mut() {
                    last.alias = Some(a.clone());
                    if flags & flags::INP_HIST != 0 {
                        last.flags |= flags::INP_HISTCONT;
                    } else {
                        last.flags |= flags::INP_ALCONT;
                    }
                }
            }
        }

        // Update counts
        if new_flags & flags::INP_CONT != 0 {
            self.buf_ct += self.buf.len();
        } else {
            self.buf_ct = self.buf.len();
        }
        self.flags = new_flags;
    }

    /// Pop the top entry from the input stack
    fn inpop_top(&mut self) {
        if let Some(entry) = self.stack.pop() {
            self.buf = entry.buf;
            self.pos = entry.pos;
            self.flags = entry.flags;
            self.buf_ct = self.buf.len().saturating_sub(self.pos);

            // Handle alias continuation
            if self.flags & (flags::INP_ALCONT | flags::INP_HISTCONT) != 0 {
                // Mark alias as no longer in use
                // Check for trailing space (inalmore)
            }
        }
    }

    /// Pop the stack including all continuations
    pub fn inpop(&mut self) {
        loop {
            let was_cont = self.flags & flags::INP_CONT != 0;
            self.inpop_top();
            if !was_cont {
                break;
            }
        }
    }

    /// Expunge any aliases from the input stack
    pub fn inpop_alias(&mut self) {
        while self.flags & flags::INP_ALIAS != 0 {
            self.inpop_top();
        }
    }

    /// Set the input line directly
    pub fn inputsetline(&mut self, s: &str, flags: u32) {
        self.buf = s.to_string();
        self.pos = 0;

        if flags & flags::INP_CONT != 0 {
            self.buf_ct += self.buf.len();
        } else {
            self.buf_ct = self.buf.len();
        }
        self.flags = flags;
    }

    /// Flush remaining input (on error)
    pub fn inerrflush(&mut self) {
        while !self.lexstop && self.buf_ct > 0 {
            let _ = self.ingetc();
        }
    }

    /// Get pointer to remaining input
    pub fn ingetptr(&self) -> &str {
        if self.pos < self.buf.len() {
            &self.buf[self.pos..]
        } else {
            ""
        }
    }

    /// Check if current input is from an alias
    pub fn input_has_alias(&self) -> Option<&str> {
        let mut flags = self.flags;

        for entry in self.stack.iter().rev() {
            if flags & flags::INP_CONT == 0 {
                break;
            }
            if let Some(ref alias) = entry.alias {
                return Some(alias);
            }
            flags = entry.flags;
        }
        None
    }

    /// Add character to raw input accumulator
    fn raw_add(&mut self, c: char) {
        self.raw_input.push(c);
    }

    /// Remove last character from raw input
    fn raw_back(&mut self) {
        self.raw_input.pop();
    }

    /// Get and clear raw input
    pub fn take_raw_input(&mut self) -> String {
        std::mem::take(&mut self.raw_input)
    }

    /// Check if we have pending input
    pub fn has_input(&self) -> bool {
        self.buf_ct > 0 || !self.pushback.is_empty()
    }

    /// Get remaining character count
    pub fn remaining(&self) -> usize {
        self.buf_ct + self.pushback.len()
    }
}

/// Meta character marker
pub const META: char = '\u{83}';

/// Check if a character needs meta encoding
fn is_meta(c: char) -> bool {
    let b = c as u32;
    b < 32 || (b >= 0x83 && b <= 0x9b)
}

/// Check if a character is an internal token
fn is_tok(c: char) -> bool {
    let b = c as u32;
    b >= 0x83 && b <= 0x9b
}

/// Encode a meta character
fn meta_encode(c: char) -> char {
    char::from_u32((c as u32) ^ 32).unwrap_or(c)
}

/// Decode a meta character
pub fn meta_decode(c: char) -> char {
    char::from_u32((c as u32) ^ 32).unwrap_or(c)
}

/// Read entire file into memory
pub fn zstuff(path: &str) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// String input source for simple string parsing
pub struct StringInput {
    input: InputBuffer,
}

impl StringInput {
    pub fn new(s: &str) -> Self {
        let mut input = InputBuffer::new();
        input.strin = true;
        input.inputsetline(s, 0);
        StringInput { input }
    }

    pub fn getc(&mut self) -> Option<char> {
        self.input.ingetc()
    }

    pub fn ungetc(&mut self, c: char) {
        self.input.inungetc(c);
    }

    pub fn is_eof(&self) -> bool {
        self.input.lexstop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_buffer_basic() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("hello", 0);

        assert_eq!(buf.ingetc(), Some('h'));
        assert_eq!(buf.ingetc(), Some('e'));
        assert_eq!(buf.ingetc(), Some('l'));
        assert_eq!(buf.ingetc(), Some('l'));
        assert_eq!(buf.ingetc(), Some('o'));
        assert_eq!(buf.ingetc(), None);
    }

    #[test]
    fn test_input_ungetc() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("abc", 0);

        assert_eq!(buf.ingetc(), Some('a'));
        assert_eq!(buf.ingetc(), Some('b'));
        buf.inungetc('b');
        assert_eq!(buf.ingetc(), Some('b'));
        assert_eq!(buf.ingetc(), Some('c'));
    }

    #[test]
    fn test_input_stack() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("outer", 0);

        assert_eq!(buf.ingetc(), Some('o'));

        // Push new input
        buf.inpush("inner", flags::INP_CONT, None);
        assert_eq!(buf.ingetc(), Some('i'));
        assert_eq!(buf.ingetc(), Some('n'));
        assert_eq!(buf.ingetc(), Some('n'));
        assert_eq!(buf.ingetc(), Some('e'));
        assert_eq!(buf.ingetc(), Some('r'));

        // Should continue to outer
        assert_eq!(buf.ingetc(), Some('u'));
        assert_eq!(buf.ingetc(), Some('t'));
    }

    #[test]
    fn test_line_number_tracking() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("a\nb\nc", flags::INP_LINENO);

        assert_eq!(buf.lineno, 1);
        buf.ingetc(); // a
        buf.ingetc(); // \n
        assert_eq!(buf.lineno, 2);
        buf.ingetc(); // b
        buf.ingetc(); // \n
        assert_eq!(buf.lineno, 3);
    }

    #[test]
    fn test_string_input() {
        let mut input = StringInput::new("test");

        assert_eq!(input.getc(), Some('t'));
        assert_eq!(input.getc(), Some('e'));
        assert_eq!(input.getc(), Some('s'));
        assert_eq!(input.getc(), Some('t'));
        assert!(input.is_eof() || input.getc().is_none());
    }

    #[test]
    fn test_meta_encoding() {
        assert!(is_meta('\x00'));
        assert!(is_meta('\x1f'));
        assert!(!is_meta('a'));
        assert!(!is_meta('Z'));

        let encoded = meta_encode('\x00');
        let decoded = meta_decode(encoded);
        assert_eq!(decoded, '\x00');
    }

    #[test]
    fn test_ingetptr() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("hello world", 0);

        buf.ingetc(); // h
        buf.ingetc(); // e
        buf.ingetc(); // l
        buf.ingetc(); // l
        buf.ingetc(); // o

        assert_eq!(buf.ingetptr(), " world");
    }

    #[test]
    fn test_inerrflush() {
        let mut buf = InputBuffer::new();
        buf.inputsetline("remaining input", 0);

        buf.ingetc(); // consume one char
        buf.inerrflush();

        assert!(buf.lexstop || buf.buf_ct == 0);
    }
}
