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

/// Numeric comparison — direct port of src/zsh/Src/sort.c:137-172
/// (the `if (sortnumeric)` branch of eltpcmp).
///
/// Algorithm: walk both strings until they diverge (or both end);
/// `ao` records the start so we can rewind into a digit run. On
/// divergence the C source distinguishes two sub-cases. First,
/// signed-mode where one side starts with `-DIGIT` and the other
/// starts with `DIGIT` — the negative side is less. Second, either
/// side is a digit at the divergence point — walk back to the start
/// of the digit run on both sides (they share a common prefix up to
/// `as`), skip leading zeros, find the first different digit, then
/// count remaining digits to decide which number is longer (longer
/// wins for positive; reversed for negative via `mul`). Otherwise
/// plain byte-wise compare from `as`.
fn compare_numeric(a: &str, b: &str, signed_mode: bool) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let n = ab.len().min(bb.len());

    // Walk to first divergence (or shared end).
    let mut i = 0;
    while i < n && ab[i] == bb[i] {
        i += 1;
    }

    let ac = ab.get(i).copied().unwrap_or(0);
    let bc = bb.get(i).copied().unwrap_or(0);
    let is_digit = |c: u8| c.is_ascii_digit();

    let mut mul: i32 = 0;
    let mut cmp: i32 = (ac as i32) - (bc as i32);

    // Signed-mode sign-vs-digit early branches (sort.c:143-151).
    if signed_mode {
        if ac == b'-' && ab.get(i + 1).copied().map(is_digit).unwrap_or(false) && is_digit(bc) {
            return Ordering::Less;
        }
        if bc == b'-' && bb.get(i + 1).copied().map(is_digit).unwrap_or(false) && is_digit(ac) {
            return Ordering::Greater;
        }
    }

    // Digit-run compare (sort.c:152-171).
    if is_digit(ac) || is_digit(bc) {
        // Rewind to the start of the digit run. Both strings share
        // bytes [..i] so the rewind position is the same on both.
        let mut start = i;
        while start > 0 && is_digit(ab[start - 1]) {
            start -= 1;
        }
        // Determine sign multiplier (signed mode + leading `-`).
        if signed_mode && start > 0 && ab[start - 1] == b'-' {
            mul = -1;
        } else {
            mul = 1;
        }

        // We need to compare the FULL digit runs starting at `start`,
        // not just from `i` — because in `0042` vs `0050` the runs
        // differ at position 2 but the leading-zero skip changes
        // alignment. zsh skips leading zeros first, then walks digit-
        // by-digit.
        let run_a: Vec<u8> = ab[start..]
            .iter()
            .copied()
            .take_while(|&c| is_digit(c))
            .collect();
        let run_b: Vec<u8> = bb[start..]
            .iter()
            .copied()
            .take_while(|&c| is_digit(c))
            .collect();
        let stripped_a: &[u8] = {
            let z = run_a.iter().take_while(|&&c| c == b'0').count();
            &run_a[z..]
        };
        let stripped_b: &[u8] = {
            let z = run_b.iter().take_while(|&&c| c == b'0').count();
            &run_b[z..]
        };
        // Longer (post-zero) run wins (more digits = bigger number).
        match stripped_a.len().cmp(&stripped_b.len()) {
            Ordering::Greater => {
                return if mul >= 0 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            Ordering::Less => {
                return if mul >= 0 {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            Ordering::Equal => {
                // Same length — compare digit-by-digit.
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
                // Numbers are equal — fall through to compare what
                // follows the run.
                let after_a = &ab[start + run_a.len()..];
                let after_b = &bb[start + run_b.len()..];
                let _ = (after_a, after_b);
                cmp = 0;
            }
        }
    }

    // Default byte-wise compare from divergence point (sort.c:174-175,
    // strcmp(as, bs)).
    let _ = mul;
    if cmp == 0 {
        ab[i..].cmp(&bb[i..])
    } else if cmp < 0 {
        Ordering::Less
    } else {
        Ordering::Greater
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
    if chars.peek().is_none_or(|c| !c.is_ascii_digit()) {
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

/// Sort an array of strings.
/// Port of `strmetasort()` from Src/sort.c:234 — the public entry
/// point that wraps `qsort()` over `eltpcmp` with the same flag
/// vocabulary (`SORTIT_NUMERICALLY`, `SORTIT_BACKWARDS`, etc.).
pub fn strmetasort(arr: &mut [String], sort_flags: u32) {
    arr.sort_by(|a, b| zstrcmp(a, b, sort_flags));
}

/// Sort array in place with natural (numeric) ordering.
/// Convenience wrapper around `strmetasort()` (Src/sort.c:234) with
/// the `NUMERIC | NUMERIC_SIGNED` flag pair the C source uses for
/// `${(n)array}` parameter expansion.
pub fn natural_sort(arr: &mut [String]) {
    strmetasort(arr, flags::NUMERIC | flags::NUMERIC_SIGNED);
}

/// Sort array in place with reverse order.
/// Convenience wrapper around `strmetasort()` (Src/sort.c:234) with
/// the `SORTIT_BACKWARDS` flag the C source uses for `${(O)array}`
/// parameter expansion.
pub fn reverse_sort(arr: &mut [String]) {
    strmetasort(arr, flags::REVERSE);
}

/// Sort array case-insensitively.
/// Convenience wrapper around `strmetasort()` (Src/sort.c:234) with
/// the `SORTIT_IGNORING_CASE` flag the C source uses for the
/// `${(i)array}` parameter expansion.
pub fn case_insensitive_sort(arr: &mut [String]) {
    strmetasort(arr, flags::CASE_INSENSITIVE);
}

/// Sort array of `SortElt` structures.
/// Port of the `eltpcmp()`-driven qsort loop in Src/sort.c:44 —
/// keeps each element's `cmp` form (lowercased / unbackslashed)
/// separate from `orig` so the comparator reads the prepared key
/// while the array still holds the user-visible original strings.
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

/// Create comparison key for sorting.
/// Port of the `tricat()` / `casemodify()` prep step from Src/sort.c
/// (~line 100, where `eltpcmp` builds `e->cmp` before sorting). The
/// C source allocates a heap copy with case folding applied; this
/// Rust version returns the same prepared key.
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
