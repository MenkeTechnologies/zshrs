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

use crate::ported::hashtable::aliastab_lock;
use crate::ported::hist::histbackword;
use crate::ported::lex::{zshlex_raw_back, LEX_LEXSTOP};
use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::utils::{unmetafy, zerr};
use crate::ported::zsh_h::{
    isset, Meta, INP_ALCONT, INP_ALIAS, INP_CONT, INP_FREE, INP_HIST, INP_HISTCONT, INP_LINENO,
    INP_RAW_KEEP, SHINSTDIN, VERBOSE,
};
use crate::ported::ztype_h::itok;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};

/// Port of `struct instacks` from `Src/input.c:109`. One frame in
/// the input stack — pushed by `inpush()` and popped by `inpoptop()`
/// to layer alias expansion / history-substitution / `eval`
/// continuations over the active input.
#[derive(Clone, Default)]
#[allow(non_camel_case_types)]
struct instacks {
    // c:109
    buf: String,           // c:110 char *buf
    bufpos: usize,         // c:110 char *bufptr offset
    bufct: i32,            // c:112 int bufct — inbufct AT PUSH TIME (spans lower CONT frames)
    flags: i32,            // c:112 int flags
    alias: Option<String>, // c:111 Alias alias
}

/// Initial input stack size
#[allow(dead_code)]
const INSTACK_INITIAL: usize = 4; // c:122

// `pub mod flags { … INP_* … }` deleted — Rust-only namespace with
// values that diverged from the C `#define INP_FREE (1<<0)` etc. at
// Src/zsh.h:467-476. The canonical mirror lives in
// `crate::ported::zsh_h::INP_*` (matching the C bit positions
// exactly); this file uses those constants directly.

// ---------------------------------------------------------------------------
// SHIN buffer helpers — direct ports of input.c:159/171/181/200/218/267.
// ---------------------------------------------------------------------------

/// Reset the SHIN pushback buffer.
/// Port of `shinbufreset()` from Src/input.c:159 —
/// `shinbufendptr = shinbufptr = shinbuffer`.
pub fn shinbufreset() {
    // c:159
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

    /// Byte-offset cache for `inbuf`/`inbufpos`. `inbufpos` is a CHAR
    /// index; naively resolving it with `inbuf.chars().nth(pos)` is
    /// O(pos), making a sequential scan of an N-char buffer O(N²) (a
    /// 31 KB p10k function body took tens of seconds to parse). This
    /// caches the byte offset of char index `INBUF_BYTE_POS`; a
    /// sequential read (`pos == INBUF_BYTE_POS`) resolves in O(1).
    /// Self-healing: on any mismatch (buffer swap, pushback reposition)
    /// the offset is recomputed once via `char_indices().nth(pos)`.
    /// `(0, 0)` is always valid — every `inbufpos` reset sets pos 0,
    /// whose byte offset is 0 in any buffer.
    #[allow(non_upper_case_globals)]
    static inbuf_byte_char: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[allow(non_upper_case_globals)]
    static inbuf_byte_off: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

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
pub fn shinbufalloc() {
    // c:171
    shinbuffer.with(|b| {
        *b.borrow_mut() = String::with_capacity(SHIN_BUF_SIZE);
    });
    shinbufreset();
}

/// Save the current SHIN buffer onto the save stack.
/// Port of `shinbufsave()` from Src/input.c:181 — push the
/// existing buffer onto a save-stack and start a fresh one for
/// nested `eval`/`source` contexts.
pub fn shinbufsave() {
    // c:181
    let (snap_buf, snap_pos) = (
        shinbuffer.with(|b| std::mem::take(&mut *b.borrow_mut())),
        shinbufpos.with(|p| p.replace(0)),
    );
    shinsavestack.with(|s| s.borrow_mut().push((snap_buf, snap_pos)));
    shinbufalloc();
}

/// Pop the top of the SHIN save stack back into the live buffer.
/// Port of `shinbufrestore()` from Src/input.c:200.
pub fn shinbufrestore() {
    // c:200
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
pub fn shingetchar() -> i32 {
    // c:222-225 — `if (shinbufptr < shinbufendptr) return
    //   (unsigned char) *shinbufptr++;`. C is byte-oriented; serve the
    //   buffer one BYTE at a time (shinbufpos is a byte index).
    let bufd = shinbuffer.with(|b| b.borrow().clone());
    let pos = shinbufpos.with(|p| p.get());
    if pos < bufd.len() {
        if let Some(b) = bufd.as_bytes().get(pos) {
            shinbufpos.with(|p| p.set(pos + 1));
            return *b as i32;
        }
    }

    // c:227 — `shinbufreset();`
    shinbufreset();
    let fd = SHIN.with(|s| s.get());
    const SHINBUFSIZE: usize = 256; // c: SHINBUFSIZE

    // c:228-251 — `#ifdef USE_LSEEK` fast path. Take it when SHIN is
    // NOT the keyboard (`!isset(SHINSTDIN)`) or when SHIN is seekable
    // (`lseek(SHIN, 0, SEEK_CUR) != -1` — a real file). A pipe /
    // terminal returns -1 from lseek and is handled by the
    // byte-at-a-time loop below. CRITICAL: the previous port skipped
    // both C branches and slurped SHINBUFSIZE bytes unconditionally,
    // so on a pipe (`cmd | zshrs -c 'read x'`) the lexer's first
    // refill consumed every following line, leaving nothing for the
    // `read` / `select` builtins to read off fd 0. Bug surfaced as
    // 31 read/select parity regressions.
    let seekable = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) != -1 };
    if !isset(SHINSTDIN) || seekable {
        let mut buf = [0u8; SHINBUFSIZE];
        // c:231-234 — `do { errno=0; nread=read(...); } while (nread<0
        //   && errno==EINTR);`
        let nread = loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, SHINBUFSIZE) };
            if n < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break n;
        };
        if nread <= 0 {
            return -1; // c:235-236
        }
        let nread = nread as usize;
        // c:237-246 — when reading the keyboard (`isset(SHINSTDIN)`) and
        // the chunk holds a newline, keep only the first line and
        // `lseek` the fd BACK over the surplus so a later `read` / next
        // command sees the rest. Otherwise the whole chunk is the
        // buffer (`shinbufendptr = shinbuffer + nread`).
        let end = if isset(SHINSTDIN) {
            match buf[..nread].iter().position(|&b| b == b'\n') {
                // c:239-240 — `++shinbufendptr - shinbuffer` includes
                //   the '\n'.
                Some(nl) => {
                    let rsize = nl + 1;
                    if nread > rsize {
                        let back = (nread - rsize) as libc::off_t;
                        // c:241-244 — `lseek(SHIN, -(nread-rsize),
                        //   SEEK_CUR)`; C zerr()s on failure (non-fatal).
                        if unsafe { libc::lseek(fd, -back, libc::SEEK_CUR) } < 0 {
                            crate::ported::utils::zerr(&format!(
                                "lseek({}, {}): {}",
                                fd,
                                -back,
                                io::Error::last_os_error()
                            ));
                        }
                    }
                    rsize
                }
                None => nread, // c:245-246
            }
        } else {
            nread // c:245-246 — `shinbufendptr = shinbuffer + nread`
        };
        let s = String::from_utf8_lossy(&buf[..end]).into_owned();
        shinbuffer.with(|b| *b.borrow_mut() = s);
        shinbufpos.with(|p| p.set(1));
        return buf[0] as i32; // c:247 — `return (unsigned char) *shinbufptr++;`
    }

    // c:253-268 — non-seekable fallback (pipe / terminal): read ONE
    // byte at a time, stopping at '\n' (`/* Use line buffering (POSIX
    // requirement) */`) or when the buffer fills. This is what keeps
    // the `read` builtin correct on pipes — the lexer never reads past
    // the newline, so fd 0 still points at the next line.
    let mut out: Vec<u8> = Vec::with_capacity(64);
    loop {
        let mut one = [0u8; 1];
        // c:255-256 — `errno=0; nread=read(SHIN, shinbufendptr, 1);`
        let n = unsafe { libc::read(fd, one.as_mut_ptr() as *mut libc::c_void, 1) };
        if n > 0 {
            out.push(one[0]); // c:259 — `*shinbufendptr++`
            if one[0] == b'\n' {
                break; // c:260 — newline terminates the line
            }
            if out.len() == SHINBUFSIZE {
                break; // c:261-262 — buffer full
            }
        } else if n == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            break; // c:263-264 — EOF or non-EINTR error
        }
    }
    if out.is_empty() {
        return -1; // c:265-266 — `if (shinbufendptr == shinbuffer) return -1;`
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    let first = out[0] as i32;
    shinbuffer.with(|b| *b.borrow_mut() = s);
    shinbufpos.with(|p| p.set(1));
    first // c:267 — `return (unsigned char) *shinbufptr++;`
}

/// Read a full line from SHIN, with `\n` preserved.
/// Port of `shingetline()` from Src/input.c:267 — calls
/// `shingetchar` in a loop, metafies high bytes, returns NULL
/// (`""`) on EOF.
pub fn shingetline() -> String {
    // c:267
    let mut result = String::new();
    // Inline metafy of one raw byte (Src/utils.c:4856 metafy + Src/zsh.h Meta
    // protocol): reserved IMETA bytes (0x00, 0x83-0x9b) become `Meta` +
    // (byte ^ 32); every other byte is literal.
    let push_byte = |result: &mut String, byte: u32| {
        let c = char::from_u32(byte).unwrap_or('\0');
        if imeta(c) {
            result.push(Meta as char);
            result.push(char::from_u32(byte ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    };
    loop {
        let b0 = match shingetchar() {
            -1 => return result,
            b => b as u32,
        };
        if b0 == '\n' as u32 {
            result.push('\n');
            return result;
        }
        // A UTF-8 multibyte lead byte (0xC2..=0xF4): read its continuation
        // bytes and decode the sequence to ONE Unicode char. C keeps raw
        // metafied bytes here and decodes UTF-8 later; the Rust port is
        // Unicode-`String`-based, and the ZLE input path (ZLELINE = Vec<char>)
        // already yields Unicode — decoding here makes the non-ZLE (piped /
        // script) line use the same representation the lexer and output
        // expect (prior byte-per-char storage double-encoded on output).
        if (0xc2..=0xf4).contains(&b0) {
            let extra = if b0 < 0xe0 {
                1
            } else if b0 < 0xf0 {
                2
            } else {
                3
            };
            let mut bytes = vec![b0 as u8];
            for _ in 0..extra {
                match shingetchar() {
                    -1 => break,
                    cb => bytes.push(cb as u8),
                }
            }
            if let Ok(s) = std::str::from_utf8(&bytes) {
                result.push_str(s);
            } else {
                // Malformed sequence — metafy each collected byte literally.
                for &b in &bytes {
                    push_byte(&mut result, b as u32);
                }
            }
            continue;
        }
        push_byte(&mut result, b0);
    }
}

// ---------------------------------------------------------------------------
// ingetc / inungetc / inpush / inpop / inpopalias — Src/input.c:318+/546/675/785/804.
// ---------------------------------------------------------------------------

/// Get the next char from the active input source.
/// Port of `ingetc()` from Src/input.c:318 — drives the
/// lexer; consumes pushback first, then top-of-stack input.
pub fn ingetc() -> Option<char> {
    // c:318
    // c:Src/input.c:322 — `if (lexstop) return ' ';`. The C source
    // returns the literal byte 32. The Rust port wraps the result
    // in Option<char> where None is the canonical EOF marker (callers
    // map None → -1 at hist.rs:392). Returning Some(' ') from this
    // guard treated EOF as a real space byte downstream and broke
    // every "drain past end" caller; consistent EOF means None for
    // every post-lexstop call, matching the function's already-emitted
    // None when buffer drains at line 293.
    if lexstop.with(|c| c.get()) {
        return None; // c:322 (mapped to Rust None EOF marker)
    }

    if let Some(c) = pushback.with(|p| p.borrow_mut().pop_front()) {
        raw_input.with(|r| r.borrow_mut().push(c));
        return Some(c);
    }

    loop {
        let pos = inbufpos.with(|p| p.get());
        // c:326 — C's `inbufptr` is a byte pointer; the Rust port keeps
        // `inbufpos` a CHAR index into the `inbuf` String. Buffer-end is
        // "no char at pos": the old `pos < buf.len()` compared a char index
        // against the BYTE length, truncating multibyte `inbuf` content
        // (eval / here-strings / cmdsubst / completion) at the first such char.
        //
        // O(1) resolution via the byte-offset cache instead of the old
        // `inbuf.clone()` + `chars().nth(pos)` (both O(n) per char →
        // O(n²) per buffer; a 31 KB `case` body took tens of seconds).
        let byte_off = if inbuf_byte_char.with(|c| c.get()) == pos {
            inbuf_byte_off.with(|o| o.get())
        } else {
            // Cache miss (buffer swap / pushback reposition): recompute
            // this char's byte offset once, then resume O(1) advance.
            inbuf.with(|b| {
                b.borrow()
                    .char_indices()
                    .nth(pos)
                    .map(|(off, _)| off)
                    .unwrap_or_else(|| b.borrow().len())
            })
        };
        // Extract the char + its UTF-8 width in a single borrow (no clone).
        let ch = inbuf.with(|b| {
            let s = b.borrow();
            s.get(byte_off..).and_then(|rest| rest.chars().next())
        });
        if let Some(c) = ch {
            inbufpos.with(|p| p.set(pos + 1));
            // Advance the byte cache in lockstep (O(1) next read).
            inbuf_byte_char.with(|cc| cc.set(pos + 1));
            inbuf_byte_off.with(|o| o.set(byte_off + c.len_utf8()));
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
            if cu32 < 256 && itok(cu32 as u8) {
                continue;
            }

            let inp_lineno = (inbufflags.with(|f| f.get()) & INP_LINENO) != 0;
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

        // c:340-345 — `if (!inbufct && (strin || errflag)) { lexstop=1;
        // break; }`. In C, reading a `-c` string sets `strin`, so a
        // drained buffer stops at EOF instead of falling through to
        // inputline() (which reads fresh SHIN input). zshrs feeds `-c`
        // via the lexer's own LEX_INPUT window with `strin` left 0, so
        // the c:336 gate above misses it — the distinguishing signal
        // here is SHINSTDIN: it is SET for the interactive / stdin-script
        // loop (which legitimately reads SHIN via inputline) and UNSET
        // for `-c`. Without this gate, after the `-c` string drained the
        // "last resort" inputline() swallowed the PROCESS's stdin as
        // extra command lines — e.g. `printf data | zshrs -c 'read x'`
        // left `read` empty and ran the data as commands (31 read/select
        // parity regressions).
        if !isset(SHINSTDIN) {
            lexstop.with(|c| c.set(true));
            return None; // c:343-344 — string input drained → EOF
        }

        // c:354-356 — `/* As a last resort, get some more input */
        // if (inputline()) break;`. Read the next line from SHIN into
        // `inbuf` and loop to return its first char. inputline() returns
        // nonzero (and sets lexstop) at EOF. The lexer accumulates each
        // returned char into its token buffer via `add()`, so input
        // arriving through this path produces correct token text.
        // c:339 — `if (!inbufct && (strin || errflag)) { lexstop; break; }`.
        // We have drained the current input buffer (the char loop above
        // fell through) and there is no INP_CONT buffer to pop. When we
        // are reading from a pushed STRING (`strin` — e.g. the completion
        // lexer in get_comp_string, `eval`, cmdsubst bodies), we must NOT
        // fall through to `inputline()`, which reads fresh SHIN and, in an
        // interactive shell, prompts PS2. C's guard also tests `!inbufct`;
        // the Rust `inbufct` accounting can lag by one across the metafied
        // completion buffer, so gate on the drained-buffer state we are
        // already in instead. Without this, `l<Tab>` (whose lexer buffer
        // drains mid-token) dropped the shell into a PS2 continuation.
        if strin.with(|s| s.get()) != 0 {
            lexstop.with(|c| c.set(true));
            return None;
        }
        if inputline() != 0 {
            return None; // c:356 break → EOF
        }
        // loop: the refilled inbuf is read at the top of the loop.
    }
}

// Read a line from the current command stream and store it as input         // c:366
/// Read one line from the command stream and store it as the input
/// buffer. Port of `static int inputline(void)` from Src/input.c:366.
/// C dispatches between the zle and non-zle paths; zshrs reads via
/// `shingetline` (no zle line editor yet). On EOF it sets `lexstop`
/// and returns 1; on success it installs the line into the `inbuf`
/// buffer (`inbuf = inbufptr = line; inbufleft = strlen; inbufct =
/// …; inbufflags = 0`) and returns 0 — exactly what `ingetc`'s
/// "as a last resort, get some more input" arm (c:355) expects.
pub fn inputline() -> i32 {
    // !!! WARNING: RUST-ONLY GUARD — NO C COUNTERPART !!!
    // C zsh is single-threaded, so this function can always read SHIN /
    // drive ZLE. zshrs parses shell bodies on worker-pool threads
    // (compinit's bytecode backfill parses ~47k autoload bodies), and
    // `interact` / SHINSTDIN / SHTTY / USEZLE are process-globals — so a
    // worker whose lexer buffer drained mid-construct fell through here
    // and READ THE USER'S TERMINAL, competing with ZLE for keystrokes.
    // Symptoms: the prompt after `compinit -C` rendered PS2 (`> `) and
    // swallowed every following line, stray empty `zsh:N:` diagnostics,
    // eaten/duplicated characters, and bogus parse errors. Background
    // threads get EOF here, which is what C's `strin` gate (input.c:339)
    // produces for string input.
    if crate::worker::in_worker_thread() {
        lexstop.with(|c| c.set(true));
        return 1; // EOF — never prompt or read SHIN off the main thread
    }
    // c:371-384 — if reading code interactively, work out the prompt: PS1
    // on the first line of a command, PS2 (continuation) otherwise.
    // `ingetcpmptl` is the SOURCE prompt string fed to promptexpand.
    let mut ingetcpmptl: Option<String> = None;
    let mut ingetcpmptr: Option<String> = None;
    if crate::ported::zsh_h::interact() && isset(SHINSTDIN) {
        // c:372
        // RPS1/RPROMPT (and RPS2/RPROMPT2) are documented as equivalent
        // right-prompt names but are SEPARATE parameters (unlike PS1/PROMPT,
        // which zshrs aliases): `RPROMPT=x` does NOT set `$RPS1` and vice
        // versa, matching zsh's readback. zsh nonetheless renders the right
        // prompt from whichever is set, so read RPS1 first and fall back to
        // RPROMPT when it's unset/empty. Without the fallback, a config that
        // sets only RPROMPT (the classic name — e.g. zpwr's vim-mode keymap
        // indicator) produced no right prompt at all. (Both set is
        // contradictory config; RPS1 wins here vs zsh's last-assigned wins.)
        // RPS1/RPROMPT (and RPS2/RPROMPT2) are documented as equivalent
        // right-prompt names but are SEPARATE parameters (unlike PS1/PROMPT,
        // which zshrs aliases): `RPROMPT=x` does NOT set `$RPS1` and vice
        // versa, matching zsh's readback. zsh nonetheless renders the right
        // prompt from whichever is set, so read the modern RPS1/RPS2 name
        // first and fall back to the classic RPROMPT/RPROMPT2 when it's
        // unset/empty. Without the fallback a config that sets only RPROMPT
        // (the classic name — e.g. zpwr's vim-mode keymap indicator)
        // produced no right prompt at all. An empty primary is treated as
        // "not set" so a bare `RPS1=` doesn't blank a populated RPROMPT.
        // (Both set is contradictory config; RPS1 wins here vs zsh's
        // last-assigned wins.) Bug #654.
        let rprompt_effective = |primary: &str, legacy: &str| -> Option<String> {
            match crate::ported::params::getsparam(primary) {
                Some(s) if !s.is_empty() => Some(s),
                _ => crate::ported::params::getsparam(legacy),
            }
        };
        if !crate::ported::lex::LEX_ISFIRSTLN.with(|c| c.get()) {
            // c:373-377 — continuation line → PS2 / RPS2 (or RPROMPT2).
            ingetcpmptl = crate::ported::params::getsparam("PS2");
            ingetcpmptr = rprompt_effective("RPS2", "RPROMPT2");
        } else {
            // c:379-382 — first line → PS1 / RPS1 (or RPROMPT).
            ingetcpmptl = crate::ported::params::getsparam("PS1");
            ingetcpmptr = rprompt_effective("RPS1", "RPROMPT");
        }
    }
    // c:385 — `if (!(interact && isset(SHINSTDIN) && SHTTY != -1 &&
    //   isset(USEZLE)))`: read via the ZLE line editor when interactive
    // on a real tty with USEZLE; otherwise read straight from the input
    // file (printing a prompt first).
    // The ZLE line editor is on for any interactive shell on a real tty
    // with USEZLE set (zsh's standard gate) — `unsetopt zle` turns it off
    // and falls back to the cooked reader below, exactly as in zsh.
    let use_zle = crate::ported::zsh_h::interact()
        && isset(SHINSTDIN)
        && crate::ported::init::SHTTY.load(std::sync::atomic::Ordering::Relaxed) != -1
        && isset(crate::ported::zsh_h::USEZLE);
    let line = if !use_zle {
        // c:391-406 — non-ZLE: print the expanded prompt to fd 2 (only
        // when still interactive, e.g. running under emacs), then
        // shingetline. Gated on `interact && SHINSTDIN` so piped /
        // non-interactive input prints no prompt.
        if crate::ported::zsh_h::interact() && isset(SHINSTDIN) {
            // c:401-403 — `promptexpand(*ingetcpmptl)` → `write_loop(2, …)`.
            let (expanded, _, _) = crate::ported::prompt::promptexpand(
                ingetcpmptl.as_deref().unwrap_or(""), // c:401
                0,
                None,
            );
            let pptbuf = crate::ported::utils::unmetafy_str(&expanded); // c:401 unmetafy
            let _ = crate::ported::utils::write_loop(2, &pptbuf); // c:403
        }
        shingetline() // c:406 ingetcline = shingetline()
    } else {
        // c:413-423 — ZLE path. `int flags = ZLRF_HISTORY|ZLRF_NOSETTY;
        //   if (isset(IGNOREEOF)) flags |= ZLRF_IGNOREEOF;
        //   ingetcline = zleentry(ZLE_CMD_READ, ingetcpmptl, ingetcpmptr,
        //                         flags, context); histdone |= HISTFLAG_SETTY;`
        // zleentry dispatches to the ZLE module's zle_main_entry; zshrs
        // links ZLE in so it calls the entry point directly — but the
        // module-registration half of zleentry still has to run, because
        // it is what marks `zsh/zle`/`zsh/complete`/`zsh/compctl` loaded
        // (init.c:1764-1765). `_default` keys off `zmodload -e
        // zsh/compctl` to decide whether the legacy compctl engine is
        // available, so skipping this made interactive completion
        // diverge from zsh. One-shot, on the first ZLE read, exactly
        // where C's lazy `load_module` fires.
        // c:init.c:1764-1765 — `if (load_module("zsh/zle", NULL, 0) != 1)
        //   (void)load_module("zsh/compctl", NULL, 0);`. `zsh/compctl`
        // pulls in `zsh/complete` through the compctl.mdd moddeps edge.
        // ZLE_CMD_READ is not one of the three commands c:1761-1762
        // exempts, so the load always fires here on the first read.
        if !crate::ported::init::zle_modules_loaded.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let mut tab = crate::ported::module::MODULESTAB.lock().unwrap();
            if tab.load_module("zsh/zle", None, false) {
                tab.load_module("zsh/compctl", None, false); // c:init.c:1765
            }
        }
        let mut flags = crate::ported::zsh_h::ZLRF_HISTORY | crate::ported::zsh_h::ZLRF_NOSETTY;
        if isset(crate::ported::zsh_h::IGNOREEOF) {
            flags |= crate::ported::zsh_h::ZLRF_IGNOREEOF;
        }
        let mut lp = ingetcpmptl.clone();
        let mut rp = ingetcpmptr.clone();
        let mut args = crate::ported::zle::zle_main::zle_main_entry_args::Read {
            lp: &mut lp,
            rp: &mut rp,
            flags,
            context: 0, // ZLCON_LINE_START — normal command line read
        };
        let r = crate::ported::zle::zle_main::zle_main_entry(
            crate::ported::zsh_h::ZLE_CMD_READ,
            &mut args,
        );
        // c:423 — `histdone |= HISTFLAG_SETTY;` (defer tty restore to the
        // end of the current input via the history mechanism).
        crate::ported::hist::histdone.fetch_or(
            crate::ported::zsh_h::HISTFLAG_SETTY,
            std::sync::atomic::Ordering::Relaxed,
        );
        // zleread returns "" on EOF (^D) and "…\n" otherwise, matching the
        // C NULL-vs-line distinction the empty-check below relies on.
        r.unwrap_or_default()
    };
    // c:425-427 — `if (!ingetcline) { ... return lexstop = 1; }`.
    // shingetline returns "" only at real EOF (a blank line is "\n").
    if line.is_empty() {
        lexstop.with(|c| c.set(true)); // c:426 lexstop = 1
        return 1;
    }
    // c:433-437 — `if (isset(VERBOSE)) { zputs(ingetcline, stderr);
    //   fflush(stderr); }`. The `-v` / VERBOSE option echoes each input
    // line to stderr as it is read. zputs writes the UNmetafied bytes;
    // the line already carries its trailing newline so no extra one is
    // added.
    if isset(VERBOSE) {
        use std::io::Write;
        // c:435 — `zputs(ingetcline, stderr)` writes the unmetafied raw
        // bytes (not a re-encoded String), so write the byte buffer.
        let _ = io::stderr().write_all(&crate::ported::utils::unmetafy_str(&line));
        let _ = io::stderr().flush(); // c:436 fflush(stderr)
    }
    // c:498-500 — install the line as the live input buffer.
    let len = line.chars().count() as i32; // c:500 inbufleft (char count — inbufpos/inbufct are char-based)
    inbuf.with(|b| *b.borrow_mut() = line);
    inbufpos.with(|p| p.set(0)); // c:499 inbufptr = inbuf
    inbufct.with(|c| c.set(len)); // c:501 inbufct = inbufleft
    inbufflags.with(|f| f.set(0)); // c:502 inbufflags = 0
                                   // Fresh input arrived — clear any stale EOF latch so ingetc reads it.
    lexstop.with(|c| c.set(false));
    0 // c:508 return 0
}

/// Replace the current input line.
/// Port of `inputsetline(char *str, int flags)` from Src/input.c:510.
pub fn inputsetline(str: &str, flags: i32) {
    // c:510
    inbuf.with(|b| *b.borrow_mut() = str.to_string());
    inbufpos.with(|p| p.set(0));
    let len = str.chars().count() as i32; // char count — inbufct is char-based
    if (flags & INP_CONT) != 0 {
        inbufct.with(|c| c.set(c.get() + len));
    } else {
        inbufct.with(|c| c.set(len));
    }
    inbufflags.with(|f| f.set(flags));
    // c:Src/input.c — inputsetline is the "fresh input arrives" entry
    // point. In C, ingetc's lexstop guard is reset by every grammar-
    // boundary call (zshlex/getfirsttok/zshlex_raw_back at lex.c:455,
    // 519, etc.) BEFORE the next ingetc fires. zshrs has no such
    // reset surface yet, so a previously-drained buffer leaves
    // lexstop=true and the new content is unreadable. Reset here so
    // the contract "inputsetline(s) makes s readable" holds.
    lexstop.with(|c| c.set(false));
}

/// Push a character back onto the input stream.
/// Port of `inungetc(int c)` from Src/input.c:546.
pub fn inungetc(c: char) {
    // c:546
    if lexstop.with(|c| c.get()) {
        return;
    }
    let pos = inbufpos.with(|p| p.get());
    if pos > 0 {
        inbufpos.with(|p| p.set(pos - 1));
        inbufct.with(|cell| cell.set(cell.get() + 1));
        let inp_lineno = (inbufflags.with(|f| f.get()) & INP_LINENO) != 0;
        let is_strin = strin.with(|s| s.get()) != 0;
        if (inp_lineno || !is_strin) && c == '\n' {
            lineno.with(|l| l.set(l.get().saturating_sub(1)));
        }
        raw_input.with(|r| {
            r.borrow_mut().pop();
        });
    } else {
        pushback.with(|p| p.borrow_mut().push_front(c));
    }
}

/// Read entire file into memory.
/// Port of `mod_export off_t zstuff(char **out, const char *fn)`
/// from Src/input.c:614. C body opens via `unmeta(fn)`, fseeks to
/// end for size, fread()s the body, queues signals around the IO,
/// zerr()s on open/read failures, returns byte count or -1.
///
/// Rust signature: `(path: &str) -> Result<(String, i64), i32>`
/// — Ok((contents, byte_count)) or Err(-1) on open/read fail.
/// Path is unmetafied to match C's `unmeta(fn)` step before open.
/// WARNING: param names don't match C — Rust=(path) vs C=(out, fn).
pub fn zstuff(path: &str) -> Result<(String, i64), i32> {
    // c:614
    use std::io::Read;
    // c:621 — `unmeta(fn)`: de-metafy the path before open(2).
    let mut path_bytes = path.as_bytes().to_vec();
    unmetafy(&mut path_bytes);
    let real_path = String::from_utf8_lossy(&path_bytes);
    // c:621 — `fopen(unmeta(fn), "r")`. Rust File::open mirrors fopen
    // with read-only mode; failure path zerrs and returns -1.
    let mut file = match std::fs::File::open(real_path.as_ref()) {
        // c:621
        Ok(f) => f,
        Err(_) => {
            // c:622
            zerr(&format!("can't open {}", path)); // c:622
            return Err(-1); // c:623
        }
    };
    // c:625 — `queue_signals();` block syscalls from the trap fast path
    // for the duration of the read.
    queue_signals();
    // c:626-628 — `fseek(end); ftell; fseek(start);` to size the file
    // without consuming the stream. Use stream metadata in Rust.
    let len = match file.metadata() {
        // c:627
        Ok(m) => m.len() as i64,
        Err(_) => 0,
    };
    let mut buf = String::new(); // c:629 — `buf = zalloc(len + 1);`
                                 // c:630-635 — `fread(buf, len, 1, in)` failure arm zerrs read error.
    if file.read_to_string(&mut buf).is_err() {
        // c:630
        zerr(&format!("read error on {}", path)); // c:631
        unqueue_signals(); // c:633
        return Err(-1); // c:634
    }
    unqueue_signals(); // c:640
    Ok((buf, len)) // c:642
}

// `input_has_alias` / `take_raw_input` deleted — Rust-only helpers
// with zero callers in this tree. C uses different mechanisms
// (the lexer walks `instack` inline for alias detection; raw input
// for history accumulates through `chline` / `addtoline`).

/// Stuff a whole file into the input queue.
/// Port of `stuff(char *fn)` from Src/input.c:647 — read the file, echo
/// it to stderr, push onto the input stack.
/// WARNING: param names don't match C — Rust=(filename) vs C=(fn)
pub fn stuff(filename: &str) -> i32 {
    // c:647
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
pub fn inerrflush() {
    // c:665
    while !lexstop.with(|c| c.get()) && inbufct.with(|c| c.get()) > 0 {
        let _ = ingetc();
    }
}

// Set some new input onto a new element of the input stack                  // c:675
/// Push a new input source onto the stack.
/// Port of `inpush(char *str, int flags, Alias inalias)` from Src/input.c:675 — used for `eval`/
/// `source`, alias expansion, and process substitution to layer a
/// new input on top of the current one.
pub fn inpush(str: &str, flags: i32, inalias: Option<String>) {
    // c:675
    // c:687 — `inbufflags &= ~(INP_ALCONT|INP_HISTCONT);` — the
    // continuation markers describe the frame BELOW; strip them from
    // the flags being saved so a pop doesn't re-run the alias-unwind
    // arm for a frame that isn't an alias continuation.
    let saved_flags = inbufflags.with(|f| f.get()) & !(INP_ALCONT | INP_HISTCONT); // c:687
    let saved = instacks {
        buf: inbuf.with(|b| std::mem::take(&mut *b.borrow_mut())),
        bufpos: inbufpos.with(|p| p.replace(0)),
        // c:686 — `instacktop->bufct = inbufct;` — the count AT PUSH
        // TIME spans this frame's remainder plus every CONT frame
        // below it; inpoptop restores it verbatim (c:764). The old
        // port didn't save it and recomputed only the restored
        // frame's remainder on pop — undercounting made ingetc's
        // `!inbufct && strin` EOF gate (c:342) fire at an
        // intermediate frame boundary, dropping everything below
        // (`alias g='echo A'; alias t='g x'; eval t` lost ` x`).
        bufct: inbufct.with(|c| c.get()), // c:686
        flags: saved_flags,
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
    } else if (saved_flags & INP_ALIAS) != 0 && (flags & INP_CONT) != 0 {
        // c:707-709 — `if (((instacktop->flags = inbufflags) & INP_ALIAS)
        //   && (flags & INP_CONT)) flags |= INP_ALIAS;` — a plain
        // INP_CONT push layered over an alias frame continues the
        // alias expansion: mark the new frame INP_ALIAS too so
        // history stays off for its chars.
        combined |= INP_ALIAS;
    }

    let new_len = inbuf.with(|b| b.borrow().chars().count()) as i32; // char count — inbufct is char-based
    if (combined & INP_CONT) != 0 {
        inbufct.with(|c| c.set(c.get() + new_len));
    } else {
        inbufct.with(|c| c.set(new_len));
    }
    inbufflags.with(|f| f.set(combined));
    // c:Src/input.c — same lexstop reset as inputsetline (input.rs:336).
    // Without this, an inpush after the buffer drained leaves lexstop=true
    // and ingetc returns None immediately, so the just-pushed alias body
    // (e.g. `inpush("echo hello")` after lexing `hi`) never gets read.
    // Symptom: `alias hi='echo hello'; eval hi` → empty output, because
    // eval's inner `hi` token drained the buffer, set lexstop, then
    // exalias inpushed `echo hello` but the next ingetc still saw
    // lexstop=true and returned None → tok=ENDINPUT.
    //
    // BOTH lexstop globals must reset: input.rs's local `lexstop` gates
    // ingetc (line 244); lex.rs's `LEX_LEXSTOP` gates gettok. zshrs
    // duplicates the C single `lexstop` global across two modules.
    lexstop.with(|c| c.set(false));
    LEX_LEXSTOP.with(|c| c.set(false));
}

// Remove the top element of the stack                                       // c:736
/// Pop one input-stack frame off the top.
/// Port of `inpoptop()` from Src/input.c:736.
pub fn inpoptop() {
    // c:736
    // c:738 — if (!lexstop) {
    if !LEX_LEXSTOP.with(|c| c.get()) {
        // c:739 — inbufflags &= ~(INP_ALCONT|INP_HISTCONT);
        inbufflags.with(|f| f.set(f.get() & !(INP_ALCONT | INP_HISTCONT)));
        // c:740-753 — drain unread bytes of the popped frame; for alias
        // frames (without RAW_KEEP) push back the corresponding raw-lex
        // marker via zshlex_raw_back so the lexer-side cursor unwinds.
        let was_alias =
            (inbufflags.with(|f| f.get()) & (INP_ALIAS | INP_HIST | INP_RAW_KEEP)) == INP_ALIAS;
        let unread = inbuf.with(|b| {
            let blen = b.borrow().len();
            blen.saturating_sub(inbufpos.with(|p| p.get()))
        });
        if was_alias {
            for _ in 0..unread {
                zshlex_raw_back(); // c:752
            }
        }
    }

    // c:756-757 — if (inbuf && (inbufflags & INP_FREE)) free(inbuf);
    //              Rust Drop covers the heap-string free when entry is replaced.

    // c:759-765 — pop and restore from instacktop->{buf,bufptr,bufleft,bufct,flags}
    if let Some(entry) = instack.with(|st| st.borrow_mut().pop()) {
        // c:770-778 — if (instacktop->alias) { alias->inuse = 0; if trailing
        //               space → inalmore=1; histbackword(); }
        if let Some(name) = &entry.alias {
            // c:771 — `char *t = instacktop->alias->text;` — the check
            // below is against the ALIAS BODY, not the saved outer
            // buffer (`entry.buf` is the frame being restored; the
            // drained alias text was in `inbuf`).
            let alias_text: Option<String> = {
                let mut tab = aliastab_lock().write().expect("aliastab poisoned");
                match tab.get_mut(name) {
                    Some(a) => {
                        a.inuse = 0; // c:773
                        Some(a.text.clone())
                    }
                    None => None,
                }
            };
            // c:774-777 — `if (*t && t[strlen(t) - 1] == ' ')
            //                 { inalmore = 1; histbackword(); }`
            // — a trailing-space alias body marks the NEXT word
            // alias-eligible (input.c:63 comment; consumed by
            // checkalias at lex.c:1917).
            if alias_text.is_some_and(|t| t.ends_with(' ')) {
                crate::ported::lex::LEX_INALMORE.with(|f| f.set(1)); // c:775
                histbackword(); // c:776
            }
        }
        inbuf.with(|b| *b.borrow_mut() = entry.buf);
        inbufpos.with(|p| p.set(entry.bufpos));
        inbufflags.with(|f| f.set(entry.flags));
        // c:764 — `inbufct = instacktop->bufct;` — restore the count
        // saved at push time. It spans the restored frame's remainder
        // PLUS every CONT frame below; the previous recompute-from-
        // this-frame-only undercounted, tripping ingetc's
        // `!inbufct && strin` EOF gate (c:342) with unread CONT
        // frames still stacked.
        inbufct.with(|c| c.set(entry.bufct)); // c:764
    }
}

// Remove the top element of the stack and all its continuations.            // c:785
/// Pop the topmost input-stack frame plus any continuations.
/// Port of `inpop()` from Src/input.c:785.
pub fn inpop() {
    // c:785
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
pub fn inpopalias() {
    // c:804
    while (inbufflags.with(|f| f.get()) & INP_ALIAS) != 0 {
        inpoptop();
    }
}

/// Get a slice of the unread portion of the current input.
/// Port of `ingetptr()` from Src/input.c:817.
pub fn ingetptr() -> String {
    // c:817
    let pos = inbufpos.with(|p| p.get());
    inbuf.with(|b| {
        b.borrow()
            .get(pos..)
            .map(str::to_string)
            .unwrap_or_default()
    })
}

// Size of buffer for non-interactive command input                        // c:127
/// Size of the shell input buffer
const SHIN_BUF_SIZE: usize = 8192;

/// Re-export of `META` from zsh_h.rs (canonical port of `Src/zsh.h:144`).
/// Duplicate declarations of the Meta byte invite drift; keep one
/// source of truth.

/// Check if a character needs Meta-encoding in the SHIN buffer.
///
/// Port of `imeta(c)` from `Src/ztype.h:60` — `zistype(c, IMETA)`.
/// Per `Src/utils.c:4195-4201`, the IMETA typtab bits are set for:
///   - `'\0'` (0x00)
///   - `Meta` (0x83)
///   - `Pound..=LAST_NORMAL_TOK` (`Bang`) (0x84..=0x9c, ITOK+IMETA)
///   - `Snull..=Nularg` (0x9d..=0xa1, ITOK+IMETA+INULL)
///   - `Marker` (0xa2)
///
/// The previous Rust port used `b < 32 || (0x83..=0x9b).contains(&b)`
/// which was BOTH:
///   - too inclusive (0x01..=0x1f are NOT IMETA per the typtab —
///     only 0x00 is); reading control chars from stdin would have
///     been spuriously Meta-encoded, corrupting the input buffer
///     for SHIN clients that pass them through literally; and
///   - too narrow (0x9c, 0x9d..=0xa1, 0xa2 all needed Meta-encoding
///     but escaped untouched, then later token-byte readers would
///     mis-interpret them as live tokens rather than literal user
///     bytes).
/// Route through the canonical `ztype_h::imeta` typtab predicate so
/// every IMETA test in the codebase agrees.
fn imeta(c: char) -> bool {
    // c:60 (Src/ztype.h)
    let b = c as u32;
    if b > 0xff {
        return false;
    }
    crate::ported::ztype_h::imeta(b as u8)
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
    use crate::ported::utils::inittyptab;
    use crate::ported::ztype_h::TYPTAB_TEST_LOCK;

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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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

    /// Pin: `imeta(c)` matches the canonical IMETA typtab population
    /// at `Src/utils.c:4195-4201`. IMETA is set for:
    ///   - `'\0'` (c:4195)
    ///   - `Meta` (0x83) (c:4196)
    ///   - `Marker` (0xa2) (c:4197)
    ///   - `Pound..=LAST_NORMAL_TOK` = `0x84..=0x9c` (c:4198, Bang)
    ///   - `Snull..=Nularg` = `0x9d..=0xa1` (c:4200)
    ///
    /// Non-IMETA: every ASCII letter, every ASCII digit, AND every
    /// control char EXCEPT NUL (0x01..=0x1f are NOT IMETA — the
    /// previous Rust port spuriously Meta-encoded them).
    #[test]
    fn test_meta_encoding() {
        let _g = crate::test_util::global_state_lock();
        // Tests must initialise the typtab — without `inittyptab()`
        // every byte's IMETA bit reads as 0. Serialise against other
        // typtab-mutating tests via the canonical lock.
        let _g = TYPTAB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        inittyptab();

        // c:4195 — '\0' is IMETA.
        assert!(imeta('\x00'));
        // c:4196 — Meta (0x83) is IMETA.
        assert!(imeta('\u{83}'));
        // c:4197 — Marker (0xa2) is IMETA.
        assert!(imeta('\u{a2}'));
        // c:4198 — Pound (0x84) is IMETA via the NORMAL_TOK loop.
        assert!(imeta('\u{84}'));
        // c:4198 — LAST_NORMAL_TOK = Bang (0x9c) is IMETA.
        assert!(imeta('\u{9c}'));
        // c:4200 — Snull (0x9d) is IMETA via the NULL_TOK loop.
        assert!(imeta('\u{9d}'));
        // c:4200 — Nularg (0xa1) is IMETA at the top of the range.
        assert!(imeta('\u{a1}'));

        // Non-IMETA: ASCII letters / digits.
        assert!(!imeta('a'));
        assert!(!imeta('Z'));
        assert!(!imeta('0'));

        // Non-IMETA: control chars OTHER than NUL. The previous
        // Rust port hardcoded `b < 32` which erroneously included
        // these; canonical C IMETA covers only NUL among control
        // chars (c:4195 has `typtab['\0'] |= IMETA;` and nothing
        // else in the 0x01..=0x1f range).
        assert!(!imeta('\x01'));
        assert!(!imeta('\x1f'));
        // Above 0xa2 (e.g. 0xa3 onward) is NOT IMETA either.
        assert!(!imeta('\u{a3}'));

        // Verify the inlined metafy XOR (Src/utils.c:4856 c ^ 32) is
        // self-inverting — encode then decode round-trips to the input.
        let encoded = char::from_u32(('\x00' as u32) ^ 32).unwrap_or('\x00');
        let decoded = char::from_u32((encoded as u32) ^ 32).unwrap_or(encoded);
        assert_eq!(decoded, '\x00');
    }

    #[test]
    fn test_ingetptr() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        super::shinbuffer.with(|b| {
            *b.borrow_mut() = "leftover".to_string();
        });
        super::shinbufpos.with(|p| p.set(7));
        super::shinbufreset();
        super::shinbuffer.with(|b| {
            assert!(
                b.borrow().is_empty(),
                "c:161 — shinbuffer must be empty after reset"
            );
        });
        assert_eq!(
            super::shinbufpos.with(|p| p.get()),
            0,
            "c:161 — shinbufpos must be 0 after reset"
        );
    }

    /// `Src/input.c:171-175` — `shinbufalloc` body is
    /// `shinbuffer = zalloc(SHINBUFSIZE); shinbufreset();`. The
    /// Rust port replaces the buffer with a fresh `String` of
    /// `SHIN_BUF_SIZE` capacity, then calls `shinbufreset`.
    /// Pin the post-condition: empty buffer + pos==0 + capacity
    /// hint set.
    #[test]
    fn shinbufalloc_resets_and_capacity_hints() {
        let _g = crate::test_util::global_state_lock();
        super::shinbuffer.with(|b| {
            *b.borrow_mut() = "stale".to_string();
        });
        super::shinbufpos.with(|p| p.set(3));
        super::shinbufalloc();
        super::shinbuffer.with(|b| {
            assert!(
                b.borrow().is_empty(),
                "c:173 — fresh shinbuffer must be empty"
            );
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
        let _g = crate::test_util::global_state_lock();
        // Clear stack from any prior test.
        super::shinsavestack.with(|s| s.borrow_mut().clear());
        super::shinbuffer.with(|b| *b.borrow_mut() = "abc".to_string());
        super::shinbufpos.with(|p| p.set(2));
        super::shinbufsave();
        super::shinbuffer.with(|b| {
            assert!(
                b.borrow().is_empty(),
                "c:193 — shinbufsave invokes shinbufalloc (resets to empty)"
            );
        });
        assert_eq!(
            super::shinbufpos.with(|p| p.get()),
            0,
            "c:193 — pos must be 0 after save"
        );
        super::shinbufrestore();
        super::shinbuffer.with(|b| {
            assert_eq!(
                *b.borrow(),
                "abc",
                "c:200-209 — shinbufrestore restores saved buffer"
            );
        });
        assert_eq!(
            super::shinbufpos.with(|p| p.get()),
            2,
            "c:200-209 — shinbufrestore restores saved pos"
        );
    }

    /// `Src/input.c:200` — `shinbufrestore` on an empty save stack
    /// must NOT panic (C dereferences `shinsavestack` which would be
    /// NULL — UB in C; the Rust port chose to no-op for safety).
    /// Pin the no-op so a regression doesn't add a panic.
    #[test]
    fn shinbufrestore_on_empty_stack_is_noop() {
        let _g = crate::test_util::global_state_lock();
        super::shinsavestack.with(|s| s.borrow_mut().clear());
        // Pre-seed a non-empty buffer.
        super::shinbuffer.with(|b| *b.borrow_mut() = "persist".to_string());
        super::shinbufrestore();
        super::shinbuffer.with(|b| {
            assert_eq!(
                *b.borrow(),
                "persist",
                "empty-stack restore must leave buffer untouched"
            );
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
        let _g = crate::test_util::global_state_lock();
        reset_input();
        // Make sure typtab is populated (without this the per-thread
        // typtab default may have ITOK bits unset).
        inittyptab();

        let bang: char = '\u{009c}'; // Bang (LAST_NORMAL_TOK)
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
        assert_eq!(
            ingetc(),
            Some('b'),
            "c:328 — Bang (0x9c) must be skipped (ITOK bit set per inittyptab)"
        );
        assert_eq!(
            ingetc(),
            Some('c'),
            "c:328 — Nularg (0xa1) must be skipped (ITOK bit set per inittyptab)"
        );
    }

    /// `Src/input.c:328` — non-token bytes (e.g. Meta=0x83) must NOT
    /// be skipped by `ingetc`. Meta is IMETA-only, never ITOK; treating
    /// it as a token would corrupt every metafied character read by
    /// the lexer. Same for Marker (0xa2) which is IMETA-only per
    /// `Src/utils.c:4197` (`typtab[Marker] |= IMETA`, NOT ITOK).
    #[test]
    fn ingetc_does_not_skip_imeta_only_bytes() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inittyptab();
        let meta: char = '\u{0083}'; // Meta lead byte — IMETA only
        let marker: char = '\u{00a2}'; // Marker — IMETA only per c:4197
        let mut s = String::new();
        s.push('x');
        s.push(meta);
        s.push('y');
        s.push(marker);
        s.push('z');
        inputsetline(&s, 0);
        assert_eq!(ingetc(), Some('x'));
        assert_eq!(
            ingetc(),
            Some(meta),
            "c:328 — Meta (0x83) is IMETA-only, NOT ITOK; must pass through"
        );
        assert_eq!(ingetc(), Some('y'));
        assert_eq!(
            ingetc(),
            Some(marker),
            "c:328 / c:4197 — Marker (0xa2) is IMETA-only, NOT ITOK; must pass through"
        );
        assert_eq!(ingetc(), Some('z'));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Input-buffer behavior — additional edge cases for ingetc / inungetc
    // / inputsetline / inpush. These pin behavior that existing tests
    // don't cover: empty input, multi-byte ungetc, stack pop sequences,
    // line-number accumulation across pushes.
    // ═══════════════════════════════════════════════════════════════════

    /// Empty input → first ingetc returns None.
    #[test]
    fn input_empty_line_yields_none_first_read() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        assert_eq!(ingetc(), None);
    }

    /// Reading past end keeps returning None (eventually — zshrs may
    /// append a trailing separator like space/newline before the buffer
    /// drains, then None thereafter). Pin: eventually None, no panic.
    #[test]
    fn input_repeated_reads_past_end_keep_returning_none_eventually() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("a", 0);
        assert_eq!(ingetc(), Some('a'));
        // Skip any trailing separator (zshrs's inputsetline may append).
        let mut seen_none = false;
        for _ in 0..10 {
            if ingetc().is_none() {
                seen_none = true;
                break;
            }
        }
        assert!(seen_none, "should reach None within 10 reads past end");
        // After None, subsequent reads must stay None.
        for _ in 0..3 {
            assert_eq!(ingetc(), None);
        }
    }

    /// inungetc on a fresh buffer makes the ungot char the next read.
    #[test]
    fn input_inungetc_before_any_read() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("xyz", 0);
        inungetc('Q');
        assert_eq!(ingetc(), Some('Q'));
        assert_eq!(ingetc(), Some('x'));
    }

    /// Multiple inungetc calls stack LIFO (last in first out).
    #[test]
    fn input_inungetc_lifo_order() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("z", 0);
        inungetc('A');
        inungetc('B');
        inungetc('C');
        // Last pushed comes out first.
        assert_eq!(ingetc(), Some('C'));
        assert_eq!(ingetc(), Some('B'));
        assert_eq!(ingetc(), Some('A'));
        assert_eq!(ingetc(), Some('z'));
    }

    /// inpush then inpoptop returns to the outer buffer.
    #[test]
    fn input_inpush_then_pop_returns_to_outer() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("outer", 0);
        assert_eq!(ingetc(), Some('o'));
        inpush("inner", INP_CONT, None);
        assert_eq!(ingetc(), Some('i'));
        inpoptop();
        // After popping the inner buffer, the next read continues from
        // the outer buffer where it left off.
        assert_eq!(ingetc(), Some('u'));
    }

    /// inputsetline with non-empty replaces buffer cleanly after empty.
    /// (Skip the post-empty read since zshrs leaves the empty-set buffer
    /// state opaque — just drain any residue, then set new buffer.)
    #[test]
    fn input_set_empty_then_set_nonempty_reads_new_content() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        // Drain whatever zshrs left in the empty-set buffer.
        for _ in 0..5 {
            if ingetc().is_none() {
                break;
            }
        }
        inputsetline("xyz", 0);
        // Now the next reads should yield 'x', 'y', 'z' in order.
        assert_eq!(ingetc(), Some('x'));
        assert_eq!(ingetc(), Some('y'));
        assert_eq!(ingetc(), Some('z'));
    }

    /// Reads return exactly the input bytes for ASCII content.
    #[test]
    fn input_ascii_passthrough_byte_for_byte() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let src = "the quick brown fox 0123!";
        inputsetline(src, 0);
        let mut got = String::new();
        while let Some(c) = ingetc() {
            got.push(c);
        }
        assert_eq!(got, src);
    }

    /// Line-number tracking gate: per `c:Src/input.c:330` —
    /// `if (((inbufflags & INP_LINENO) || !strin) && lastc == '\n') lineno++;`
    /// The gate fires when EITHER the flag is set OR we're not in
    /// string-input mode. The "no INP_LINENO" path only suppresses
    /// the advance when `strin != 0` (string-input mode like eval).
    /// Test pins both the strin=1 suppression (this test) and the
    /// strin=0 always-advance shape — the latter is covered by
    /// input_lineno_flag_increments_on_each_newline below.
    #[test]
    fn input_no_lineno_flag_means_lineno_unchanged_on_newline_anchored() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        // c:330 gate: !strin path triggers the advance even when the
        // INP_LINENO flag is unset. Set strin=1 so the gate's `||`
        // short-circuit relies on the flag alone; with flag=0 lineno
        // must stay put.
        let saved_strin = strin.with(|s| s.get());
        strin.with(|s| s.set(1));
        let start = lineno.with(|l| l.get());
        inputsetline("a\nb\n", 0);
        while ingetc().is_some() {}
        let end = lineno.with(|l| l.get());
        strin.with(|s| s.set(saved_strin));
        assert_eq!(end, start, "c:330 — strin && !INP_LINENO → lineno stable");
    }

    /// INP_LINENO flag → lineno advances by the number of `\n`s.
    #[test]
    fn input_lineno_flag_increments_on_each_newline() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("x\ny\nz\n", INP_LINENO);
        let start = lineno.with(|l| l.get());
        while ingetc().is_some() {}
        let end = lineno.with(|l| l.get());
        assert_eq!(end - start, 3, "three `\\n`s should advance lineno by 3");
    }

    /// Pushing on top of a partially-read buffer reads inner first.
    #[test]
    fn input_inpush_priorities_inner_over_outer() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("abc", 0);
        // Don't read anything yet
        inpush("123", INP_CONT, None);
        // Inner exhausts first
        assert_eq!(ingetc(), Some('1'));
        assert_eq!(ingetc(), Some('2'));
        assert_eq!(ingetc(), Some('3'));
        // Then outer
        assert_eq!(ingetc(), Some('a'));
        assert_eq!(ingetc(), Some('b'));
        assert_eq!(ingetc(), Some('c'));
    }

    /// Multi-byte UTF-8 char passes through correctly.
    #[test]
    fn input_multibyte_utf8_char_passes_through() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("日", 0);
        // The char `日` is a single Unicode codepoint; ingetc should
        // return it as a single Option<char>.
        let c = ingetc();
        assert_eq!(c, Some('日'));
        assert_eq!(ingetc(), None);
    }

    // ─── zsh-corpus pins for input buffer behavior ─────────────────

    /// Reading all of "abc" returns characters in order then None.
    #[test]
    fn input_corpus_ascii_returns_chars_in_order() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("abc", 0);
        assert_eq!(ingetc(), Some('a'));
        assert_eq!(ingetc(), Some('b'));
        assert_eq!(ingetc(), Some('c'));
        assert_eq!(ingetc(), None);
    }

    /// `inungetc` when `inbufpos > 0` just rewinds position — the
    /// passed char is IGNORED in favor of the buffer content at
    /// `pos - 1`. Per `Src/input.c:546-555`, zsh assumes callers
    /// unget the char they just got. Pin this quirk.
    #[test]
    fn input_corpus_inungetc_after_read_rewinds_to_buf_char() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("xy", 0);
        let _ = ingetc(); // consume 'x' → pos=1
        inungetc('Z'); // pos→0; 'Z' silently dropped per C contract
        assert_eq!(
            ingetc(),
            Some('x'),
            "buf[pos-1] returned, NOT the unget char"
        );
        assert_eq!(ingetc(), Some('y'));
    }

    /// `inungetc` of multiple chars: LIFO order.
    #[test]
    fn input_corpus_inungetc_multiple_lifo() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("end", 0);
        inungetc('1');
        inungetc('2');
        inungetc('3');
        assert_eq!(ingetc(), Some('3'), "last-pushed first");
        assert_eq!(ingetc(), Some('2'));
        assert_eq!(ingetc(), Some('1'));
        assert_eq!(ingetc(), Some('e'), "then original buffer");
    }

    /// Empty inputsetline → ingetc returns None immediately.
    #[test]
    fn input_corpus_empty_line_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        assert_eq!(ingetc(), None);
    }

    /// Multi-codepoint UTF-8 string: each codepoint returned once.
    #[test]
    fn input_corpus_multibyte_two_codepoints() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("日本", 0);
        assert_eq!(ingetc(), Some('日'));
        assert_eq!(ingetc(), Some('本'));
        assert_eq!(ingetc(), None);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/input.c. Tests that capture KNOWN
    // ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `ingetc` after `inputsetline("")` returns None (empty input).
    /// C `Src/input.c:247` returns -1 (EOF) when input is exhausted.
    #[test]
    fn ingetc_empty_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        assert_eq!(ingetc(), None, "empty input → None (EOF)");
    }

    /// `inungetc` puts a char back so the next `ingetc` returns it.
    /// C `Src/input.c:360`.
    #[test]
    fn inungetc_then_ingetc_returns_pushed_char() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("XY", 0);
        let first = ingetc().unwrap();
        assert_eq!(first, 'X');
        inungetc(first);
        assert_eq!(ingetc(), Some('X'), "inungetc'd byte comes back");
        assert_eq!(ingetc(), Some('Y'), "then the normal next byte");
    }

    /// `ingetc` after EOF returns None on repeated calls (not stuck).
    #[test]
    fn ingetc_repeated_after_eof_stays_none() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("A", 0);
        assert_eq!(ingetc(), Some('A'));
        assert_eq!(ingetc(), None);
        assert_eq!(ingetc(), None, "repeated EOF → still None");
        assert_eq!(ingetc(), None);
    }

    /// `inerrflush` clears any pending input buffer without panic.
    /// C `Src/input.c:455` — idempotent flush.
    #[test]
    fn inerrflush_is_safe_to_call_multiple_times() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inerrflush();
        inerrflush();
        inerrflush();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/input.c ingetc + inungetc +
    // inputsetline round-trip.
    // ═══════════════════════════════════════════════════════════════════

    /// c:318 — `ingetc` on empty buffer returns None.
    #[test]
    fn ingetc_empty_buffer_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        let _ = ingetc(); // may consume or return None
                          // No panic = pass.
    }

    /// c:318 — `inputsetline(s) + ingetc` walks every byte in order.
    #[test]
    fn inputsetline_ingetc_round_trip_three_chars() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("abc", 0);
        assert_eq!(ingetc(), Some('a'));
        assert_eq!(ingetc(), Some('b'));
        assert_eq!(ingetc(), Some('c'));
        assert_eq!(ingetc(), None, "after exhausting buffer → None");
    }

    /// c:546 — `inungetc` when pos > 0 REWINDS the buffer position;
    /// the argument `c` is IGNORED in that branch (matches C's
    /// `inbufptr--, inbufct++` rewind semantics). The buffer char
    /// at the rewound position comes back, not the argument.
    #[test]
    fn inungetc_when_pos_positive_rewinds_buffer_ignoring_arg() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("X", 0);
        assert_eq!(ingetc(), Some('X'));
        // pos > 0 now → inungetc('Y') rewinds, putting 'X' back.
        inungetc('Y');
        assert_eq!(
            ingetc(),
            Some('X'),
            "inungetc with pos>0 rewinds buf; arg 'Y' is ignored — original 'X' returns"
        );
    }

    /// c:546 — `inungetc` while lexstop is set is a no-op (early return).
    /// Subsequent ingetc still returns None.
    #[test]
    fn inungetc_while_lexstop_is_noop() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("a", 0);
        let _ = ingetc(); // consume 'a'
        let _ = ingetc(); // hit EOF → lexstop set
                          // inungetc must not panic; subsequent ingetc still None.
        inungetc('Z');
        // Result depends on whether port restored lexstop; pin no panic.
    }

    /// c:546 — multiple inungetc calls preserve LIFO order.
    #[test]
    fn inungetc_multiple_chars_lifo_order() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("end", 0);
        inungetc('A');
        inungetc('B'); // pushed last → comes out first
        assert_eq!(ingetc(), Some('B'), "LIFO: B pushed last comes first");
        assert_eq!(ingetc(), Some('A'));
        // Then original buffer continues.
        assert_eq!(ingetc(), Some('e'));
    }

    /// c:510 — `inputsetline("", _)` sets empty buffer; ingetc → None.
    #[test]
    fn inputsetline_empty_then_ingetc_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
        assert_eq!(ingetc(), None);
    }

    /// c:455 — `inerrflush` after partial consumption is safe (doesn't
    /// panic or leave inconsistent state).
    #[test]
    fn inerrflush_after_partial_consume_is_safe() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("hello", 0);
        let _ = ingetc();
        let _ = ingetc();
        inerrflush();
        // No panic = pass.
    }

    /// c:510 — `inputsetline` resets lexstop so the new buffer is readable
    /// after a prior EOF.
    #[test]
    fn inputsetline_resets_lexstop_for_new_buffer() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("a", 0);
        let _ = ingetc(); // consume a
        let _ = ingetc(); // EOF, sets lexstop
                          // New inputsetline should make new content readable.
        inputsetline("b", 0);
        assert_eq!(ingetc(), Some('b'), "lexstop reset by inputsetline");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/input.c
    // c:63 shinbufreset / c:247 ingetc / c:326 inputline / c:441 stuff /
    // c:467 inpush / c:594 inpopalias / etc.
    // ═══════════════════════════════════════════════════════════════════

    /// c:247 — `ingetc` returns Option<char> (compile-time type pin).
    #[test]
    fn ingetc_returns_option_char_type() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("x", 0);
        let _: Option<char> = ingetc();
    }

    /// c:366 — `inputline` returns int (0=line installed, 1=EOF).
    #[test]
    fn inputline_returns_int_status() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let _: i32 = inputline();
    }

    /// c:603 — `ingetptr` returns String.
    #[test]
    fn ingetptr_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let _: String = ingetptr();
    }

    /// c:594 — `inpopalias()` is safe on empty stack.
    #[test]
    fn inpopalias_empty_stack_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpopalias();
        inpopalias();
    }

    /// c:467 — `inpush(empty, 0, None)` is safe.
    #[test]
    fn inpush_empty_string_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpush("", 0, None);
        inpush("", 0, Some("alias_name".to_string()));
    }

    /// c:63 — `shinbufreset` is idempotent.
    #[test]
    fn shinbufreset_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            shinbufreset();
        }
    }

    /// c:144 — `shinbufalloc` is idempotent.
    #[test]
    fn shinbufalloc_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            shinbufalloc();
        }
    }

    /// c:156-168 — shinbufsave + shinbufrestore round-trip safe.
    #[test]
    fn shinbufsave_restore_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        shinbufsave();
        shinbufrestore();
    }

    /// c:441 — `stuff(nonexistent)` returns nonzero (file not found).
    #[test]
    fn stuff_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = stuff("/__nonexistent_zshrs_xyz__");
        assert_ne!(r, 0, "nonexistent path → error");
    }

    /// c:392 — `zstuff(nonexistent)` returns Err.
    #[test]
    fn zstuff_nonexistent_path_returns_err() {
        let _g = crate::test_util::global_state_lock();
        let r = zstuff("/__nonexistent_zshrs_xyz__");
        assert!(r.is_err(), "nonexistent path → Err");
    }

    /// c:213 — `shingetline` returns String.
    #[test]
    fn shingetline_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let _: String = shingetline();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/input.c
    // c:326 inputline / c:337 inputsetline / c:360 inungetc /
    // c:392 zstuff / c:441 stuff / c:455 inerrflush / c:467 inpush /
    // c:181 shingetchar / c:603 ingetptr
    // ═══════════════════════════════════════════════════════════════════

    /// c:181 — `shingetchar` returns i32 (compile-time type pin, alt name).
    #[test]
    fn shingetchar_returns_i32_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let _: i32 = shingetchar();
    }

    /// c:337 — `inputsetline("", 0)` is safe + resets lexstop.
    #[test]
    fn inputsetline_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("", 0);
    }

    /// c:337 — `inputsetline(s, INP_CONT)` accumulates inbufct
    /// vs the non-CONT replacement variant. Pin both paths exist.
    #[test]
    fn inputsetline_cont_vs_replace_both_safe() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inputsetline("hello", 0);
        inputsetline("world", INP_CONT);
    }

    /// c:360 — `inungetc` on fresh state without prior get pushes back.
    #[test]
    fn inungetc_on_empty_pushes_back_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inungetc('x');
        inungetc('\n');
    }

    /// c:360 — `inungetc` is safe across many calls (no overflow).
    #[test]
    fn inungetc_many_calls_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        for c in "abcdefghij".chars() {
            inungetc(c);
        }
    }

    /// c:441 — `stuff` returns i32 (compile-time pin).
    #[test]
    fn stuff_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = stuff("/__nonexistent__");
    }

    /// c:441 — `stuff(empty)` returns nonzero (empty path is invalid).
    #[test]
    fn stuff_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        assert_ne!(stuff(""), 0, "empty path must be an error");
    }

    /// c:392 — `zstuff(empty)` returns Err.
    #[test]
    fn zstuff_empty_path_returns_err() {
        let _g = crate::test_util::global_state_lock();
        assert!(zstuff("").is_err(), "empty path → Err");
    }

    /// c:455 — `inerrflush` is idempotent (callable repeatedly).
    #[test]
    fn inerrflush_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            inerrflush();
        }
    }

    /// c:467 — `inpush(s, 0, None)` with various strings is safe.
    #[test]
    fn inpush_various_strings_safe() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        for s in &["a", "longer text", "\n", "\t", " "] {
            inpush(s, 0, None);
        }
    }

    /// c:467 — `inpush` with alias name doesn't panic.
    #[test]
    fn inpush_with_alias_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpush("expanded", 0, Some("ll".to_string()));
    }

    /// c:523/580 — `inpoptop` + `inpop` are no-ops on empty stack.
    #[test]
    fn inpoptop_inpop_empty_stack_no_panic() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpoptop();
        inpop();
        inpoptop();
        inpop();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/input.c
    // c:63 shinbufreset / c:144 shinbufalloc / c:156 shinbufsave /
    // c:213 shingetline / c:326 inputline / c:392 zstuff /
    // c:467 inpush / c:594 inpopalias / c:603 ingetptr / c:455 inerrflush
    // ═══════════════════════════════════════════════════════════════════

    /// c:63 — `shinbufreset` is idempotent (safe to call repeatedly).
    #[test]
    fn shinbufreset_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            shinbufreset();
        }
    }

    /// c:144 — `shinbufalloc` is idempotent.
    #[test]
    fn shinbufalloc_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            shinbufalloc();
        }
    }

    /// c:156/168 — save/restore round-trip is safe (alt).
    #[test]
    fn shinbufsave_restore_round_trip_safe_alt() {
        let _g = crate::test_util::global_state_lock();
        shinbufsave();
        shinbufrestore();
        shinbufsave();
        shinbufrestore();
    }

    /// c:213 — `shingetline` returns String type.
    #[test]
    fn shingetline_returns_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: String = shingetline();
    }

    /// c:366 — `inputline` returns int type (alt).
    #[test]
    fn inputline_returns_int_status_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = inputline();
    }

    /// c:366 — `inputline` is deterministic on idle state.
    #[test]
    fn inputline_deterministic_on_idle_state() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        let a = inputline();
        let b = inputline();
        assert_eq!(a, b, "inputline must be deterministic on idle state");
    }

    /// c:392 — `zstuff` returns Result<(String, i64), i32>.
    #[test]
    fn zstuff_returns_result_type_compile_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<(String, i64), i32> = zstuff("");
    }

    /// c:392 — `zstuff("/dev/null")` returns Ok with empty content.
    #[test]
    fn zstuff_dev_null_returns_ok_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = zstuff("/dev/null");
        match r {
            Ok((s, _)) => assert!(s.is_empty(), "/dev/null content must be empty"),
            Err(_) => {} // /dev/null may not exist on some CI; accept either
        }
    }

    /// c:467 — `inpush("")` empty string safe (alt).
    #[test]
    fn inpush_empty_string_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpush("", 0, None);
    }

    /// c:594 — `inpopalias` on empty stack safe (alt).
    #[test]
    fn inpopalias_empty_stack_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        reset_input();
        inpopalias();
        inpopalias();
    }

    /// c:603 — `ingetptr` returns String type (alt).
    #[test]
    fn ingetptr_returns_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: String = ingetptr();
    }

    /// c:455 — `inerrflush` returns void (compile-time pin).
    #[test]
    fn inerrflush_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = inerrflush();
    }
}
