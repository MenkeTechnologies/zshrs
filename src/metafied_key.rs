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
use std::cmp::Ordering;

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

/// A `zstrcmp` operand prepared once for repeated comparison: the
/// [`metafied_key`] bytes plus a NUL terminator, and whether they hold a
/// backslash. This is what C's comparators already have in hand — a
/// NUL-terminated `char *` — and a sort comparator calls `zstrcmp` O(n log n)
/// times per sort, so preparing it per comparison (an ASCII scan, a NUL scan,
/// a backslash scan and a terminating copy for each operand) dominated large
/// completions. Build one per element before the sort and compare with
/// [`zstrcmp_operands`].
pub struct MetafiedOperand {
    /// Metafied bytes followed by one NUL. Metafication escapes NUL, so the
    /// terminator is the only NUL in the buffer.
    bytes: Vec<u8>,
    /// `bytes` contains a `\` — the only case where
    /// `SORTIT_IGNORING_BACKSLASHES` can change `zstrcmp`'s answer.
    backslash: bool,
}

impl MetafiedOperand {
    pub fn new(s: &str) -> Self {
        let mut bytes = metafied_key(s).into_owned();
        let backslash = bytes.contains(&b'\\');
        bytes.push(0);
        MetafiedOperand { bytes, backslash }
    }

    /// The metafied bytes without the terminator.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.bytes.len() - 1]
    }
}

/// `zstrcmp(a, b, flags)` (`Src/sort.c:191`) on two prepared operands.
///
/// When the sort is not numeric and neither operand holds a backslash,
/// `zstrcmp`'s backslash skip loop (c:120-131) cannot change the outcome and
/// the result is the bare c:134 `strcoll(as, bs)` on the whole strings —
/// the same shortcut `crate::ported::sort::zstrcmp` takes after scanning
/// for `\`. The operands are already NUL-terminated, so that call is made
/// here directly. Every other case goes through `zstrcmp` itself, so the
/// backslash and numeric rules have one implementation.
pub fn zstrcmp_operands(a: &MetafiedOperand, b: &MetafiedOperand, flags: u32) -> Ordering {
    #[cfg(unix)]
    {
        let numeric = (crate::ported::zsh_h::SORTIT_NUMERICALLY
            | crate::ported::zsh_h::SORTIT_NUMERICALLY_SIGNED) as u32;
        if flags & numeric == 0 && !a.backslash && !b.backslash {
            // c:134 — `cmp = strcoll(as, bs)`.
            let c = unsafe {
                libc::strcoll(
                    a.bytes.as_ptr() as *const libc::c_char,
                    b.bytes.as_ptr() as *const libc::c_char,
                )
            };
            return c.cmp(&0);
        }
    }
    crate::ported::sort::zstrcmp(a.as_bytes(), b.as_bytes(), flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zstrcmp_operands` short-cuts straight to `strcoll` when neither
    /// operand has a backslash and the sort is not numeric. That shortcut
    /// must give exactly `zstrcmp`'s answer on the same metafied bytes for
    /// every flag combination `matchcmp` uses — including the operands that
    /// DO take the slow path (backslashes, digit runs, non-ASCII).
    #[test]
    fn operands_agree_with_zstrcmp_on_every_matchcmp_flag_set() {
        use crate::ported::zsh_h::{SORTIT_IGNORING_BACKSLASHES, SORTIT_NUMERICALLY};
        let words = [
            "", "a", "A", "alpha.txt", "README.md", "file10", "file9", "file09",
            "x\\ y", "x y", "\\EurydiceCM\\", "Zotero", "dot", "日本語", "中文字",
            "ascii", "a\\\\b", "a\\b", "-5", "5", "αβ", "zz",
        ];
        let base = SORTIT_IGNORING_BACKSLASHES as u32;
        for flags in [base, base | SORTIT_NUMERICALLY as u32] {
            for a in words {
                for b in words {
                    let want = crate::ported::sort::zstrcmp(
                        metafied_key(a),
                        metafied_key(b),
                        flags,
                    );
                    let got = zstrcmp_operands(
                        &MetafiedOperand::new(a),
                        &MetafiedOperand::new(b),
                        flags,
                    );
                    assert_eq!(got, want, "{a:?} vs {b:?} flags={flags}");
                }
            }
        }
    }

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
