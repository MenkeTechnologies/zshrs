//! Input buffering and stack management for zshrs
//!
//! Direct port from zsh/Src/input.c
//!
//! the shell input fd                                                       // c:78
//! total # of characters waiting to be read                                 // c:88
//! the flags controlling the input routines in input.c                      // c:93
//! Reset the input buffer for SHIN, discarding any pending input            // c:155
//! stuff a whole file into memory and return it                             // c:610
//! flush input queue                                                        // c:661
//!
//! This module handles:
//! - Reading input from files, strings, and the line editor
//! - Input stack for alias expansion and history substitution
//! - Character-by-character input with push-back support
//! - Meta-character encoding for internal tokens

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read};

// Size of buffer for non-interactive command input                        // c:127
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
#[derive(Debug, Clone, Default)]
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

/// Input buffer + nested input stack.
/// Port of the file-static `instack` + `inbuf` / `inbufpos` /
/// `inbufct` / `lexstop` / `strin` slots Src/input.c keeps —
/// `inpush()` (line 675) pushes a new level, `inpop()` (line 785)
/// pops it, `ingetc()` (line 318) reads from the top.
pub struct InputBuffer {
    /// Stack of input sources
    stack: Vec<InputStackEntry>,
    /// Current input buffer
    buf: String,
    /// Position in current buffer
    pos: usize,
    /// Current flags
    pub flags: u32,
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

    /// Reset the SHIN (shell input fd) buffer.
    /// Port of `shinbufreset()` from Src/input.c:159.
    pub fn shin_buf_reset(&mut self) {                                       // c:159
        self.shin_buffer.clear();
        self.shin_pos = 0;
    }

    /// Allocate a new SHIN buffer.
    /// Port of `shinbufalloc()` from Src/input.c:171.
    pub fn shin_buf_alloc(&mut self) {                                       // c:171
        self.shin_buffer = String::with_capacity(SHIN_BUF_SIZE);
        self.shin_buf_reset();
    }

    /// Save the current SHIN buffer state.
    /// Port of `shinbufsave()` from Src/input.c:181 — pushes the
    /// existing buffer onto a save-stack and starts a fresh one
    /// for nested `eval`/`source` contexts.
    pub fn shin_buf_save(&mut self) {
        self.shin_save_stack
            .push((std::mem::take(&mut self.shin_buffer), self.shin_pos));
        self.shin_buf_alloc();
    }

    /// Restore the previously-saved SHIN buffer state.
    /// Port of `shinbufrestore()` from Src/input.c:200.
    pub fn shin_buf_restore(&mut self) {                                     // c:200
        if let Some((buffer, pos)) = self.shin_save_stack.pop() {
            self.shin_buffer = buffer;
            self.shin_pos = pos;
        }
    }

    /// Read the next character from the underlying reader,
    /// refilling the SHIN buffer as needed.
    /// Port of `shingetchar()` from Src/input.c:218.
    pub fn shin_getchar<R: Read>(&mut self, reader: &mut BufReader<R>) -> Option<char> { // c:218
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
    /// Read a full line from the underlying reader.
    /// Port of `shingetline()` from Src/input.c:267.
    pub fn shin_getline<R: Read>(&mut self, reader: &mut BufReader<R>) -> Option<String> { // c:267
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
                    if imeta(c) {
                        // Inline metafy XOR per Src/utils.c:4856 metafy()
                        // and Src/zsh.h Meta protocol — c ^ 32 maps the
                        // 5 reserved bytes (0x00, 0x83-0x9b) to printable
                        // form for the SHIN buffer.
                        result.push(META);
                        result.push(char::from_u32((c as u32) ^ 32).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        }
    }

    /// Get the next character from input
    // Will call inputline() to get a new line where necessary.             // c:313
    /// Get the next char from the active input source.
    /// Port of `ingetc()` from Src/input.c:318 — drives the
    /// lexer; consumes pushback first, then top-of-stack input.
    pub fn ingetc(&mut self) -> Option<char> {                               // c:318
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

                // Skip internal tokens (range 0x83..=0x9b)
                if (0x83..=0x9b).contains(&(c as u32)) {
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
    /// Push a character back onto the input stream.
    /// Port of `inungetc()` from Src/input.c:546.
    pub fn inungetc(&mut self, c: char) {                                    // c:546
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
    /// Push a new input source onto the stack.
    /// Port of `inpush()` from Src/input.c:675 — used for
    // Set some new input onto a new element of the input stack             // c:671
    // Set some new input onto a new element of the input stack               // c:671
    /// `eval`/`source`, alias expansion, and process
    /// substitution to layer a new input on top of the current
    /// one.
    pub fn inpush(&mut self, s: &str, flags: u32, alias: Option<String>) {   // c:675
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

    // Remove the top element of the stack and all its continuations.        // c:781
    /// Pop the top input source.
    /// Port of `inpop()` from Src/input.c:785.
    pub fn inpop(&mut self) {                                                // c:785
        loop {
            let was_cont = self.flags & flags::INP_CONT != 0;
            self.inpop_top();
            if !was_cont {
                break;
            }
        }
    }

    /// Expunge any aliases from the input stack
    /// Pop the top input level only if it's an alias frame.
    /// Port of `inpopalias()` from Src/input.c:804 — used to
    /// unwind alias expansion without disturbing the underlying
    /// source.
    pub fn inpop_alias(&mut self) {
        while self.flags & flags::INP_ALIAS != 0 {
            self.inpop_top();
        }
    }

    /// Set the input line directly
    /// Replace the current input line.
    /// Port of `inputsetline()` from Src/input.c:510.
    pub fn inputsetline(&mut self, s: &str, flags: u32) {                    // c:510
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
    /// Discard pending input after a parse error.
    /// Port of `inerrflush()` from Src/input.c:665.
    pub fn inerrflush(&mut self) {                                           // c:665
        while !self.lexstop && self.buf_ct > 0 {
            let _ = self.ingetc();
        }
    }

    /// Get pointer to remaining input
    /// Get a slice of the unread portion of the current
    /// input.
    /// Port of `ingetptr()` from Src/input.c:817.
    pub fn ingetptr(&self) -> &str {                                         // c:817
        if self.pos < self.buf.len() {
            &self.buf[self.pos..]
        } else {
            ""
        }
    }

    /// Check if current input is from an alias
    /// Look up an active alias frame on the input stack.
    /// Port of `imeta()` from Src/input.c:831 — the
    /// alias-loop guard the lexer uses to avoid recursing into
    /// the same alias twice.
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
fn imeta(c: char) -> bool {
    let b = c as u32;
    b < 32 || (0x83..=0x9b).contains(&b)
}

/// Read entire file into memory
/// Read a file as a string for `source`/`stuff` semantics.
/// Port of `zstuff()` from Src/input.c:614 — the C source uses
/// it for `Functions/Misc/run-help` and similar autoload paths.
pub fn zstuff(path: &str) -> io::Result<String> {                            // c:614
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
        assert!(imeta('\x00'));
        assert!(imeta('\x1f'));
        assert!(!imeta('a'));
        assert!(!imeta('Z'));

        // Verify the inlined metafy XOR (Src/utils.c:4856 c ^ 32) is
        // self-inverting — encode then decode round-trips to the input.
        let encoded = char::from_u32(('\x00' as u32) ^ 32).unwrap_or('\x00');
        let decoded = char::from_u32((encoded as u32) ^ 32).unwrap_or(encoded);
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

// ===========================================================
// Direct ports of static input-buffer helpers from Src/input.c.
// The C source uses file-static globals (`shinbuffer`, `shinbufptr`,
// `shinsavestack`, `instack`, `inbuf*`); the Rust port mirrors them
// with a thread-local InputBuffer accessed by these free fns.
// ===========================================================

thread_local! {
    /// Singleton InputBuffer mirroring the C source's file-static
    /// SHIN + input-stack globals. The free-fn ports below all
    /// dispatch through this so the public ABI matches Src/input.c.
    static INPUT: std::cell::RefCell<InputBuffer> = std::cell::RefCell::new(InputBuffer::new());
}

/// Reset the SHIN pushback buffer.
/// Port of `shinbufreset()` from Src/input.c:159 —
/// `shinbufendptr = shinbufptr = shinbuffer`.
pub fn shinbufreset() {                                                      // c:159
    INPUT.with(|b| b.borrow_mut().shin_buf_reset());
}

/// Allocate a fresh SHIN buffer.
/// Port of `shinbufalloc()` from Src/input.c:171.
pub fn shinbufalloc() {                                                      // c:171
    INPUT.with(|b| b.borrow_mut().shin_buf_alloc());
}

/// Save the current SHIN buffer onto the save stack.
/// Port of `shinbufsave()` from Src/input.c:181 — push the
/// existing buffer onto a save-stack and start a fresh one for
/// nested `eval`/`source` contexts.
pub fn shinbufsave() {                                                       // c:181
    INPUT.with(|b| b.borrow_mut().shin_buf_save());
}

/// Pop the top of the SHIN save stack back into the live buffer.
/// Port of `shinbufrestore()` from Src/input.c:200.
pub fn shinbufrestore() {                                                    // c:200
    INPUT.with(|b| b.borrow_mut().shin_buf_restore());
}

// Get a character from SHIN, -1 if none available                         // c:214
/// Read one byte from SHIN; returns -1 on EOF.
/// Port of `shingetchar()` from Src/input.c:218. C source pulls
/// from `shinbuffer` first then falls through to `read(2)` on the
/// SHIN fd; Rust mirrors via the InputBuffer's `shin_getchar`
/// reading from `std::io::stdin`.
pub fn shingetchar() -> i32 {                                               // c:218
    use std::io::BufReader;
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    INPUT.with(|b| match b.borrow_mut().shin_getchar(&mut reader) {
        Some(c) => c as i32,
        None => -1,
    })
}

/// Read a full line from SHIN, with `\n` preserved.
/// Port of `shingetline()` from Src/input.c:267 — calls
/// `shingetchar` in a loop, metafies high bytes, returns NULL
/// (`""`) on EOF.
pub fn shingetline() -> String {                                             // c:267
    use std::io::BufReader;
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    INPUT.with(|b| b.borrow_mut().shin_getline(&mut reader).unwrap_or_default())
}

// Read a line from the current command stream and store it as input       // c:362
/// Read one line into the input stack.
/// Port of `inputline()` from Src/input.c:366. C source dispatches
/// between zle / non-zle paths and `shingetline` /
/// `zleentry(READ)`. Rust port reads via shingetline (no zle yet),
/// returns "" on EOF and sets lexstop the same way.
pub fn inputline() -> String {                                              // c:366
    let line = shingetline();
    if line.is_empty() {
        INPUT.with(|b| b.borrow_mut().lexstop = true);
    }
    line
}

/// Stuff a whole file into the input queue.
/// Port of `stuff()` from Src/input.c:647 — read the file, echo
/// it to stderr, push onto the input stack.
pub fn stuff(filename: &str) -> i32 {                                        // c:647
    use std::io::Write;
    let buf = match std::fs::read_to_string(filename) {
        Ok(b) => b,
        Err(_) => return 1,
    };
    let _ = std::io::stderr().write_all(buf.as_bytes());
    let _ = std::io::stderr().flush();
    INPUT.with(|b| {
        b.borrow_mut().inpush(&buf, flags::INP_FREE, None);
    });
    0
}

// Remove the top element of the stack                                       // c:732
/// Pop the topmost input-stack frame.
/// Port of `inpoptop()` from Src/input.c:736.
pub fn inpoptop() {                                                          // c:736
    INPUT.with(|b| b.borrow_mut().inpop());
}

/// Pop all input frames added by alias expansion.
/// Port of `inpopalias()` from Src/input.c:804.
pub fn inpopalias() {                                                        // c:804
    INPUT.with(|b| b.borrow_mut().inpop_alias());
}
