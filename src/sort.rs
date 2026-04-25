//! Zsh string sorting - Direct port from zsh/Src/sort.c
//!
//! Provides comparison and sorting functions for shell strings,
//! including numeric sorting and various comparison modes.

use std::cmp::Ordering;

/// Sort flags from sort.c
pub mod flags {
    pub const NUMERIC: u32 = 1 << 0; // -n: numeric sort
    pub const REVERSE: u32 = 1 << 1; // -O: reverse order
    pub const CASE_INSENSITIVE: u32 = 1 << 2; // -i: case insensitive
    pub const NO_BACKSLASH: u32 = 1 << 3; // ignore backslashes
    pub const NUMERIC_SIGNED: u32 = 1 << 4; // handle negative numbers
}

/// Sort element with comparison string and length
#[derive(Clone, Debug)]
pub struct SortElt {
    pub orig: String,
    pub cmp: String,
    pub len: Option<usize>, // None = standard null-terminated, Some = embedded nulls
}

impl SortElt {
    pub fn new(s: &str) -> Self {
        SortElt {
            orig: s.to_string(),
            cmp: s.to_string(),
            len: None,
        }
    }

    pub fn with_len(s: &str, len: usize) -> Self {
        SortElt {
            orig: s.to_string(),
            cmp: s.to_string(),
            len: Some(len),
        }
    }
}

/// Compare two strings according to sort flags (from sort.c eltpcmp)
pub fn zstrcmp(a: &str, b: &str, sort_flags: u32) -> Ordering {
    let reverse = (sort_flags & flags::REVERSE) != 0;
    let numeric = (sort_flags & flags::NUMERIC) != 0;
    let numeric_signed = (sort_flags & flags::NUMERIC_SIGNED) != 0;
    let no_backslash = (sort_flags & flags::NO_BACKSLASH) != 0;
    let case_insensitive = (sort_flags & flags::CASE_INSENSITIVE) != 0;

    let mut result = compare_strings(
        a,
        b,
        numeric,
        numeric_signed,
        no_backslash,
        case_insensitive,
    );

    if reverse {
        result = result.reverse();
    }
    result
}

fn compare_strings(
    a: &str,
    b: &str,
    numeric: bool,
    numeric_signed: bool,
    no_backslash: bool,
    case_insensitive: bool,
) -> Ordering {
    let a_chars: Vec<char> = if no_backslash {
        a.chars().filter(|&c| c != '\\').collect()
    } else {
        a.chars().collect()
    };

    let b_chars: Vec<char> = if no_backslash {
        b.chars().filter(|&c| c != '\\').collect()
    } else {
        b.chars().collect()
    };

    let a_str: String = a_chars.into_iter().collect();
    let b_str: String = b_chars.into_iter().collect();

    if numeric {
        return compare_numeric(&a_str, &b_str, numeric_signed);
    }

    if case_insensitive {
        a_str.to_lowercase().cmp(&b_str.to_lowercase())
    } else {
        a_str.cmp(&b_str)
    }
}

/// Numeric comparison from sort.c
fn compare_numeric(a: &str, b: &str, signed_mode: bool) -> Ordering {
    let a_num = parse_leading_number(a, signed_mode);
    let b_num = parse_leading_number(b, signed_mode);

    match (a_num, b_num) {
        (Some(an), Some(bn)) => {
            let cmp = an.partial_cmp(&bn).unwrap_or(Ordering::Equal);
            if cmp != Ordering::Equal {
                return cmp;
            }
            // Numbers are equal, compare remaining strings
            let a_rest = skip_number(a, signed_mode);
            let b_rest = skip_number(b, signed_mode);
            compare_numeric(a_rest, b_rest, signed_mode)
        }
        (Some(_), None) => Ordering::Greater, // Numbers before non-numbers? Or vice versa?
        (None, Some(_)) => Ordering::Less,
        (None, None) => {
            // Both are non-numeric at this point, do string comparison
            let (a_head, a_tail) = split_at_number(a);
            let (b_head, b_tail) = split_at_number(b);

            let head_cmp = a_head.cmp(&b_head);
            if head_cmp != Ordering::Equal {
                return head_cmp;
            }

            if a_tail.is_empty() && b_tail.is_empty() {
                return Ordering::Equal;
            }

            compare_numeric(a_tail, b_tail, signed_mode)
        }
    }
}

fn parse_leading_number(s: &str, signed_mode: bool) -> Option<f64> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }

    let mut chars = s.chars().peekable();
    let mut num_str = String::new();

    // Handle sign
    if signed_mode {
        if let Some(&c) = chars.peek() {
            if c == '-' || c == '+' {
                num_str.push(chars.next().unwrap());
            }
        }
    }

    // Check if next char is digit
    if chars.peek().map_or(true, |c| !c.is_ascii_digit()) {
        return None;
    }

    // Collect digits
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num_str.push(chars.next().unwrap());
        } else if c == '.' {
            num_str.push(chars.next().unwrap());
            // Collect decimal digits
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    num_str.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            break;
        } else {
            break;
        }
    }

    num_str.parse::<f64>().ok()
}

fn skip_number(s: &str, signed_mode: bool) -> &str {
    let s = s.trim_start();
    let mut idx = 0;
    let chars: Vec<char> = s.chars().collect();

    // Skip sign
    if signed_mode && !chars.is_empty() && (chars[0] == '-' || chars[0] == '+') {
        idx += 1;
    }

    // Skip digits
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        idx += 1;
    }

    // Skip decimal part
    if idx < chars.len() && chars[idx] == '.' {
        idx += 1;
        while idx < chars.len() && chars[idx].is_ascii_digit() {
            idx += 1;
        }
    }

    &s[s.chars().take(idx).map(|c| c.len_utf8()).sum::<usize>()..]
}

fn split_at_number(s: &str) -> (&str, &str) {
    let idx = s
        .chars()
        .position(|c| c.is_ascii_digit())
        .unwrap_or(s.len());

    let byte_idx = s.chars().take(idx).map(|c| c.len_utf8()).sum::<usize>();
    (&s[..byte_idx], &s[byte_idx..])
}

/// Sort an array of strings (from sort.c strmetasort)
pub fn strmetasort(arr: &mut [String], sort_flags: u32) {
    arr.sort_by(|a, b| zstrcmp(a, b, sort_flags));
}

/// Sort array in place with natural (numeric) ordering
pub fn natural_sort(arr: &mut [String]) {
    strmetasort(arr, flags::NUMERIC | flags::NUMERIC_SIGNED);
}

/// Sort array in place with reverse order
pub fn reverse_sort(arr: &mut [String]) {
    strmetasort(arr, flags::REVERSE);
}

/// Sort array case-insensitively
pub fn case_insensitive_sort(arr: &mut [String]) {
    strmetasort(arr, flags::CASE_INSENSITIVE);
}

/// Sort array of SortElt structures
pub fn sort_elts(elts: &mut [SortElt], sort_flags: u32) {
    let reverse = (sort_flags & flags::REVERSE) != 0;
    let numeric = (sort_flags & flags::NUMERIC) != 0;
    let numeric_signed = (sort_flags & flags::NUMERIC_SIGNED) != 0;
    let no_backslash = (sort_flags & flags::NO_BACKSLASH) != 0;
    let case_insensitive = (sort_flags & flags::CASE_INSENSITIVE) != 0;

    elts.sort_by(|a, b| {
        let mut result = compare_strings(
            &a.cmp,
            &b.cmp,
            numeric,
            numeric_signed,
            no_backslash,
            case_insensitive,
        );
        if reverse {
            result = result.reverse();
        }
        result
    });
}

/// Create comparison key for sorting (from sort.c tricat style)
pub fn make_sort_key(s: &str, case_insensitive: bool) -> String {
    if case_insensitive {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zstrcmp_basic() {
        assert_eq!(zstrcmp("abc", "def", 0), Ordering::Less);
        assert_eq!(zstrcmp("def", "abc", 0), Ordering::Greater);
        assert_eq!(zstrcmp("abc", "abc", 0), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_reverse() {
        assert_eq!(zstrcmp("abc", "def", flags::REVERSE), Ordering::Greater);
        assert_eq!(zstrcmp("def", "abc", flags::REVERSE), Ordering::Less);
    }

    #[test]
    fn test_zstrcmp_case_insensitive() {
        assert_eq!(
            zstrcmp("ABC", "abc", flags::CASE_INSENSITIVE),
            Ordering::Equal
        );
        assert_eq!(
            zstrcmp("ABC", "def", flags::CASE_INSENSITIVE),
            Ordering::Less
        );
    }

    #[test]
    fn test_zstrcmp_numeric() {
        assert_eq!(zstrcmp("file2", "file10", flags::NUMERIC), Ordering::Less);
        assert_eq!(
            zstrcmp("file10", "file2", flags::NUMERIC),
            Ordering::Greater
        );
        assert_eq!(zstrcmp("100", "20", flags::NUMERIC), Ordering::Greater);
    }

    #[test]
    fn test_zstrcmp_numeric_signed() {
        let f = flags::NUMERIC | flags::NUMERIC_SIGNED;
        assert_eq!(zstrcmp("-5", "3", f), Ordering::Less);
        assert_eq!(zstrcmp("-10", "-2", f), Ordering::Less);
        assert_eq!(zstrcmp("5", "-3", f), Ordering::Greater);
    }

    #[test]
    fn test_natural_sort() {
        let mut arr = vec![
            "file10".to_string(),
            "file2".to_string(),
            "file1".to_string(),
            "file20".to_string(),
        ];
        natural_sort(&mut arr);
        assert_eq!(arr, vec!["file1", "file2", "file10", "file20"]);
    }

    #[test]
    fn test_strmetasort() {
        let mut arr = vec![
            "zebra".to_string(),
            "apple".to_string(),
            "mango".to_string(),
        ];
        strmetasort(&mut arr, 0);
        assert_eq!(arr, vec!["apple", "mango", "zebra"]);
    }

    #[test]
    fn test_reverse_sort() {
        let mut arr = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        reverse_sort(&mut arr);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_case_insensitive_sort() {
        let mut arr = vec![
            "Banana".to_string(),
            "apple".to_string(),
            "Cherry".to_string(),
        ];
        case_insensitive_sort(&mut arr);
        assert_eq!(arr, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn test_no_backslash() {
        assert_eq!(
            zstrcmp("a\\bc", "abc", flags::NO_BACKSLASH),
            Ordering::Equal
        );
    }

    #[test]
    fn test_make_sort_key() {
        assert_eq!(make_sort_key("Hello", false), "Hello");
        assert_eq!(make_sort_key("Hello", true), "hello");
    }
}
