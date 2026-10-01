//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! Where zshrs keeps zsh's lexer tokens in a `char` string.
//!
//! C's tokens are the BYTES 0x84..=0xa2 (`Pound` .. `Marker`,
//! c:Src/zsh.h:159-224). C can use those byte values because every string
//! that reaches the token-aware code is METAFIED: a data byte in that
//! range is stored as `Meta` + (byte ^ 32) (c:Src/utils.c:4856), so data
//! never equals a token.
//!
//! zshrs stores text as Rust `char`s and does not metafy values, so it
//! cannot put tokens at U+0084..=U+00A2: those are real characters — NBSP
//! (U+00A0), `¡`, `¢`, NEL and the C1 controls — and a value containing
//! one was read as a token by every untokenize / quote / pattern path
//! (`${(qq)v}` on an NBSP printed `'\'`). The tokens therefore live in the
//! Private Use Area at [`TOKEN_BASE`] + the C byte: the low byte still IS
//! the C token byte, their order is C's, and no real text collides.
//!
//! The helpers below are for the code that crosses between the two forms:
//! byte buffers (wordcode, `.zwc`, metafied strings) carry the C byte; a
//! `char` string carries the PUA scalar.

/// The Private Use Area offset added to a C token byte.
pub const TOKEN_BASE: u32 = 0xe000;

/// First and last C token bytes (`Pound`, `Marker`; c:Src/zsh.h:159, :224).
pub const FIRST_TOKEN_BYTE: u8 = 0x84;
/// See [`FIRST_TOKEN_BYTE`].
pub const LAST_TOKEN_BYTE: u8 = 0xa2;

/// Is `c` one of zshrs's token chars (the PUA image of a C token byte)?
#[inline]
pub fn is_token_char(c: char) -> bool {
    let u = c as u32;
    (TOKEN_BASE + FIRST_TOKEN_BYTE as u32..=TOKEN_BASE + LAST_TOKEN_BYTE as u32).contains(&u)
}

/// The token char for C token byte `b`, or `None` when `b` is not a token
/// byte.
#[inline]
pub fn token_char_from_byte(b: u8) -> Option<char> {
    (FIRST_TOKEN_BYTE..=LAST_TOKEN_BYTE)
        .contains(&b)
        .then(|| char::from_u32(TOKEN_BASE + b as u32).expect("PUA scalar"))
}

/// The C token byte a token char stands for, or `None` for any other char.
#[inline]
pub fn token_byte(c: char) -> Option<u8> {
    is_token_char(c).then(|| (c as u32 - TOKEN_BASE) as u8)
}

/// C's `itok()` (c:Src/ztype.h:52) for a token char: the typtab ITOK bit of
/// the C byte it stands for (Pound ..= Nularg; Marker is IMETA only). Any
/// other char, including real U+0084..=U+00A2 text, is not a token.
#[inline]
pub fn itok_char(c: char) -> bool {
    token_byte(c).is_some_and(crate::ported::ztype_h::itok)
}

/// C's `untokenize()` (c:Src/exec.c:2077-2099) over a byte buffer that may
/// hold raw (metafied, possibly non-UTF-8) bytes: each token char's UTF-8
/// form (`EE 82 xx`) becomes its `ztokens` glyph, `Nularg` is dropped, and
/// every other byte is kept as is. A plain byte walk for C's 0x84..=0xa1
/// would instead hit the continuation bytes of real text (the 0x97 of `日`).
pub fn untokenize_bytes(b: &[u8]) -> Vec<u8> {
    let ztokens = crate::ported::glob::ZTOKENS.as_bytes();
    let nularg = token_byte(crate::ported::zsh_h::Nularg).expect("Nularg is a token");
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if let [0xee, 0x82, t, ..] = b[i..] {
            if crate::ported::ztype_h::itok(t) {
                if t != nularg {
                    out.push(ztokens[(t - FIRST_TOKEN_BYTE) as usize]);
                }
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_chars_round_trip_and_real_text_is_not_a_token() {
        for b in FIRST_TOKEN_BYTE..=LAST_TOKEN_BYTE {
            let c = token_char_from_byte(b).unwrap();
            assert!(is_token_char(c));
            assert_eq!(token_byte(c), Some(b));
        }
        for c in ['\u{a0}', '\u{a1}', '\u{a2}', '\u{85}', 'a', '$', '\u{e083}', '\u{e0a3}'] {
            assert!(!is_token_char(c), "{c:?}");
        }
        assert_eq!(token_char_from_byte(0x83), None);
        assert_eq!(token_char_from_byte(0xa3), None);
    }

    #[test]
    fn constants_are_the_pua_images_of_the_c_bytes() {
        use crate::ported::zsh_h::*;
        assert_eq!(token_byte(Pound), Some(0x84));
        assert_eq!(token_byte(Stringg), Some(0x85));
        assert_eq!(token_byte(Bang), Some(0x9c));
        assert_eq!(token_byte(Bnullkeep), Some(0xa0));
        assert_eq!(token_byte(Nularg), Some(0xa1));
        assert_eq!(token_byte(Marker), Some(0xa2));
    }
}
