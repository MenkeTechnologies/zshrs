//! String manipulation and sorting for zshrs
//!
//! Direct port from zsh/Src/string.c and zsh/Src/sort.c
//!
//! Provides:
//! - String duplication and concatenation utilities
//! - Locale-aware string comparison
//! - Numeric-aware string sorting
//! - Case-insensitive and backslash-ignoring comparison

use std::cmp::Ordering;

/// Duplicate a string (equivalent to dupstring/ztrdup in C)
#[inline]
pub fn dupstring(s: &str) -> String {
    s.to_string()
}

/// Duplicate a string with a specified length
pub fn dupstring_wlen(s: &str, len: usize) -> String {
    if len >= s.len() {
        s.to_string()
    } else {
        s[..len].to_string()
    }
}

/// Concatenate three strings
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {
    let mut result = String::with_capacity(s1.len() + s2.len() + s3.len());
    result.push_str(s1);
    result.push_str(s2);
    result.push_str(s3);
    result
}

/// Concatenate two strings
pub fn bicat(s1: &str, s2: &str) -> String {
    let mut result = String::with_capacity(s1.len() + s2.len());
    result.push_str(s1);
    result.push_str(s2);
    result
}

/// Duplicate a prefix of a string
pub fn dupstrpfx(s: &str, len: usize) -> String {
    dupstring_wlen(s, len)
}

/// Append a string to another, returning the result
pub fn appstr(base: &str, append: &str) -> String {
    bicat(base, append)
}

/// Get pointer to the last character of a string
pub fn strend(s: &str) -> Option<char> {
    s.chars().last()
}

/// Sort flags
pub mod sort_flags {
    pub const SORTIT_BACKWARDS: u32 = 1;
    pub const SORTIT_NUMERICALLY: u32 = 2;
    pub const SORTIT_NUMERICALLY_SIGNED: u32 = 4;
    pub const SORTIT_IGNORING_CASE: u32 = 8;
    pub const SORTIT_IGNORING_BACKSLASHES: u32 = 16;
}

/// Compare two strings with various options
pub fn zstrcmp(a: &str, b: &str, flags: u32) -> Ordering {
    let ignore_case = flags & sort_flags::SORTIT_IGNORING_CASE != 0;
    let ignore_backslash = flags & sort_flags::SORTIT_IGNORING_BACKSLASHES != 0;
    let numeric = flags & sort_flags::SORTIT_NUMERICALLY != 0;
    let numeric_signed = flags & sort_flags::SORTIT_NUMERICALLY_SIGNED != 0;

    // Prepare strings for comparison
    let (a_cmp, b_cmp): (std::borrow::Cow<str>, std::borrow::Cow<str>) = if ignore_case {
        (
            std::borrow::Cow::Owned(a.to_lowercase()),
            std::borrow::Cow::Owned(b.to_lowercase()),
        )
    } else {
        (std::borrow::Cow::Borrowed(a), std::borrow::Cow::Borrowed(b))
    };

    let (a_final, b_final): (std::borrow::Cow<str>, std::borrow::Cow<str>) = if ignore_backslash {
        (
            std::borrow::Cow::Owned(a_cmp.replace('\\', "")),
            std::borrow::Cow::Owned(b_cmp.replace('\\', "")),
        )
    } else {
        (a_cmp, b_cmp)
    };

    if numeric || numeric_signed {
        numeric_compare(&a_final, &b_final, numeric_signed)
    } else {
        a_final.cmp(&b_final)
    }
}

/// Numeric-aware string comparison
fn numeric_compare(a: &str, b: &str, signed: bool) -> Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    loop {
        let a_next = a_chars.peek().copied();
        let b_next = b_chars.peek().copied();

        match (a_next, b_next) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ac), Some(bc)) => {
                // Check if we're at the start of a number
                let a_is_digit = ac.is_ascii_digit();
                let b_is_digit = bc.is_ascii_digit();
                let a_is_neg = signed
                    && ac == '-'
                    && a_chars
                        .clone()
                        .nth(1)
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false);
                let b_is_neg = signed
                    && bc == '-'
                    && b_chars
                        .clone()
                        .nth(1)
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false);

                if a_is_digit || b_is_digit || a_is_neg || b_is_neg {
                    // Extract and compare numbers
                    let a_num = extract_number(&mut a_chars, signed);
                    let b_num = extract_number(&mut b_chars, signed);

                    match a_num.cmp(&b_num) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    // Regular character comparison
                    a_chars.next();
                    b_chars.next();
                    match ac.cmp(&bc) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                }
            }
        }
    }
}

/// Extract a number from a character iterator
fn extract_number<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    signed: bool,
) -> i64 {
    let mut negative = false;
    let mut num: i64 = 0;
    let mut has_digit = false;

    // Check for sign
    if signed {
        if let Some(&'-') = chars.peek() {
            chars.next();
            negative = true;
        } else if let Some(&'+') = chars.peek() {
            chars.next();
        }
    }

    // Skip leading zeros
    while let Some(&'0') = chars.peek() {
        chars.next();
        has_digit = true;
    }

    // Collect digits
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            has_digit = true;
            num = num
                .saturating_mul(10)
                .saturating_add((c as i64) - ('0' as i64));
            chars.next();
        } else {
            break;
        }
    }

    if !has_digit {
        return 0;
    }

    if negative {
        -num
    } else {
        num
    }
}

/// Sort an array of strings with various options
pub fn strmetasort(array: &mut [String], flags: u32) {
    if array.len() < 2 {
        return;
    }

    let backwards = flags & sort_flags::SORTIT_BACKWARDS != 0;

    array.sort_by(|a, b| {
        let cmp = zstrcmp(a, b, flags);
        if backwards {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Sort string slices with various options
pub fn sort_strings(array: &mut [&str], flags: u32) {
    if array.len() < 2 {
        return;
    }

    let backwards = flags & sort_flags::SORTIT_BACKWARDS != 0;

    array.sort_by(|a, b| {
        let cmp = zstrcmp(a, b, flags);
        if backwards {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Natural sort comparison (numbers sorted numerically within strings)
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    zstrcmp(a, b, sort_flags::SORTIT_NUMERICALLY)
}

/// Case-insensitive comparison
pub fn strcasecmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// Find first occurrence of substring
pub fn strstr(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

/// Check if string starts with prefix
pub fn strprefix(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// Check if string ends with suffix
pub fn strsuffix(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// Join strings with a separator
pub fn strjoin<I, S>(iter: I, sep: &str) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    iter.into_iter()
        .map(|s| s.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

/// Split string by separator
pub fn strsplit(s: &str, sep: char) -> Vec<&str> {
    s.split(sep).collect()
}

/// Trim whitespace from both ends
pub fn strtrim(s: &str) -> &str {
    s.trim()
}

/// Convert string to lowercase
pub fn strlower(s: &str) -> String {
    s.to_lowercase()
}

/// Convert string to uppercase
pub fn strupper(s: &str) -> String {
    s.to_uppercase()
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
        assert_eq!(dupstring_wlen("hello", 3), "hel");
        assert_eq!(dupstring_wlen("hi", 10), "hi");
    }

    #[test]
    fn test_tricat() {
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("hello", " ", "world"), "hello world");
    }

    #[test]
    fn test_bicat() {
        assert_eq!(bicat("hello", "world"), "helloworld");
    }

    #[test]
    fn test_strend() {
        assert_eq!(strend("hello"), Some('o'));
        assert_eq!(strend(""), None);
    }

    #[test]
    fn test_zstrcmp_basic() {
        assert_eq!(zstrcmp("abc", "abc", 0), Ordering::Equal);
        assert_eq!(zstrcmp("abc", "abd", 0), Ordering::Less);
        assert_eq!(zstrcmp("abd", "abc", 0), Ordering::Greater);
    }

    #[test]
    fn test_zstrcmp_case_insensitive() {
        let flags = sort_flags::SORTIT_IGNORING_CASE;
        assert_eq!(zstrcmp("ABC", "abc", flags), Ordering::Equal);
        assert_eq!(zstrcmp("ABC", "ABD", flags), Ordering::Less);
    }

    #[test]
    fn test_zstrcmp_ignore_backslash() {
        let flags = sort_flags::SORTIT_IGNORING_BACKSLASHES;
        assert_eq!(zstrcmp("a\\bc", "abc", flags), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_numeric() {
        let flags = sort_flags::SORTIT_NUMERICALLY;
        assert_eq!(zstrcmp("file2", "file10", flags), Ordering::Less);
        assert_eq!(zstrcmp("file10", "file2", flags), Ordering::Greater);
        assert_eq!(zstrcmp("file10", "file10", flags), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_numeric_signed() {
        let flags = sort_flags::SORTIT_NUMERICALLY_SIGNED;
        assert_eq!(zstrcmp("-5", "3", flags), Ordering::Less);
        assert_eq!(zstrcmp("-10", "-2", flags), Ordering::Less);
    }

    #[test]
    fn test_strmetasort() {
        let mut arr = vec![
            "file10".to_string(),
            "file2".to_string(),
            "file1".to_string(),
        ];
        strmetasort(&mut arr, sort_flags::SORTIT_NUMERICALLY);
        assert_eq!(arr, vec!["file1", "file2", "file10"]);
    }

    #[test]
    fn test_strmetasort_backwards() {
        let mut arr = vec!["a".to_string(), "c".to_string(), "b".to_string()];
        strmetasort(&mut arr, sort_flags::SORTIT_BACKWARDS);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_natural_cmp() {
        assert_eq!(natural_cmp("item2", "item10"), Ordering::Less);
    }

    #[test]
    fn test_strcasecmp() {
        assert_eq!(strcasecmp("Hello", "HELLO"), Ordering::Equal);
        assert_eq!(strcasecmp("abc", "ABD"), Ordering::Less);
    }

    #[test]
    fn test_strstr() {
        assert_eq!(strstr("hello world", "world"), Some(6));
        assert_eq!(strstr("hello", "xyz"), None);
    }

    #[test]
    fn test_strprefix_suffix() {
        assert!(strprefix("hello", "hel"));
        assert!(!strprefix("hello", "ell"));
        assert!(strsuffix("hello", "llo"));
        assert!(!strsuffix("hello", "ell"));
    }

    #[test]
    fn test_strjoin() {
        assert_eq!(strjoin(["a", "b", "c"], ","), "a,b,c");
        assert_eq!(strjoin(Vec::<&str>::new(), ","), "");
    }

    #[test]
    fn test_strsplit() {
        assert_eq!(strsplit("a,b,c", ','), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_strtrim() {
        assert_eq!(strtrim("  hello  "), "hello");
    }

    #[test]
    fn test_case_conversion() {
        assert_eq!(strlower("HeLLo"), "hello");
        assert_eq!(strupper("HeLLo"), "HELLO");
    }
}
