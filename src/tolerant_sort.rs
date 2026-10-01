//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C's completion and parameter-sort code sorts with `qsort`, whose comparator
//! contract is merely "return <0/0/>0"; glibc/BSD `qsort` never inspects the
//! comparator for consistency, so a non-transitive comparator just yields an
//! unspecified-but-valid permutation. Rust's `slice::sort_by` /
//! `sort_unstable_by` instead PANIC ("comparison function does not correctly
//! implement a total order") the moment they detect inconsistency.
//!
//! The natural / numeric ordering in `zstrcmp` (`NUMERICGLOBSORT`, the `(n)`
//! subscript flag, `matchcmp`, `eltpcmp`, `cd_sort`) is a well-known
//! non-transitive comparator, so `sort_by` crashed the shell with a `<TAB>`
//! that completed a large match set. [`qsort_tolerant`] reproduces `qsort`'s
//! tolerance: it never crashes regardless of comparator consistency.

use std::cmp::Ordering;

use crate::metafied_key::MetafiedOperand;
use crate::ported::zle::comp_h::Cmatch;
use crate::ported::zle::compcore::matchcmp;

/// Bottom-up **stable** merge sort that TOLERATES a comparator which is not a
/// strict weak ordering — the stand-in for C's `qsort`. O(n log n), ties keep
/// their input order, never panics on an inconsistent `cmp`.
///
/// The sort runs over indices so a non-transitive `cmp` can only affect the
/// final order — it can never cause an out-of-bounds access or a non-terminating
/// loop.
///
/// !!! MEASURED DIVERGENCE FROM `qsort(3)` — this sort is STABLE, `qsort` is
/// not. `man 3 qsort` (Darwin): "The algorithms implemented by qsort(),
/// qsort_r(), and heapsort() are not stable; that is, if two members compare
/// as equal, their order in the sorted array is undefined." Wherever the C
/// code lets a `qsort` TIE decide something observable, this function answers
/// "input order" and the C answers "whatever the platform libc's quicksort
/// partitioning left first". Reproducing the C answer is not portable: the
/// same zsh binary gives a different answer on a different libc, so there is
/// no single order to port to.
///
/// One such site is measured: `makearray`'s CGF_NOSORT duplicate pass
/// (`compcore.c:3299-3303`, ported at `compcore.rs` `makearray`) sorts a COPY
/// of the match array only to mark the LATER member of each equal pair
/// `CMF_DELETE`, and the surviving member keeps its slot in the DISPLAY array.
/// When the same match set is `compadd`ed twice into one group (`_git`'s
/// `__git_recent_commits` does, once per completer pass), the tie picks which
/// copy dies, so the listing order is the tie order. `git show <TAB><TAB>`
/// lists 20 commits in two ascending runs under zsh/Darwin and in plain
/// insertion order here; the sets and the descriptions are identical.
pub fn qsort_tolerant<T: Clone, F>(v: &mut [T], mut cmp: F)
where
    F: FnMut(&T, &T) -> Ordering,
{
    let n = v.len();
    if n < 2 {
        return;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let mut tmp: Vec<usize> = vec![0usize; n];
    let mut width = 1;
    while width < n {
        let mut i = 0;
        while i < n {
            let left = i;
            let mid = (i + width).min(n);
            let right = (i + 2 * width).min(n);
            let (mut l, mut r, mut k) = (left, mid, left);
            while l < mid && r < right {
                // Take from the left run unless it strictly follows the right
                // run — keeps the sort stable.
                if cmp(&v[idx[l]], &v[idx[r]]) == Ordering::Greater {
                    tmp[k] = idx[r];
                    r += 1;
                } else {
                    tmp[k] = idx[l];
                    l += 1;
                }
                k += 1;
            }
            while l < mid {
                tmp[k] = idx[l];
                l += 1;
                k += 1;
            }
            while r < right {
                tmp[k] = idx[r];
                r += 1;
                k += 1;
            }
            i += 2 * width;
        }
        std::mem::swap(&mut idx, &mut tmp);
        width *= 2;
    }
    // Apply the permutation IN PLACE (`v[i] = old_v[idx[i]]`) by walking each
    // cycle with `swap`. C's `qsort` permutes fixed-size records and allocates
    // nothing; the previous code materialised the result with one deep
    // `clone()` per element and then copied it back, so a 46765-match
    // completion sort deep-copied 46765 `Cmatch` records (a dozen owned
    // `String`s each) for nothing. `swap` moves the records bit-for-bit and
    // touches no heap.
    // `tmp` is finished as merge scratch — reuse it as the cycle-visited
    // marker so the in-place pass allocates nothing at all.
    let visited = &mut tmp;
    visited.iter_mut().for_each(|s| *s = 0);
    for start in 0..n {
        if visited[start] != 0 {
            continue;
        }
        let mut i = start;
        loop {
            visited[i] = 1;
            let j = idx[i];
            // `j == i` can only happen at a fixed point; `j == start` closes
            // the cycle. Either way this cycle is done.
            if j == start || j == i {
                break;
            }
            v.swap(i, j);
            i = j;
        }
    }
}

/// The `zstrcmp` operands `matchcmp` (`compcore.c:3173`) reads from one
/// match: `(*m)->str` (c:3181-3182) and `(*m)->disp` (c:3191-3192), in the
/// metafied, NUL-terminated form C's `Cmatch` already stores them in.
/// Built once per match per sort by [`qsort_matches`].
pub struct MatchSortKey {
    pub str: MetafiedOperand,
    pub disp: Option<MetafiedOperand>,
}

impl MatchSortKey {
    pub fn new(m: &Cmatch) -> Self {
        MatchSortKey {
            str: MetafiedOperand::new(m.str.as_deref().unwrap_or("")),
            disp: m.disp.as_deref().map(MetafiedOperand::new),
        }
    }
}

/// `qsort(rp, n, sizeof(Cmatch), matchcmp)` — `makearray`'s two sorts
/// (`compcore.c:3262` and c:3301) — over `ord`, an index permutation into
/// `src` (the port's stand-in for C's array of `Cmatch` pointers).
///
/// The comparator's operands are prepared once per match up front: C's
/// `Cmatch` holds them ready-made, and preparing them inside the comparator
/// repeated that work on each of the O(n log n) comparisons. Measured on
/// `arch <TAB>` (47058 command names; `_description`'s `_setup` reads
/// `$compstate[nmatches]` four times, and each read re-sorts the group the
/// way C's `permmatches` does) the per-comparison preparation was 61% of the
/// completion.
pub fn qsort_matches(ord: &mut [usize], src: &[Cmatch]) {
    let keys: Vec<MatchSortKey> = src.iter().map(MatchSortKey::new).collect();
    qsort_tolerant(ord, |a: &usize, b: &usize| {
        matchcmp(&src[*a], &keys[*a], &src[*b], &keys[*b])
    });
}

#[cfg(test)]
mod tests {
    use super::qsort_tolerant;
    use std::cmp::Ordering;

    #[test]
    fn sorts_like_a_total_order() {
        let mut v = vec![3, 1, 4, 1, 5, 9, 2, 6];
        qsort_tolerant(&mut v, |a, b| a.cmp(b));
        assert_eq!(v, vec![1, 1, 2, 3, 4, 5, 6, 9]);
    }

    #[test]
    fn is_stable() {
        // Sort by first field only; equal keys must keep input order.
        let mut v = vec![(1, 'a'), (1, 'b'), (0, 'c'), (1, 'd')];
        qsort_tolerant(&mut v, |a, b| a.0.cmp(&b.0));
        assert_eq!(v, vec![(0, 'c'), (1, 'a'), (1, 'b'), (1, 'd')]);
    }

    #[test]
    fn tolerates_non_transitive_comparator_without_panicking() {
        // A deliberately inconsistent comparator (rock-paper-scissors on mod 3)
        // that would make std sort_by panic. We only require: no panic, and a
        // valid permutation of the input.
        let mut v: Vec<i32> = (0..30).collect();
        qsort_tolerant(&mut v, |a, b| match (a.rem_euclid(3), b.rem_euclid(3)) {
            (0, 1) | (1, 2) | (2, 0) => Ordering::Less,
            (1, 0) | (2, 1) | (0, 2) => Ordering::Greater,
            _ => a.cmp(b),
        });
        let mut check = v.clone();
        check.sort();
        assert_eq!(check, (0..30).collect::<Vec<_>>());
    }

    /// `makearray`'s `qsort(..., matchcmp)` over a completion-sized match
    /// set. `arch <TAB>` sorts ~47k command names four times (one per
    /// `$compstate[nmatches]` read that finds new matches), and with the
    /// comparator re-deriving its operands per call that took ~8s per sort in
    /// the debug build and the cell never drew within the harness's 10s. The
    /// bound is far above the prepared-operand cost and far below the old one.
    /// The order must also be what pairwise `matchcmp` says it is.
    #[test]
    fn qsort_matches_sorts_a_large_match_set_in_bounded_time() {
        use super::{qsort_matches, MatchSortKey};
        use crate::ported::zle::comp_h::Cmatch;
        use crate::ported::zle::compcore::{matchcmp, MATCHORDER};
        let _g = crate::test_util::global_state_lock();
        MATCHORDER.store(0, std::sync::atomic::Ordering::Relaxed);
        // Deterministic scramble of 40000 distinct command-like names.
        let n = 40_000usize;
        let src: Vec<Cmatch> = (0..n)
            .map(|i| {
                let k = (i * 7919) % n;
                let mut m = Cmatch::default();
                m.str = Some(format!("cmd-{}{}", ["git", "Zip", "ls", "x_"][k % 4], k));
                m
            })
            .collect();
        let mut ord: Vec<usize> = (0..n).collect();
        let t = std::time::Instant::now();
        qsort_matches(&mut ord, &src);
        let took = t.elapsed();
        assert!(
            took < std::time::Duration::from_secs(4),
            "sorting {n} matches took {took:?}"
        );
        let mut seen = vec![false; n];
        for &i in &ord {
            assert!(!seen[i], "index {i} twice");
            seen[i] = true;
        }
        for w in ord.windows(2) {
            let (a, b) = (&src[w[0]], &src[w[1]]);
            assert_ne!(
                matchcmp(a, &MatchSortKey::new(a), b, &MatchSortKey::new(b)),
                Ordering::Greater,
                "{:?} sorted before {:?}",
                a.str,
                b.str
            );
        }
    }
}
