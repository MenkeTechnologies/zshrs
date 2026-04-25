//! String manipulation utilities for zshrs
//!
//! Port from zsh/Src/string.c (201 lines)
//!
//! In C, zsh needs custom string functions because of its heap allocator
//! (zhalloc vs zalloc) and metafied string encoding. In Rust, String
//! handles all allocation and UTF-8 natively, so these become thin wrappers.

/// Duplicate a string (from string.c dupstring)
/// In Rust: String::clone() or .to_string()
pub fn dupstring(s: &str) -> String {
    s.to_string()
}

/// Duplicate string with known length (from string.c dupstring_wlen)
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    s[..len.min(s.len())].to_string()
}

/// Duplicate string on permanent storage (from string.c ztrdup)
/// In Rust there's no heap/permanent distinction
pub fn ztrdup(s: &str) -> String {
    s.to_string()
}

/// Duplicate wide string (from string.c wcs_ztrdup)
pub fn wcs_ztrdup(s: &str) -> String {
    s.to_string()
}

/// Concatenate three strings (from string.c tricat)
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    let mut result = String::with_capacity(s1.len() + s2.len() + s3.len());
    result.push_str(s1);
    result.push_str(s2);
    result.push_str(s3);
    result
}

/// Concatenate three strings on heap (from string.c zhtricat)
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {
    tricat(s1, s2, s3)
}

/// Concatenate two strings on heap (from string.c dyncat)
pub fn dyncat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Concatenate two strings on permanent storage (from string.c bicat)
pub fn bicat(s1: &str, s2: &str) -> String {
    format!("{}{}", s1, s2)
}

/// Duplicate string prefix of given length (from string.c dupstrpfx)
pub fn dupstrpfx(s: &str, len: usize) -> String {
    s[..len.min(s.len())].to_string()
}

/// Duplicate string prefix on permanent storage (from string.c ztrduppfx)
pub fn ztrduppfx(s: &str, len: usize) -> String {
    dupstrpfx(s, len)
}

/// Append string to allocated string (from string.c appstr)
pub fn appstr(base: &mut String, append: &str) {
    base.push_str(append);
}

/// Return pointer to last character (from string.c strend)
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
