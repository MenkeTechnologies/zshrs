//! !!! WARNING: RUST-ONLY MODULE — NO C COUNTERPART !!!
//!
//! The METAFIED byte spelling of a shell string, for the `zstrcmp`
//! call sites whose C operands are metafied.
//!
//! C's `zstrcmp` (`Src/sort.c:191`) collates whatever bytes its caller
//! hands it, and the callers disagree on the form:
//!
//! | call site                                   | operand     |
//! |---------------------------------------------|-------------|
//! | `strmetasort` `Src/sort.c:299-315`          | unmetafied  |
//! | `gmatchcmp` GS_NAME `Src/glob.c:945`        | unmetafied (`uname`, c:1963-1973) |
//! | `cd_sort` `Src/Zle/computil.c:235`          | unmetafied (`sortstr`, c:301-302) |
//! | `gmatchcmp` GS_EXEC `Src/glob.c:981`        | METAFIED (`getsparam("REPLY")`, c:1938-1941) |
//! | `matchcmp` `Src/Zle/compcore.c:3194`        | METAFIED (`Cmatch->str` / `->disp`) |
//!
//! Metafication rewrites `{0x00} ∪ [0x83, 0xa2]` as `Meta`, `b ^ 32`
//! (`Src/utils.c:4195-4201`, `metafy` c:4856). Those bytes sit inside
//! UTF-8 continuation ranges, so for most non-ASCII text the metafied
//! operand is not valid multibyte and `strcoll` collates it byte-wise.
//! zshrs keeps strings unmetafied, so the two metafied call sites have to
//! rebuild C's bytes before comparing or a UTF-8 locale reorders them.
//!
//! The ported `utils::metafy` returns a `String` and is lossy on exactly
//! these inputs (a metafied `日` is `e6 83 b7 a5`), and `src/ported/` may
//! not gain functions, which is why this lives here.

use std::borrow::Cow;

/// The bytes C would hold for `s` after `metafy()` (`Src/utils.c:4856`).
///
/// `s` is first taken back to its raw bytes with `unmetafy_str`, because a
/// zshrs `String` carries a raw non-UTF-8 byte as the char pair
/// `U+0083`, `U+00(b ^ 32)`. Borrows when `s` is ASCII: metafication is
/// the identity there, and this runs inside sort comparators.
pub fn metafied_key(s: &str) -> Cow<'_, [u8]> {
    if s.is_ascii() && !s.as_bytes().contains(&0) {
        return Cow::Borrowed(s.as_bytes());
    }
    let raw = crate::ported::utils::unmetafy_str(s);
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 2);
    for b in raw {
        // c:Src/utils.c:4880-4884
        if crate::ported::utils::imeta_byte(b) {
            out.push(crate::ported::zsh_h::Meta);
            out.push(b ^ 32);
        } else {
            out.push(b);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_borrowed_unchanged() {
        assert!(matches!(metafied_key("alpha.txt"), Cow::Borrowed(b"alpha.txt")));
    }

    #[test]
    fn continuation_bytes_in_the_imeta_range_are_escaped() {
        // 日 = e6 97 a5: only 0x97 is in [0x83, 0xa2].
        assert_eq!(&*metafied_key("日"), &[0xe6, 0x83, 0x97 ^ 32, 0xa5][..]);
        // α = ce b1: nothing to escape.
        assert_eq!(&*metafied_key("α"), &[0xce, 0xb1][..]);
    }

    #[test]
    fn a_meta_encoded_raw_byte_is_metafied_as_that_byte() {
        // zshrs spells the raw byte 0x90 as U+0083 U+00B0 (0x90 ^ 32).
        let s: String = ['a', '\u{83}', char::from(0x90u8 ^ 32)].iter().collect();
        assert_eq!(&*metafied_key(&s), &[b'a', 0x83, 0x90 ^ 32][..]);
    }
}
