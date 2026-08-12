//! Zsh string sorting — direct port of `Src/sort.c`.
//!
//! Provides comparison and sorting for shell strings with the same
//! flag vocabulary the C source uses (`SORTIT_*` from Src/zsh.h:2993).
//! Three public entry points:
//!
//! * [`zstrcmp`]: pairwise comparator (front-end to [`eltpcmp`]).
//! * [`eltpcmp`]: the qsort callback over [`sortelt`] pairs.
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

use crate::ported::zsh_h::sortelt;
use crate::zsh_h::{
    SORTIT_ANYOLDHOW, SORTIT_BACKWARDS, SORTIT_IGNORING_BACKSLASHES, SORTIT_IGNORING_CASE,
    SORTIT_NUMERICALLY, SORTIT_NUMERICALLY_SIGNED, SORTIT_SOMEHOW,
};
use libc;
use std::cmp::Ordering;
use std::ffi::CString;

/// Port of `eltpcmp(const void *a, const void *b)` from `Src/sort.c:44`.
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
/// WARNING: param names don't match C — Rust=(a, b, sort_flags) vs C=(a, b)
pub fn eltpcmp(a: &sortelt, b: &sortelt, sort_flags: u32) -> Ordering {
    // c:44
    let reverse = (sort_flags & (SORTIT_BACKWARDS as u32)) != 0;
    // C's `len == -1` sentinel = "no embedded NULs, use strlen".
    let a_has_len = a.len >= 0;
    let b_has_len = b.len >= 0;
    let result = if !a_has_len && !b_has_len {
        zstrcmp(&a.cmp, &b.cmp, sort_flags & !(SORTIT_BACKWARDS as u32))
    } else {
        // Embedded-null path: compare first min(a.len, b.len)
        // bytes; equal-prefix-but-different-length means the
        // shorter sorts lower.
        let la = if a_has_len {
            a.len as usize
        } else {
            a.cmp.len()
        };
        let lb = if b_has_len {
            b.len as usize
        } else {
            b.cmp.len()
        };
        let len = la.min(lb);
        let ab = a.cmp.as_bytes();
        let bb = b.cmp.as_bytes();
        let take_a = ab.len().min(len);
        let take_b = bb.len().min(len);
        match ab[..take_a].cmp(&bb[..take_b]) {
            Ordering::Equal => match (a_has_len, b_has_len) {
                (true, true) => la.cmp(&lb),
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => Ordering::Equal,
            },
            o => o,
        }
    };
    if reverse {
        result.reverse()
    } else {
        result
    }
}

/// Port of `int zstrcmp(const char *as, const char *bs, int sortflags)` from
/// `Src/sort.c:191`.
///
/// **Structural divergence from C:** C `zstrcmp` is a 20-line wrapper
/// that sets `sortdir`/`sortnobslash`/`sortnumeric` globals then calls
/// `eltpcmp(&aeptr, &beptr)` (which is the comparator). The Rust port
/// has those roles **inverted**: `zstrcmp` holds the full comparator
/// body (numeric run-detect, backslash strip, strcoll fallback) and
/// `eltpcmp` is the wrapper that handles embedded-NUL `len` paths
/// then delegates to `zstrcmp`. Net work matches C; only the name
/// holding the body is swapped. Restructuring to match C role
/// assignment requires moving the comparator body into `eltpcmp` and
/// extending the latter's signature to accept the flag bits directly
/// (tracked as a follow-up; not a behavioral bug).
/// WARNING: param names don't match C — Rust=(a, bs, sortflags) vs C=(as, bs, sortflags)
pub fn zstrcmp(a: &str, bs: &str, sortflags: u32) -> Ordering {
    // c:191
    let sortnumeric = if sortflags & (SORTIT_NUMERICALLY_SIGNED as u32) != 0 {
        -1 // c:209-210
    } else if sortflags & (SORTIT_NUMERICALLY as u32) != 0 {
        1
    } else {
        0
    };
    let numeric = sortnumeric != 0;
    let numeric_signed = sortnumeric < 0;
    let no_backslash = (sortflags & (SORTIT_IGNORING_BACKSLASHES as u32)) != 0;

    // c:120-131 — `sortnobslash`. C skips AT MOST ONE backslash on each side
    // per aligned position, breaks at the first mismatch, and then collates the
    // RAW remainders from the divergence point (c:134). It is NOT a strip-all:
    //
    //     while (*as && *bs) {
    //         if (*as == '\\') as++;
    //         if (*bs == '\\') bs++;
    //         if (*as != *bs || !*as) break;
    //         as++; bs++;
    //     }
    //
    // The two agree on an escaped metacharacter (`\ `, `\(`) — both yield the
    // bare character. They diverge on a DOUBLED `\\`, i.e. a filename holding a
    // literal backslash: strip-all deletes both, while C consumes one and
    // leaves a 0x5C standing, which collates after `Z` (0x5A) and before `d`
    // (0x64). That is the whole `cd ~/` divergence — a directory named
    // `\EurydiceCM\` sorted to index 11 (between `Downloads/` and `Exercism/`)
    // where zsh puts it at 47 (between `Zotero/` and `dotTraceSnapshots/`).
    // Both shells' full 59-entry orders were reproduced from these two
    // algorithms, so the model is pinned rather than inferred.
    //
    // SCOPE: applied to the collation path only. C's numeric block (c:137-172)
    // rewinds with `for (; as > ao && idigit(as[-1]); as--, bs--)` where `ao`
    // is the ORIGINAL string start captured at c:49 — before this loop runs —
    // so handing it these remainders without that bound would corrupt
    // numeric-glob-sort. The numeric path therefore keeps its existing
    // behaviour untouched; `SORTIT_NUMERICALLY | SORTIT_IGNORING_BACKSLASHES`
    // together remain an approximation, and knowingly so.
    // c:120-134 — C advances the `as`/`bs` POINTERS and hands the
    // remainders straight to `strcoll`; the comparator allocates
    // nothing. This is a sort comparator, called O(n log n) times (a
    // 46765-match `compadd -k functions` completion means ~725k calls),
    // so the port's unconditional `to_string()` pair — plus a second
    // pair on the strip path — turned every comparison into four heap
    // allocations. Borrow instead, and own only on the fallback arm
    // that genuinely rewrites the bytes.
    let mut a_str: &str = a;
    let mut b_str: &str = bs;
    let a_owned: String;
    let b_owned: String;
    if no_backslash {
        let mut done = false;
        if !numeric {
            let (ab, bb) = (a.as_bytes(), bs.as_bytes());
            let (mut i, mut j) = (0usize, 0usize);
            while i < ab.len() && j < bb.len() {
                if ab[i] == b'\\' {
                    i += 1; // c:122-123
                }
                if bb[j] == b'\\' {
                    j += 1; // c:124-125
                }
                // c:126-127 — `if (*as != *bs || !*as) break;`. A run off the
                // end of either side is C reading its NUL terminator, which
                // fails the equality test just the same.
                if i >= ab.len() || j >= bb.len() || ab[i] != bb[j] {
                    break;
                }
                i += 1; // c:128-129
                j += 1;
            }
            // Byte indices only land off a char boundary if a multibyte
            // sequence straddled the divergence; slicing there would panic, so
            // fall through to the old whole-string form in that case.
            if let (Some(ar), Some(br)) = (a.get(i..), bs.get(j..)) {
                a_str = ar;
                b_str = br;
                done = true;
            }
        }
        if !done {
            a_owned = a_str.chars().filter(|&c| c != '\\').collect();
            b_owned = b_str.chars().filter(|&c| c != '\\').collect();
            a_str = &a_owned;
            b_str = &b_owned;
        }
    }
    // NOTE: c:Src/sort.c::zstrcmp does NOT honor SORTIT_IGNORING_CASE.
    // The case-fold happens in strmetasort's pre-pass at c:290-372
    // (lowercase into a separate buffer before eltpcmp + strcoll/strcmp).
    // Callers wanting case-insensitive sort must pre-lower or route
    // through strmetasort. Companion test
    // `test_zstrcmp_ignores_case_flag_per_c` pins this behavior.

    // c:134 — `cmp = strcoll(as, bs)`. zsh ALWAYS computes the locale
    // collation first; numeric mode only OVERRIDES it when a digit run
    // is involved at the divergence point (sort.c:137-172 leaves `cmp`
    // untouched otherwise, since the byte-diff recompute is
    // `#ifndef HAVE_STRCOLL`). strcoll gives the case-insensitive
    // primary ordering, so non-numeric portions of a `(n)`-sorted array
    // must use it too — `${(n)a}` of `banana Mango apple zebra` sorts
    // case-insensitively just like `${(o)a}`.
    let strcoll_cmp = |a: &str, b: &str| -> Ordering {
        #[cfg(unix)]
        {
            // !!! RUST-ONLY ADAPTER — NO C COUNTERPART !!!
            // c:134 `cmp = strcoll(as, bs)` — C's operands are already
            // NUL-terminated `char *`, so the collation call copies
            // nothing. A Rust `&str` is not NUL-terminated, so a copy is
            // unavoidable; keep it on the STACK for the short strings
            // that dominate here (match names, filenames, array
            // elements) and fall back to a heap `CString` only when one
            // does not fit. zstrcmp is a SORT COMPARATOR — a 46765-entry
            // `compadd -k functions` completion runs it ~725k times — so
            // the two `CString::new` allocations this replaces were the
            // single hottest symbol in the completion profile.
            const SCRATCH: usize = 256;
            let mut abuf = [0u8; SCRATCH];
            let mut bbuf = [0u8; SCRATCH];
            // C stops at the NUL terminator; mirror that by truncating at
            // the first embedded 0 byte (same rule the old `cstr_head`
            // applied) and reject anything too long for the buffer.
            let fill = |s: &str, buf: &mut [u8; SCRATCH]| -> bool {
                let sb = s.as_bytes();
                let n = sb.iter().position(|&x| x == 0).unwrap_or(sb.len());
                if n >= SCRATCH {
                    return false;
                }
                buf[..n].copy_from_slice(&sb[..n]);
                buf[n] = 0;
                true
            };
            if fill(a, &mut abuf) && fill(b, &mut bbuf) {
                let c = unsafe {
                    libc::strcoll(
                        abuf.as_ptr() as *const libc::c_char,
                        bbuf.as_ptr() as *const libc::c_char,
                    )
                };
                return c.cmp(&0);
            }
            let cstr_head = |s: &str| -> CString {
                let bs = s.as_bytes();
                let n = bs.iter().position(|&x| x == 0).unwrap_or(bs.len());
                CString::new(&bs[..n]).unwrap_or_else(|_| CString::new(vec![0u8]).expect("nul"))
            };
            let c = unsafe { libc::strcoll(cstr_head(a).as_ptr(), cstr_head(b).as_ptr()) };
            c.cmp(&0)
        }
        #[cfg(not(unix))]
        {
            a.cmp(b)
        }
    };

    // Numeric comparison — direct port of the `if (sortnumeric)`
    // block at Src/sort.c:137-172. Walks both strings to first
    // divergence, then either short-circuits on signed-mode
    // `-DIGIT` vs `DIGIT`, or rewinds to the start of the digit
    // run, skips leading zeros, compares run lengths (longer =
    // bigger; flipped for negatives via `mul`). When NO digit run is
    // involved (or the runs compare equal) `cmp` keeps the strcoll base
    // (c:134), matching zsh's HAVE_STRCOLL path.
    let cmp_numeric = |a: &str, bs: &str, signed_mode: bool| -> Ordering {
        let ab = a.as_bytes();
        let bb = bs.as_bytes();
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
        let _ = cmp;
        // c:134 — no digit-run override fired (non-digit divergence, or
        // digit runs compared equal). Keep the strcoll base.
        strcoll_cmp(a, bs)
    };

    if numeric {
        cmp_numeric(a_str, b_str, numeric_signed)
    } else {
        strcoll_cmp(a_str, b_str)
    }
}

/// Port of `strmetasort(char **array, int sortwhat, int *unmetalenp)` from `Src/sort.c:234`.
// lengths.                                                                 // c:234
/// C signature: `void strmetasort(char **array, int sortwhat,
/// int *unmetalenp)`. `unmetalenp = None` (i.e. C's `NULL`) means
/// the strings are still metafied (no embedded NULs). When
/// `Some(slice)`, the slice is C's parallel array of per-element
/// pre-unmetafied lengths; after sort it's re-ordered in lockstep
/// with `arr` so the lengths track their owning strings.
/// WARNING: param names don't match C — Rust=(sort_flags, unmetalenp) vs C=(array, sortwhat, unmetalenp)
pub fn strmetasort(
    // c:234
    arr: &mut [String],
    sort_flags: u32,
    unmetalenp: Option<&mut [usize]>,
) {
    if arr.len() < 2 {
        return;
    }

    // Build sortelts up front, applying transforms once (C does the
    // same at sort.c:289-385 inside the prep loop).
    let apply_transforms = |s: &str| -> String {
        let mut t = s.to_string();
        if sort_flags & (SORTIT_IGNORING_CASE as u32) != 0 {
            t = t.to_lowercase(); // c:329-374
        }
        if sort_flags & (SORTIT_IGNORING_BACKSLASHES as u32) != 0 {
            t = t.chars().filter(|&c| c != '\\').collect(); // c:375-385
        }
        t
    };
    let elts: Vec<sortelt> = match unmetalenp.as_deref() {
        Some(lens) => arr
            .iter()
            .zip(lens.iter())
            .map(|(s, &l)| sortelt {
                orig: s.clone(),
                cmp: apply_transforms(s),
                origlen: l as i32,
                len: l as i32,
            })
            .collect(),
        None => arr
            .iter()
            .map(|s| sortelt {
                orig: s.clone(),
                cmp: apply_transforms(s),
                origlen: -1,
                len: -1,
            })
            .collect(),
    };

    // Sort indices so we can remap arr+unmetalenp in lockstep
    // (C's qsort over SortElt* pointers achieves the same).
    let mut indices: Vec<usize> = (0..elts.len()).collect();
    // qsort-tolerant: eltpcmp→zstrcmp numeric/natural sort is not a strict weak
    // order, so Rust's sort_by would PANIC. C uses qsort (unspecified order,
    // never crashes).
    crate::tolerant_sort::qsort_tolerant(&mut indices, |&i, &j| {
        eltpcmp(&elts[i], &elts[j], sort_flags)
    });

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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrcmp("abc", "def", 0), Ordering::Less);
        assert_eq!(zstrcmp("def", "abc", 0), Ordering::Greater);
        assert_eq!(zstrcmp("abc", "abc", 0), Ordering::Equal);
    }

    #[test]
    fn test_zstrcmp_ignores_backwards_per_c() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("abc", "def", SORTIT_BACKWARDS as u32),
            zstrcmp("abc", "def", 0)
        );
    }

    #[test]
    fn test_zstrcmp_ignores_case_flag_per_c() {
        let _g = crate::test_util::global_state_lock();
        let with = zstrcmp("ABC", "abc", SORTIT_IGNORING_CASE as u32);
        let without = zstrcmp("ABC", "abc", 0);
        assert_eq!(with, without);
    }

    #[test]
    fn test_zstrcmp_numeric() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("file2", "file10", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
        assert_eq!(
            zstrcmp("file10", "file2", SORTIT_NUMERICALLY as u32),
            Ordering::Greater
        );
        assert_eq!(
            zstrcmp("100", "20", SORTIT_NUMERICALLY as u32),
            Ordering::Greater
        );
    }

    #[test]
    fn test_zstrcmp_numeric_signed() {
        let _g = crate::test_util::global_state_lock();
        let f = (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32;
        assert_eq!(zstrcmp("-5", "3", f), Ordering::Less);
        assert_eq!(zstrcmp("-10", "-2", f), Ordering::Less);
        assert_eq!(zstrcmp("5", "-3", f), Ordering::Greater);
    }

    #[test]
    fn test_natural_sort() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec![
            "file10".to_string(),
            "file2".to_string(),
            "file1".to_string(),
            "file20".to_string(),
        ];
        strmetasort(
            &mut arr,
            (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32,
            None,
        );
        assert_eq!(arr, vec!["file1", "file2", "file10", "file20"]);
    }

    #[test]
    fn test_strmetasort() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        strmetasort(&mut arr, SORTIT_BACKWARDS as u32, None);
        assert_eq!(arr, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_case_insensitive_sort() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("a\\bc", "abc", SORTIT_IGNORING_BACKSLASHES as u32),
            Ordering::Equal
        );
    }

    #[test]
    fn test_strmetasort_lowercases_via_ignoring_case() {
        let _g = crate::test_util::global_state_lock();
        // Coverage for the case-fold transform inside `strmetasort`'s
        // build-elts pass.
        let mut arr = vec!["BANANA".to_string(), "apple".to_string()];
        strmetasort(&mut arr, SORTIT_IGNORING_CASE as u32, None);
        assert_eq!(arr, vec!["apple", "BANANA"]);
    }

    #[test]
    fn test_strmetasort_strips_backslashes_via_ignoring_bs() {
        let _g = crate::test_util::global_state_lock();
        // Backslash-strip in cmp form lets `\\b` compare equal to `b`.
        let mut arr = vec!["a\\b".to_string(), "ab".to_string()];
        strmetasort(&mut arr, SORTIT_IGNORING_BACKSLASHES as u32, None);
        // Stable on equal: original order preserved when cmp is equal.
        assert_eq!(arr[0], "a\\b");
        assert_eq!(arr[1], "ab");
    }

    #[test]
    fn test_eltpcmp_embedded_null_shorter_sorts_below() {
        let _g = crate::test_util::global_state_lock();
        // Two strings with the same prefix but different lengths.
        // C: "the string that's finished sorts below the other"
        // (sort.c:88-89). With len markers, prefix "abc" + 0 sorts
        // below prefix "abc" + 0 + "d" (the longer continuation).
        let a = sortelt {
            orig: "abc".to_string(),
            cmp: "abc".to_string(),
            origlen: 3,
            len: 3,
        };
        let b = sortelt {
            orig: "abc".to_string(),
            cmp: "abc".to_string(),
            origlen: 5,
            len: 5,
        };
        assert_eq!(eltpcmp(&a, &b, 0), Ordering::Less);
        assert_eq!(eltpcmp(&b, &a, 0), Ordering::Greater);
    }

    #[test]
    fn test_strmetasort_reorders_lens_in_lockstep() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut empty: Vec<String> = vec![];
        strmetasort(&mut empty, 0, None);
        assert!(empty.is_empty());

        let mut single = vec!["only".to_string()];
        strmetasort(&mut single, SORTIT_BACKWARDS as u32, None);
        assert_eq!(single, vec!["only"]);
    }

    // ═══════════════════════════════════════════════════════════════════
    // zstrcmp — sort comparator with flag modes.
    // ═══════════════════════════════════════════════════════════════════

    use std::cmp::Ordering;

    /// Plain string compare — equal strings → Equal.
    #[test]
    fn zstrcmp_equal_strings_return_equal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("foo", "foo", SORTIT_ANYOLDHOW as u32),
            Ordering::Equal
        );
    }

    /// Default mode: lex compare — "apple" < "banana".
    #[test]
    fn zstrcmp_lex_order_default() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("apple", "banana", SORTIT_ANYOLDHOW as u32),
            Ordering::Less
        );
        assert_eq!(
            zstrcmp("banana", "apple", SORTIT_ANYOLDHOW as u32),
            Ordering::Greater
        );
    }

    /// Default mode: case-sensitive — "ABC" < "abc" (ASCII).
    #[test]
    fn zstrcmp_default_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("ABC", "abc", SORTIT_ANYOLDHOW as u32),
            Ordering::Less
        );
    }

    /// IGNORING_CASE on zstrcmp directly is a NO-OP per C semantics:
    /// `c:Src/sort.c::zstrcmp` doesn't honor the flag itself. The
    /// case-fold happens in strmetasort's pre-pass (c:290-372) which
    /// lowercases each element into a separate buffer before calling
    /// the comparator. zsh's `(io)` sort flow goes through strmetasort
    /// so the user-visible result IS case-insensitive — but at this
    /// single-comparator level, "ABC" still sorts Less than "abc"
    /// because they're byte-different. Companion test
    /// `test_zstrcmp_ignores_case_flag_per_c` pins the same contract.
    #[test]
    fn zstrcmp_ignore_case_makes_abc_equal_to_uppercase_anchored() {
        let _g = crate::test_util::global_state_lock();
        // strcoll under the test runner's locale returns Less for ABC<abc.
        let with = zstrcmp("ABC", "abc", SORTIT_IGNORING_CASE as u32);
        let without = zstrcmp("ABC", "abc", SORTIT_ANYOLDHOW as u32);
        assert_eq!(
            with, without,
            "c:zstrcmp ignores IGNORING_CASE; pre-pass lives in strmetasort"
        );
    }

    /// IGNORING_CASE: "AbC" < "abd".
    #[test]
    fn zstrcmp_ignore_case_still_compares_differing_letters() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("AbC", "abd", SORTIT_IGNORING_CASE as u32),
            Ordering::Less
        );
    }

    /// NUMERICALLY: "2" < "10".
    #[test]
    fn zstrcmp_numeric_mode_two_less_than_ten() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("2", "10", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
    }

    /// Plain lex: "10" < "2".
    #[test]
    fn zstrcmp_lex_mode_ten_less_than_two() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrcmp("10", "2", SORTIT_ANYOLDHOW as u32), Ordering::Less);
    }

    /// Numeric mode handles embedded numbers: "file2" < "file10".
    #[test]
    fn zstrcmp_numeric_mode_embedded_numbers() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("file2", "file10", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
    }

    /// Numeric mode falls back to lex for no-digit prefix tie.
    #[test]
    fn zstrcmp_numeric_mode_no_digits_falls_back_to_lex() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("abc", "abd", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
    }

    /// Both empty → Equal.
    #[test]
    fn zstrcmp_both_empty_returns_equal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrcmp("", "", SORTIT_ANYOLDHOW as u32), Ordering::Equal);
    }

    /// Empty < non-empty.
    #[test]
    fn zstrcmp_empty_less_than_non_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrcmp("", "x", SORTIT_ANYOLDHOW as u32), Ordering::Less);
        assert_eq!(zstrcmp("x", "", SORTIT_ANYOLDHOW as u32), Ordering::Greater);
    }

    /// "foo" < "foobar" (prefix is less).
    #[test]
    fn zstrcmp_prefix_is_less_than_longer_string() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("foo", "foobar", SORTIT_ANYOLDHOW as u32),
            Ordering::Less
        );
    }

    /// Symmetry — sign(cmp(a,b)) == -sign(cmp(b,a)).
    #[test]
    fn zstrcmp_is_antisymmetric() {
        let _g = crate::test_util::global_state_lock();
        for (a, b) in [
            ("alpha", "beta"),
            ("file2", "file10"),
            ("", "x"),
            ("foo", "foobar"),
        ] {
            let ab = zstrcmp(a, b, SORTIT_ANYOLDHOW as u32);
            let ba = zstrcmp(b, a, SORTIT_ANYOLDHOW as u32);
            assert_eq!(ab, ba.reverse(), "antisymmetry for ({a:?}, {b:?})");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/sort.c flag-mode edge cases.
    // ═══════════════════════════════════════════════════════════════════

    /// NUMERICALLY without SIGNED: leading minus is treated as part of
    /// the lex prefix, not a sign. C `sort.c:eltpcmp` checks
    /// `SORTIT_NUMERICALLY_SIGNED` separately for sign-aware parse.
    #[test]
    fn zstrcmp_numeric_unsigned_treats_minus_as_lex() {
        let _g = crate::test_util::global_state_lock();
        // Without SIGNED, "-5" and "5" both fall through; "-" sorts as
        // its byte value vs the digit-start.
        let ord = zstrcmp("-5", "5", SORTIT_NUMERICALLY as u32);
        // "-" (0x2D) < "5" (0x35) lex-wise → Less.
        assert_eq!(ord, Ordering::Less);
    }

    /// NUMERICALLY: trailing tail after the digit run continues lex.
    /// "file2bc" vs "file2bd" → Less (after numeric 2==2, lex 'c'<'d').
    #[test]
    fn zstrcmp_numeric_continues_lex_after_digits() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrcmp("file2bc", "file2bd", SORTIT_NUMERICALLY as u32),
            Ordering::Less
        );
    }

    /// NUMERICALLY: leading zeros don't change numeric value;
    /// "file02" == "file2" numerically, then ties broken by length.
    #[test]
    fn zstrcmp_numeric_leading_zeros_compare_equal_value() {
        let _g = crate::test_util::global_state_lock();
        // C: numeric compare strips leading zeros; same value → fall
        // through to lex on the surrounding bytes (length differs).
        let ord = zstrcmp("file02", "file2", SORTIT_NUMERICALLY as u32);
        // Sanity: not Greater (shorter equal-value should sort low).
        assert_ne!(ord, Ordering::Greater);
    }

    /// strmetasort with SORTIT_BACKWARDS reverses output order.
    #[test]
    fn strmetasort_backwards_reverses_lex_order() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        strmetasort(&mut arr, SORTIT_BACKWARDS as u32, None);
        assert_eq!(arr, vec!["gamma", "beta", "alpha"]);
    }

    /// strmetasort is stable across equal cmp keys (case-insensitive
    /// sort preserves original order of "apple" vs "Apple").
    #[test]
    fn strmetasort_stable_on_equal_keys() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec![
            "apple".to_string(),
            "Apple".to_string(),
            "APPLE".to_string(),
        ];
        strmetasort(&mut arr, SORTIT_IGNORING_CASE as u32, None);
        assert_eq!(arr[0], "apple");
        assert_eq!(arr[1], "Apple");
        assert_eq!(arr[2], "APPLE");
    }

    /// IGNORING_BACKSLASHES strips backslashes from cmp form, so
    /// "a\\b\\c" and "abc" compare equal at strmetasort level.
    #[test]
    fn strmetasort_ignore_backslashes_equates_escaped_unescaped() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["a\\b\\c".to_string(), "abc".to_string()];
        strmetasort(&mut arr, SORTIT_IGNORING_BACKSLASHES as u32, None);
        // Both compare equal → stable order preserves original.
        assert_eq!(arr[0], "a\\b\\c");
        assert_eq!(arr[1], "abc");
    }

    /// strmetasort with empty unmetalenp slice should not panic.
    #[test]
    fn strmetasort_empty_unmetalenp_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec![];
        let mut lens: Vec<usize> = vec![];
        strmetasort(&mut arr, 0, Some(&mut lens));
        assert!(arr.is_empty());
        assert!(lens.is_empty());
    }

    /// eltpcmp with reverse flag inverts ordering result.
    #[test]
    fn eltpcmp_backwards_inverts_ordering() {
        let _g = crate::test_util::global_state_lock();
        let a = sortelt {
            orig: "a".to_string(),
            cmp: "a".to_string(),
            origlen: 1,
            len: -1,
        };
        let b = sortelt {
            orig: "b".to_string(),
            cmp: "b".to_string(),
            origlen: 1,
            len: -1,
        };
        let fwd = eltpcmp(&a, &b, 0);
        let rev = eltpcmp(&a, &b, SORTIT_BACKWARDS as u32);
        assert_eq!(fwd, Ordering::Less);
        assert_eq!(rev, Ordering::Greater);
    }

    /// Identical elts compare Equal even under reverse — reverse of
    /// Equal is still Equal (preserves stability anchor).
    #[test]
    fn eltpcmp_equal_under_reverse_stays_equal() {
        let _g = crate::test_util::global_state_lock();
        let a = sortelt {
            orig: "x".to_string(),
            cmp: "x".to_string(),
            origlen: 1,
            len: -1,
        };
        let b = sortelt {
            orig: "x".to_string(),
            cmp: "x".to_string(),
            origlen: 1,
            len: -1,
        };
        assert_eq!(eltpcmp(&a, &b, SORTIT_BACKWARDS as u32), Ordering::Equal);
    }

    /// strmetasort on already-sorted input returns identical order
    /// (idempotency / no-op).
    #[test]
    fn strmetasort_already_sorted_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let expected = arr.clone();
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, expected);
    }

    /// Numeric mode with all-digit strings sorts by numeric value.
    #[test]
    fn strmetasort_numeric_sorts_by_value() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["100".to_string(), "20".to_string(), "3".to_string()];
        strmetasort(
            &mut arr,
            (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32,
            None,
        );
        assert_eq!(arr, vec!["3", "20", "100"]);
    }

    /// Numeric+backwards combined: descending numeric order.
    #[test]
    fn strmetasort_numeric_backwards_descends() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["1".to_string(), "10".to_string(), "2".to_string()];
        strmetasort(
            &mut arr,
            (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED | SORTIT_BACKWARDS) as u32,
            None,
        );
        assert_eq!(arr, vec!["10", "2", "1"]);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/sort.c
    // c:55 eltpcmp / c:114 zstrcmp / c:283 strmetasort
    // ═══════════════════════════════════════════════════════════════════

    /// c:114 — `zstrcmp(a, a)` is Equal (reflexive) for arbitrary input.
    #[test]
    fn zstrcmp_reflexive_full_sweep() {
        for s in ["", "a", "abc", "1", "100", "Hello World", "日本"] {
            assert_eq!(
                zstrcmp(s, s, 0),
                Ordering::Equal,
                "zstrcmp({:?}, {:?}) must be Equal",
                s,
                s
            );
        }
    }

    /// c:114 — `zstrcmp` is antisymmetric: if a<b then b>a.
    #[test]
    fn zstrcmp_antisymmetric() {
        let pairs = [("a", "b"), ("apple", "banana"), ("", "x"), ("abc", "abd")];
        for (a, b) in pairs {
            let ab = zstrcmp(a, b, 0);
            let ba = zstrcmp(b, a, 0);
            assert_eq!(
                ab.reverse(),
                ba,
                "zstrcmp must be antisymmetric for ({:?}, {:?})",
                a,
                b
            );
        }
    }

    /// c:114 — `zstrcmp` is deterministic.
    #[test]
    fn zstrcmp_is_deterministic() {
        for (a, b) in [("foo", "bar"), ("a", "z"), ("100", "20")] {
            let first = zstrcmp(a, b, 0);
            for _ in 0..5 {
                assert_eq!(
                    zstrcmp(a, b, 0),
                    first,
                    "zstrcmp({:?}, {:?}) must be deterministic",
                    a,
                    b
                );
            }
        }
    }

    /// c:114 — `zstrcmp` numeric mode: leading zeros differ from bare digits
    /// (007 != 7 in zsh — uses string comparison after numeric prefix consumed).
    #[test]
    fn zstrcmp_numeric_leading_zeros_string_compare() {
        let r = zstrcmp(
            "007",
            "7",
            (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED) as u32,
        );
        // zsh numeric mode compares the digit-runs as numbers first, but
        // ties fall through to lex; "007" vs "7" both equal numeric 7 yet
        // lex-order differs by leading zeros → non-Equal.
        assert_ne!(
            r,
            Ordering::Equal,
            "007 vs 7 differ after numeric tie via lex fallback"
        );
    }

    /// c:114 — `zstrcmp` flag-0 (lex mode) is case-sensitive.
    #[test]
    fn zstrcmp_lex_mode_case_sensitive() {
        let r = zstrcmp("abc", "ABC", 0);
        assert_ne!(r, Ordering::Equal, "lex mode must distinguish ABC from abc");
    }

    /// c:283 — `strmetasort` empty array is no-op.
    #[test]
    fn strmetasort_empty_array_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec![];
        strmetasort(&mut arr, 0, None);
        assert!(arr.is_empty());
    }

    /// c:283 — `strmetasort` single-element array is unchanged.
    #[test]
    fn strmetasort_single_element_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["only".to_string()];
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, vec!["only"]);
    }

    /// c:283 — `strmetasort` idempotent: sort-then-sort = sort.
    #[test]
    fn strmetasort_double_sort_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["c", "a", "b", "d"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        strmetasort(&mut arr, 0, None);
        let after_first = arr.clone();
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, after_first, "double sort must be idempotent");
    }

    /// c:55 — `eltpcmp` is deterministic for the same input pair.
    #[test]
    fn eltpcmp_is_deterministic() {
        let a = sortelt {
            orig: "a".to_string(),
            cmp: "a".to_string(),
            origlen: 1,
            len: 1,
        };
        let b = sortelt {
            orig: "b".to_string(),
            cmp: "b".to_string(),
            origlen: 1,
            len: 1,
        };
        let first = eltpcmp(&a, &b, 0);
        for _ in 0..5 {
            assert_eq!(eltpcmp(&a, &b, 0), first, "eltpcmp must be deterministic");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/sort.c
    // c:55 eltpcmp / c:114 zstrcmp / c:283 strmetasort
    // ═══════════════════════════════════════════════════════════════════

    /// c:55 — `eltpcmp` returns Ordering (compile-time pin).
    #[test]
    fn eltpcmp_returns_ordering_type() {
        let a = sortelt {
            orig: "a".into(),
            cmp: "a".into(),
            origlen: 1,
            len: 1,
        };
        let b = sortelt {
            orig: "b".into(),
            cmp: "b".into(),
            origlen: 1,
            len: 1,
        };
        let _: Ordering = eltpcmp(&a, &b, 0);
    }

    /// c:55 — `eltpcmp(a, a, _)` (same element) returns Equal.
    #[test]
    fn eltpcmp_same_element_returns_equal() {
        let a = sortelt {
            orig: "x".into(),
            cmp: "x".into(),
            origlen: 1,
            len: 1,
        };
        assert_eq!(
            eltpcmp(&a, &a, 0),
            Ordering::Equal,
            "comparing element with itself must return Equal"
        );
    }

    /// c:55 — `eltpcmp(a, b, _)` and `eltpcmp(b, a, _)` are opposites
    /// (antisymmetry property of cmp functions).
    #[test]
    fn eltpcmp_antisymmetric() {
        let a = sortelt {
            orig: "a".into(),
            cmp: "a".into(),
            origlen: 1,
            len: 1,
        };
        let b = sortelt {
            orig: "b".into(),
            cmp: "b".into(),
            origlen: 1,
            len: 1,
        };
        let ab = eltpcmp(&a, &b, 0);
        let ba = eltpcmp(&b, &a, 0);
        assert_eq!(
            ab.reverse(),
            ba,
            "cmp(a,b) must be reverse of cmp(b,a); got {:?} vs {:?}",
            ab,
            ba
        );
    }

    /// c:114 — `zstrcmp` returns Ordering (compile-time pin).
    #[test]
    fn zstrcmp_returns_ordering_type() {
        let _: Ordering = zstrcmp("a", "b", 0);
    }

    /// c:114 — `zstrcmp("", "")` returns Equal (empty=empty, alt).
    #[test]
    fn zstrcmp_both_empty_returns_equal_alt() {
        assert_eq!(zstrcmp("", "", 0), Ordering::Equal);
    }

    /// c:114 — `zstrcmp(x, x, _)` returns Equal (identity).
    #[test]
    fn zstrcmp_identity_returns_equal() {
        for s in ["", "a", "abc", "hello world", "日本"] {
            assert_eq!(
                zstrcmp(s, s, 0),
                Ordering::Equal,
                "identity zstrcmp({:?}, {:?}, 0) must be Equal",
                s,
                s
            );
        }
    }

    /// c:114 — `zstrcmp` antisymmetric: cmp(a, b) == reverse(cmp(b, a)) (alt).
    #[test]
    fn zstrcmp_antisymmetric_alt() {
        for (a, b) in &[("a", "b"), ("abc", "abd"), ("", "x"), ("xy", "x")] {
            let ab = zstrcmp(a, b, 0);
            let ba = zstrcmp(b, a, 0);
            assert_eq!(
                ab.reverse(),
                ba,
                "antisymmetry: zstrcmp({:?},{:?}) = {:?} ≠ reverse({:?})",
                a,
                b,
                ab,
                ba
            );
        }
    }

    /// c:283 — `strmetasort` returns void (compile-time pin).
    #[test]
    fn strmetasort_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec![];
        let _: () = strmetasort(&mut arr, 0, None);
    }

    /// c:283 — `strmetasort` sorts ascending lexically by default.
    #[test]
    fn strmetasort_sorts_lex_ascending() {
        let _g = crate::test_util::global_state_lock();
        let mut arr = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, vec!["a", "b", "c"], "default lex ascending");
    }

    /// c:283 — `strmetasort` preserves length (no element drop/duplicate).
    #[test]
    fn strmetasort_preserves_length() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = (0..20).map(|i| format!("item_{}", i)).collect();
        let before_len = arr.len();
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr.len(), before_len, "length must be preserved");
    }

    /// c:283 — `strmetasort` of already-sorted is no-op.
    #[test]
    fn strmetasort_already_sorted_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let before = arr.clone();
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, before, "already-sorted unchanged");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/sort.c
    // c:55 eltpcmp / c:114 zstrcmp / c:283 strmetasort /
    // SORTIT_* flag invariants
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/zsh.h:2993` — `SORTIT_ANYOLDHOW = 0` (sentinel for "no flags").
    #[test]
    fn sortit_anyoldhow_is_zero_sentinel() {
        assert_eq!(SORTIT_ANYOLDHOW, 0);
    }

    /// `Src/zsh.h:2993` — SORTIT_* (excluding sentinel) are pairwise distinct.
    #[test]
    fn sortit_flags_pairwise_distinct() {
        let bits = [
            SORTIT_IGNORING_CASE,
            SORTIT_NUMERICALLY,
            SORTIT_NUMERICALLY_SIGNED,
            SORTIT_BACKWARDS,
            SORTIT_IGNORING_BACKSLASHES,
            SORTIT_SOMEHOW,
        ];
        let unique: std::collections::HashSet<_> = bits.iter().copied().collect();
        assert_eq!(
            unique.len(),
            bits.len(),
            "SORTIT_* flags must be pairwise distinct"
        );
    }

    /// `Src/zsh.h:2993` — every non-sentinel SORTIT_* is a single bit.
    #[test]
    fn sortit_flags_all_powers_of_two() {
        for v in [
            SORTIT_IGNORING_CASE,
            SORTIT_NUMERICALLY,
            SORTIT_NUMERICALLY_SIGNED,
            SORTIT_BACKWARDS,
            SORTIT_IGNORING_BACKSLASHES,
            SORTIT_SOMEHOW,
        ] {
            assert!(
                (v as u32).is_power_of_two(),
                "SORTIT_* {} must be single bit",
                v
            );
        }
    }

    /// `Src/zsh.h:2993` — SORTIT_* OR covers bits 0..=5 (63).
    #[test]
    fn sortit_flags_or_covers_low_6_bits() {
        let or_all = SORTIT_IGNORING_CASE
            | SORTIT_NUMERICALLY
            | SORTIT_NUMERICALLY_SIGNED
            | SORTIT_BACKWARDS
            | SORTIT_IGNORING_BACKSLASHES
            | SORTIT_SOMEHOW;
        assert_eq!(or_all, 63, "SORTIT_* must cover bits 0..=5");
    }

    /// c:114 — `zstrcmp` reflexivity for non-empty strings.
    #[test]
    fn zstrcmp_reflexive_non_empty() {
        for s in ["a", "abc", "  ", "hello world"] {
            assert_eq!(
                zstrcmp(s, s, 0),
                Ordering::Equal,
                "zstrcmp({:?}, {:?}, 0) must be Equal",
                s,
                s
            );
        }
    }

    /// c:114 — `zstrcmp` does NOT honor SORTIT_BACKWARDS; reversal is
    /// applied by `eltpcmp` (Src/sort.c:55) BEFORE delegating to
    /// `zstrcmp`. Pin the documented "flag is ignored here" contract.
    #[test]
    fn zstrcmp_does_not_honor_backwards_flag() {
        let normal = zstrcmp("a", "b", 0);
        let with_back = zstrcmp("a", "b", SORTIT_BACKWARDS as u32);
        assert_eq!(
            normal, with_back,
            "zstrcmp must not invert on BACKWARDS — that's eltpcmp's job"
        );
    }

    /// c:114 — `zstrcmp` does NOT honor SORTIT_IGNORING_CASE; case
    /// fold happens in `strmetasort` pre-pass (c:290-372). Pin the
    /// documented "flag is ignored here" contract.
    #[test]
    fn zstrcmp_does_not_honor_ignoring_case_flag() {
        let no_flag = zstrcmp("ABC", "abc", 0);
        let with_flag = zstrcmp("ABC", "abc", SORTIT_IGNORING_CASE as u32);
        assert_eq!(
            no_flag, with_flag,
            "zstrcmp must not case-fold; flag is no-op at this level"
        );
    }

    /// c:114 — `zstrcmp` numeric flag orders "2" < "10" (not lex).
    #[test]
    fn zstrcmp_numeric_orders_two_before_ten() {
        let r = zstrcmp("2", "10", SORTIT_NUMERICALLY as u32);
        assert_eq!(
            r,
            Ordering::Less,
            "numeric sort: 2 < 10 (lex would say 1 < 2)"
        );
    }

    /// c:283 — `strmetasort` sorts longer corpus correctly.
    #[test]
    fn strmetasort_corpus_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec![
            "zebra".into(),
            "apple".into(),
            "mango".into(),
            "banana".into(),
        ];
        strmetasort(&mut arr, 0, None);
        assert_eq!(
            arr,
            vec![
                "apple".to_string(),
                "banana".into(),
                "mango".into(),
                "zebra".into()
            ]
        );
    }

    /// c:283 — `strmetasort` empty input doesn't panic.
    #[test]
    fn strmetasort_empty_input_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = Vec::new();
        strmetasort(&mut arr, 0, None);
        assert!(arr.is_empty());
    }

    /// c:283 — `strmetasort` single-element input is no-op.
    #[test]
    fn strmetasort_single_element_no_op() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec!["only".into()];
        strmetasort(&mut arr, 0, None);
        assert_eq!(arr, vec!["only".to_string()]);
    }

    /// c:283 — `strmetasort` backwards flag reverses order (alt).
    #[test]
    fn strmetasort_backwards_reverses_lex_order_alt() {
        let _g = crate::test_util::global_state_lock();
        let mut arr: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        strmetasort(&mut arr, SORTIT_BACKWARDS as u32, None);
        assert_eq!(arr, vec!["c".to_string(), "b".into(), "a".into()]);
    }
}
