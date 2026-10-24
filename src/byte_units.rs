//! !!! WARNING: RUST-ONLY MODULE — NO C COUNTERPART !!!
//!
//! The `unsetopt multibyte` spelling of a shell string for the pattern
//! matcher and the `${…}` match walkers.
//!
//! Without MULTIBYTE, zsh matches and substitutes in raw BYTES: the matcher
//! steps one byte (`charnext`, c:Src/pattern.c:1946) and the walkers step one
//! byte (`MB_METACHARINC`, c:Src/glob.c:2958 and on), so `x=é; ${x//?/.}` is
//! `..` and `${x/[[:alpha:]]/X}` can replace half of a character. C gets that
//! for free because its strings ARE bytes.
//!
//! zshrs strings are UTF-8, so a byte inside a character is not a place a
//! `&str` can be cut. zshrs already spells a byte that is not valid UTF-8 as
//! the char pair `Meta` (U+0083) + `byte ^ 32` (`script_bytes`,
//! `utils::unmetafy_str`), and the matcher reads such a pair as ONE raw byte
//! in either mode (`pattern.rs` `charinc` / `charrefinc`). The byte spelling
//! below writes EVERY non-ASCII byte that way, in the pattern and in the
//! subject alike, so each byte is one unit, every unit boundary is a char
//! boundary, and literal bytes compare equal on both sides. [`canonical`]
//! turns a result back into the ordinary form (valid UTF-8 as chars, any
//! other byte as a pair), which is what C's `metafy` of the result bytes
//! amounts to here.

use std::borrow::Cow;

/// `true` when the MULTIBYTE option is unset, i.e. a match unit is a byte.
///
/// Read from the live slot with its declared default (on,
/// c:Src/options.c:197), as `utils::mb_metacharlenconv` does.
pub fn byte_mode() -> bool {
    !crate::ported::options::opt_state_get("multibyte").unwrap_or(true)
}

/// Spell every non-ASCII byte of `s` as a `Meta` pair. ASCII, existing
/// pairs and token chars (the pattern compiler's PUA tokens) are kept.
pub fn byte_spelling(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(2 * s.len());
    let mut it = s.chars().peekable();
    let mut buf = [0u8; 4];
    while let Some(c) = it.next() {
        if c.is_ascii() || crate::token_char::is_token_char(c) {
            out.push(c);
        } else if c == '\u{83}' && it.peek().is_some_and(|&n| (n as u32) >= 0x80) {
            // Already one raw byte (c:Src/utils.c:4862-4863).
            out.push(c);
            out.push(it.next().unwrap());
        } else {
            for &b in c.encode_utf8(&mut buf).as_bytes() {
                out.push('\u{83}'); // Meta, c:Src/zsh.h:144
                out.push(char::from(b ^ 32));
            }
        }
    }
    Cow::Owned(out)
}

/// Byte offsets of every unit boundary of `s`, plus `s.len()`. A unit is a
/// character, except that a `Meta` pair is one unit (one raw byte).
pub fn unit_bounds(s: &str) -> Vec<usize> {
    let mut v = Vec::with_capacity(s.len() + 1);
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        v.push(i);
        if c == '\u{83}' && it.peek().is_some_and(|&(_, n)| (n as u32) >= 0x80) {
            it.next();
        }
    }
    v.push(s.len());
    v
}

/// The ordinary zshrs form of the bytes `s` spells: valid UTF-8 as chars,
/// anything else as `Meta` pairs.
pub fn canonical(s: &str) -> String {
    crate::script_bytes::decode_script_bytes(&crate::ported::utils::unmetafy_str(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_spelling_makes_one_unit_per_byte() {
        let s = byte_spelling("aé漢");
        // 1 ASCII + 2 + 3 bytes.
        assert_eq!(unit_bounds(&s).len() - 1, 6);
        assert_eq!(crate::ported::utils::unmetafy_str(&s), "aé漢".as_bytes());
        assert_eq!(canonical(&s), "aé漢");
    }

    #[test]
    fn canonical_keeps_a_cut_character_as_raw_bytes() {
        let s = byte_spelling("é");
        let bounds = unit_bounds(&s);
        let half = &s[..bounds[1]];
        assert_eq!(crate::ported::utils::unmetafy_str(&canonical(half)), [0xc3]);
        // The two halves put back together are the character again.
        assert_eq!(canonical(&format!("{}{}", half, &s[bounds[1]..])), "é");
    }

    #[test]
    fn byte_spelling_keeps_tokens_and_existing_pairs() {
        let star = crate::ported::zsh_h::Star;
        let pair = "\u{83}\u{df}"; // raw byte 0xff
        let s = format!("{star}{pair}");
        assert_eq!(byte_spelling(&s), s);
    }
}
