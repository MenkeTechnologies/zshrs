//! String manipulation utilities for zshrs
//!
//! Port from zsh/Src/string.c (201 lines)
//!
//! In C, zsh needs custom string functions because of its heap allocator
//! (zhalloc vs zalloc) and metafied string encoding. In Rust, String
//! handles all allocation and UTF-8 natively, so these become thin wrappers.

/// Duplicate a string into heap storage.
/// Port of `dupstring()` from Src/string.c:33 — in C the heap-arena
/// variant of `ztrdup()`. Rust's `String` always owns its allocation
/// so the heap/permanent distinction collapses to `to_string()`.
pub fn dupstring(s: &str) -> String {
    s.to_string()
}

/// Duplicate a string with explicit length.
/// Port of `dupstring_wlen()` from Src/string.c:48 — used when the
/// source isn't NUL-terminated (slice of a larger buffer).
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    s[..len.min(s.len())].to_string()
}

/// Duplicate a string into permanent storage.
/// Port of `ztrdup()` from Src/string.c:62 — C zsh's canonical
/// `strdup(3)` analog. Rust collapses this to `to_string()`.
pub fn ztrdup(s: &str) -> String {
    s.to_string()
}

/// Duplicate a wide-character string into permanent storage.
/// Port of `wcs_ztrdup()` from Src/string.c:77 — wide-char
/// counterpart of `ztrdup`. In Rust UTF-8 strings cover both.
pub fn wcs_ztrdup(s: &str) -> String {
    s.to_string()
}

/// Concatenate three strings into a new permanent string.
/// Port of `tricat()` from Src/string.c:98 — used heavily by the
/// completion machinery for prefix+match+suffix assembly.
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    let mut result = String::with_capacity(s1.len() + s2.len() + s3.len());
    result.push_str(s1);
    result.push_str(s2);
    result.push_str(s3);
    result
}

/// Concatenate three strings into a new heap-arena string.
/// Port of `zhtricat()` from Src/string.c:114 — heap-arena
/// variant of `tricat`.
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {
    tricat(s1, s2, s3)
}

/// Concatenate two strings into a new heap-arena string.
/// Port of `dyncat()` from Src/string.c:131.
pub fn dyncat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Concatenate two strings into a new permanent string.
/// Port of `bicat()` from Src/string.c:145.
pub fn bicat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Duplicate the first `len` chars into a heap-arena string.
/// Port of `dupstrpfx()` from Src/string.c:161.
pub fn dupstrpfx(s: &str, len: usize) -> String {
    s[..len.min(s.len())].to_string()
}

/// Duplicate the first `len` chars into permanent storage.
/// Port of `ztrduppfx()` from Src/string.c:172.
pub fn ztrduppfx(s: &str, len: usize) -> String {
    dupstrpfx(s, len)
}

/// Append a string in-place.
/// Port of `appstr()` from Src/string.c:186 — the C source uses
/// `strcat(3)` after realloc; Rust's `push_str` does both.
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Get the last character of a string.
/// Port of `strend()` from Src/string.c:196 — C source returns
/// the pointer to the NUL predecessor; Rust returns the char.
pub fn strend(s: &str) -> Option<char> {
    s.chars().next_back()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dupstring() {
        assert_eq!(dupstring("hello"), "hello");
    }

    #[test]
    fn test_dupstring_wlen() {
        assert_eq!(dupstring_wlen("hello world", 5), "hello");
    }

    #[test]
    fn test_tricat() {
        assert_eq!(tricat("a", "b", "c"), "abc");
    }

    #[test]
    fn test_bicat() {
        assert_eq!(bicat("hello", " world"), "hello world");
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
    fn test_strend() {
        assert_eq!(strend("hello"), Some('o'));
        assert_eq!(strend(""), None);
    }

    #[test]
    fn test_dupstrpfx() {
        assert_eq!(dupstrpfx("hello world", 5), "hello");
    }
}
