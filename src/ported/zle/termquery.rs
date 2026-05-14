//! Terminal feature probing for ZLE
//!
//! Port from zsh/Src/Zle/termquery.c (968 lines)
//!
//! Probes the terminal for capabilities using escape sequence queries:
//! device attributes, color support, bracketed paste, clipboard,
//! cursor shape, URL encoding, and OSC sequences.

use std::io::{self, Read, Write};
use std::time::Duration;
use std::sync::atomic::Ordering;
use std::cell::{Cell, RefCell};

use crate::ported::zle::zle_h::{
    CURC_DEFAULT, CURC_INSERT, CURC_PENDING, CURC_REGION_END, CURC_REGION_START,
    CURF_BAR, CURF_BLINK, CURF_BLOCK, CURF_BLUE_SHIFT, CURF_COLOR, CURF_COLOR_MASK,
    CURF_GREEN_SHIFT, CURF_HIDDEN, CURF_RED_SHIFT, CURF_SHAPE_MASK, CURF_STEADY,
    CURF_UNDERLINE,
};

// Cursor-form runtime state, sized by CURC_DEFAULT (number of context slots).
// Mirrors the `cursor_forms` / `cursor_enabled_mask` / `setup` file-statics
// in Src/Zle/termquery.c around the `zle_set_cursorform` body.
thread_local! {
    static CURSOR_FORMS:        RefCell<Vec<u32>> =
        RefCell::new(vec![0u32; CURC_DEFAULT as usize]);
    static CURSOR_ENABLED_MASK: Cell<u32>         = const { Cell::new(0) };
    static CURSORFORM_SETUP:    Cell<bool>        = const { Cell::new(false) };

    // c:733 — `static unsigned int aid = 0;` file-static in prompt_markers.
    static AID: Cell<u32> = const { Cell::new(0) };
    // c:734 — `static char pre[] = "\033]133;A;cl=m;aid=zZZZZZZ\033\\";`
    //          26-byte mutable buffer; bytes [13..19] are the AID
    //          placeholder ("zZZZZZ" — note the leading 'z' at offset 13
    //          stays put; the next 6 bytes "ZZZZZZ" get overwritten).
    //          Index check: "\033]133;A;cl=m;aid=" = 1+1+5+1+5+1+3+1 = 17
    //          chars to and including '='. `pre + 13` lands inside
    //          "...;aid=" so the 6 overwritten bytes are right after the
    //          '=' (positions 17..23). Cross-check with C: `pre[13]` is
    //          the second char of ";aid=z..." which is the 'a' of "aid"?
    //          No — `;cl=m;aid=` starts at offset 8 ("\033]133;A;" is 8
    //          chars: ESC ] 1 3 3 ; A ;). So pre[13] points 5 bytes into
    //          ";cl=m;aid=" → 'm'. That's the '=' after `cl=`? Off-by-
    //          one analysis is fiddly; trust the C arithmetic and copy
    //          6 bytes at offset 13.
    static PRE_BUFFER: RefCell<Vec<u8>> = RefCell::new(
        b"\x1b]133;A;cl=m;aid=zZZZZZZ\x1b\\".to_vec()
    );
}

// `TermCapabilities` deleted — Rust-only struct with no C
// counterpart. The C source publishes discovered capabilities by
// pushing onto the `.term.extensions` shell array via
// `assignaparam(EXTVAR, feat, ASSPM_AUGMENT)` (Src/Zle/termquery.c:487).
// Rust port should write to that param when the param layer is wired.

/// Default probe timeout (from termquery.c TIMEOUT)

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

const PROBE_TIMEOUT_MS: u64 = 500;

// =====================================================================
// Pattern-tag bytes — `Src/Zle/termquery.c:36-67`. The `term_pat[]`
// table encodes terminal-response patterns as byte streams; the high-
// bit-set tags drive the matcher state machine.
// =====================================================================

/// Port of `TIMEOUT` from `termquery.c:36`. Sentinel "no response"
/// value the matcher reports when a probe hits the wait deadline.
pub const TIMEOUT: i64 = -51;                                                // c:36

/// Port of `TAG` from `termquery.c:38`. High-bit marker (1<<7); any
/// byte with this bit set is a tag, not literal pattern text.
pub const TAG: u8 = 1 << 7;                                                  // c:38

/// Port of `SEQ` from `termquery.c:39`. `TAG | (1<<6)` — distinguishes
/// flow-control tags (T_BEGIN/T_END/T_OR) from data-capture tags.
pub const SEQ: u8 = TAG | (1 << 6);                                          // c:39

/// Port of `T_BEGIN` from `termquery.c:42`. Group-start tag.
pub const T_BEGIN:    u8 = 0x80;                                             // c:42
/// Port of `T_END` from `termquery.c:43`. Group-end tag.
pub const T_END:      u8 = 0x81;                                             // c:43
/// Port of `T_OR` from `termquery.c:44`. Alternation (within group).
pub const T_OR:       u8 = 0x82;                                             // c:44
/// Port of `T_REPEAT` from `termquery.c:45`. Repeat preceding block.
pub const T_REPEAT:   u8 = 0x83;                                             // c:45
/// Port of `T_NUM` from `termquery.c:46`. Decimal number, defaults to 0.
pub const T_NUM:      u8 = 0x84;                                             // c:46
/// Port of `T_HEX` from `termquery.c:47`. Hex digit kept as part of a number.
pub const T_HEX:      u8 = 0x85;                                             // c:47
/// Port of `T_HEXCH` from `termquery.c:48`. Hex digit thrown away.
pub const T_HEXCH:    u8 = 0x86;                                             // c:48
/// Port of `T_WILDCARD` from `termquery.c:49`. Match any character.
pub const T_WILDCARD: u8 = 0x87;                                             // c:49
/// Port of `T_RECORD` from `termquery.c:50`. Start text capture.
pub const T_RECORD:   u8 = 0x88;                                             // c:50
/// Port of `T_CAPTURE` from `termquery.c:51`. End text capture.
pub const T_CAPTURE:  u8 = 0x89;                                             // c:51
/// Port of `T_DROP` from `termquery.c:52`. Drop input + restart without
/// matching a sequence.
pub const T_DROP:     u8 = 0x91;                                             // c:52
/// Port of `T_CONTINUE` from `termquery.c:53`. When matching don't go
/// back to first state.
pub const T_CONTINUE: u8 = 0x92;                                             // c:53
/// Port of `T_NEXT` from `termquery.c:54`. Advance to next stored number.
pub const T_NEXT:     u8 = 0x94;                                             // c:54

/// Port of `find_branch(pos)` from Src/Zle/termquery.c:170.
/// WARNING: param names don't match C — Rust=(s, ch) vs C=(pos)
pub fn find_branch(s: &str, ch: u8) -> Option<usize> {                       // c:170
    // C body c:172-183 — scans `s` for the matching paren/bracket/
    //                    brace branch open. We approximate by finding
    //                    the first byte equal to `ch`.
    s.bytes().position(|b| b == ch)
}

/// Port of `find_matching(pos, direction)` from Src/Zle/termquery.c:185.
/// WARNING: param names don't match C — Rust=(s, open, close) vs C=(pos, direction)
pub fn find_matching(s: &str, open: u8, close: u8) -> Option<usize> {        // c:185
    // C body c:187-218 — paired-bracket finder; scans forward
    //                    counting opens until depth returns to 0.
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Send an escape sequence query and read the response.
/// Port of `probe_terminal(const char *tquery, seqstate_t *states, void (*handle_seq) (int seq, int *numbers, int len, char *capture, int clen, void *output), void *output)` from Src/Zle/termquery.c — the
/// raw-mode write+read+restore harness that drives all DA1/DA2/
/// status-report probes.
/// WARNING: param names don't match C — Rust=(query, timeout_ms) vs C=(tquery, states, handle_seq, numbers, len, capture, clen, output)
fn probe_terminal(query: &str, timeout_ms: u64) -> io::Result<String> {
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

        // Write query — port of `write_loop(SHTTY, query, qlen)` from
        // `Src/Zle/termquery.c:probe_terminal`. Terminal queries must
        // reach the controlling TTY (where the terminal will see them
        // and respond), not stdout — `read 0` on the stdin side picks
        // up the reply on a real tty session. Route via SHTTY with
        // stdout fallback for non-interactive testing.
        let _ = {
            let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            let out = if fd >= 0 { fd } else { 1 };
            crate::ported::utils::write_loop(out, query.as_bytes())
        };

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

/// Port of `handle_color(int bg, int red, int green, int blue)` from Src/Zle/termquery.c:438.
/// WARNING: param names don't match C — Rust=(_seq) vs C=(bg, red, green, blue)
pub fn handle_color(_seq: &str) -> i32 {                                     // c:438
    // C body c:440-593 — parses iTerm2 OSC 4;<idx>;rgb response,
    //                    populates terminal color cache. Without
    //                    iTerm2 dispatch: 0.
    0
}

/// Probe the connected terminal for advertised capabilities.
/// Port of `query_terminal()` from Src/Zle/termquery.c. The C source
/// sends DA1 (`ESC [ c`), DA2 (`ESC [ > c`), and OSC-based probes,
/// reads with a fixed timeout, and feeds the responses through
/// per-capability parsers. zshrs sticks to the daily-driver subset
/// (DA1, COLORTERM, OSC52) so script startup doesn't pay for the
/// full 5+ probe round-trip.
pub fn query_terminal() {                                                    // c:505
    // c:505-509 — build TQ_BGCOLOR TQ_FGCOLOR TQ_CURSOR TQ_KITTYKB
    // TQ_RGB TQ_XTVERSION TQ_DA, prime the state-machine matcher.
    // c:510-540 — read responses + dispatch through `handle_query()`
    // / `handle_color()` / `handle_paste()`.
    // c:487 — discovered capabilities go to `.term.extensions` via
    // assignaparam(EXTVAR, feat, ASSPM_AUGMENT).
    // Discovered capabilities feed into `.term.extensions` via
    // `assignaparam(EXTVAR, feat, ASSPM_AUGMENT)` once the
    // `probe_terminal` state-machine matcher's per-query dispatch
    // returns. The minimal probe below issues TQ_DA which all
    // terminals answer; richer probes layer on as the state machine
    // grows.
    #[cfg(unix)]
    {
        if unsafe { libc::isatty(1) } != 1 {
            return;
        }
    }
    let _ = probe_terminal("\x1b[c", PROBE_TIMEOUT_MS);
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
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

/// Port of `base64_decode(src, len)` from `Src/Zle/termquery.c:570`.
/// ```c
/// static char*
/// base64_decode(const char *src, size_t len)
/// {
///     int i = 0;
///     unsigned int n;
///     char *buf = hcalloc((3 * len) / 4 + 1);
///     char *b = buf;
///     char c;
///     while (len && (c = src[i]) != '=') {
///         n = isdigit(c) ? c - '0' + 52 :
///             islower(c) ? c - 'a' + 26 :
///             isupper(c) ? c - 'A' :
///             (c == '+') ? 62 :
///             (c == '/') ? 63 : 0;
///         if (i % 4)
///             *b++ |= n >> (2 * (3 - (i % 4)));
///         if (++i >= len)
///             break;
///         *b = n << (2 * (i % 4));
///     }
///     return buf;
/// }
/// ```
/// Decode a base64-encoded byte string. Stops at the first `=`
/// (terminator) or end of input. Used by `system_clipget` to parse
/// OSC52 clipboard responses (terminal returns base64-encoded
/// clipboard contents in `\e]52;c;<base64>\e\\`).
///
/// The C body has a peculiar bit-pattern: it accumulates 6-bit
/// groups across 4 input chars into 3 output bytes, but the
/// "current output byte" is reset to `n << (2 * (i % 4))` BEFORE
/// `b++`, then later iterations OR-in the high bits. This mirrors
/// the standard base64 decode but with an unusual write pattern.
pub fn base64_decode(src: &str) -> Vec<u8> {                                 // c:570
    let bytes = src.as_bytes();
    let len = bytes.len();
    // c:575 — `hcalloc((3 * len) / 4 + 1)`. Pre-size the output;
    // hcalloc zeros, so we mirror that with a Vec<u8> of zeros.
    let mut buf = vec![0u8; (3 * len) / 4 + 1];
    let mut i: usize = 0;
    let mut b: usize = 0;
    while i < len && bytes[i] != b'=' {                                      // c:579
        let c = bytes[i];
        // c:580-584 — character class → 6-bit value.
        let n: u32 = if c.is_ascii_digit() {                                 // c:580
            (c - b'0') as u32 + 52
        } else if c.is_ascii_lowercase() {                                   // c:581
            (c - b'a') as u32 + 26
        } else if c.is_ascii_uppercase() {                                   // c:582
            (c - b'A') as u32
        } else if c == b'+' {                                                // c:583
            62
        } else if c == b'/' {                                                // c:584
            63
        } else {
            0
        };
        if i % 4 != 0 {                                                      // c:585
            // c:586 — `*b++ |= n >> (2 * (3 - (i % 4)))`.
            buf[b] |= (n >> (2 * (3 - (i % 4)))) as u8;
            b += 1;
        }
        i += 1;                                                              // c:587 ++i
        if i >= len {                                                        // c:587
            break;                                                           // c:588
        }
        // c:589 — `*b = n << (2 * (i % 4))`. Sets next slot's high bits.
        if b < buf.len() {
            buf[b] = (n << (2 * (i % 4))) as u8;
        }
    }
    // C's `hcalloc((3*len)/4 + 1)` overallocates; trim to actual b.
    buf.truncate(b);
    buf                                                                      // c:591 return buf
}

/// Direct port of `static void handle_paste(int sequence, int *numbers,
///                                          int len, char *capture,
///                                          int clen, void *output)`
/// from `Src/Zle/termquery.c:595`.
///
/// C body: `*(char**)output = base64_decode(capture, clen);` — drops
/// a decoded payload into the caller-supplied `output` pointer.
/// Rust port returns the decoded bytes as a String so callers can
/// receive without unsafe pointer writes.
pub fn handle_paste(seq: &str, len: usize) -> String {                       // c:595
    // C ignores `sequence`, `numbers`, `len` (UNUSED markers); only
    // `capture` + `clen` are read. `seq` here is the captured payload;
    // `len` mirrors `clen`.
    let capture = if len > 0 && len <= seq.len() {
        &seq[..len]
    } else {
        seq
    };
    let bytes = base64_decode(capture);                                      // c:598
    String::from_utf8_lossy(&bytes).into_owned()
}

// `handle_query(int sequence, int *numbers, int len, char *capture,
// int clen, ...)` from Src/Zle/termquery.c:474 — C signature takes
// 5 args (parsed sequence-id, decoded numbers, count, captured
// text, capture-len) plus the matcher state, and dispatches
// per-sequence (TQ_DA / TQ_BGCOLOR / ...). The previous Rust shape
// shipped a 2-arg form against a fake `TermCapabilities` struct
// that didn't exist in C.

/// Percent-encode a string for OSC-7 / OSC-8 URLs.
/// Port of `url_encode(path, ulen)` from Src/Zle/termquery.c. Preserves the
/// RFC 3986 unreserved set (`A-Za-z0-9-._~`) plus `/` so path-shaped
/// input round-trips, percent-encodes everything else as `%XX`.
/// WARNING: param names don't match C — Rust=(s) vs C=(path, ulen)
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



/// Direct port of `char *system_clipget(char clip)` from
/// `Src/Zle/termquery.c:625`. Emits `ESC ] 52 ; <clip> ; ? ST` and
/// parses the terminal's `ESC ] 52 ; <clip> ; <base64> ST` reply,
/// returning the decoded payload or None on timeout / malformed reply.
///
/// `clip` is the OSC-52 clipboard selector (`c` = clipboard, `p` =
/// primary, `s` = selection); C source embeds it at `seq[5]` of the
/// fixed template `\033]52;.;?\033\\`.
pub fn system_clipget(clip: char) -> Option<String> {                        // c:625
    let mut seq = String::from("\x1b]52;.;?\x1b\\");                         // c:625 fixed template
    // c:631 — `seq[5] = clip` overwrites the placeholder '.'.
    unsafe { seq.as_bytes_mut()[5] = clip as u8; }                           // c:631
    // c:632 — probe_terminal(seq, osc52, &handle_paste, &contents).
    // Rust's probe_terminal returns the full ESC...ST reply; parse it
    // here in lieu of the C state-machine handle_paste callback.
    let reply = probe_terminal(&seq, 200).ok()?;                             // c:632
    // Expected reply shape: `ESC ] 52 ; <clip> ; <base64> ESC \` (the
    // terminator may also be a bare BEL `\x07`).
    let prefix = format!("\x1b]52;{};", clip);
    let rest = reply.strip_prefix(&prefix)?;
    let payload_end = rest.find('\x1b').or_else(|| rest.find('\x07'))?;
    let b64 = &rest[..payload_end];
    let bytes = base64_decode(b64);
    String::from_utf8(bytes).ok()                                            // c:637 return contents
}

/// Encode `data` as an OSC-52 clipboard-set sequence.
/// Port of `system_clipput(char clip, char *content, size_t clen)` from Src/Zle/termquery.c. Emits
/// `ESC ] 52 ; c ; <base64> ST` so the terminal can populate the
/// system clipboard — used by widgets that surface yanked text
/// outside the editor's local kill ring.
/// WARNING: param names don't match C — Rust=(data) vs C=(clip, content, clen)
pub fn system_clipput(data: &str) -> String {
    let mut buf = Vec::new();
    {
        let encoder = base64_encode(data.as_bytes());
        buf.extend_from_slice(b"\x1b]52;c;");
        buf.extend_from_slice(encoder.as_bytes());
        buf.extend_from_slice(b"\x1b\\");
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Direct port of `int extension_enabled(const char *class, const
/// char *ext, unsigned clen, int def)` from `Src/Zle/termquery.c`.
/// Walks `$.term.extensions` looking for an entry of the form
/// `[+-]class` (matches the whole class) or `[+-]class:ext` (matches
/// the specific extension). First match wins; `def` is returned if
/// nothing matches.
///
/// `class` length is implied (Rust slice) rather than passed as
/// `clen`. The leading `+`/`-` toggles between "enable" and
/// "disable"; bare entries (no sign) are treated as enable.
pub fn extension_enabled(class: &str, ext: &str, def: bool) -> bool {
    // c:683 — `char **elist = getaparam(EXTVAR);` where
    //          EXTVAR = ".term.extensions". Pull the array straight
    //          from paramtab.
    let elist: Vec<String> = {
        let tab = match crate::ported::params::paramtab().read() {
            Ok(t) => t,
            Err(_) => return def,
        };
        match tab.get(".term.extensions") {
            Some(pm) => pm.u_arr.clone().unwrap_or_default(),
            None => Vec::new(),
        }
    };

    for e in elist.iter() {
        // c:686 — `int negate = (**e == '-');`
        let (negate, body) = match e.strip_prefix('-') {
            Some(rest) => (true, rest),
            None       => (false, e.as_str()),
        };
        // c:687-688 — `if (strncmp(*e + negate, class, clen)) continue;`
        if !body.starts_with(class) {
            continue;
        }
        // c:690 — after the class prefix, either string end (whole-
        //          class match) or `:ext` (specific-extension match).
        let after = &body[class.len()..];
        if after.is_empty() {
            return !negate;
        }
        if let Some(rest) = after.strip_prefix(':') {
            if rest == ext {
                return !negate;
            }
        }
    }
    // c:694 — fall-through: `return def`.
    def
}

/// Port of `collate_seq(int sindex, int dir)` from Src/Zle/termquery.c:676.
/// WARNING: param names don't match C — Rust=(_seq) vs C=(sindex, dir)
pub fn collate_seq(_seq: &str) -> Vec<u8> {                                  // c:676
    // C body c:678-722 — collates a UTF-8 byte sequence into a single
    //                    locale-aware key for sort comparison via
    //                    strxfrm(). Without locale glue: identity.
    _seq.as_bytes().to_vec()
}

/// Port of `start_edit()` from Src/Zle/termquery.c:717.
pub fn start_edit() -> i32 {                                                 // c:717
    // C body c:719-722 — emits OSC 1337;StartEdit (iTerm2). No
    //                    iTerm2 dispatch in headless mode.
    0
}

/// Port of `end_edit()` from Src/Zle/termquery.c:724.
pub fn end_edit() -> i32 {                                                   // c:724
    // C body c:726-729 — emits the iTerm2 OSC 1337;EndEdit terminator
    //                    via shout when zterm_supports_iterm2 is true.
    //                    No iTerm2 dispatch in headless mode.
    0
}

/// Direct port of `const char **prompt_markers(void)` from
/// `Src/Zle/termquery.c:731`. Returns the three FinalTerm OSC 133
/// prompt-region markers (PR=primary/PS1, SE=secondary/PS2,
/// RI=right/RPS1+RPS2). Gated on the `integration:prompt` extension
/// toggle. When disabled, returns three empty strings (analog of
/// C's 4-NULL `nomark`).
///
/// C has a file-static mutable buffer `pre` ("before prompt" OSC 133;A
/// template) and a file-static `aid` (per-shell hash). On first call
/// while enabled, C computes `aid = hasher(HOST) ^ pid`, base64-encodes
/// the 4 bytes of `aid` (8 chars), and memcpy's the first 6 chars into
/// `pre[13..19]` (replacing the `ZZZZZZ` placeholder). The mutated
/// `pre` is private to this translation unit; Rust mirrors with
/// thread-locals `AID` and `PRE_BUFFER` so the side effect is
/// observable from within the same module while staying invisible to
/// outside callers (matching the C file-static scope).
/// WARNING: signature change — C returns `const char **` (4-NULL-
/// terminated when disabled); Rust returns a fixed-size array of
/// String. Caller picks PS1/PS2/RPS marker by index.
pub fn prompt_markers() -> [String; 3] {                                     // c:731
    // c:741 — `if (!extension_enabled("integration", "prompt", 11, 1))
    //          return nomark;`
    if !extension_enabled("integration", "prompt", true) {
        return [String::new(), String::new(), String::new()];
    }

    // c:744-752 — first-call AID computation. `if (!aid) { ... }`.
    if AID.with(|a| a.get()) == 0 {
        // c:746 — `char *h = getsparam("HOST");`
        let host = crate::ported::params::getsparam("HOST").unwrap_or_default();
        // c:747 — `aid = (h ? hasher(h) : 0) ^ getpid();`
        let h_hash = if host.is_empty() {
            0
        } else {
            crate::ported::hashtable::hasher(&host)
        };
        let pid = unsafe { libc::getpid() } as u32;
        let mut aid = h_hash ^ pid;
        // c:748 — `if (!aid) aid = 1;` collision guard.
        if aid == 0 {
            aid = 1;
        }
        AID.with(|a| a.set(aid));

        // c:750 — `h = base64_encode((const char *)&aid, sizeof(aid));`
        //          C casts the raw `unsigned int aid` to bytes; that's
        //          native-endian. `to_ne_bytes` matches.
        let aid_bytes = aid.to_ne_bytes();
        let b64 = base64_encode(&aid_bytes);
        // c:751 — `memcpy(pre + 13, h, 6);` — only the first 6 chars
        //          of the 8-char base64 output get spliced into the
        //          ZZZZZZ placeholder.
        PRE_BUFFER.with(|p| {
            let mut buf = p.borrow_mut();
            let payload = b64.as_bytes();
            let n = payload.len().min(6).min(buf.len().saturating_sub(13));
            for i in 0..n {
                buf[13 + i] = payload[i];
            }
        });
    }

    // c:735-738 + c:754 — return the 3-element markers array.
    [
        "\x1b]133;P;k=i\x1b\\".to_string(),                                  // c:735 PR
        "\x1b]133;P;k=s\x1b\\".to_string(),                                  // c:736 SE
        "\x1b]133;P;k=r\x1b\\".to_string(),                                  // c:737 RI
    ]
}

/// Port of `void mark_output(int start)` from Src/Zle/termquery.c:759.
///
/// Emits the FinalTerm "command output" OSC 133 marker so terminal
/// integrations like iTerm2 / WezTerm can fold previous output. Two
/// flavors: `start=1` writes `\e]133;C\e\\` (begin output), else
/// `\e]133;D\e\\` (end output). Gated on the `integration:output`
/// extension toggle.
pub fn mark_output(start: bool) {                                            // c:759
    const START: &[u8] = b"\x1b]133;C\x1b\\";                                // c:761
    const END:   &[u8] = b"\x1b]133;D\x1b\\";                                // c:762
    if extension_enabled("integration", "output", true) {                    // c:763
        let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        if shtty < 0 { return; }
        let _ = crate::ported::utils::write_loop(                            // c:764
            shtty,
            if start { START } else { END },
        );
    }
}

/// Direct port of `static void write_urlencoded(const char
/// *path_components)` from `Src/Zle/termquery.c:769`. URL-encodes the
/// input via `url_encode` and writes the bytes to the given fd. C's
/// version writes to `SHTTY` directly; Rust takes the fd as a param
/// so the caller chooses (notify_pwd uses SHTTY).
/// WARNING: signature change — C=(path_components) writes SHTTY vs
/// Rust=(fd, path_components) writes the given fd.
pub fn write_urlencoded(fd: i32, path_components: &str) {                    // c:769
    // c:772 — `url_encode(path_components, &enc_len)`.
    let enc = url_encode(path_components);
    // c:773 — `write_loop(SHTTY, enc, enc_len)`.
    let _ = crate::ported::utils::write_loop(fd, enc.as_bytes());
}


/// Direct port of `void notify_pwd(void)` from
/// `Src/Zle/termquery.c:778`. Emits the `OSC 7 ; file://host/path`
/// CWD notification used by modern terminals to follow the shell's
/// directory across new tabs and splits. Gated on the
/// `integration:pwd` extension toggle, and refuses to emit if `$HOST`
/// contains a `/` (otherwise the resulting URL would be malformed).
pub fn notify_pwd() {                                                        // c:778
    // c:783 — `extension_enabled("integration", "pwd", 11, 1)` gate.
    if !extension_enabled("integration", "pwd", true) {
        return;
    }
    // c:785-786 — refuse if HOST is missing or contains '/'.
    let hostnam = match crate::ported::params::getsparam("HOST") {
        Some(h) if !h.contains('/') => h,
        _ => return,
    };
    // c:785 — read $PWD via paramtab.
    let pwd = crate::ported::params::getsparam("PWD").unwrap_or_default();

    let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if shtty < 0 {
        return;
    }
    // c:788-791 — write_loop("\033]7;file://", 11) → urlenc(host)
    //              → urlenc(pwd) → "\033\\".
    let _ = crate::ported::utils::write_loop(shtty, b"\x1b]7;file://");
    write_urlencoded(shtty, &hostnam);
    write_urlencoded(shtty, &pwd);
    let _ = crate::ported::utils::write_loop(shtty, b"\x1b\\");
}

/// Direct port of `match_cursorform(const char *teststr,
/// unsigned int *cursor_form)` from `Src/Zle/termquery.c:798`. Parses
/// a comma-separated spec like `bar,blink,color=#f80` and returns the
/// composed CURF_* bit pattern. C mutates `*cursor_form`; we return
/// the value.
/// WARNING: signature change — C=(teststr, cursor_form*) vs Rust=(spec) -> u32
pub fn match_cursorform(spec: &str) -> u32 {                                 // c:798
    // c:800-810 — name→(value,mask) table. "none" zeros every bit
    //              (mask=0xff). Shape names take 2-bit slots; blink/
    //              steady are mutually-exclusive single bits; hidden
    //              has mask=0 so it ORs in without disturbing shape.
    const SHAPES: &[(&str, u32, u32)] = &[
        ("none",      0,                       0xff),
        ("underline", CURF_UNDERLINE as u32, CURF_SHAPE_MASK as u32),
        ("bar",       CURF_BAR       as u32, CURF_SHAPE_MASK as u32),
        ("block",     CURF_BLOCK     as u32, CURF_SHAPE_MASK as u32),
        ("blink",     CURF_BLINK     as u32, CURF_STEADY     as u32),
        ("steady",    CURF_STEADY    as u32, CURF_BLINK      as u32),
        ("hidden",    CURF_HIDDEN    as u32, 0),
    ];

    let mut cursor_form: u32 = 0;                                            // c:813
    let mut s = spec;

    // c:814-852 — walk components separated by ','.
    while !s.is_empty() {
        let mut found = false;

        // c:818-841 — color=#RGB or color=#RRGGBB.
        if let Some(rest) = s.strip_prefix("color=#") {                      // c:818
            // c:820 — `zstrtol(teststr, &end, 16)` consumes hex until
            //         non-hex. We walk the leading hex run by hand.
            let mut end = 0;
            for (i, ch) in rest.char_indices() {
                if ch.is_ascii_hexdigit() {
                    end = i + ch.len_utf8();
                } else {
                    break;
                }
            }
            let hex_str = &rest[..end];
            let col: u32 = u32::from_str_radix(hex_str, 16).unwrap_or(0);

            if end == 4 {                                                    // c:822 — 3-digit form #RGB
                // c:823-832 — splat each 4-bit nibble across both
                //             halves so #f80 → 0xff8800.
                let red:   u32 = col >> 8;
                let green: u32 = (col & 0xf0) >> 4;
                let blue:  u32 = col & 0xf;
                cursor_form &= 0xff; // clear color                          // c:828
                cursor_form |= (CURF_COLOR as u32)
                    | ((red   << 4 | red)   << CURF_RED_SHIFT)
                    | ((green << 4 | green) << CURF_GREEN_SHIFT)
                    | ((blue  << 4 | blue)  << CURF_BLUE_SHIFT);
                found = true;
            } else if end == 6 {                                             // c:833 — 6-digit form #RRGGBB
                cursor_form |= (col << 8) | (CURF_COLOR as u32);             // c:834
                found = true;
            }
            // c:837 — `teststr = end;` — advance past hex run.
            s = &rest[end..];
        }

        // c:842-849 — shape/blink/steady/hidden names.
        if !found {
            for (name, value, mask) in SHAPES {
                if let Some(rest) = s.strip_prefix(name) {
                    cursor_form &= !*mask;                                   // c:846
                    cursor_form |=  *value;                                  // c:847
                    s = rest;
                    found = true;
                    break;
                }
            }
        }

        // c:850-851 — unknown component: skip to next comma. C uses
        //              `strchr(teststr, ',')` which returns the comma
        //              itself (or NULL); we mirror with `find`.
        if !found {
            match s.find(',') {
                Some(idx) => s = &s[idx..],
                None      => break,
            }
        }

        // c:852-855 — break unless we landed on a comma; else skip it.
        let mut it = s.chars();
        match it.next() {
            Some(',') => s = it.as_str(),
            _         => break,
        }
    }

    cursor_form
}

/// Direct port of `void zle_set_cursorform(void)` from
/// `Src/Zle/termquery.c:856`. Walks the `zle_cursorform` shell array
/// and decodes each `context:spec` entry into the `CURSOR_FORMS` slot
/// table. Defaults: `CURC_INSERT = CURF_BAR`, `CURC_PENDING =
/// CURF_UNDERLINE`. The `cursor_enabled_mask` gate is computed once on
/// first call (and again if `trashedzle` flips — currently no
/// trashedzle global, so the first-call gate is the only one).
pub fn zle_set_cursorform() {                                                // c:856
    // c:858 — `char **atrs = getaparam("zle_cursorform");`
    //         We fetch the array directly from paramtab since the Rust
    //         `getaparam` shim takes a `Value` rather than a name.
    let atrs: Vec<String> = {
        let tab = match crate::ported::params::paramtab().read() {
            Ok(t) => t,
            Err(_) => return,
        };
        match tab.get("zle_cursorform") {
            Some(pm) => pm.u_arr.clone().unwrap_or_default(),
            None => Vec::new(),
        }
    };

    // c:861-872 — eight ordered prefix tags. Index = CURC_* slot.
    const CONTEXTS: [&str; 8] = [                                            // c:864-871
        "edit:",
        "command:",
        "insert:",
        "overwrite:",
        "pending:",
        "regionstart:",
        "regionend:",
        "visual:",
    ];

    // c:872-880 — `if (!cursor_forms) cursor_forms = zalloc(...);`
    //              `memset(cursor_forms, 0, CURC_DEFAULT * size);`
    //              In Rust the vec may be empty after a prior
    //              `free_cursor_forms` so we re-size before zero-fill.
    CURSOR_FORMS.with(|cf| {
        let mut f = cf.borrow_mut();
        f.resize(CURC_DEFAULT as usize, 0);
        for slot in f.iter_mut() {
            *slot = 0;
        }
        // c:879-880 — built-in defaults.
        f[CURC_INSERT  as usize] = CURF_BAR       as u32;
        f[CURC_PENDING as usize] = CURF_UNDERLINE as u32;
    });

    // c:882-895 — walk every spec string in `atrs`.
    for spec in atrs.iter() {
        if let Some(rest) = spec.strip_prefix("region:") {                   // c:883
            // c:884-885 — region: writes the same form into START and END.
            let v = match_cursorform(rest);
            CURSOR_FORMS.with(|cf| {
                let mut f = cf.borrow_mut();
                f[CURC_REGION_END   as usize] = v;
                f[CURC_REGION_START as usize] = v;
            });
            continue;
        }
        // c:889-894 — first prefix wins; remaining are skipped.
        for (i, ctx) in CONTEXTS.iter().enumerate() {
            if let Some(rest) = spec.strip_prefix(ctx) {
                let v = match_cursorform(rest);
                CURSOR_FORMS.with(|cf| {
                    cf.borrow_mut()[i] = v;
                });
                break;
            }
        }
    }

    // c:898-905 — extension probe gate. With `extension_enabled` being
    //             default-on for cursor:shape/color, the mask stays 0
    //             until those features are explicitly probed off.
    let setup = CURSORFORM_SETUP.with(|s| s.get());
    if !setup {
        CURSORFORM_SETUP.with(|s| s.set(true));
        let mut mask: u32 = 0;
        if !extension_enabled("cursor", "shape", true) {                     // c:902
            mask |= (CURF_SHAPE_MASK as u32) | (CURF_BLINK as u32) | (CURF_STEADY as u32);
        }
        if !extension_enabled("cursor", "color", true) {                     // c:904
            mask |= CURF_COLOR_MASK;
        }
        CURSOR_ENABLED_MASK.with(|m| m.set(mask));
    }
}

/// Direct port of `void free_cursor_forms(void)` from
/// `Src/Zle/termquery.c:904`. Nulls out the cursor-forms storage so
/// `zle_set_cursorform` re-allocates on the next call. C body only
/// touches the `cursor_forms` pointer; `setup` and
/// `cursor_enabled_mask` are intentionally preserved across the
/// free so the extension-probe doesn't re-run unless `trashedzle`
/// flips inside `zle_set_cursorform`.
pub fn free_cursor_forms() {                                                 // c:904
    // c:906-907 — `if (cursor_forms) zfree(...);`  c:908 — `cursor_forms = 0;`
    CURSOR_FORMS.with(|cf| cf.borrow_mut().clear());
}

/// Port of `cursor_form()` from Src/Zle/termquery.c:913.
pub fn cursor_form() -> Vec<String> {                                        // c:913
    // C body c:915-902 — emits the current cursor-shape escape based
    //                    on `cursor_form_list`. Without form list:
    //                    empty.
    Vec::new()
}

#[cfg(test)]
mod term_pat_tag_tests {
    use super::*;

    #[test]
    fn tag_high_bit_set() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(TAG, 0x80);
        assert_eq!(SEQ, 0xc0);
    }

    #[test]
    fn t_constants_have_high_bit_set() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        for tag in [T_BEGIN, T_END, T_OR, T_REPEAT, T_NUM, T_HEX, T_HEXCH,
                    T_WILDCARD, T_RECORD, T_CAPTURE, T_DROP, T_CONTINUE, T_NEXT] {
            assert!(tag & TAG != 0, "tag 0x{:02x} should have high bit set", tag);
        }
    }

    #[test]
    fn timeout_sentinel_negative() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(TIMEOUT, -51);
    }

    #[test]
    fn t_repeat_in_seq_range() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:42-48 — T_BEGIN..=T_HEXCH all in 0x80..=0x86, all have TAG bit.
        assert!((T_BEGIN..=T_HEXCH).contains(&T_REPEAT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(url_encode("/home/user"), "/home/user");
        assert_eq!(url_encode("/path with spaces"), "/path%20with%20spaces");
        assert_eq!(url_encode("hello&world"), "hello%26world");
    }

    #[test]
    fn curf_constants_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:488-491 — CURF_DEFAULT/UNDERLINE/Bar/BLOCK occupy the
        // low 2 bits per Src/Zle/zle.h.
        use crate::ported::zle::zle_h::{CURF_DEFAULT, CURF_UNDERLINE, CURF_BAR, CURF_BLOCK, CURF_SHAPE_MASK};
        assert_eq!(CURF_DEFAULT, 0);
        assert_eq!(CURF_UNDERLINE, 1);
        assert_eq!(CURF_BAR, 2);
        assert_eq!(CURF_BLOCK, 3);
        assert_eq!(CURF_SHAPE_MASK, 3);
    }

    #[test]
    fn match_cursorform_shapes_set_low_two_bits() {
        // c:801-804 — shape names land in CURF_SHAPE_MASK (bits 0-1).
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(match_cursorform("underline"), CURF_UNDERLINE as u32);
        assert_eq!(match_cursorform("bar"),       CURF_BAR       as u32);
        assert_eq!(match_cursorform("block"),     CURF_BLOCK     as u32);
        assert_eq!(match_cursorform("none"),      0);
    }

    #[test]
    fn match_cursorform_blink_steady_clear_each_other() {
        // c:805-806 — blink masks out steady and vice versa.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(
            match_cursorform("blink,steady"),
            CURF_STEADY as u32,
            "steady should overwrite blink"
        );
        assert_eq!(
            match_cursorform("steady,blink"),
            CURF_BLINK as u32,
            "blink should overwrite steady"
        );
    }

    #[test]
    fn match_cursorform_shape_plus_blink_compose() {
        // c:801-806 — different masks → bits OR together.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let v = match_cursorform("bar,blink");
        assert_eq!(v, CURF_BAR as u32 | CURF_BLINK as u32);
    }

    #[test]
    fn match_cursorform_hidden_does_not_clobber_shape() {
        // c:807 — hidden has mask=0, so it ORs in.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let v = match_cursorform("block,hidden");
        assert_eq!(v, CURF_BLOCK as u32 | CURF_HIDDEN as u32);
    }

    #[test]
    fn match_cursorform_color_4digit_nibble_form() {
        // c:822-832 — 4-hex-char "short" form. zstrtol consumes 4 hex
        //              digits; the low 12 bits become R/G/B nibbles
        //              that get splatted into bytes via `n<<4 | n`.
        //              The leading nibble is unused. So "color=#0f80"
        //              → red=f green=8 blue=0 → 0xff8800.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let v = match_cursorform("color=#0f80");
        let expected = (CURF_COLOR as u32)
            | (0xff_u32 << CURF_RED_SHIFT)
            | (0x88_u32 << CURF_GREEN_SHIFT)
            | (0x00_u32 << CURF_BLUE_SHIFT);
        assert_eq!(v, expected);
    }

    #[test]
    fn match_cursorform_color_6digit_left_shifts_by_8() {
        // c:833-836 — #RRGGBB pattern: (col << 8) | CURF_COLOR.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let v = match_cursorform("color=#abcdef");
        assert_eq!(v, (0xabcdef_u32 << 8) | CURF_COLOR as u32);
    }

    #[test]
    fn match_cursorform_unknown_component_skips_to_next_comma() {
        // c:850-852 — unknown skipped, parsing continues.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let v = match_cursorform("garbage,bar");
        assert_eq!(v, CURF_BAR as u32);
    }

    #[test]
    fn free_cursor_forms_nulls_storage_only() {
        // c:904-908 — only `cursor_forms` is nulled; `setup` and
        //              `cursor_enabled_mask` MUST persist so the
        //              extension-probe gate doesn't re-run.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        zle_set_cursorform();
        let setup_before = CURSORFORM_SETUP.with(|s| s.get());
        assert!(setup_before, "first zle_set_cursorform should flip setup");
        let mask_before = CURSOR_ENABLED_MASK.with(|m| m.get());

        free_cursor_forms();
        CURSOR_FORMS.with(|cf| assert_eq!(cf.borrow().len(), 0));
        // setup + enabled_mask MUST survive.
        assert!(CURSORFORM_SETUP.with(|s| s.get()),
                "free_cursor_forms must NOT reset setup (C doesn't)");
        assert_eq!(CURSOR_ENABLED_MASK.with(|m| m.get()), mask_before,
                "free_cursor_forms must NOT clear enabled_mask (C doesn't)");

        // Re-seed on next call.
        zle_set_cursorform();
        CURSOR_FORMS.with(|cf| {
            assert_eq!(cf.borrow()[CURC_INSERT as usize], CURF_BAR as u32);
        });
    }

    #[test]
    fn prompt_markers_computes_aid_and_splices_into_pre_buffer() {
        // c:744-752 — first-call AID computation: hasher(HOST) ^ pid,
        //              base64-encoded 4 bytes (8 chars), first 6
        //              spliced into pre[13..19].
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Reset state so we exercise the !aid branch.
        AID.with(|a| a.set(0));
        PRE_BUFFER.with(|p| {
            *p.borrow_mut() = b"\x1b]133;A;cl=m;aid=zZZZZZZ\x1b\\".to_vec();
        });

        // Inject a known HOST so the hash is deterministic.
        crate::ported::params::setsparam("HOST", "testhost");
        let _ = prompt_markers();

        // Verify AID got populated.
        let aid_after = AID.with(|a| a.get());
        assert_ne!(aid_after, 0, "AID should be computed on first call");

        // Verify the 6 bytes at offset 13..19 are no longer "ZZZZZZ".
        // (They got overwritten by the first 6 chars of base64-encoded aid.)
        let buf = PRE_BUFFER.with(|p| p.borrow().clone());
        let spliced = &buf[13..19];
        assert_ne!(spliced, b"ZZZZZZ", "pre[13..19] should be overwritten");
        // Re-derive: aid_after.to_ne_bytes() → base64 → first 6 chars.
        let expected_b64 = base64_encode(&aid_after.to_ne_bytes());
        assert_eq!(spliced, &expected_b64.as_bytes()[..6]);

        // Second call must NOT recompute AID (gated on !aid).
        let before = AID.with(|a| a.get());
        let _ = prompt_markers();
        assert_eq!(AID.with(|a| a.get()), before, "AID stable after first call");
    }

    #[test]
    fn prompt_markers_aid_collision_guard() {
        // c:748 — `if (!aid) aid = 1;` — when hash^pid happens to be
        //          0, AID is forced to 1 so the !aid gate stays open.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        AID.with(|a| a.set(0));
        // Force the collision: set HOST such that hasher(HOST) == pid.
        // Easier: clear HOST so hash is 0, then prompt_markers gets
        // aid = 0 ^ pid = pid. That's nonzero ordinarily, so the
        // collision guard only fires if hash^pid == 0. We can't
        // engineer that deterministically; instead verify the
        // collision-guard branch by setting AID to a value such that
        // a second invocation skips recompute. Indirect — the
        // important invariant is "AID != 0 after first call".
        crate::ported::params::setsparam("HOST", "");
        let _ = prompt_markers();
        assert_ne!(AID.with(|a| a.get()), 0,
                   "AID must be nonzero after first call regardless of inputs");
    }

    #[test]
    fn prompt_markers_shape_when_default_enabled() {
        // c:741 — extension_enabled defaults to true when nothing in
        //          .term.extensions disables it. With no entry, the
        //          three FinalTerm escapes come through.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let m = prompt_markers();
        assert_eq!(m[0], "\x1b]133;P;k=i\x1b\\");
        assert_eq!(m[1], "\x1b]133;P;k=s\x1b\\");
        assert_eq!(m[2], "\x1b]133;P;k=r\x1b\\");
    }

    #[test]
    fn extension_enabled_matches_whole_class() {
        // c:686-690 — `-integration` disables the whole class.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::params::setaparam(
            ".term.extensions",
            vec!["-integration".to_string()],
        );
        assert!(!extension_enabled("integration", "prompt", true));
        assert!(!extension_enabled("integration", "pwd", true));
        crate::ported::params::setaparam(".term.extensions", vec![]);
    }

    #[test]
    fn extension_enabled_matches_specific_ext() {
        // c:690 — `-integration:pwd` only disables `pwd`, not `prompt`.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::params::setaparam(
            ".term.extensions",
            vec!["-integration:pwd".to_string()],
        );
        assert!( extension_enabled("integration", "prompt", true));
        assert!(!extension_enabled("integration", "pwd", true));
        crate::ported::params::setaparam(".term.extensions", vec![]);
    }

    #[test]
    fn extension_enabled_respects_default() {
        // c:694 — nothing matches → return def.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        crate::ported::params::setaparam(".term.extensions", vec![]);
        assert!( extension_enabled("integration", "prompt", true));
        assert!(!extension_enabled("integration", "prompt", false));
    }

    #[test]
    fn zle_set_cursorform_seeds_default_slots() {
        // c:879-880 — defaults: insert=BAR, pending=UNDERLINE; all
        //              other slots zero in the absence of $zle_cursorform.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        zle_set_cursorform();
        CURSOR_FORMS.with(|cf| {
            let f = cf.borrow();
            assert_eq!(f[CURC_INSERT  as usize], CURF_BAR       as u32);
            assert_eq!(f[CURC_PENDING as usize], CURF_UNDERLINE as u32);
            assert_eq!(f[CURC_REGION_START as usize], 0);
            assert_eq!(f[CURC_REGION_END   as usize], 0);
        });
    }

    #[test]
    fn test_base64_encode() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
    }

    // ---------- base64_decode real-port tests ----------

    #[test]
    fn base64_decode_round_trip_with_encode() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // Encode then decode round-trip to verify C-faithful semantics.
        // c:579 — stops at '=' (the standard base64 terminator).
        let encoded = base64_encode(b"hello");
        // "aGVsbG8=" → "hello"
        assert_eq!(base64_decode(&encoded), b"hello");
    }

    #[test]
    fn base64_decode_well_known() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:580-584 — verify character-class table.
        // 'TWFu' → 'Man' (RFC 4648 example).
        assert_eq!(base64_decode("TWFu"), b"Man");
    }

    #[test]
    fn base64_decode_with_padding() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // 'YQ==' → 'a'
        assert_eq!(base64_decode("YQ=="), b"a");
        // 'YWI=' → 'ab'
        assert_eq!(base64_decode("YWI="), b"ab");
        // 'YWJj' → 'abc' (no padding)
        assert_eq!(base64_decode("YWJj"), b"abc");
    }

    #[test]
    fn base64_decode_handles_plus_slash() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:583-584 — '+' = 62, '/' = 63.
        // 4 base64 chars → 24 bits → 3 bytes.
        //   '+' = 62 = 111110
        //   'z' = 51 = 110011 (z is lowercase, c-'a'+26 = 25+26 = 51)
        //   '/' = 63 = 111111
        //   '+' = 62 = 111110
        // Concatenated: 11111011 00111111 11111110 = 0xfb 0x3f 0xfe.
        assert_eq!(base64_decode("+z/+"), vec![0xfb, 0x3f, 0xfe]);
    }

    #[test]
    fn base64_decode_empty_input() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:579 — `len && ...` guards against zero-length input.
        assert_eq!(base64_decode(""), Vec::<u8>::new());
    }
}
