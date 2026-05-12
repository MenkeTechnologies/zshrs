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

/// Port of `dupstring()` from `Src/string.c:33`.
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

/// Port of `dupstring_wlen()` from `Src/string.c:48`.
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
/// Port of `dupstring_wlen` from `Src/string.c:48`.
pub fn dupstring_wlen(s: &str, len: usize) -> String {                       // c:48
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

/// Port of `ztrdup()` from `Src/string.c:62`.
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

/// Port of `wcs_ztrdup()` from `Src/string.c:77`.
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

/// Port of `tricat()` from `Src/string.c:98`.
///
/// C body uses three `strcpy` calls into a `zalloc(l1+l2+l3+1)`
/// buffer. Rust port pre-sizes the `String` to avoid reallocation
/// and pushes the three slices in order.
///
// To concatenate four or more strings, see zjoin().                       // c:93
/// "Permanent" allocation lane in C; Rust's `String` is always
/// owned so the lane choice is irrelevant.
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {                      // c:98
    let mut result = String::with_capacity(s1.len() + s2.len() + s3.len());
    result.push_str(s1);
    result.push_str(s2);
    result.push_str(s3);
    result
}

/// Port of `zhtricat()` from `Src/string.c:114`.
///
/// Heap-arena variant of [`tricat`] in C. Same Rust impl since
/// the lanes collapse.
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {                    // c:114
    tricat(s1, s2, s3)
}

/// Port of `dyncat()` from `Src/string.c:131`.
///
/// C body:
/// ```c
/// ptr = (char *) zhalloc(l1 + strlen(s2) + 1);
/// strcpy(ptr, s1);
/// strcpy(ptr + l1, s2);
/// return ptr;
/// ```
///
// concatenate s1 and s2 in dynamically allocated buffer                    // c:127
/// Heap-arena two-string concat.
pub fn dyncat(s1: &str, s2: &str) -> String {                                // c:131
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Port of `bicat()` from `Src/string.c:145`.
///
/// Same shape as [`dyncat`], but C uses the permanent-storage
/// `zalloc` lane. Rust port: identical body.
pub fn bicat(s1: &str, s2: &str) -> String {                                 // c:145
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Port of `dupstrpfx()` from `Src/string.c:161`.
///
/// C body:
/// ```c
/// char *r = zhalloc(len + 1);
/// memcpy(r, s, len);
/// r[len] = '\0';
/// return r;
/// ```
///
// like dupstring(), but with a specified length                             // c:157
/// Byte-counted prefix copy. The previous Rust port used
/// `s[..len]` which panics on non-UTF-8 boundary; this port
/// matches C's `memcpy` semantics via byte slicing.
pub fn dupstrpfx(s: &str, len: usize) -> String {                            // c:161
    let bytes = s.as_bytes();
    let n = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

/// Port of `ztrduppfx()` from `Src/string.c:172`.
///
/// Same body as [`dupstrpfx`], but C uses the permanent-storage
/// lane. Lanes collapse in Rust.
pub fn ztrduppfx(s: &str, len: usize) -> String {
    dupstrpfx(s, len)
}

/// Port of `appstr()` from `Src/string.c:186`.
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
/// Port of `appstr` from `Src/string.c:186`.
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Port of `strend()` from `Src/string.c:196`.
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
/// Port of `strend` from `Src/string.c:196`.
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
}
