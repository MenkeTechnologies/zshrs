//! Adversarial tests for `zsh::sort` flag combinations
//! (`src/ported/sort.rs`, port of `Src/sort.c`).
//!
//! The in-module tests in `src/ported/sort.rs` cover each `SORTIT_*`
//! flag in isolation (forward-only zstrcmp / a single-flag
//! strmetasort). The flags are bitmask-combined in real callers
//! — `${(no)array}` is `SORTIT_NUMERICALLY | SORTIT_BACKWARDS`,
//! `${(noi)array}` adds `SORTIT_IGNORING_CASE`, etc. None of those
//! combinations are pinned. The tests below target three specific
//! correctness invariants that combination paths must satisfy:
//!
//! 1. `eltpcmp(BACKWARDS)` MUST invert the embedded-NUL
//!    "shorter sorts below" rule.
//! 2. `strmetasort(NUMERICALLY | BACKWARDS)` MUST produce
//!    descending natural order — not lexical-reverse and not
//!    ascending-natural.
//! 3. `strmetasort(IGNORING_CASE | BACKWARDS)` MUST preserve the
//!    case-fold equivalence under reversal — `"Banana"` and
//!    `"apple"` compare via lowered cmp form, then the result
//!    reverses *after* the comparator. Bug class: a regen that
//!    reverses BEFORE the case-fold transform would produce a
//!    different ordering.

use std::cmp::Ordering;
use zsh::sort::{eltpcmp, strmetasort};
use zsh::zsh_h::{
    sortelt, SORTIT_BACKWARDS, SORTIT_IGNORING_CASE, SORTIT_NUMERICALLY, SORTIT_NUMERICALLY_SIGNED,
};

fn elt(s: &str, len: i32) -> sortelt {
    sortelt {
        orig: s.to_string(),
        cmp: s.to_string(),
        origlen: len,
        len,
    }
}

/// `eltpcmp` with embedded-NUL `len` markers MUST reverse the
/// "equal prefix, shorter sorts below" rule when `SORTIT_BACKWARDS`
/// is set. Existing test `test_eltpcmp_embedded_null_shorter_sorts_below`
/// pins forward direction (a.len=3, b.len=5 → Less). The
/// `SORTIT_BACKWARDS` flag MUST flip that to Greater. Pre-fix bug
/// class: a regen that reverses ONLY the lex byte-compare result
/// but leaves the `(true,true) => la.cmp(&lb)` length-tiebreak
/// branch un-reversed would produce wrong ordering when equal
/// prefixes hit the tiebreaker.
#[test]
fn eltpcmp_backwards_inverts_embedded_null_shorter_sorts_below_rule() {
    let a = elt("abc", 3);
    let b = elt("abc", 5);
    // Forward sanity check — pins the rule we're inverting.
    assert_eq!(
        eltpcmp(&a, &b, 0),
        Ordering::Less,
        "forward direction: shorter equal-prefix sorts below"
    );
    // BACKWARDS MUST invert.
    assert_eq!(
        eltpcmp(&a, &b, SORTIT_BACKWARDS as u32),
        Ordering::Greater,
        "BACKWARDS must invert the embedded-NUL length tiebreaker; \
         a regen that reverses only the byte-compare result and \
         leaves the length tiebreak branch un-reversed would still \
         return Less here"
    );
    // And the swap is symmetric.
    assert_eq!(
        eltpcmp(&b, &a, SORTIT_BACKWARDS as u32),
        Ordering::Less,
        "BACKWARDS reversal must be symmetric on swap"
    );
}

/// `strmetasort(NUMERICALLY | BACKWARDS)` MUST sort by natural-numeric
/// value, descending. Test array `[file2, file10, file1, file20]`:
/// - Ascending natural: `[file1, file2, file10, file20]`
/// - Descending natural: `[file20, file10, file2, file1]`
/// - Lex-reverse (the wrong answer): `[file2, file20, file10, file1]`
///   (because lex-`file2` > lex-`file10` since `'2' > '1'` at byte 4)
///
/// A regen that applies BACKWARDS as a post-sort `Vec::reverse()` of
/// the ascending result would coincidentally produce the right answer
/// here — but a regen that ANDs out `SORTIT_NUMERICALLY` when
/// BACKWARDS is set (e.g. an incorrect `flags & !BACKWARDS` mask
/// applied at the wrong level) would emit lex-reverse, which the
/// final-element assertion below catches.
#[test]
fn strmetasort_combined_numeric_backwards_emits_descending_natural_order() {
    let mut arr = vec![
        "file2".to_string(),
        "file10".to_string(),
        "file1".to_string(),
        "file20".to_string(),
    ];
    strmetasort(
        &mut arr,
        (SORTIT_NUMERICALLY | SORTIT_BACKWARDS) as u32,
        None,
    );
    assert_eq!(
        arr,
        vec!["file20", "file10", "file2", "file1"],
        "NUMERICALLY|BACKWARDS must yield descending natural order"
    );
    // Strict catcher for the lex-reverse failure mode (silent
    // dropping of NUMERICALLY inside the BACKWARDS path):
    assert_eq!(
        arr[1], "file10",
        "second element must be file10 (descending natural), not file20 (lex reverse)"
    );
}

/// `strmetasort(IGNORING_CASE | BACKWARDS)` MUST sort by case-folded
/// comparison and then reverse — `apple` and `Banana` lowercase to
/// `apple < banana` so ascending-case-insensitive is
/// `[apple, Banana, Cherry]`; reversed = `[Cherry, Banana, apple]`.
/// Pre-fix bug class: a regen that reverses BEFORE applying the
/// case-fold transform (or applies the lowercase to `cmp` but skips
/// the post-sort reversal when both flags are set) would produce a
/// different ordering. Existing tests `test_reverse_sort` and
/// `test_case_insensitive_sort` each cover ONE flag alone — neither
/// catches a combination regression.
#[test]
fn strmetasort_combined_case_insensitive_backwards_emits_reverse_of_folded_order() {
    let mut arr = vec![
        "Banana".to_string(),
        "apple".to_string(),
        "Cherry".to_string(),
    ];
    strmetasort(
        &mut arr,
        (SORTIT_IGNORING_CASE | SORTIT_BACKWARDS) as u32,
        None,
    );
    assert_eq!(
        arr,
        vec!["Cherry", "Banana", "apple"],
        "IGNORING_CASE|BACKWARDS must lowercase for compare, then reverse — \
         preserving the original-case forms in the output (orig field), \
         not the lowercased cmp form"
    );
    // Specifically catch a regression that lowercases the OUTPUT
    // strings, not just the comparison form.
    assert!(
        arr.iter().any(|s| s == "Cherry"),
        "output must preserve original case 'Cherry' (not 'cherry'); \
         the case-fold lives on the cmp field only"
    );
}

/// `strmetasort(NUMERICALLY | NUMERICALLY_SIGNED | BACKWARDS)` MUST
/// sort signed-numeric descending. With `[-5, 3, -10, 0]` (as strings
/// since strmetasort takes &mut [String]):
/// - Ascending signed natural: `[-10, -5, 0, 3]`
/// - Descending signed natural: `[3, 0, -5, -10]`
///
/// A regen that drops NUMERICALLY_SIGNED in the BACKWARDS path would
/// treat `-` as a literal char (lex compare with `-` < ASCII digits)
/// and produce a wrong ordering — caught by the position of `-10`
/// being LAST in descending, not first.
#[test]
fn strmetasort_combined_signed_numeric_backwards_descending() {
    let mut arr = vec![
        "-5".to_string(),
        "3".to_string(),
        "-10".to_string(),
        "0".to_string(),
    ];
    strmetasort(
        &mut arr,
        (SORTIT_NUMERICALLY | SORTIT_NUMERICALLY_SIGNED | SORTIT_BACKWARDS) as u32,
        None,
    );
    assert_eq!(
        arr,
        vec!["3", "0", "-5", "-10"],
        "NUMERICALLY|NUMERICALLY_SIGNED|BACKWARDS must yield signed descending"
    );
    assert_eq!(
        arr[3], "-10",
        "most-negative element must be LAST (descending); \
         a regen that drops SIGNED in the BACKWARDS path would \
         put '-10' first via lex order on '-'"
    );
}
