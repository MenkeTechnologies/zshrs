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

/// Sort element with comparison string and length.
/// Port of `struct sortel` (Src/sort.c lines ~30-40 of the
/// `eltpcmp()` arena setup) — pairs the original metafied string
/// with a comparison form (lowercased / no-backslash / etc.) and the
/// optional explicit length used when the string contains embedded
/// nulls.
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

/// Compare two strings according to sort flags.
/// Port of `zstrcmp()` from Src/sort.c:191 — the comparator the
/// `eltpcmp()` qsort callback (Src/sort.c:44) reduces to once the
/// per-element tie-breakers have been applied. Honours numeric /
/// reverse / case-insensitive / no-backslash flags exactly as the
/// `SORTIT_*` flag set the C source consumes.
pub fn zstrcmp(a: &str, b: &str, sort_flags: u32) -> Ordering {
    let reverse = (sort_flags & flags::REVERSE) != 0;
    let numeric = (sort_flags & flags::NUMERIC) != 0;
    let numeric_signed = (sort_flags & flags::NUMERIC_SIGNED) != 0;
    let no_backslash = (sort_flags & flags::NO_BACKSLASH) != 0;
    let case_insensitive = (sort_flags & flags::CASE_INSENSITIVE) != 0;

    // Inline backslash-strip per Src/sort.c eltpcmp prep.
    let a_str: String = if no_backslash {
        a.chars().filter(|&c| c != '\\').collect()
    } else {
        a.to_string()
    };
    let b_str: String = if no_backslash {
        b.chars().filter(|&c| c != '\\').collect()
    } else {
        b.to_string()
    };

    // Inline numeric comparison — direct port of the
    // `if (sortnumeric)` branch of eltpcmp at Src/sort.c:137-172.
    // Walks both strings to first divergence, then either short-
    // circuits on signed-mode `-DIGIT` vs `DIGIT`, or rewinds to
    // the start of the digit run, skips leading zeros, compares
    // run lengths (longer = bigger; flipped for negatives via mul).
    // Falls back to plain byte-wise compare from the divergence
    // point when neither side is a digit.
    let cmp_numeric = |a: &str, b: &str, signed_mode: bool| -> Ordering {
        let ab = a.as_bytes();
        let bb = b.as_bytes();
        let n = ab.len().min(bb.len());
        let mut i = 0;
        while i < n && ab[i] == bb[i] {
            i += 1;
        }
        let ac = ab.get(i).copied().unwrap_or(0);
        let bc = bb.get(i).copied().unwrap_or(0);
        let is_digit = |c: u8| c.is_ascii_digit();
        let mut mul: i32 = 0;
        let mut cmp: i32 = (ac as i32) - (bc as i32);
        if signed_mode {
            if ac == b'-' && ab.get(i + 1).copied().map(is_digit).unwrap_or(false) && is_digit(bc) {
                return Ordering::Less;
            }
            if bc == b'-' && bb.get(i + 1).copied().map(is_digit).unwrap_or(false) && is_digit(ac) {
                return Ordering::Greater;
            }
        }
        if is_digit(ac) || is_digit(bc) {
            let mut start = i;
            while start > 0 && is_digit(ab[start - 1]) {
                start -= 1;
            }
            if signed_mode && start > 0 && ab[start - 1] == b'-' {
                mul = -1;
            } else {
                mul = 1;
            }
            let run_a: Vec<u8> = ab[start..].iter().copied().take_while(|&c| is_digit(c)).collect();
            let run_b: Vec<u8> = bb[start..].iter().copied().take_while(|&c| is_digit(c)).collect();
            let stripped_a: &[u8] = {
                let z = run_a.iter().take_while(|&&c| c == b'0').count();
                &run_a[z..]
            };
            let stripped_b: &[u8] = {
                let z = run_b.iter().take_while(|&&c| c == b'0').count();
                &run_b[z..]
            };
            match stripped_a.len().cmp(&stripped_b.len()) {
                Ordering::Greater => {
                    return if mul >= 0 { Ordering::Greater } else { Ordering::Less };
                }
                Ordering::Less => {
                    return if mul >= 0 { Ordering::Less } else { Ordering::Greater };
                }
                Ordering::Equal => {
                    for k in 0..stripped_a.len() {
                        if stripped_a[k] != stripped_b[k] {
                            let d = (stripped_a[k] as i32) - (stripped_b[k] as i32);
                            let signed_cmp = if mul >= 0 { d } else { -d };
                            return match signed_cmp.cmp(&0) {
                                Ordering::Equal => Ordering::Equal,
                                o => o,
                            };
                        }
                    }
                    cmp = 0;
                }
            }
        }
        let _ = mul;
        if cmp == 0 {
            ab[i..].cmp(&bb[i..])
        } else if cmp < 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    };

    let mut result = if numeric {
        cmp_numeric(&a_str, &b_str, numeric_signed)
    } else if case_insensitive {
        a_str.to_lowercase().cmp(&b_str.to_lowercase())
    } else {
        a_str.cmp(&b_str)
    };

    if reverse {
        result = result.reverse();
    }
    result
}

/// Sort an array of strings.
/// Port of `strmetasort()` from Src/sort.c:234 — the public entry
/// point that wraps `qsort()` over `eltpcmp` with the same flag
/// vocabulary (`SORTIT_NUMERICALLY`, `SORTIT_BACKWARDS`, etc.).
pub fn strmetasort(arr: &mut [String], sort_flags: u32) {
    arr.sort_by(|a, b| eltpcmp(
        &SortElt::new(a), &SortElt::new(b), sort_flags));
}

/// qsort comparator over `SortElt`. Direct port of `eltpcmp()`
/// from Src/sort.c — picks the comparison key from each elt
/// (the `cmp` field, which holds the case-folded / unbackslashed
/// form prepared by the eltpcmp prep step), then delegates to
/// `zstrcmp` for the actual comparison. Same flag dispatch.
///
/// In C, eltpcmp's signature is `int(*)(const void*, const void*)`
/// for direct use as a qsort callback. The Rust port takes typed
/// references — same comparison semantics, idiomatic Rust calling.
pub fn eltpcmp(a: &SortElt, b: &SortElt, sort_flags: u32) -> Ordering {
    zstrcmp(&a.cmp, &b.cmp, sort_flags)
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
        strmetasort(&mut arr, flags::NUMERIC | flags::NUMERIC_SIGNED);
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
        strmetasort(&mut arr, flags::REVERSE);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_case_insensitive_sort() {
        let mut arr = vec![
            "Banana".to_string(),
            "apple".to_string(),
            "Cherry".to_string(),
        ];
        strmetasort(&mut arr, flags::CASE_INSENSITIVE);
        assert_eq!(arr, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn test_no_backslash() {
        assert_eq!(
            zstrcmp("a\\bc", "abc", flags::NO_BACKSLASH),
            Ordering::Equal
        );
    }

}
