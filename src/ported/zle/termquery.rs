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

// `TermCapabilities` deleted — Rust-only struct with no C
// counterpart. The C source publishes discovered capabilities by
// pushing onto the `.term.extensions` shell array via
// `assignaparam(EXTVAR, feat, ASSPM_AUGMENT)` (Src/Zle/termquery.c:487).
// Rust port should write to that param when the param layer is wired.

/// Default probe timeout (from termquery.c TIMEOUT)

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::extensions::widget::*;
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
    // Real body deferred — needs param resolver + the full state-
    // machine `probe_terminal()` matcher wired. The previous Rust
    // placeholder shipped a wrong return type (`TermCapabilities`)
    // and an env-var-only path that bypassed the real probes.
    #[cfg(unix)]
    {
        if unsafe { libc::isatty(1) } != 1 {
            return;
        }
    }
    let _ = probe_terminal("\x1b[c", PROBE_TIMEOUT_MS);
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

// Direct port of `handle_query(int sequence, int *numbers, int len,
// char *capture, int clen, ...)` from Src/Zle/termquery.c:474 deferred
// — the C signature takes 5 args (parsed sequence-id, decoded
// numbers, count, captured text, capture-len) plus the matcher state,
// and dispatches per-sequence (TQ_DA / TQ_BGCOLOR / ...). The
// previous Rust placeholder shipped a 2-arg shape against the fake
// `TermCapabilities` struct that didn't exist in C.

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

/// Test whether a named terminal extension is enabled in the current
/// session.
/// Port of `extension_enabled(const char *class, const char *ext, unsigned clen, int def)` from Src/Zle/termquery.c. The C
/// source consults the cached probe state stored on the global
/// `caps`; we re-derive each call from `$TERM` / `$COLORTERM` /
/// `$TERM_PROGRAM` since the probe-once-cache machinery isn't wired
/// through this crate yet.
/// WARNING: param names don't match C — Rust=(name) vs C=(class, ext, clen, def)
pub fn extension_enabled(name: &str) -> bool {
    match name {
        "bracketed-paste" => std::env::var("TERM")
            .map(|t| !t.starts_with("dumb") && !t.starts_with("cons"))
            .unwrap_or(false),
        "truecolor" => std::env::var("COLORTERM")
            .map(|v| v == "truecolor" || v == "24bit")
            .unwrap_or(false),
        "osc7" | "osc133" => std::env::var("TERM_PROGRAM")
            .map(|v| matches!(v.as_str(), "iTerm.app" | "WezTerm" | "kitty"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Direct port of `void zle_set_cursorform(void)` from
/// `Src/Zle/termquery.c:856`. C reads the `zle_cursorform` shell
/// parameter and emits the CSI-q DECSCUSR sequence corresponding
/// to the CURF_* bit-encoding (CURF_UNDERLINE/CURF_BAR/CURF_BLOCK
/// + CURF_BLINK/CURF_STEADY/CURF_HIDDEN). Real body deferred —
/// needs the param resolver wired through this call site. The
/// previous Rust placeholder shipped a fake `CursorShape` enum
/// param and 7-arm match that don't exist in C.
pub fn zle_set_cursorform() {                                                // c:856
    // c:856 — `char **atrs = getaparam("zle_cursorform");`
    // c:859 — pick the per-keymap-context entry via `cursor_forms[]`.
    // c:868-953 — emit CSI Ps SP q with CURF_* decode.
}

/// Encode the `OSC 7 ; file://host/path` CWD notification used by
/// modern terminals to track the shell's directory across new tabs
/// and splits.
/// Port of `notify_pwd()` from Src/Zle/termquery.c. The C source
/// emits the same sequence at `chpwd` time.
/// WARNING: param names don't match C — Rust=(path) vs C=()
pub fn notify_pwd(path: &str) -> String {
    let hostname = crate::utils::gethostname();
    format!("\x1b]7;file://{}{}\x1b\\", hostname, url_encode(path))
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

/// Port of `collate_seq(int sindex, int dir)` from Src/Zle/termquery.c:676.
/// WARNING: param names don't match C — Rust=(_seq) vs C=(sindex, dir)
pub fn collate_seq(_seq: &str) -> Vec<u8> {                                  // c:676
    // C body c:678-722 — collates a UTF-8 byte sequence into a single
    //                    locale-aware key for sort comparison via
    //                    strxfrm(). Without locale glue: identity.
    _seq.as_bytes().to_vec()
}

/// Port of `cursor_form()` from Src/Zle/termquery.c:913.
pub fn cursor_form() -> Vec<String> {                                        // c:913
    // C body c:915-902 — emits the current cursor-shape escape based
    //                    on `cursor_form_list`. Without form list:
    //                    empty.
    Vec::new()
}

/// Port of `end_edit()` from Src/Zle/termquery.c:724.
pub fn end_edit() -> i32 {                                                   // c:724
    // C body c:726-729 — emits the iTerm2 OSC 1337;EndEdit terminator
    //                    via shout when zterm_supports_iterm2 is true.
    //                    No iTerm2 dispatch in headless mode.
    0
}

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

/// Port of `free_cursor_forms()` from Src/Zle/termquery.c:904.
pub fn free_cursor_forms() {                                                 // c:904
    // C body c:906-911 — frees the cursor_form_list strings via
    //                    zfree+next walk. Drop covers it; no-op.
}

/// Port of `handle_color(int bg, int red, int green, int blue)` from Src/Zle/termquery.c:438.
/// WARNING: param names don't match C — Rust=(_seq) vs C=(bg, red, green, blue)
pub fn handle_color(_seq: &str) -> i32 {                                     // c:438
    // C body c:440-593 — parses iTerm2 OSC 4;<idx>;rgb response,
    //                    populates terminal color cache. Without
    //                    iTerm2 dispatch: 0.
    0
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
    if extension_enabled("integration") {                                    // c:763
        let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        if shtty < 0 { return; }
        let _ = crate::ported::utils::write_loop(                            // c:764
            shtty,
            if start { START } else { END },
        );
    }
}

/// Port of `match_cursorform(const char *teststr, unsigned int *cursor_form)` from Src/Zle/termquery.c:798.
/// WARNING: param names don't match C — Rust=(_name) vs C=(teststr, cursor_form)
pub fn match_cursorform(_name: &str) -> Option<String> {                     // c:798
    // C body c:800-902 — looks up named cursor form (e.g. "block",
    //                    "underline", "bar") in cursor_form_list and
    //                    returns its escape. No form list: None.
    None
}

/// Port of `prompt_markers()` from Src/Zle/termquery.c:731.
pub fn prompt_markers() -> i32 {                                             // c:731
    // C body c:733-757 — emits OSC 133;A,B,C,D markers around the
    //                    prompt for FinalTerm-aware terminals. No
    //                    FinalTerm dispatch.
    0
}

/// Port of `start_edit()` from Src/Zle/termquery.c:717.
pub fn start_edit() -> i32 {                                                 // c:717
    // C body c:719-722 — emits OSC 1337;StartEdit (iTerm2). No
    //                    iTerm2 dispatch in headless mode.
    0
}

/// Port of `write_urlencoded(const char *path_components)` from Src/Zle/termquery.c:769.
pub fn write_urlencoded(path_components: &str) -> String {                                 // c:769
    // C body c:771-796 — URL-encodes a string for OSC 8 hyperlink
    //                    emission. Real escape: convert non-printable
    //                    + reserved bytes to %HH.
    let mut out = String::with_capacity(path_components.len());
    for &b in path_components.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
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
