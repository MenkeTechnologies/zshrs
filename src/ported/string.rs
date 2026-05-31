//! String manipulation utilities for zshrs
//!
//! Direct port of `Src/string.c` (201 lines, 11 ported).
//!
//! Duplicate string on heap when length is known                            // c:44
//! Append a string to an allocated string, reallocating to make room.      // c:182
//!
//! C zsh distinguishes two allocation lanes — `zalloc` (permanent
//! storage, freed by `zsfree`) and `zhalloc` (heap-arena, bulk-
//! freed at the end of the current dispatch). Rust's `String` always
//! owns its allocation and `Drop`s when it falls out of scope, so the
//! two lanes collapse into one. The function names below are kept
//! verbatim for caller-side parity with the C source — passing
//! through to a single owned `String` regardless of whether C would
//! have used zalloc or zhalloc.
//!
//! Byte-faithfulness: C's `memcpy(r, s, len)` copies bytes without
//! regard for UTF-8 boundaries. The Rust ports use `as_bytes` slicing
//! plus `from_utf8_lossy` so a `len` that lands mid-codepoint doesn't
//! panic — matching the C behavior of producing a possibly-truncated
//! byte string.

/// Port of `dupstring(const char *s)` from `Src/string.c:33`.
///
/// C body:
/// ```c
/// if (!s) return NULL;
/// t = (char *) zhalloc(strlen(s) + 1);
/// strcpy(t, s);
/// return t;
/// ```
///
/// Heap-arena duplicate. Rust takes `&str` (NULL is impossible);
/// the heap-arena lane collapses to a regular `String`.
pub fn dupstring(s: &str) -> String {
    // c:33
    s.to_string()
}

/// Port of `dupstring_wlen(const char *s, unsigned len)` from `Src/string.c:48`.
///
/// C body:
/// ```c
/// if (!s) return NULL;
/// t = (char *) zhalloc(len + 1);
/// memcpy(t, s, len);
/// t[len] = '\0';
/// return t;
/// ```
///
/// Byte-counted heap-arena duplicate. The previous Rust port did
/// `s[..len.min(s.len())]` which panics if `len` lands on a non-
/// UTF-8 boundary. C just `memcpy`s the bytes; this port matches
/// that semantic via `as_bytes` slicing + `from_utf8_lossy`.
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    // c:48
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

/// Port of `ztrdup(const char *s)` from `Src/string.c:62`.
///
/// C body:
/// ```c
/// if (!s) return NULL;
/// t = (char *) zalloc(strlen(s) + 1);
/// strcpy(t, s);
/// return t;
/// ```
///
/// Permanent-storage duplicate (C's strdup analog). Rust collapses
/// to `to_string()` since there's no per-allocation lane choice.
pub fn ztrdup(s: &str) -> String {
    // c:62
    s.to_string()
}

/// Port of `wcs_ztrdup(const wchar_t *s)` from `Src/string.c:77`.
///
/// C body (under `#ifdef MULTIBYTE_SUPPORT`):
/// ```c
/// if (!s) return NULL;
/// t = (wchar_t *) zalloc(sizeof(wchar_t) * (wcslen(s) + 1));
/// wcscpy(t, s);
/// return t;
/// ```
///
/// Wide-char duplicate. Rust `String` is UTF-8 which subsumes the
/// wchar_t representation; the conversion is identity.
pub fn wcs_ztrdup(s: &str) -> String {
    // c:77
    s.to_string()
}

/// Port of `tricat(char const *s1, char const *s2, char const *s3)` from `Src/string.c:98`.
///
/// C body uses three `strcpy` calls into a `zalloc(l1+l2+l3+1)`
/// buffer. Rust port pre-sizes the `String` to avoid reallocation
/// and pushes the three slices in order.
///
// To concatenate four or more strings, see zjoin().                       // c:98
/// "Permanent" allocation lane in C; Rust's `String` is always
/// owned so the lane choice is irrelevant.
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    // c:98
    let mut result = String::with_capacity(s1.len() + s2.len() + s3.len());
    result.push_str(s1);
    result.push_str(s2);
    result.push_str(s3);
    result
}

/// Port of `zhtricat(char const *s1, char const *s2, char const *s3)` from `Src/string.c:114`.
///
/// Heap-arena variant of [`tricat`] in C. Same Rust impl since
/// the lanes collapse.
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {
    // c:114
    tricat(s1, s2, s3)
}

/// Port of `dyncat(const char *s1, const char *s2)` from `Src/string.c:131`.
///
/// C body:
/// ```c
/// ptr = (char *) zhalloc(l1 + strlen(s2) + 1);
/// strcpy(ptr, s1);
/// strcpy(ptr + l1, s2);
/// return ptr;
/// ```
///
// concatenate s1 and s2 in dynamically allocated buffer                    // c:131
/// Heap-arena two-string concat.
pub fn dyncat(s1: &str, s2: &str) -> String {
    // c:131
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Port of `bicat(const char *s1, const char *s2)` from `Src/string.c:145`.
///
/// Same shape as [`dyncat`], but C uses the permanent-storage
/// `zalloc` lane. Rust port: identical body.
pub fn bicat(s1: &str, s2: &str) -> String {
    // c:145
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Port of `dupstrpfx(const char *s, int len)` from `Src/string.c:161`.
///
/// C body:
/// ```c
/// char *r = zhalloc(len + 1);
/// memcpy(r, s, len);
/// r[len] = '\0';
/// return r;
/// ```
///
// like dupstring(), but with a specified length                             // c:161
/// Byte-counted prefix copy. The previous Rust port used
/// `s[..len]` which panics on non-UTF-8 boundary; this port
/// matches C's `memcpy` semantics via byte slicing.
pub fn dupstrpfx(s: &str, len: usize) -> String {
    // c:161
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

/// Port of `ztrduppfx(const char *s, int len)` from `Src/string.c:172`.
///
/// Same body as [`dupstrpfx`], but C uses the permanent-storage
/// lane. Lanes collapse in Rust.
pub fn ztrduppfx(s: &str, len: usize) -> String {
    dupstrpfx(s, len)
}

/// Port of `appstr(char *base, char const *append)` from `Src/string.c:186`.
///
/// C body:
/// ```c
/// return strcat(realloc(base, strlen(base) + strlen(append) + 1),
///               append);
/// ```
///
/// C reallocates `base` (which may move) and returns the new
/// pointer. Rust's `&mut String` mutates in place; the equivalent
/// of C's "return the new pointer" is "the caller's reference is
/// still valid after the push" — `String::push_str` reallocates
/// transparently if needed.
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Port of `strend(char *str)` from `Src/string.c:196`.
///
/// C body:
/// ```c
/// if (*str == '\0') return str;
/// return str + strlen(str) - 1;
/// ```
///
/// C returns a pointer into the input — to the last character if
/// the string is non-empty, or to the NUL byte (i.e. the start)
/// if empty. Rust port returns the trailing byte slice for the
/// closest pointer-shape parity:
/// - Empty input → empty `&str` (the "`*str == '\\0'`" branch).
/// - Non-empty input → the trailing UTF-8 character as a `&str`
///   slice.
pub fn strend(str: &str) -> &str {
    if str.is_empty() {
        return str;
    }
    let bytes = str.as_bytes();
    // Walk back to the start of the last UTF-8 codepoint.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] & 0xC0 != 0x80 {
            // Codepoint boundary (not a continuation byte).
            return &str[i..];
        }
    }
    str
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dupstring() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstring("hello"), "hello");
        assert_eq!(dupstring(""), "");
    }

    #[test]
    fn test_dupstring_wlen() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstring_wlen("hello world", 5), "hello");
        // len longer than string is clamped (matches Rust `min` —
        // C would walk past the NUL which is UB; the safe analog
        // here is to return the whole string).
        assert_eq!(dupstring_wlen("hi", 50), "hi");
        // len of 0 returns empty.
        assert_eq!(dupstring_wlen("hello", 0), "");
    }

    #[test]
    fn test_dupstring_wlen_byte_safe_at_codepoint_boundary() {
        let _g = crate::test_util::global_state_lock();
        // C: `memcpy(t, s, len)` copies bytes regardless of UTF-8
        // boundary. The previous Rust port panicked on
        // `s[..len.min(s.len())]` if `len` landed mid-codepoint.
        // Use a 2-byte UTF-8 character: 'é' is 0xC3 0xA9.
        let s = "café";
        // bytes: c, a, f, 0xC3, 0xA9
        // len=4 lands inside the 'é' — must not panic.
        let r = dupstring_wlen(s, 4);
        // Replacement char produced by from_utf8_lossy on the
        // truncated 0xC3 byte.
        assert!(r.starts_with("caf"));
    }

    #[test]
    fn test_ztrdup() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrdup("permanent"), "permanent");
    }

    #[test]
    fn test_wcs_ztrdup() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcs_ztrdup("ünicode"), "ünicode");
    }

    #[test]
    fn test_tricat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("", "", ""), "");
        assert_eq!(tricat("foo", "", "bar"), "foobar");
    }

    #[test]
    fn test_zhtricat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zhtricat("x", "y", "z"), "xyz");
    }

    #[test]
    fn test_bicat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(bicat("hello", " world"), "hello world");
        assert_eq!(bicat("", ""), "");
    }

    #[test]
    fn test_dyncat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyncat("foo", "bar"), "foobar");
    }

    #[test]
    fn test_appstr() {
        let _g = crate::test_util::global_state_lock();
        let mut s = "hello".to_string();
        appstr(&mut s, " world");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_dupstrpfx() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstrpfx("hello world", 5), "hello");
        assert_eq!(dupstrpfx("hi", 50), "hi");
        assert_eq!(dupstrpfx("hi", 0), "");
    }

    #[test]
    fn test_dupstrpfx_byte_safe() {
        let _g = crate::test_util::global_state_lock();
        // 'é' = 0xC3 0xA9. len=1 inside it must not panic.
        let _ = dupstrpfx("é", 1);
    }

    #[test]
    fn test_ztrduppfx() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrduppfx("hello", 3), "hel");
    }

    #[test]
    fn test_strend_returns_last_codepoint() {
        let _g = crate::test_util::global_state_lock();
        // C returns pointer to last char (or to NUL on empty).
        // Rust returns the trailing &str slice for pointer-shape parity.
        assert_eq!(strend("hello"), "o");
        assert_eq!(strend(""), "");
        // Multibyte: 'é' is 2 bytes; strend returns the whole codepoint.
        assert_eq!(strend("café"), "é");
        // Single ASCII char.
        assert_eq!(strend("a"), "a");
    }

    /// c:98 — `tricat(s1,s2,s3)` is the canonical 3-string concat used
    /// everywhere zsh builds `${prefix}${name}${suffix}`. Regression
    /// dropping any segment silently corrupts every param-subst path.
    #[test]
    fn tricat_concatenates_three_segments_in_order() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("", "b", "c"), "bc");
        assert_eq!(tricat("a", "", "c"), "ac");
        assert_eq!(tricat("a", "b", ""), "ab");
    }

    /// c:131 — `dyncat(s1,s2)` is the 2-string concat counterpart.
    #[test]
    fn dyncat_concatenates_two_segments() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyncat("hello", " world"), "hello world");
        assert_eq!(dyncat("", "x"), "x");
    }

    /// c:62 — `ztrdup` is the owning copy. Verifies the duplicate
    /// is independent of the source after a mutating clear.
    #[test]
    fn ztrdup_returns_independent_owned_copy() {
        let _g = crate::test_util::global_state_lock();
        let mut src = String::from("original");
        let dup = ztrdup(&src);
        src.clear();
        assert_eq!(dup, "original", "dup must survive source-side clear");
    }

    /// c:161 — `dupstrpfx(s, len)` returns first `len` bytes; len > s.len()
    /// must NOT panic — returns whole string. Critical for any
    /// truncation path that doesn't pre-clamp.
    #[test]
    fn dupstrpfx_handles_len_larger_than_input() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstrpfx("ab", 100), "ab");
        assert_eq!(dupstrpfx("hello", 0), "");
        assert_eq!(dupstrpfx("hello", 3), "hel");
    }

    /// c:131 — `dyncat` with both empty inputs returns empty (no
    /// phantom delimiters).
    #[test]
    fn dyncat_empty_inputs_return_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dyncat("", ""), "");
    }

    /// `Src/string.c:144-155` — `bicat(s1, s2)` is the
    /// permanent-storage variant of `dyncat`. C body computes
    /// `zalloc(strlen(s1)+strlen(s2)+1)` then `strcpy(ptr, s1)` and
    /// `strcpy(ptr+l1, s2)`. Two-segment concat, never reorders.
    #[test]
    fn bicat_concatenates_in_order_with_either_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(bicat("foo", "bar"), "foobar");
        assert_eq!(
            bicat("", "bar"),
            "bar",
            "c:152 — strcpy(ptr, \"\") writes only the NUL, ptr+0 starts s2"
        );
        assert_eq!(
            bicat("foo", ""),
            "foo",
            "c:153 — strcpy(ptr+3, \"\") writes only the NUL"
        );
        assert_eq!(bicat("", ""), "");
    }

    /// `Src/string.c:172-178` — `ztrduppfx(s, len)` body is identical
    /// to `dupstrpfx` (same `memcpy`/NUL pattern at c:175-177); only
    /// the allocator differs (`zalloc` vs `zhalloc`). Both lanes
    /// collapse to `String` in the Rust port. Behaviour parity with
    /// `dupstrpfx` is the contract — a regression that diverged the
    /// two would silently leak storage-lane assumptions into callers.
    #[test]
    fn ztrduppfx_matches_dupstrpfx_byte_for_byte() {
        let _g = crate::test_util::global_state_lock();
        for (s, len) in [("hello", 3usize), ("ab", 100), ("hello", 0), ("", 5)] {
            assert_eq!(
                ztrduppfx(s, len),
                dupstrpfx(s, len),
                "ztrduppfx/dupstrpfx divergence at ({:?}, {})",
                s,
                len
            );
        }
    }

    /// `Src/string.c:186-189` — `appstr(base, append)` C body is
    /// `strcat(realloc(base, strlen(base)+strlen(append)+1), append)`.
    /// Append-in-place semantics: post-condition is `base == base ++ append`.
    /// Empty append → base unchanged. Empty base → result equals append.
    #[test]
    fn appstr_appends_in_place() {
        let _g = crate::test_util::global_state_lock();
        let mut b = String::from("foo");
        appstr(&mut b, "bar");
        assert_eq!(b, "foobar");
        // c:188 — strcat with empty s2 leaves base unchanged.
        appstr(&mut b, "");
        assert_eq!(b, "foobar", "appending empty must leave base unchanged");
        // Empty base + nonempty append.
        let mut e = String::new();
        appstr(&mut e, "xyz");
        assert_eq!(e, "xyz");
    }

    /// `Src/string.c:195-201` — `strend(str)`. C body:
    /// `if (*str == '\0') return str; return str + strlen(str) - 1;`.
    /// Single-char input → that char (no underflow on `len-1`).
    /// Multi-char input → last char only.
    #[test]
    fn strend_returns_only_last_character_for_multichar_input() {
        let _g = crate::test_util::global_state_lock();
        // c:200 — `str + strlen(str) - 1` for "hello" (len=5) → 'o'.
        assert_eq!(strend("hello"), "o");
        // c:200 — len=2 → 'b'.
        assert_eq!(strend("ab"), "b");
        // c:198 — empty input falls through `*str == '\0'` branch and
        // returns the empty string (the pointer-to-NUL in C).
        assert_eq!(strend(""), "");
    }

    /// `Src/string.c:32-42` — `dupstring(s)`. C body:
    /// `if (!s) return NULL; t = zhalloc(strlen(s)+1); strcpy(t,s); return t;`.
    /// Empty string round-trips (no underflow on len=0).
    #[test]
    fn dupstring_returns_owned_copy_with_identity_content() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstring("hello"), "hello");
        assert_eq!(
            dupstring(""),
            "",
            "c:39 — empty input → len 0+1, strcpy copies NUL"
        );
        // Non-ASCII (UTF-8) round-trips byte-identical.
        assert_eq!(dupstring("café"), "café");
        assert_eq!(dupstring("字"), "字");
    }

    /// `Src/string.c:47-58` — `dupstring_wlen(s, len)`. C body:
    /// `memcpy(t, s, len); t[len] = '\\0';`. Byte-counted copy — len
    /// can be less than, equal to, or greater than `strlen(s)`. The
    /// Rust port via `as_bytes()` slicing must match `memcpy`
    /// semantics, including the `len > s.len()` case which clamps
    /// (C would read past the buffer — UB; Rust port clamps to
    /// avoid panic per the impl note at c:50).
    #[test]
    fn dupstring_wlen_respects_byte_length_and_clamps_overflow() {
        let _g = crate::test_util::global_state_lock();
        // c:55 — memcpy(t, s, len) for len < strlen.
        assert_eq!(dupstring_wlen("hello world", 5), "hello");
        // len == 0 → empty.
        assert_eq!(dupstring_wlen("hello", 0), "");
        // Clamp: Rust port returns whole string rather than reading
        // past the buffer (C would have been UB).
        assert_eq!(dupstring_wlen("ab", 100), "ab");
        // Exact-length boundary.
        assert_eq!(dupstring_wlen("foo", 3), "foo");
    }

    /// `Src/string.c:76-85` — `wcs_ztrdup(const wchar_t *s)`. C body
    /// is the wide-char version of `ztrdup`: copies the wchar_t string
    /// into a zalloc'd buffer. Rust UTF-8 `String` subsumes the
    /// wchar_t representation — identity copy.
    #[test]
    fn wcs_ztrdup_returns_independent_copy() {
        let _g = crate::test_util::global_state_lock();
        let mut src = String::from("widechar");
        let dup = wcs_ztrdup(&src);
        src.clear();
        assert_eq!(
            dup, "widechar",
            "wide-char dup must survive source-side mutation"
        );
        // Non-ASCII paths.
        assert_eq!(wcs_ztrdup("éàü字"), "éàü字");
    }

    /// `Src/string.c:113-128` — `zhtricat(s1, s2, s3)`. C body uses
    /// heap-arena allocator (zhalloc) instead of permanent zalloc.
    /// Both lanes collapse to `String` in Rust; behaviour must match
    /// tricat exactly. Pin parity with tricat for the same three
    /// inputs — a regression diverging the two would silently change
    /// memory ownership in C but produce wrong content if anything
    /// changed at the byte level.
    #[test]
    fn zhtricat_matches_tricat_byte_for_byte() {
        let _g = crate::test_util::global_state_lock();
        for (a, b, c) in [
            ("foo", "bar", "baz"),
            ("", "x", ""),
            ("a", "", "z"),
            ("", "", ""),
        ] {
            assert_eq!(
                zhtricat(a, b, c),
                tricat(a, b, c),
                "lane divergence at ({:?}, {:?}, {:?})",
                a,
                b,
                c
            );
        }
    }

    /// `Src/string.c:171-181` — `ztrduppfx(s, len)` is `dupstrpfx`
    /// with permanent storage. We already pinned the body-identical
    /// contract above; this test pins behaviour for `len > strlen`
    /// specifically (the C source would `memcpy` past the source
    /// buffer — UB; the Rust port clamps).
    #[test]
    fn ztrduppfx_clamps_oversize_len_safely() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrduppfx("hi", 100), "hi");
        assert_eq!(ztrduppfx("", 5), "");
        assert_eq!(ztrduppfx("abc", 2), "ab");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Concatenation helpers — `tricat`, `bicat`, `dyncat`, `dupstring`,
    // `appstr`. Each pinned against direct string equality. Empty-arg
    // edge cases pinned explicitly so a regression that drops empties
    // surfaces immediately.
    // ═══════════════════════════════════════════════════════════════════

    // ── tricat: 3-string concatenation ───────────────────────────────
    /// Plain 3-string concat.
    #[test]
    fn tricat_three_non_empty_strings() {
        assert_eq!(tricat("foo", "bar", "baz"), "foobarbaz");
    }

    /// First arg empty — others stay.
    #[test]
    fn tricat_empty_first_keeps_others() {
        assert_eq!(tricat("", "bar", "baz"), "barbaz");
    }

    /// Middle arg empty.
    #[test]
    fn tricat_empty_middle_keeps_others() {
        assert_eq!(tricat("foo", "", "baz"), "foobaz");
    }

    /// Last arg empty.
    #[test]
    fn tricat_empty_last_keeps_others() {
        assert_eq!(tricat("foo", "bar", ""), "foobar");
    }

    /// All empty → empty.
    #[test]
    fn tricat_all_empty_yields_empty() {
        assert_eq!(tricat("", "", ""), "");
    }

    /// Multi-byte UTF-8 across all three positions.
    #[test]
    fn tricat_multibyte_utf8_concatenates_correctly() {
        assert_eq!(tricat("日", "本", "語"), "日本語");
    }

    // ── bicat: 2-string concatenation ────────────────────────────────
    /// Plain bicat.
    #[test]
    fn bicat_two_non_empty_strings() {
        assert_eq!(bicat("hello", " world"), "hello world");
    }

    /// First empty.
    #[test]
    fn bicat_empty_first_returns_second() {
        assert_eq!(bicat("", "world"), "world");
    }

    /// Second empty.
    #[test]
    fn bicat_empty_second_returns_first() {
        assert_eq!(bicat("hello", ""), "hello");
    }

    /// Both empty.
    #[test]
    fn bicat_both_empty_yields_empty() {
        assert_eq!(bicat("", ""), "");
    }

    // ── dyncat: dynamic concat (same as bicat for our purposes) ─────
    /// dyncat plain.
    #[test]
    fn dyncat_two_strings() {
        assert_eq!(dyncat("abc", "xyz"), "abcxyz");
    }

    /// dyncat with empties.
    #[test]
    fn dyncat_empties() {
        assert_eq!(dyncat("", "x"), "x");
        assert_eq!(dyncat("x", ""), "x");
        assert_eq!(dyncat("", ""), "");
    }

    // ── appstr: in-place append ─────────────────────────────────────
    /// appstr appends to existing buffer.
    #[test]
    fn appstr_appends_to_existing_string() {
        let mut s = String::from("hello");
        appstr(&mut s, " world");
        assert_eq!(s, "hello world");
    }

    /// appstr to empty buffer.
    #[test]
    fn appstr_to_empty_buffer_yields_argument() {
        let mut s = String::new();
        appstr(&mut s, "value");
        assert_eq!(s, "value");
    }

    /// appstr of empty is no-op.
    #[test]
    fn appstr_empty_argument_is_noop() {
        let mut s = String::from("preserved");
        appstr(&mut s, "");
        assert_eq!(s, "preserved");
    }

    /// appstr multiple times accumulates.
    #[test]
    fn appstr_repeated_calls_accumulate() {
        let mut s = String::new();
        appstr(&mut s, "a");
        appstr(&mut s, "b");
        appstr(&mut s, "c");
        assert_eq!(s, "abc");
    }

    // ── strend: slice from last codepoint ───────────────────────────
    // NOTE: zshrs's strend() returns the slice STARTING at the last
    // codepoint, NOT a past-the-end pointer like C's strend.
    // Pin the actual observed contract.

    /// `strend("hello")` returns `"o"` (last codepoint).
    #[test]
    fn strend_returns_slice_starting_at_last_codepoint() {
        assert_eq!(strend("hello"), "o");
    }

    /// `strend("")` returns empty.
    #[test]
    fn strend_empty_input_returns_empty() {
        assert_eq!(strend(""), "");
    }

    /// `strend("a")` returns `"a"` (single char IS the last codepoint).
    #[test]
    fn strend_single_char_returns_self() {
        assert_eq!(strend("a"), "a");
    }

    /// Multi-byte: `strend("日本")` returns just `"本"` (last codepoint
    /// regardless of byte width).
    #[test]
    fn strend_multibyte_returns_last_codepoint_only() {
        assert_eq!(strend("日本"), "本");
    }

    // ── ztrdup / dupstring: identity copy ───────────────────────────
    /// dupstring is value-equal to input.
    #[test]
    fn dupstring_returns_identical_content() {
        assert_eq!(dupstring("hello world"), "hello world");
        assert_eq!(dupstring(""), "");
        assert_eq!(dupstring("日本語"), "日本語");
    }

    /// ztrdup mirrors dupstring for ASCII (and UTF-8 in zshrs's port).
    #[test]
    fn ztrdup_identity_for_ascii_and_utf8() {
        assert_eq!(ztrdup("hello"), "hello");
        assert_eq!(ztrdup(""), "");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/string.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:91 — `wcs_ztrdup` round-trip on ASCII.
    #[test]
    fn wcs_ztrdup_ascii_round_trip() {
        assert_eq!(wcs_ztrdup("hello"), "hello");
        assert_eq!(wcs_ztrdup(""), "");
    }

    /// c:118 — `zhtricat("a", "b", "c")` returns "abc" (3-string concat
    /// via heap arena).
    #[test]
    fn zhtricat_joins_three_strings() {
        assert_eq!(zhtricat("a", "b", "c"), "abc");
        assert_eq!(zhtricat("", "middle", ""), "middle");
        assert_eq!(zhtricat("pre", "", "suf"), "presuf");
    }

    /// c:118 — `zhtricat` matches `tricat` (heap-arena vs permanent
    /// distinction collapses in Rust).
    #[test]
    fn zhtricat_matches_tricat() {
        for (a, b, c) in &[("x", "y", "z"), ("", "", ""), ("foo", "bar", "baz")] {
            assert_eq!(zhtricat(a, b, c), tricat(a, b, c));
        }
    }

    /// c:135/147 — `dyncat` matches `bicat` for any input pair.
    #[test]
    fn dyncat_matches_bicat() {
        for (a, b) in &[("foo", "bar"), ("", ""), ("hello", "")] {
            assert_eq!(dyncat(a, b), bicat(a, b));
        }
    }

    /// c:161 — `dupstrpfx("hello", 3)` returns "hel".
    #[test]
    fn dupstrpfx_takes_byte_prefix() {
        assert_eq!(dupstrpfx("hello", 3), "hel");
        assert_eq!(dupstrpfx("hello", 0), "");
        // Overflow clamps to len.
        assert_eq!(dupstrpfx("hi", 100), "hi");
    }

    /// c:172 — `ztrduppfx` matches `dupstrpfx` (lanes collapse).
    #[test]
    fn ztrduppfx_matches_dupstrpfx() {
        for (s, n) in &[("hello", 3), ("", 0), ("foo", 100)] {
            assert_eq!(ztrduppfx(s, *n), dupstrpfx(s, *n));
        }
    }

    /// c:186 — `appstr` accumulates across multiple calls.
    #[test]
    fn appstr_accumulates_multiple_pushes() {
        let mut s = String::from("a");
        appstr(&mut s, "b");
        appstr(&mut s, "c");
        appstr(&mut s, "d");
        assert_eq!(s, "abcd");
    }

    /// c:186 — `appstr(_, "")` is no-op.
    #[test]
    fn appstr_empty_append_is_noop() {
        let mut s = String::from("hello");
        appstr(&mut s, "");
        assert_eq!(s, "hello");
    }

    /// c:196 — `strend("")` returns empty &str.
    #[test]
    fn strend_empty_returns_empty() {
        assert_eq!(strend(""), "");
    }

    /// c:196 — `strend("a")` returns "a" (single char).
    #[test]
    fn strend_single_char_returns_self_pin() {
        assert_eq!(strend("a"), "a");
    }

    /// c:196 — `strend("abc")` returns "c" (last char).
    #[test]
    fn strend_returns_last_ascii_char() {
        assert_eq!(strend("abc"), "c");
        assert_eq!(strend("hello"), "o");
    }

    /// c:55 — `dupstring_wlen("hello", 100)` clamps to byte length.
    #[test]
    fn dupstring_wlen_overlong_clamps() {
        assert_eq!(dupstring_wlen("hi", 100), "hi");
        assert_eq!(dupstring_wlen("", 100), "");
    }

    /// c:35 — `dupstring` returns OWNED String (caller can mutate
    /// without affecting source).
    #[test]
    fn dupstring_returns_owned_independent_copy() {
        let src = "hello";
        let mut dup = dupstring(src);
        dup.push_str("_mut");
        assert_eq!(src, "hello", "source unchanged");
        assert_eq!(dup, "hello_mut");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/string.c
    // c:35 dupstring / c:74 ztrdup / c:91 wcs_ztrdup / c:105 tricat /
    // c:118 zhtricat / c:135 dyncat / c:147 bicat / c:169 dupstrpfx /
    // c:197 appstr / c:216 strend
    // ═══════════════════════════════════════════════════════════════════

    /// c:35 — `dupstring("")` empty returns empty String.
    #[test]
    fn dupstring_empty_returns_empty_pin() {
        assert_eq!(dupstring(""), "");
    }

    /// c:74 — `ztrdup("")` empty returns empty String.
    #[test]
    fn ztrdup_empty_returns_empty() {
        assert_eq!(ztrdup(""), "");
    }

    /// c:74 — `ztrdup` is pure.
    #[test]
    fn ztrdup_is_pure() {
        for s in ["", "abc", "hello world", "日本"] {
            let first = ztrdup(s);
            for _ in 0..3 {
                assert_eq!(ztrdup(s), first, "ztrdup({:?}) must be pure", s);
            }
        }
    }

    /// c:91 — `wcs_ztrdup("")` empty returns empty String.
    #[test]
    fn wcs_ztrdup_empty_returns_empty() {
        assert_eq!(wcs_ztrdup(""), "");
    }

    /// c:105 — `tricat("", "", "")` all empty returns empty.
    #[test]
    fn tricat_all_empty_returns_empty() {
        assert_eq!(tricat("", "", ""), "");
    }

    /// c:105 — `tricat("a", "b", "c")` concatenates to "abc".
    #[test]
    fn tricat_concatenates_three_parts() {
        assert_eq!(tricat("a", "b", "c"), "abc");
    }

    /// c:135 — `dyncat("", "")` both empty returns empty.
    #[test]
    fn dyncat_both_empty_returns_empty() {
        assert_eq!(dyncat("", ""), "");
    }

    /// c:147 — `bicat("", "")` both empty returns empty.
    #[test]
    fn bicat_both_empty_returns_empty() {
        assert_eq!(bicat("", ""), "");
    }

    /// c:147 — `bicat("a", "b")` concatenates to "ab".
    #[test]
    fn bicat_concatenates_two_parts() {
        assert_eq!(bicat("a", "b"), "ab");
    }

    /// c:169 — `dupstrpfx("", 0)` empty returns empty.
    #[test]
    fn dupstrpfx_empty_zero_len_returns_empty() {
        assert_eq!(dupstrpfx("", 0), "");
    }

    /// c:169 — `dupstrpfx("abc", 0)` zero len returns empty.
    #[test]
    fn dupstrpfx_zero_len_returns_empty() {
        assert_eq!(dupstrpfx("abc", 0), "");
    }

    /// c:216 — `strend("abc")` returns the last character slice.
    #[test]
    fn strend_returns_last_char_slice() {
        let s = "abc";
        let e = strend(s);
        assert_eq!(e.len(), 1, "strend returns 1-char str");
        assert_eq!(e.chars().next(), Some('c'));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/string.c
    // c:35 dupstring / c:74 ztrdup / c:105 tricat / c:135 dyncat
    // c:147 bicat / c:169 dupstrpfx / c:216 strend + determinism pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:35 — `dupstring` returns String (compile-time type pin).
    #[test]
    fn dupstring_returns_string_type() {
        let _: String = dupstring("anything");
    }

    /// c:74 — `ztrdup` returns String (compile-time type pin).
    #[test]
    fn ztrdup_returns_string_type() {
        let _: String = ztrdup("anything");
    }

    /// c:105 — `tricat` returns String (compile-time type pin).
    #[test]
    fn tricat_returns_string_type() {
        let _: String = tricat("a", "b", "c");
    }

    /// c:147 — `bicat` returns String (compile-time type pin).
    #[test]
    fn bicat_returns_string_type() {
        let _: String = bicat("a", "b");
    }

    /// c:216 — `strend` returns &str (compile-time type pin).
    #[test]
    fn strend_returns_str_type() {
        let s = "abc";
        let _: &str = strend(s);
    }

    /// c:35 — `dupstring` is deterministic (pure function).
    #[test]
    fn dupstring_is_deterministic() {
        for s in ["", "a", "hello world", "café"] {
            let first = dupstring(s);
            for _ in 0..3 {
                assert_eq!(
                    dupstring(s),
                    first,
                    "dupstring({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:105 — `tricat` is deterministic.
    #[test]
    fn tricat_is_deterministic() {
        let first = tricat("foo", "bar", "baz");
        for _ in 0..3 {
            assert_eq!(
                tricat("foo", "bar", "baz"),
                first,
                "tricat must be deterministic"
            );
        }
    }

    /// c:118 — `zhtricat` is identical to `tricat` (same body, different
    /// storage lane). Verify byte-for-byte parity.
    #[test]
    fn zhtricat_matches_tricat_byte_for_byte_pin() {
        for (a, b, c) in [
            ("", "", ""),
            ("foo", "bar", "baz"),
            ("x", "", "y"),
            ("a", "b", ""),
            ("café", "日", "中"),
        ] {
            assert_eq!(
                zhtricat(a, b, c),
                tricat(a, b, c),
                "zhtricat({:?},{:?},{:?}) must match tricat",
                a,
                b,
                c
            );
        }
    }

    /// c:55 — `dupstring_wlen(s, len)` length-bounded copy returns
    /// String (compile-time type pin).
    #[test]
    fn dupstring_wlen_returns_string_type() {
        let _: String = dupstring_wlen("hello", 3);
    }

    /// c:55 — `dupstring_wlen` with len == s.len() returns whole string.
    #[test]
    fn dupstring_wlen_exact_len_returns_whole_string() {
        assert_eq!(
            dupstring_wlen("hello", 5),
            "hello",
            "len == s.len() must return whole string"
        );
    }

    /// c:197 — `appstr(base, "")` empty append leaves base unchanged.
    #[test]
    fn appstr_empty_append_leaves_base_unchanged() {
        let mut s = "base".to_string();
        appstr(&mut s, "");
        assert_eq!(s, "base", "empty append is no-op");
    }

    /// c:197 — `appstr("", x)` append-to-empty equals just x.
    #[test]
    fn appstr_to_empty_yields_appendage() {
        let mut s = String::new();
        appstr(&mut s, "added");
        assert_eq!(s, "added", "append to empty base = appendage");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/string.c
    // c:74 ztrdup / c:91 wcs_ztrdup / c:135 dyncat / c:147 bicat /
    // c:169 dupstrpfx / c:180 ztrduppfx / c:216 strend
    // ═══════════════════════════════════════════════════════════════════

    /// c:74 — `ztrdup` returns String (compile-time pin, alt).
    #[test]
    fn ztrdup_returns_string_pin_alt() {
        let _: String = ztrdup("");
    }

    /// c:74 — `ztrdup(s)` preserves content for ASCII + multibyte.
    #[test]
    fn ztrdup_preserves_content() {
        for s in ["", "x", "hello world", "日本語", "café"] {
            assert_eq!(ztrdup(s), s, "ztrdup must preserve {:?}", s);
        }
    }

    /// c:91 — `wcs_ztrdup` returns String (compile-time pin).
    #[test]
    fn wcs_ztrdup_returns_string_type() {
        let _: String = wcs_ztrdup("");
    }

    /// c:135 — `dyncat("", "")` returns empty (alt).
    #[test]
    fn dyncat_both_empty_returns_empty_alt() {
        assert_eq!(dyncat("", ""), "", "empty + empty → empty");
    }

    /// c:135 — `dyncat(s, "")` equals s (right-identity).
    #[test]
    fn dyncat_right_identity_empty() {
        for s in ["", "x", "hello"] {
            assert_eq!(dyncat(s, ""), s, "dyncat({:?}, '') must equal {:?}", s, s);
        }
    }

    /// c:135 — `dyncat("", s)` equals s (left-identity).
    #[test]
    fn dyncat_left_identity_empty() {
        for s in ["", "x", "hello"] {
            assert_eq!(dyncat("", s), s, "dyncat('', {:?}) must equal {:?}", s, s);
        }
    }

    /// c:147 — `bicat("", "")` returns empty (alt).
    #[test]
    fn bicat_both_empty_returns_empty_alt() {
        assert_eq!(bicat("", ""), "");
    }

    /// c:147 — `bicat` returns String (compile-time pin, alt).
    #[test]
    fn bicat_returns_string_pin_alt() {
        let _: String = bicat("a", "b");
    }

    /// c:169 — `dupstrpfx(s, 0)` returns empty (zero-len prefix).
    #[test]
    fn dupstrpfx_zero_returns_empty() {
        for s in ["", "x", "hello"] {
            assert_eq!(dupstrpfx(s, 0), "", "dupstrpfx({:?}, 0) must be empty", s);
        }
    }

    /// c:180 — `ztrduppfx(s, 0)` returns empty (zero-len prefix).
    #[test]
    fn ztrduppfx_zero_returns_empty() {
        for s in ["", "x", "hello"] {
            assert_eq!(ztrduppfx(s, 0), "");
        }
    }

    /// c:216 — `strend("")` returns "" (empty input has empty end, alt).
    #[test]
    fn strend_empty_returns_empty_alt() {
        assert_eq!(strend(""), "", "empty → empty end");
    }

    /// c:216 — `strend` is deterministic (pure).
    #[test]
    fn strend_is_deterministic() {
        for s in ["", "x", "abc", "hello"] {
            let first = strend(s);
            for _ in 0..3 {
                assert_eq!(strend(s), first, "strend({:?}) must be pure", s);
            }
        }
    }
}
