//! String manipulation utilities for zshrs
//!
//! Direct port of `Src/string.c` (201 lines, 11 fns).
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
pub fn dupstring(s: &str) -> String {                                        // c:33
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
pub fn dupstring_wlen(s: &str, len: usize) -> String {                       // c:48
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
pub fn ztrdup(s: &str) -> String {                                           // c:62
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
pub fn wcs_ztrdup(s: &str) -> String {                                       // c:77
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
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {                      // c:98
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
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {                    // c:114
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
pub fn dyncat(s1: &str, s2: &str) -> String {                                // c:131
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Port of `bicat(const char *s1, const char *s2)` from `Src/string.c:145`.
///
/// Same shape as [`dyncat`], but C uses the permanent-storage
/// `zalloc` lane. Rust port: identical body.
pub fn bicat(s1: &str, s2: &str) -> String {                                 // c:145
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
pub fn dupstrpfx(s: &str, len: usize) -> String {                            // c:161
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
        assert_eq!(dupstring("hello"), "hello");
        assert_eq!(dupstring(""), "");
    }

    #[test]
    fn test_dupstring_wlen() {
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
        assert_eq!(ztrdup("permanent"), "permanent");
    }

    #[test]
    fn test_wcs_ztrdup() {
        assert_eq!(wcs_ztrdup("ünicode"), "ünicode");
    }

    #[test]
    fn test_tricat() {
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("", "", ""), "");
        assert_eq!(tricat("foo", "", "bar"), "foobar");
    }

    #[test]
    fn test_zhtricat() {
        assert_eq!(zhtricat("x", "y", "z"), "xyz");
    }

    #[test]
    fn test_bicat() {
        assert_eq!(bicat("hello", " world"), "hello world");
        assert_eq!(bicat("", ""), "");
    }

    #[test]
    fn test_dyncat() {
        assert_eq!(dyncat("foo", "bar"), "foobar");
    }

    #[test]
    fn test_appstr() {
        let mut s = "hello".to_string();
        appstr(&mut s, " world");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_dupstrpfx() {
        assert_eq!(dupstrpfx("hello world", 5), "hello");
        assert_eq!(dupstrpfx("hi", 50), "hi");
        assert_eq!(dupstrpfx("hi", 0), "");
    }

    #[test]
    fn test_dupstrpfx_byte_safe() {
        // 'é' = 0xC3 0xA9. len=1 inside it must not panic.
        let _ = dupstrpfx("é", 1);
    }

    #[test]
    fn test_ztrduppfx() {
        assert_eq!(ztrduppfx("hello", 3), "hel");
    }

    #[test]
    fn test_strend_returns_last_codepoint() {
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
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("",  "b", "c"), "bc");
        assert_eq!(tricat("a", "",  "c"), "ac");
        assert_eq!(tricat("a", "b", ""),  "ab");
    }

    /// c:131 — `dyncat(s1,s2)` is the 2-string concat counterpart.
    #[test]
    fn dyncat_concatenates_two_segments() {
        assert_eq!(dyncat("hello", " world"), "hello world");
        assert_eq!(dyncat("",       "x"),     "x");
    }

    /// c:62 — `ztrdup` is the owning copy. Verifies the duplicate
    /// is independent of the source after a mutating clear.
    #[test]
    fn ztrdup_returns_independent_owned_copy() {
        let mut src = String::from("original");
        let dup = ztrdup(&src);
        src.clear();
        assert_eq!(dup, "original",
            "dup must survive source-side clear");
    }

    /// c:161 — `dupstrpfx(s, len)` returns first `len` bytes; len > s.len()
    /// must NOT panic — returns whole string. Critical for any
    /// truncation path that doesn't pre-clamp.
    #[test]
    fn dupstrpfx_handles_len_larger_than_input() {
        assert_eq!(dupstrpfx("ab",    100), "ab");
        assert_eq!(dupstrpfx("hello", 0),   "");
        assert_eq!(dupstrpfx("hello", 3),   "hel");
    }

    /// c:131 — `dyncat` with both empty inputs returns empty (no
    /// phantom delimiters).
    #[test]
    fn dyncat_empty_inputs_return_empty() {
        assert_eq!(dyncat("", ""), "");
    }

    /// `Src/string.c:144-155` — `bicat(s1, s2)` is the
    /// permanent-storage variant of `dyncat`. C body computes
    /// `zalloc(strlen(s1)+strlen(s2)+1)` then `strcpy(ptr, s1)` and
    /// `strcpy(ptr+l1, s2)`. Two-segment concat, never reorders.
    #[test]
    fn bicat_concatenates_in_order_with_either_empty() {
        assert_eq!(bicat("foo", "bar"), "foobar");
        assert_eq!(bicat("", "bar"),    "bar",
            "c:152 — strcpy(ptr, \"\") writes only the NUL, ptr+0 starts s2");
        assert_eq!(bicat("foo", ""),    "foo",
            "c:153 — strcpy(ptr+3, \"\") writes only the NUL");
        assert_eq!(bicat("", ""),       "");
    }

    /// `Src/string.c:172-178` — `ztrduppfx(s, len)` body is identical
    /// to `dupstrpfx` (same `memcpy`/NUL pattern at c:175-177); only
    /// the allocator differs (`zalloc` vs `zhalloc`). Both lanes
    /// collapse to `String` in the Rust port. Behaviour parity with
    /// `dupstrpfx` is the contract — a regression that diverged the
    /// two would silently leak storage-lane assumptions into callers.
    #[test]
    fn ztrduppfx_matches_dupstrpfx_byte_for_byte() {
        for (s, len) in [("hello", 3usize), ("ab", 100), ("hello", 0), ("", 5)] {
            assert_eq!(ztrduppfx(s, len), dupstrpfx(s, len),
                "ztrduppfx/dupstrpfx divergence at ({:?}, {})", s, len);
        }
    }

    /// `Src/string.c:186-189` — `appstr(base, append)` C body is
    /// `strcat(realloc(base, strlen(base)+strlen(append)+1), append)`.
    /// Append-in-place semantics: post-condition is `base == base ++ append`.
    /// Empty append → base unchanged. Empty base → result equals append.
    #[test]
    fn appstr_appends_in_place() {
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
        assert_eq!(dupstring("hello"), "hello");
        assert_eq!(dupstring(""), "", "c:39 — empty input → len 0+1, strcpy copies NUL");
        // Non-ASCII (UTF-8) round-trips byte-identical.
        assert_eq!(dupstring("café"),  "café");
        assert_eq!(dupstring("字"),    "字");
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
        let mut src = String::from("widechar");
        let dup = wcs_ztrdup(&src);
        src.clear();
        assert_eq!(dup, "widechar",
            "wide-char dup must survive source-side mutation");
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
        for (a, b, c) in [
            ("foo", "bar", "baz"),
            ("",    "x",   ""),
            ("a",   "",    "z"),
            ("",    "",    ""),
        ] {
            assert_eq!(zhtricat(a, b, c), tricat(a, b, c),
                "lane divergence at ({:?}, {:?}, {:?})", a, b, c);
        }
    }

    /// `Src/string.c:171-181` — `ztrduppfx(s, len)` is `dupstrpfx`
    /// with permanent storage. We already pinned the body-identical
    /// contract above; this test pins behaviour for `len > strlen`
    /// specifically (the C source would `memcpy` past the source
    /// buffer — UB; the Rust port clamps).
    #[test]
    fn ztrduppfx_clamps_oversize_len_safely() {
        assert_eq!(ztrduppfx("hi", 100), "hi");
        assert_eq!(ztrduppfx("",   5),   "");
        assert_eq!(ztrduppfx("abc", 2),  "ab");
    }
}
