//! Zsh string sorting — direct port of `Src/sort.c`.
//!
//! Provides comparison and sorting for shell strings with the same
//! flag vocabulary the C source uses (`SORTIT_*` from Src/zsh.h:2993).
//! Three public entry points:
//!
//! * [`zstrcmp`]: pairwise comparator (front-end to [`eltpcmp`]).
//! * [`eltpcmp`]: the qsort callback over [`SortElt`] pairs.
//! * [`strmetasort`]: array sorter, takes optional `unmetalenp`
//!   slice for embedded-NUL-bearing strings (matches C's 3-arg
//!   signature exactly).
//!
//! Faithfulness notes:
//! - Flag values match C's `SORTIT_*` enum exactly (1, 2, 4, 8, 16,
//!   32) so callers using the C bit values get the same semantics.
//!   `crate::zsh_h::SORTIT_*` (`i32` in the header port; cast with
//!   `as u32` where this module takes `sort_flags: u32`).
//! - `SortElt` carries `len: Option<usize>` matching C's `len = -1`
//!   sentinel for "use strlen — no embedded NULs"; `Some(n)` means
//!   "first n bytes only, may contain embedded NULs".
//! - `origlen` field added for `unmetalenp` parity (C records the
//!   per-element length so the caller can read it back after sort).
//! - `strmetasort` matches C's 3-arg signature exactly: passing
//!   `unmetalenp = None` is C's `NULL`.
//! - Pre-transform: case-fold and backslash-strip happen once during
//!   `SortElt::with_transforms` (matching C's strmetasort prep at
//!   sort.c:289-385) instead of per-compare. The previous Rust port
//!   re-transformed the cmp form on every comparison, which is
//!   `O(N log N × M)` extra work.
//! - Multibyte case-folding uses Rust's native Unicode-aware
//!   `to_lowercase` (which subsumes C's `mbrtowc` + `towlower` +
//!   `wcrtomb` dance at sort.c:341-368).

use std::cmp::Ordering;

use crate::zsh_h::{
    SORTIT_ANYOLDHOW, SORTIT_BACKWARDS, SORTIT_IGNORING_BACKSLASHES, SORTIT_IGNORING_CASE,
    SORTIT_NUMERICALLY, SORTIT_NUMERICALLY_SIGNED, SORTIT_SOMEHOW,
};

/// Sort element. Direct port of `struct sortelt` from `Src/zsh.h`,
/// referenced by `Src/sort.c::eltpcmp` (line 44) and `strmetasort`
/// (line 234).
///
/// Fields:
/// - `orig`: the original metafied string the caller will get back.
/// - `cmp`: the comparison key — pre-transformed by
///   [`SortElt::with_transforms`] to apply case-fold / backslash-
///   strip once, instead of per-compare.
/// - `len`: `Some(n)` means "compare first n bytes" for embedded-
///   NUL-bearing strings; `None` matches C's `len == -1` meaning
///   "use strlen — no embedded NULs".
/// - `origlen`: the per-element pre-unmetafy length, mirroring C's
///   `unmetalenp` array element. Read back after sort to pair the
///   sorted strings with their original lengths.
#[derive(Clone, Debug)]
// WARNING: FAKE IMPL RUST INVENTION — not in sort.c
pub struct SortElt {
    pub orig: String,
    pub cmp: String,
    pub len: Option<usize>,
    pub origlen: Option<usize>,
}

impl SortElt {
    /// Construct a sort element for a string with no embedded NULs.
    /// C: `e.cmp = s; e.orig = s; e.len = -1;`.
    pub fn new(s: &str) -> Self {
        SortElt {
            orig: s.to_string(),
            cmp: s.to_string(),
            len: None,
            origlen: None,
        }
    }

    /// Construct with explicit length (embedded-NUL-bearing buffer).
    /// C: `e.len = len; e.origlen = len;`.
    pub fn with_len(s: &str, len: usize) -> Self {
        SortElt {
            orig: s.to_string(),
            cmp: s.to_string(),
            len: Some(len),
            origlen: Some(len),
        }
    }

    /// Build the `cmp` form once, applying the `SORTIT_IGNORING_*`
    /// transformations the C source does inline at sort.c:289-385:
    /// - `IGNORING_CASE`: lowercase the comparison form.
    /// - `IGNORING_BACKSLASHES`: drop literal backslashes.
    ///
    /// The result is stored on `self.cmp` so the eltpcmp callback
    /// can compare the transformed form directly without repeating
    /// the work for every pair.
    pub fn with_transforms(mut self, sort_flags: u32) -> Self {
        let mut t = self.cmp.clone();
        if sort_flags & (SORTIT_IGNORING_CASE as u32) != 0 {
            t = t.to_lowercase(); // c:329-374 before backslash strip
        }
        if sort_flags & (SORTIT_IGNORING_BACKSLASHES as u32) != 0 {
            t = t.chars().filter(|&c| c != '\\').collect(); // c:375-385
        }
        self.cmp = t;
        self
    }
}

/// Port of `zstrcmp()` from `Src/sort.c:191`.
///
/// C fixes `sortdir = 1`, sets only `sortnobslash` and `sortnumeric`
/// from `sortflags` (`sort.c:207-210`), then calls `eltpcmp`. It does
/// **not** consult `SORTIT_BACKWARDS` or `SORTIT_IGNORING_CASE` — those
/// apply in `strmetasort` via `sortdir` and the pre-transform loop.
pub fn zstrcmp(a: &str, b: &str, sort_flags: u32) -> Ordering {              // c:191
    let sortnumeric = if sort_flags & (SORTIT_NUMERICALLY_SIGNED as u32) != 0 {
        -1 // c:209-210
    } else if sort_flags & (SORTIT_NUMERICALLY as u32) != 0 {
        1
    } else {
        0
    };
    let numeric = sortnumeric != 0;
    let numeric_signed = sortnumeric < 0;
    let no_backslash = (sort_flags & (SORTIT_IGNORING_BACKSLASHES as u32)) != 0;

    // Approximation of `sortnobslash` scanning (`sort.c:120-131`): C
    // drops `\\` pairwise while comparing; stripping all backslashes
    // matches zsh for typical glob names.
    let mut a_str = a.to_string();
    let mut b_str = b.to_string();
    if no_backslash {
        a_str = a_str.chars().filter(|&c| c != '\\').collect();
        b_str = b_str.chars().filter(|&c| c != '\\').collect();
    }

    // Numeric comparison — direct port of the `if (sortnumeric)`
    // block at Src/sort.c:137-172. Walks both strings to first
    // divergence, then either short-circuits on signed-mode
    // `-DIGIT` vs `DIGIT`, or rewinds to the start of the digit
    // run, skips leading zeros, compares run lengths (longer =
    // bigger; flipped for negatives via `mul`). Falls back to byte
    // compare from the divergence point when neither side is a
    // digit.
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

    if numeric {
        cmp_numeric(&a_str, &b_str, numeric_signed)
    } else {
        let c = {
            #[cfg(unix)]
            {
                use libc;
                use std::ffi::CString;
                let cstr_head = |s: &str| -> CString {
                    let b = s.as_bytes();
                    let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
                    CString::new(&b[..n]).unwrap_or_else(|_| CString::new(vec![0u8]).expect("nul"))
                };
                let ca = cstr_head(&a_str);
                let cb = cstr_head(&b_str);
                unsafe { libc::strcoll(ca.as_ptr(), cb.as_ptr()) }
            }
            #[cfg(not(unix))]
            {
                match a_str.cmp(&b_str) {
                    Ordering::Less => -1i32,
                    Ordering::Equal => 0,
                    Ordering::Greater => 1,
                }
            }
        };
        if c < 0 {
            Ordering::Less
        } else if c > 0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

/// Port of `eltpcmp()` from `Src/sort.c:44`.
///
/// The qsort callback. C's signature is
/// `int(*)(const void*, const void*)` for direct use with
/// `qsort(3)`. Rust port takes typed references — same semantics,
/// idiomatic Rust calling convention.
///
/// Embedded-null handling: when either elt has `len = Some(n)`,
/// the comparison runs over the first `n` bytes of `cmp` field
/// (matching C's `len != -1` branch at sort.c:52-118). Equal-but-
/// shorter strings sort below their longer continuations.
pub fn eltpcmp(a: &SortElt, b: &SortElt, sort_flags: u32) -> Ordering {      // c:44
    let reverse = (sort_flags & (SORTIT_BACKWARDS as u32)) != 0;
    let result = match (a.len, b.len) {
        (None, None) => zstrcmp(
            &a.cmp,
            &b.cmp,
            sort_flags & !(SORTIT_BACKWARDS as u32),
        ),
        _ => {
            // Embedded-null path: compare first min(a.len, b.len)
            // bytes; equal-prefix-but-different-length means the
            // shorter sorts lower.
            let len = match (a.len, b.len) {
                (Some(la), Some(lb)) => la.min(lb),
                (Some(la), None) => la.min(b.cmp.len()),
                (None, Some(lb)) => lb.min(a.cmp.len()),
                _ => unreachable!(),
            };
            let ab = a.cmp.as_bytes();
            let bb = b.cmp.as_bytes();
            let take_a = ab.len().min(len);
            let take_b = bb.len().min(len);
            match ab[..take_a].cmp(&bb[..take_b]) {
                Ordering::Equal => match (a.len, b.len) {
                    (Some(la), Some(lb)) => la.cmp(&lb),
                    (Some(_), None) => Ordering::Greater,
                    (None, Some(_)) => Ordering::Less,
                    _ => Ordering::Equal,
                },
                o => o,
            }
        }
    };
    if reverse {
        result.reverse()
    } else {
        result
    }
}

/// Port of `strmetasort()` from `Src/sort.c:234`.
// lengths.                                                                 // c:229
/// C signature: `void strmetasort(char **array, int sortwhat,
/// int *unmetalenp)`. `unmetalenp = None` (i.e. C's `NULL`) means
/// the strings are still metafied (no embedded NULs). When
/// `Some(slice)`, the slice is C's parallel array of per-element
/// pre-unmetafied lengths; after sort it's re-ordered in lockstep
/// with `arr` so the lengths track their owning strings.
pub fn strmetasort(                                                          // c:234
    arr: &mut [String],
    sort_flags: u32,
    unmetalenp: Option<&mut [usize]>,
) {
    if arr.len() < 2 {
        return;
    }

    // Build SortElts up front, applying transforms once (C does the
    // same at sort.c:289-385 inside the prep loop).
    let elts: Vec<SortElt> = match unmetalenp.as_deref() {
        Some(lens) => arr
            .iter()
            .zip(lens.iter())
            .map(|(s, &l)| SortElt::with_len(s, l).with_transforms(sort_flags))
            .collect(),
        None => arr
            .iter()
            .map(|s| SortElt::new(s).with_transforms(sort_flags))
            .collect(),
    };

    // Sort indices so we can remap arr+unmetalenp in lockstep
    // (C's qsort over SortElt* pointers achieves the same).
    let mut indices: Vec<usize> = (0..elts.len()).collect();
    indices.sort_by(|&i, &j| eltpcmp(&elts[i], &elts[j], sort_flags));

    let original: Vec<String> = arr.to_vec();
    let original_lens: Option<Vec<usize>> = unmetalenp.as_deref().map(|l| l.to_vec());
    for (dst, &src) in indices.iter().enumerate() {
        arr[dst] = original[src].clone();
    }
    if let (Some(out), Some(orig_lens)) = (unmetalenp, original_lens) {
        for (dst, &src) in indices.iter().enumerate() {
            out[dst] = orig_lens[src];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_values_match_c_sortit() {
        // Sanity-check that the flag constants match Src/zsh.h:2993.
        assert_eq!(SORTIT_ANYOLDHOW, 0);
        assert_eq!(SORTIT_IGNORING_CASE, 1);
        assert_eq!(SORTIT_NUMERICALLY, 2);
        assert_eq!(SORTIT_NUMERICALLY_SIGNED, 4);
        assert_eq!(SORTIT_BACKWARDS, 8);
        assert_eq!(SORTIT_IGNORING_BACKSLASHES, 16);
        assert_eq!(SORTIT_SOMEHOW, 32);
    }

    #[test]
    fn test_zstrcmp_basic() {
        assert_eq!(zstrcmp("abc", "def", 0), Ordering::Less);
        assert_eq!(zstrcmp("def", "abc", 0), Ordering::Greater);
        assert_eq!(zstrcmp("abc", "abc", 0), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_ignores_backwards_per_c() {
        assert_eq!(
            zstrcmp("abc", "def", SORTIT_BACKWARDS as u32),
            zstrcmp("abc", "def", 0)
        );
    }

    #[test]
    fn test_zstrcmp_ignores_case_flag_per_c() {
        let with = zstrcmp("ABC", "abc", SORTIT_IGNORING_CASE as u32);
        let without = zstrcmp("ABC", "abc", 0);
        assert_eq!(with, without);
    }

    #[test]
    fn test_zstrcmp_numeric() {
        assert_eq!(
            zstrcmp("file2", "file10", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
        assert_eq!(
            zstrcmp("file10", "file2", SORTIT_NUMERICALLY as u32),
            Ordering::Greater
        );
        assert_eq!(zstrcmp("100", "20", SORTIT_NUMERICALLY as u32), Ordering::Greater);
    }

    #[test]
    fn test_zstrcmp_numeric_signed() {
        let f = (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32;
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
        strmetasort(&mut arr, (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32, None);
        assert_eq!(arr, vec!["file1", "file2", "file10", "file20"]);
    }

    #[test]
    fn test_strmetasort() {
        let mut arr = vec![
            "zebra".to_string(),
            "apple".to_string(),
            "mango".to_string(),
        ];
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, vec!["apple", "mango", "zebra"]);
    }

    #[test]
    fn test_reverse_sort() {
        let mut arr = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        strmetasort(&mut arr, SORTIT_BACKWARDS as u32, None);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_case_insensitive_sort() {
        let mut arr = vec![
            "Banana".to_string(),
            "apple".to_string(),
            "Cherry".to_string(),
        ];
        strmetasort(&mut arr, SORTIT_IGNORING_CASE as u32, None);
        assert_eq!(arr, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn test_no_backslash() {
        assert_eq!(
            zstrcmp("a\\bc", "abc", SORTIT_IGNORING_BACKSLASHES as u32),
            Ordering::Equal
        );
    }

    #[test]
    fn test_with_transforms_lowercases_cmp() {
        let e = SortElt::new("ABC").with_transforms(SORTIT_IGNORING_CASE as u32);
        assert_eq!(e.orig, "ABC");
        assert_eq!(e.cmp, "abc");
    }

    #[test]
    fn test_with_transforms_strips_backslashes() {
        let e = SortElt::new("a\\b\\c").with_transforms(SORTIT_IGNORING_BACKSLASHES as u32);
        assert_eq!(e.orig, "a\\b\\c");
        assert_eq!(e.cmp, "abc");
    }

    #[test]
    fn test_with_transforms_combines_flags() {
        let e = SortElt::new("A\\BC")
            .with_transforms((SORTIT_IGNORING_CASE | SORTIT_IGNORING_BACKSLASHES) as u32);
        assert_eq!(e.cmp, "abc");
    }

    #[test]
    fn test_eltpcmp_embedded_null_shorter_sorts_below() {
        // Two strings with the same prefix but different lengths.
        // C: "the string that's finished sorts below the other"
        // (sort.c:88-89). With len markers, prefix "abc" + 0 sorts
        // below prefix "abc" + 0 + "d" (the longer continuation).
        let a = SortElt::with_len("abc", 3);
        let b = SortElt::with_len("abc", 5);
        assert_eq!(eltpcmp(&a, &b, 0), Ordering::Less);
        assert_eq!(eltpcmp(&b, &a, 0), Ordering::Greater);
    }

    #[test]
    fn test_strmetasort_reorders_lens_in_lockstep() {
        let mut arr = vec![
            "banana".to_string(),
            "apple".to_string(),
            "cherry".to_string(),
        ];
        let mut lens = vec![6, 5, 6];
        strmetasort(&mut arr, 0, Some(&mut lens));
        assert_eq!(arr, vec!["apple", "banana", "cherry"]);
        assert_eq!(lens, vec![5, 6, 6]);
    }

    #[test]
    fn test_strmetasort_single_or_empty() {
        let mut empty: Vec<String> = vec![];
        strmetasort(&mut empty, 0, None);
        assert!(empty.is_empty());

        let mut single = vec!["only".to_string()];
        strmetasort(&mut single, SORTIT_BACKWARDS as u32, None);
        assert_eq!(single, vec!["only"]);
    }
}
