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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read};
use std::io::Write;

/// Port of `struct instacks` from `Src/input.c:109`. One frame in
/// the input stack — pushed by `inpush()` and popped by `inpoptop()`
/// to layer alias expansion / history-substitution / `eval`
/// continuations over the active input.
#[derive(Clone, Default)]
#[allow(non_camel_case_types)]
struct instacks {                                                            // c:109
    buf: String,                                                             // c:110 char *buf
    bufpos: usize,                                                           // c:110 char *bufptr offset
    flags: i32,                                                              // c:112 int flags
    alias: Option<String>,                                                   // c:111 Alias alias
}

/// Initial input stack size
#[allow(dead_code)]
const INSTACK_INITIAL: usize = 4;                                            // c:122

// `pub mod flags { … INP_* … }` deleted — Rust-only namespace with
// values that diverged from the C `#define INP_FREE (1<<0)` etc. at
// Src/zsh.h:467-476. The canonical mirror lives in
// `crate::ported::zsh_h::INP_*` (matching the C bit positions
// exactly); this file uses those constants directly.
use crate::ported::zsh_h::{
    INP_ALCONT, INP_ALIAS, INP_APPEND, INP_CONT, INP_FREE, INP_HIST,
    INP_HISTCONT, INP_LINENO, INP_RAW_KEEP,
};

// ---------------------------------------------------------------------------
// SHIN buffer helpers — direct ports of input.c:159/171/181/200/218/267.
// ---------------------------------------------------------------------------

/// Reset the SHIN pushback buffer.
/// Port of `shinbufreset()` from Src/input.c:159 —
/// `shinbufendptr = shinbufptr = shinbuffer`.
pub fn shinbufreset() {                                                      // c:159
    shinbuffer.with(|b| b.borrow_mut().clear());
    shinbufpos.with(|p| p.set(0));
}

// ---------------------------------------------------------------------------
// File-scope mirrors of `Src/input.c` globals. Per-thread because each
// worker parses independently; the C source's process-global model
// doesn't translate directly to zshrs's parallel pipeline.
// ---------------------------------------------------------------------------

thread_local! {
    /// Port of `int SHIN` from `Src/input.c:81`. Shell input fd
    /// (typically 0 for stdin).
    #[allow(non_upper_case_globals)]
    pub static SHIN: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };

    /// Port of `int strin` from `Src/input.c:86`. Non-zero while
    /// reading from a string (via `inpush` with INP_ALIAS/INP_HIST
    /// or by `bin_eval`); short-circuits `read(2)` fallback.
    #[allow(non_upper_case_globals)]
    pub static strin: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };

    /// Port of `mod_export int inbufct` from `Src/input.c:91`.
    /// Total characters waiting to be read across `inbuf` +
    /// `instack` entries.
    #[allow(non_upper_case_globals)]
    pub static inbufct: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };

    /// Port of `int inbufflags` from `Src/input.c:96`. Bit-mask of
    /// the `INP_*` flags governing the current input level.
    #[allow(non_upper_case_globals)]
    pub static inbufflags: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };

    /// Port of `static char *inbuf` from `Src/input.c:98`. Current
    /// input buffer.
    #[allow(non_upper_case_globals)]
    static inbuf: RefCell<String> = const { RefCell::new(String::new()) };

    /// Port of `static char *inbufptr` from `Src/input.c:99`. Offset
    /// into `inbuf` where the next char will be read.
    #[allow(non_upper_case_globals)]
    static inbufpos: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    /// Input stack — port of `static struct instacks *instack` from
    /// `Src/input.c:114`. The instacktop pointer in C maps to the
    /// Vec's length here.
    static instack: RefCell<Vec<instacks>> = const { RefCell::new(Vec::new()) };

    /// `lexstop` — set when the lexer should stop pulling chars.
    /// C mirrors this in zsh.h as an extern; per-thread here.
    #[allow(non_upper_case_globals)]
    pub static lexstop: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

    /// Current line number for diagnostics. C `lineno` global.
    #[allow(non_upper_case_globals)]
    pub static lineno: std::cell::Cell<usize> = const { std::cell::Cell::new(1) };

    /// SHIN read buffer — C `shinbuffer`.
    #[allow(non_upper_case_globals)]
    static shinbuffer: RefCell<String> = const { RefCell::new(String::new()) };

    /// SHIN read offset — C `shinbufptr`.
    #[allow(non_upper_case_globals)]
    static shinbufpos: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    /// SHIN save stack — C `shinsavestack`.
    static shinsavestack: RefCell<Vec<(String, usize)>> = const { RefCell::new(Vec::new()) };

    /// Pushback queue for `inungetc`. zshrs-specific; C inlines a
    /// single inbufptr-decrement which can't model arbitrary-length
    /// pushback from a different buffer.
    static pushback: RefCell<VecDeque<char>> = const { RefCell::new(VecDeque::new()) };

    /// Raw-input accumulator for history. zshrs-specific.
    static raw_input: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Allocate a fresh SHIN buffer.
/// Port of `shinbufalloc()` from Src/input.c:171.
pub fn shinbufalloc() {                                                      // c:171
    shinbuffer.with(|b| {
        *b.borrow_mut() = String::with_capacity(SHIN_BUF_SIZE);
    });
    shinbufreset();
}

/// Save the current SHIN buffer onto the save stack.
/// Port of `shinbufsave()` from Src/input.c:181 — push the
/// existing buffer onto a save-stack and start a fresh one for
/// nested `eval`/`source` contexts.
pub fn shinbufsave() {                                                       // c:181
    let (snap_buf, snap_pos) = (
        shinbuffer.with(|b| std::mem::take(&mut *b.borrow_mut())),
        shinbufpos.with(|p| p.replace(0)),
    );
    shinsavestack.with(|s| s.borrow_mut().push((snap_buf, snap_pos)));
    shinbufalloc();
}

/// Pop the top of the SHIN save stack back into the live buffer.
/// Port of `shinbufrestore()` from Src/input.c:200.
pub fn shinbufrestore() {                                                    // c:200
    if let Some((buf, pos)) = shinsavestack.with(|s| s.borrow_mut().pop()) {
        shinbuffer.with(|b| *b.borrow_mut() = buf);
        shinbufpos.with(|p| p.set(pos));
    }
}

// Get a character from SHIN, -1 if none available                           // c:218
/// Read one byte from SHIN; returns -1 on EOF.
/// Port of `shingetchar()` from Src/input.c:218. C source pulls
/// from `shinbuffer` first then falls through to `read(2)` on the
/// SHIN fd; Rust mirrors by reading from `std::io::stdin`.
pub fn shingetchar() -> i32 {                                                // c:218
    // c:218-228 — `if (shinbufptr < shinbufendptr) return *shinbufptr++;`
    let bufd = shinbuffer.with(|b| b.borrow().clone());
    let pos = shinbufpos.with(|p| p.get());
    if pos < bufd.len() {
        if let Some(ch) = bufd.chars().nth(pos) {
            shinbufpos.with(|p| p.set(pos + 1));
            return ch as i32;
        }
    }
    // c:230-258 — refill via `read(SHIN, ...)`.
    shinbufreset();
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => -1,
        Ok(_) => {
            let first = line.chars().next().map(|c| c as i32).unwrap_or(-1);
            shinbuffer.with(|b| *b.borrow_mut() = line);
            shinbufpos.with(|p| p.set(1));
            first
        }
        Err(_) => -1,
    }
}

/// Read a full line from SHIN, with `\n` preserved.
/// Port of `shingetline()` from Src/input.c:267 — calls
/// `shingetchar` in a loop, metafies high bytes, returns NULL
/// (`""`) on EOF.
pub fn shingetline() -> String {                                             // c:267
    let mut result = String::new();
    loop {
        match shingetchar() {
            -1 => return result,
            ch_i32 => {
                let c = char::from_u32(ch_i32 as u32).unwrap_or('\0');
                if c == '\n' {
                    result.push('\n');
                    return result;
                }
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

// ---------------------------------------------------------------------------
// ingetc / inungetc / inpush / inpop / inpopalias — Src/input.c:318+/546/675/785/804.
// ---------------------------------------------------------------------------

/// Get the next char from the active input source.
/// Port of `ingetc()` from Src/input.c:318 — drives the
/// lexer; consumes pushback first, then top-of-stack input.
pub fn ingetc() -> Option<char> {                                            // c:318
    if lexstop.with(|c| c.get()) {
        return Some(' ');
    }

    if let Some(c) = pushback.with(|p| p.borrow_mut().pop_front()) {
        raw_input.with(|r| r.borrow_mut().push(c));
        return Some(c);
    }

    loop {
        let pos = inbufpos.with(|p| p.get());
        let buf = inbuf.with(|b| b.borrow().clone());
        if pos < buf.len() {
            let c = buf.chars().nth(pos)?;
            inbufpos.with(|p| p.set(pos + 1));
            inbufct.with(|c| c.set(c.get().saturating_sub(1)));

            // c:328 — `if (itok(lastc = (unsigned char) *inbufptr++)) continue;`
            // Skip internal tokens via the canonical `itok()` predicate
            // (`Src/ztype.h:52`), which tests bit `ITOK` in `typtab[c]`.
            // Per `Src/utils.c:4198-4201` `inittyptab` sets ITOK on
            // `Pound..LAST_NORMAL_TOK (0x84..0x9c)` AND `Snull..Nularg
            // (0x9d..0xa1)` — canonical token range is `0x84..=0xa1`.
            // The previous hardcoded range `0x83..=0x9b` was both too
            // inclusive (included 0x83 = Meta lead byte, IMETA-only) and
            // too narrow (excluded 0x9c..=0xa1 = Bang/Snull/Dnull/Bnull/
            // Bnullkeep/Nularg). Marker (0xa2) is intentionally NOT in
            // either set — it's IMETA-only per `c:4197`. Routing through
            // `itok()` lets future `inittyptab` adjustments propagate
            // automatically with zero changes here.
            let cu32 = c as u32;
            if cu32 < 256
                && crate::ported::ztype_h::itok(cu32 as u8)
            {
                continue;
            }

            let inp_lineno =
                (inbufflags.with(|f| f.get()) & INP_LINENO) != 0;
            let is_strin = strin.with(|s| s.get()) != 0;
            if (inp_lineno || !is_strin) && c == '\n' {
                lineno.with(|l| l.set(l.get() + 1));
            }
            raw_input.with(|r| r.borrow_mut().push(c));
            return Some(c);
        }

        // End of current buffer.
        let ct = inbufct.with(|c| c.get());
        let is_strin = strin.with(|s| s.get()) != 0;
        let is_stop = lexstop.with(|c| c.get());
        if ct == 0 && (is_strin || is_stop) {
            lexstop.with(|c| c.set(true));
            return None;
        }

        if (inbufflags.with(|f| f.get()) & INP_CONT) != 0 {
            inpoptop();
            continue;
        }

        lexstop.with(|c| c.set(true));
        return None;
    }
}

// Read a line from the current command stream and store it as input         // c:366
/// Read one line into the input stack.
/// Port of `inputline()` from Src/input.c:366. C source dispatches
/// between zle / non-zle paths and `shingetline` /
/// `zleentry(READ)`. Rust port reads via shingetline (no zle yet),
/// returns "" on EOF and sets lexstop the same way.
pub fn inputline() -> String {                                               // c:366
    let line = shingetline();
    if line.is_empty() {
        lexstop.with(|c| c.set(true));
    }
    line
}

/// Replace the current input line.
/// Port of `inputsetline(char *str, int flags)` from Src/input.c:510.
pub fn inputsetline(str: &str, flags: i32) {                               // c:510
    inbuf.with(|b| *b.borrow_mut() = str.to_string());
    inbufpos.with(|p| p.set(0));
    let len = str.len() as i32;
    if (flags & INP_CONT) != 0 {
        inbufct.with(|c| c.set(c.get() + len));
    } else {
        inbufct.with(|c| c.set(len));
    }
    inbufflags.with(|f| f.set(flags));
}

/// Push a character back onto the input stream.
/// Port of `inungetc(int c)` from Src/input.c:546.
pub fn inungetc(c: char) {                                                   // c:546
    if lexstop.with(|c| c.get()) {
        return;
    }
    let pos = inbufpos.with(|p| p.get());
    if pos > 0 {
        inbufpos.with(|p| p.set(pos - 1));
        inbufct.with(|cell| cell.set(cell.get() + 1));
        let inp_lineno =
            (inbufflags.with(|f| f.get()) & INP_LINENO) != 0;
        let is_strin = strin.with(|s| s.get()) != 0;
        if (inp_lineno || !is_strin) && c == '\n' {
            lineno.with(|l| l.set(l.get().saturating_sub(1)));
        }
        raw_input.with(|r| { r.borrow_mut().pop(); });
    } else {
        pushback.with(|p| p.borrow_mut().push_front(c));
    }
}

/// Read entire file into memory
/// Read a file as a string for `source`/`stuff` semantics.
/// Port of `zstuff(char **out, const char *fn)` from Src/input.c:614 — the C source uses
/// it for `Functions/Misc/run-help` and similar autoload paths.
/// WARNING: param names don't match C — Rust=(path) vs C=(out, fn)
pub fn zstuff(path: &str) -> io::Result<String> {                            // c:614
    std::fs::read_to_string(path)
}

// `input_has_alias` / `take_raw_input` deleted — Rust-only helpers
// with zero callers in this tree. C uses different mechanisms
// (the lexer walks `instack` inline for alias detection; raw input
// for history accumulates through `chline` / `addtoline`).

/// Stuff a whole file into the input queue.
/// Port of `stuff(char *fn)` from Src/input.c:647 — read the file, echo
/// it to stderr, push onto the input stack.
/// WARNING: param names don't match C — Rust=(filename) vs C=(fn)
pub fn stuff(filename: &str) -> i32 {                                        // c:647
    let buf = match std::fs::read_to_string(filename) {
        Ok(b) => b,
        Err(_) => return 1,
    };
    let _ = std::io::stderr().write_all(buf.as_bytes());
    let _ = std::io::stderr().flush();
    inpush(&buf, INP_FREE, None);
    0
}

/// Discard pending input after a parse error.
/// Port of `inerrflush()` from Src/input.c:665.
pub fn inerrflush() {                                                        // c:665
    while !lexstop.with(|c| c.get()) && inbufct.with(|c| c.get()) > 0 {
        let _ = ingetc();
    }
}

// Set some new input onto a new element of the input stack                  // c:675
/// Push a new input source onto the stack.
/// Port of `inpush(char *str, int flags, Alias inalias)` from Src/input.c:675 — used for `eval`/
/// `source`, alias expansion, and process substitution to layer a
/// new input on top of the current one.
pub fn inpush(str: &str, flags: i32, inalias: Option<String>) {              // c:675
    let saved = instacks {
        buf: inbuf.with(|b| std::mem::take(&mut *b.borrow_mut())),
        bufpos: inbufpos.with(|p| p.replace(0)),
        flags: inbufflags.with(|f| f.get()),
        alias: None,
    };
    instack.with(|st| st.borrow_mut().push(saved));

    inbuf.with(|b| *b.borrow_mut() = str.to_string());
    inbufpos.with(|p| p.set(0));

    let mut combined = flags;
    if (flags & (INP_ALIAS | INP_HIST)) != 0 {
        combined |= INP_CONT | INP_ALIAS;
        if let Some(a) = inalias {
            instack.with(|st| {
                if let Some(last) = st.borrow_mut().last_mut() {
                    last.alias = Some(a);
                    if (flags & INP_HIST) != 0 {
                        last.flags |= INP_HISTCONT;
                    } else {
                        last.flags |= INP_ALCONT;
                    }
                }
            });
        }
    }

    let new_len = inbuf.with(|b| b.borrow().len()) as i32;
    if (combined & INP_CONT) != 0 {
        inbufct.with(|c| c.set(c.get() + new_len));
    } else {
        inbufct.with(|c| c.set(new_len));
    }
    inbufflags.with(|f| f.set(combined));
}

// Remove the top element of the stack                                       // c:736
/// Pop one input-stack frame off the top.
/// Port of `inpoptop()` from Src/input.c:736.
pub fn inpoptop() {                                                          // c:736
    if let Some(entry) = instack.with(|st| st.borrow_mut().pop()) {
        inbuf.with(|b| *b.borrow_mut() = entry.buf);
        inbufpos.with(|p| p.set(entry.bufpos));
        inbufflags.with(|f| f.set(entry.flags));
        let remaining = inbuf.with(|b| b.borrow().len())
            .saturating_sub(entry.bufpos) as i32;
        inbufct.with(|c| c.set(remaining));
    }
}

// Remove the top element of the stack and all its continuations.            // c:785
/// Pop the topmost input-stack frame plus any continuations.
/// Port of `inpop()` from Src/input.c:785.
pub fn inpop() {                                                             // c:785
    loop {
        let was_cont = (inbufflags.with(|f| f.get()) & INP_CONT) != 0;
        inpoptop();
        if !was_cont {
            break;
        }
    }
}

/// Pop the top input level only if it's an alias frame.
/// Port of `inpopalias()` from Src/input.c:804 — used to unwind
/// alias expansion without disturbing the underlying source.
pub fn inpopalias() {                                                        // c:804
    while (inbufflags.with(|f| f.get()) & INP_ALIAS) != 0 {
        inpoptop();
    }
}

/// Get a slice of the unread portion of the current input.
/// Port of `ingetptr()` from Src/input.c:817.
pub fn ingetptr() -> String {                                                // c:817
    let pos = inbufpos.with(|p| p.get());
    inbuf.with(|b| b.borrow().get(pos..).map(str::to_string).unwrap_or_default())
}

// Size of buffer for non-interactive command input                        // c:127
/// Size of the shell input buffer
const SHIN_BUF_SIZE: usize = 8192;

/// Meta character marker
pub const META: char = '\u{83}';

/// Check if a character needs meta encoding
fn imeta(c: char) -> bool {
    let b = c as u32;
    b < 32 || (0x83..=0x9b).contains(&b)
}

// `InputBuffer` aggregate + thread_local INPUT singleton deleted.
// Every field has been split into a thread_local file-scope static
// matching the corresponding C global in `Src/input.c`. Every
// previously-method-bound fn (`ingetc`, `inungetc`, `inpush`, etc.)
// is now a free fn with the C signature. `StringInput` (Rust-only
// convenience wrapper) was deleted in a previous commit.

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only reset: clear all input statics so per-test setup
    /// starts from a clean slate (tests run in the same thread by
    /// default and would otherwise leak state between them).
    fn reset_input() {
        super::inbuf.with(|b| b.borrow_mut().clear());
        super::inbufpos.with(|p| p.set(0));
        super::inbufct.with(|c| c.set(0));
        super::inbufflags.with(|f| f.set(0));
        super::lexstop.with(|c| c.set(false));
        super::lineno.with(|l| l.set(1));
        super::instack.with(|st| st.borrow_mut().clear());
        super::pushback.with(|p| p.borrow_mut().clear());
        super::raw_input.with(|r| r.borrow_mut().clear());
    }

    #[test]
    fn test_input_buffer_basic() {
        reset_input();
        inputsetline("hello", 0);
        assert_eq!(ingetc(), Some('h'));
        assert_eq!(ingetc(), Some('e'));
        assert_eq!(ingetc(), Some('l'));
        assert_eq!(ingetc(), Some('l'));
        assert_eq!(ingetc(), Some('o'));
        assert_eq!(ingetc(), None);
    }

    #[test]
    fn test_input_ungetc() {
        reset_input();
        inputsetline("abc", 0);
        assert_eq!(ingetc(), Some('a'));
        assert_eq!(ingetc(), Some('b'));
        inungetc('b');
        assert_eq!(ingetc(), Some('b'));
        assert_eq!(ingetc(), Some('c'));
    }

    #[test]
    fn test_input_stack() {
        reset_input();
        inputsetline("outer", 0);
        assert_eq!(ingetc(), Some('o'));
        inpush("inner", INP_CONT, None);
        assert_eq!(ingetc(), Some('i'));
        assert_eq!(ingetc(), Some('n'));
        assert_eq!(ingetc(), Some('n'));
        assert_eq!(ingetc(), Some('e'));
        assert_eq!(ingetc(), Some('r'));
        assert_eq!(ingetc(), Some('u'));
        assert_eq!(ingetc(), Some('t'));
    }

    #[test]
    fn test_line_number_tracking() {
        reset_input();
        inputsetline("a\nb\nc", INP_LINENO);
        assert_eq!(lineno.with(|l| l.get()), 1);
        ingetc(); // a
        ingetc(); // \n
        assert_eq!(lineno.with(|l| l.get()), 2);
        ingetc(); // b
        ingetc(); // \n
        assert_eq!(lineno.with(|l| l.get()), 3);
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
        reset_input();
        inputsetline("hello world", 0);
        ingetc(); // h
        ingetc(); // e
        ingetc(); // l
        ingetc(); // l
        ingetc(); // o
        assert_eq!(ingetptr(), " world");
    }

    #[test]
    fn test_inerrflush() {
        reset_input();
        inputsetline("remaining input", 0);
        ingetc();
        inerrflush();
        assert!(lexstop.with(|c| c.get()) || inbufct.with(|c| c.get()) == 0);
    }

    /// `Src/input.c:159-162` — `shinbufreset` body is
    /// `shinbufendptr = shinbufptr = shinbuffer;` — reset both
    /// pointers to the start of the buffer. Rust port clears
    /// `shinbuffer` (the buffer storage) and zeros `shinbufpos`.
    /// Pin both: post-condition is empty buffer + pos==0.
    #[test]
    fn shinbufreset_clears_buffer_and_zeros_pos() {
        super::shinbuffer.with(|b| {
            *b.borrow_mut() = "leftover".to_string();
        });
        super::shinbufpos.with(|p| p.set(7));
        super::shinbufreset();
        super::shinbuffer.with(|b| {
            assert!(b.borrow().is_empty(),
                "c:161 — shinbuffer must be empty after reset");
        });
        assert_eq!(super::shinbufpos.with(|p| p.get()), 0,
            "c:161 — shinbufpos must be 0 after reset");
    }

    /// `Src/input.c:171-175` — `shinbufalloc` body is
    /// `shinbuffer = zalloc(SHINBUFSIZE); shinbufreset();`. The
    /// Rust port replaces the buffer with a fresh `String` of
    /// `SHIN_BUF_SIZE` capacity, then calls `shinbufreset`.
    /// Pin the post-condition: empty buffer + pos==0 + capacity
    /// hint set.
    #[test]
    fn shinbufalloc_resets_and_capacity_hints() {
        super::shinbuffer.with(|b| {
            *b.borrow_mut() = "stale".to_string();
        });
        super::shinbufpos.with(|p| p.set(3));
        super::shinbufalloc();
        super::shinbuffer.with(|b| {
            assert!(b.borrow().is_empty(),
                "c:173 — fresh shinbuffer must be empty");
            // Capacity hint is at least 1 (default `String::with_capacity` semantics).
            // Not pinning exact value — different libstd versions may round.
            assert!(b.borrow().capacity() >= 1);
        });
        assert_eq!(super::shinbufpos.with(|p| p.get()), 0);
    }

    /// `Src/input.c:181-194` — `shinbufsave` snapshots the current
    /// buffer onto `shinsavestack` and reinitialises via
    /// `shinbufalloc`. Pin the round-trip: save with buf="abc",
    /// pos=2 → buf reset to "" + stack contains ("abc", 2). Then
    /// `shinbufrestore` restores the saved state.
    #[test]
    fn shinbufsave_restore_round_trip() {
        // Clear stack from any prior test.
        super::shinsavestack.with(|s| s.borrow_mut().clear());
        super::shinbuffer.with(|b| *b.borrow_mut() = "abc".to_string());
        super::shinbufpos.with(|p| p.set(2));
        super::shinbufsave();
        super::shinbuffer.with(|b| {
            assert!(b.borrow().is_empty(),
                "c:193 — shinbufsave invokes shinbufalloc (resets to empty)");
        });
        assert_eq!(super::shinbufpos.with(|p| p.get()), 0,
            "c:193 — pos must be 0 after save");
        super::shinbufrestore();
        super::shinbuffer.with(|b| {
            assert_eq!(*b.borrow(), "abc",
                "c:200-209 — shinbufrestore restores saved buffer");
        });
        assert_eq!(super::shinbufpos.with(|p| p.get()), 2,
            "c:200-209 — shinbufrestore restores saved pos");
    }

    /// `Src/input.c:200` — `shinbufrestore` on an empty save stack
    /// must NOT panic (C dereferences `shinsavestack` which would be
    /// NULL — UB in C; the Rust port chose to no-op for safety).
    /// Pin the no-op so a regression doesn't add a panic.
    #[test]
    fn shinbufrestore_on_empty_stack_is_noop() {
        super::shinsavestack.with(|s| s.borrow_mut().clear());
        // Pre-seed a non-empty buffer.
        super::shinbuffer.with(|b| *b.borrow_mut() = "persist".to_string());
        super::shinbufrestore();
        super::shinbuffer.with(|b| {
            assert_eq!(*b.borrow(), "persist",
                "empty-stack restore must leave buffer untouched");
        });
    }

    /// `Src/input.c:328` — `ingetc` skips bytes for which `itok()`
    /// (the canonical predicate at `Src/ztype.h:52`) returns true.
    /// Per `Src/utils.c:4198-4201`, `inittyptab` sets ITOK on
    /// `Pound..LAST_NORMAL_TOK (0x84..0x9c)` and `Snull..Nularg
    /// (0x9d..0xa1)` — i.e. the canonical token range is `0x84..=0xa1`.
    /// A previous hardcoded `0x83..=0x9b` range was both too inclusive
    /// (included 0x83 = Meta lead byte) and too narrow (excluded
    /// 0x9c..=0xa1 = Bang/Snull/Dnull/Bnull/Bnullkeep/Nularg).
    /// Pin the canonical itok-driven skip via two endpoints:
    ///   * 0x9c (Bang) — ITOK, must be skipped.
    ///   * 0xa1 (Nularg) — ITOK, must be skipped.
    #[test]
    fn ingetc_skips_token_bytes_via_itok_predicate() {
        reset_input();
        // Make sure typtab is populated (without this the per-thread
        // typtab default may have ITOK bits unset).
        crate::ported::utils::inittyptab();

        let bang:   char = '\u{009c}'; // Bang (LAST_NORMAL_TOK)
        let nularg: char = '\u{00a1}'; // Nularg (last ITOK byte)
        let mut s = String::new();
        s.push('a');
        s.push(bang);
        s.push('b');
        s.push(nularg);
        s.push('c');
        inputsetline(&s, 0);
        // c:328 — itok bytes must be silently skipped; visible
        // sequence is "abc".
        assert_eq!(ingetc(), Some('a'));
        assert_eq!(ingetc(), Some('b'),
            "c:328 — Bang (0x9c) must be skipped (ITOK bit set per inittyptab)");
        assert_eq!(ingetc(), Some('c'),
            "c:328 — Nularg (0xa1) must be skipped (ITOK bit set per inittyptab)");
    }

    /// `Src/input.c:328` — non-token bytes (e.g. Meta=0x83) must NOT
    /// be skipped by `ingetc`. Meta is IMETA-only, never ITOK; treating
    /// it as a token would corrupt every metafied character read by
    /// the lexer. Same for Marker (0xa2) which is IMETA-only per
    /// `Src/utils.c:4197` (`typtab[Marker] |= IMETA`, NOT ITOK).
    #[test]
    fn ingetc_does_not_skip_imeta_only_bytes() {
        reset_input();
        crate::ported::utils::inittyptab();
        let meta: char = '\u{0083}';   // Meta lead byte — IMETA only
        let marker: char = '\u{00a2}'; // Marker — IMETA only per c:4197
        let mut s = String::new();
        s.push('x');
        s.push(meta);
        s.push('y');
        s.push(marker);
        s.push('z');
        inputsetline(&s, 0);
        assert_eq!(ingetc(), Some('x'));
        assert_eq!(ingetc(), Some(meta),
            "c:328 — Meta (0x83) is IMETA-only, NOT ITOK; must pass through");
        assert_eq!(ingetc(), Some('y'));
        assert_eq!(ingetc(), Some(marker),
            "c:328 / c:4197 — Marker (0xa2) is IMETA-only, NOT ITOK; must pass through");
        assert_eq!(ingetc(), Some('z'));
    }
}
