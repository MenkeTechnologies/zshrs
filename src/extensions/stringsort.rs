//! String manipulation and sorting for zshrs

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

/// The canonical `SORTIT_*` bits live in `zsh.h` (ported at
/// `crate::ported::zsh_h`). This module used to declare its own set with
/// DIFFERENT values — `SORTIT_BACKWARDS = 1` / `SORTIT_IGNORING_CASE = 8`,
/// i.e. those two swapped relative to c:Src/zsh.h:2360-2366 — so any flag
/// word crossing between the two vocabularies silently changed meaning.
/// Re-export the real ones instead of redeclaring them.
pub mod sort_flags {
    use crate::ported::zsh_h;

    /// c:Src/zsh.h — `SORTIT_BACKWARDS`.
    pub const SORTIT_BACKWARDS: u32 = zsh_h::SORTIT_BACKWARDS as u32;
    /// c:Src/zsh.h — `SORTIT_NUMERICALLY`.
    pub const SORTIT_NUMERICALLY: u32 = zsh_h::SORTIT_NUMERICALLY as u32;
    /// c:Src/zsh.h — `SORTIT_NUMERICALLY_SIGNED`.
    pub const SORTIT_NUMERICALLY_SIGNED: u32 = zsh_h::SORTIT_NUMERICALLY_SIGNED as u32;
    /// c:Src/zsh.h — `SORTIT_IGNORING_CASE`.
    pub const SORTIT_IGNORING_CASE: u32 = zsh_h::SORTIT_IGNORING_CASE as u32;
    /// c:Src/zsh.h — `SORTIT_IGNORING_BACKSLASHES`.
    pub const SORTIT_IGNORING_BACKSLASHES: u32 = zsh_h::SORTIT_IGNORING_BACKSLASHES as u32;
}

/// Compare two strings under the `SORTIT_*` bits, delegating to the
/// canonical port of `zstrcmp` (`Src/sort.c:191`) at
/// `crate::ported::sort::zstrcmp`.
///
/// This function previously carried a SECOND, from-scratch comparator with
/// its own `numeric_compare` / `extract_number` helpers. That copy had the
/// same defect the ported one was fixed for in `Src/sort.c:155` — it entered
/// the numeric branch on `a_is_digit || b_is_digit` where C requires
/// `idigit(*as) && idigit(*bs)`, so a digit facing a non-digit compared as a
/// number against 0 instead of falling through to collation. It had no
/// callers, which is exactly why the defect survived: nothing exercised it.
/// Route to the one comparator rather than keep a divergent twin alive.
pub fn zstrcmp(a: &str, b: &str, flags: u32) -> Ordering {
    // c:Src/sort.c:290-295/329 — SORTIT_IGNORING_CASE is NOT handled by the
    // comparator in C; `strmetasort` case-folds when it builds each element's
    // `cmp` key and hands the comparator the folded text. Do the same here so
    // the delegate stays the unmodified C comparator.
    if flags & sort_flags::SORTIT_IGNORING_CASE != 0 {
        return crate::ported::sort::zstrcmp(&a.to_lowercase(), &b.to_lowercase(), flags);
    }
    crate::ported::sort::zstrcmp(a, b, flags)
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstring("hello"), "hello");
        assert_eq!(dupstring(""), "");
    }

    #[test]
    fn test_dupstring_wlen() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dupstring_wlen("hello", 3), "hel");
        assert_eq!(dupstring_wlen("hi", 10), "hi");
    }

    #[test]
    fn test_tricat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tricat("a", "b", "c"), "abc");
        assert_eq!(tricat("hello", " ", "world"), "hello world");
    }

    #[test]
    fn test_bicat() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(bicat("hello", "world"), "helloworld");
    }

    #[test]
    fn test_strend() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strend("hello"), Some('o'));
        assert_eq!(strend(""), None);
    }

    #[test]
    fn test_zstrcmp_basic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrcmp("abc", "abc", 0), Ordering::Equal);
        assert_eq!(zstrcmp("abc", "abd", 0), Ordering::Less);
        assert_eq!(zstrcmp("abd", "abc", 0), Ordering::Greater);
    }

    #[test]
    fn test_zstrcmp_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        let flags = sort_flags::SORTIT_IGNORING_CASE;
        assert_eq!(zstrcmp("ABC", "abc", flags), Ordering::Equal);
        assert_eq!(zstrcmp("ABC", "ABD", flags), Ordering::Less);
    }

    #[test]
    fn test_zstrcmp_ignore_backslash() {
        let _g = crate::test_util::global_state_lock();
        let flags = sort_flags::SORTIT_IGNORING_BACKSLASHES;
        assert_eq!(zstrcmp("a\\bc", "abc", flags), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_numeric() {
        let _g = crate::test_util::global_state_lock();
        let flags = sort_flags::SORTIT_NUMERICALLY;
        assert_eq!(zstrcmp("file2", "file10", flags), Ordering::Less);
        assert_eq!(zstrcmp("file10", "file2", flags), Ordering::Greater);
        assert_eq!(zstrcmp("file10", "file10", flags), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_numeric_signed() {
        let _g = crate::test_util::global_state_lock();
        let flags = sort_flags::SORTIT_NUMERICALLY_SIGNED;
        assert_eq!(zstrcmp("-5", "3", flags), Ordering::Less);
        assert_eq!(zstrcmp("-10", "-2", flags), Ordering::Less);
    }

    #[test]
    fn test_strmetasort() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["a".to_string(), "c".to_string(), "b".to_string()];
        strmetasort(&mut arr, sort_flags::SORTIT_BACKWARDS);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_natural_cmp() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(natural_cmp("item2", "item10"), Ordering::Less);
    }

    #[test]
    fn test_strcasecmp() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strcasecmp("Hello", "HELLO"), Ordering::Equal);
        assert_eq!(strcasecmp("abc", "ABD"), Ordering::Less);
    }

    #[test]
    fn test_strstr() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strstr("hello world", "world"), Some(6));
        assert_eq!(strstr("hello", "xyz"), None);
    }

    #[test]
    fn test_strprefix_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert!(strprefix("hello", "hel"));
        assert!(!strprefix("hello", "ell"));
        assert!(strsuffix("hello", "llo"));
        assert!(!strsuffix("hello", "ell"));
    }

    #[test]
    fn test_strjoin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strjoin(["a", "b", "c"], ","), "a,b,c");
        assert_eq!(strjoin(Vec::<&str>::new(), ","), "");
    }

    #[test]
    fn test_strsplit() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strsplit("a,b,c", ','), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_strtrim() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strtrim("  hello  "), "hello");
    }

    #[test]
    fn test_case_conversion() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strlower("HeLLo"), "hello");
        assert_eq!(strupper("HeLLo"), "HELLO");
    }
}
