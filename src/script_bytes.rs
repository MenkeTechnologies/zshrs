//! !!! WARNING: RUST-ONLY MODULE — NO C COUNTERPART !!!
//!
//! Lossless decode of RAW SCRIPT BYTES into the shell pipeline's
//! `String` form.
//!
//! C zsh has no encoding requirement on script input. `Src/input.c`
//! reads bytes (`shingetchar`, c:229) and `Src/utils.c:4856 metafy`
//! escapes only the reserved IMETA bytes (`Src/utils.c:4195-4201` —
//! `'\0'`, `Meta` 0x83, `Marker` 0xa2 and the token range
//! `Pound`..`Nularg`); every other byte, including a lone ISO-8859-1
//! `0xe9`, travels through the lexer untouched and is written back
//! out verbatim. That is why a legacy byte in a completion file has
//! never bothered zsh.
//!
//! Rust `String` is UTF-8, so a raw non-UTF-8 byte cannot be stored
//! directly. zshrs already has one encoding for exactly this, used by
//! the `$'\xNN'` decoders (`src/ported/lex.rs` `getkeystring_dollar_quote`,
//! `src/extensions/compile_zsh.rs` `meta_encode_byte`) and decoded at
//! every write/exec boundary by `crate::ported::utils::unmetafy_str`:
//! a byte `>= 0x80` becomes the two chars `U+0083` (Meta) and
//! `U+00(b ^ 32)`. This module applies that same encoding to the
//! bytes a script file or SHIN hands us.
//!
//! The transform is the byte-level twin of `Src/input.c:267
//! shingetline`, which the interactive/stdin path already performs
//! inline: valid UTF-8 stays real `char`s (the lexer is Unicode-based
//! — `src/ported/lex.rs:6171` "zshrs does not metafy, it keeps UTF-8
//! `str`"), and only bytes that cannot be part of a UTF-8 sequence
//! are Meta-encoded. Clean-ASCII and valid-UTF-8 input therefore
//! decode to exactly the same `String` `read_to_string` produced, so
//! the common path is unchanged; the difference is that an invalid
//! byte no longer kills the read.

use std::io;
use std::path::Path;

/// Meta-encode ONE raw byte into the pipeline's `String` form.
///
/// Mirrors the metafy step of `Src/utils.c:4856`
/// (`if (imeta(c)) { *p++ = Meta; *p++ = c ^ 32; }`) transposed onto
/// zshrs's char-level encoding, and is the exact inverse of
/// `crate::ported::utils::unmetafy_str`.
fn push_meta_byte(out: &mut String, b: u8) {
    if b < 0x80 {
        out.push(b as char);
    } else {
        out.push('\u{83}'); // Meta — Src/zsh.h:144
        out.push(char::from(b ^ 32));
    }
}

/// Decode raw script bytes into the pipeline's `String` form without
/// losing a single byte.
///
/// Valid UTF-8 runs are copied verbatim; every byte that is not part
/// of a valid UTF-8 sequence is Meta-encoded so
/// `crate::ported::utils::unmetafy_str` reproduces it exactly at the
/// output boundary.
///
/// A literal `U+0083` that was validly UTF-8 encoded in the source
/// (bytes `c2 83`) is copied through as the `char` `U+0083`, matching
/// what `Src/input.c:267 shingetline` already does on the stdin path;
/// it then reads as a Meta marker, which is the pre-existing ambiguity
/// of the char-level encoding and is not introduced here.
pub fn decode_script_bytes(bytes: &[u8]) -> String {
    // Fast path: the overwhelmingly common case is a fully valid
    // file, where this is one validation pass and one allocation —
    // the same work `read_to_string` did.
    match std::str::from_utf8(bytes) {
        Ok(s) => return s.to_string(),
        Err(_) => {}
    }

    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => {
                let good = e.valid_up_to();
                // SAFETY-free: `valid_up_to` bytes are valid UTF-8 by
                // definition of the error.
                out.push_str(std::str::from_utf8(&rest[..good]).unwrap_or_default());
                // `error_len() == None` means the input ENDS with a
                // truncated-but-so-far-valid sequence; those trailing
                // bytes are still real bytes on disk, so encode them
                // all rather than dropping them.
                let bad = e.error_len().unwrap_or(rest.len() - good);
                for &b in &rest[good..good + bad] {
                    push_meta_byte(&mut out, b);
                }
                rest = &rest[good + bad..];
            }
        }
    }
}

/// Read a script file as raw bytes and decode it losslessly.
///
/// Replaces `fs::read_to_string` on every path that loads shell CODE
/// (`zshrs FILE`, `source`/`.`, `autoload`, `stuff`, `$(< FILE)`).
/// `read_to_string` rejected the ENTIRE file on the first invalid
/// byte, which surfaced as a hard error for a script argument and as
/// a SILENT no-op for `source` and `autoload`.
pub fn read_script_file(path: impl AsRef<Path>) -> io::Result<String> {
    Ok(decode_script_bytes(&std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::utils::unmetafy_str;

    #[test]
    fn ascii_and_utf8_decode_unchanged() {
        for s in ["", "echo hi\n", "s=日本語\n", "e=🎉 é ü\n"] {
            assert_eq!(decode_script_bytes(s.as_bytes()), s);
        }
    }

    #[test]
    fn invalid_bytes_round_trip_through_unmetafy() {
        for raw in [
            &b"echo caf\xe9\n"[..],
            &b"\xff\xfe"[..],
            &b"a\x80b\x81c\x82d"[..],
            // truncated multibyte lead at end of file
            &b"tail\xe2\x80"[..],
            // valid UTF-8 either side of a lone byte
            "日本\u{0}語".as_bytes(),
        ] {
            let decoded = decode_script_bytes(raw);
            assert_eq!(unmetafy_str(&decoded), raw, "round trip for {raw:?}");
        }
    }

    #[test]
    fn one_bad_byte_does_not_lose_the_rest() {
        let decoded = decode_script_bytes(b"echo before\necho caf\xe9\necho after\n");
        assert!(decoded.starts_with("echo before\n"));
        assert!(decoded.ends_with("echo after\n"));
        assert!(!decoded.contains('\u{FFFD}'), "no lossy replacement");
    }
}
