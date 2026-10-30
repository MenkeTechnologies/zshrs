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
/// That is why the completion sorts (`makearray`, via [`qsort_matches`]) do not
/// use this function: there a tie decides which duplicate is listed, so they
/// call libc `qsort` itself and get the same tie order zsh gets on the host.
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
///
/// This calls libc's `qsort(3)` itself, as C does, not [`qsort_tolerant`].
/// `matchcmp` returns 0 for two matches with the same `str` (and no `disp`),
/// and `makearray` lets that TIE decide what is listed: its CGF_NOSORT pass
/// (c:3299-3320) marks the LATER member of each equal-string pair CMF_MULT and
/// the earlier CMF_FMULT, on the live matches, and every `$compstate[nmatches]`
/// read re-runs it through `permmatches`. When the group grows between two
/// runs, a non-stable quicksort can order the pair differently the second
/// time, so both members end up CMF_MULT and neither is listed. `_getconf`
/// adds `PIPE_BUF` twice (`-S ''` and `-S ' '`), and `_setup` reads
/// `nmatches` before the last `compadd`: Darwin's qsort orders the pair
/// (63, 54) over the first 64 matches and (54, 63) over all 295, so zsh lists
/// 293 possibilities and no `PIPE_BUF`, where a stable sort listed one.
/// Calling the same entry point keeps the tie permutation identical to zsh's
/// on every platform. `qsort` also tolerates `matchcmp` not being a strict
/// weak order, which is what [`qsort_tolerant`] was standing in for.
pub fn qsort_matches(ord: &mut [usize], src: &[Cmatch]) {
    let keys: Vec<MatchSortKey> = src.iter().map(MatchSortKey::new).collect();
    // `qsort` passes the comparator only the two element pointers, so the
    // match table it indexes into travels through a thread-local, saved and
    // restored around the call.
    type Ctx = (*const Cmatch, *const MatchSortKey);
    thread_local! {
        static CTX: std::cell::Cell<Ctx> =
            const { std::cell::Cell::new((std::ptr::null(), std::ptr::null())) };
    }
    unsafe extern "C" fn matchcmp_thunk(a: *const libc::c_void, b: *const libc::c_void) -> libc::c_int {
        let (ia, ib) = unsafe { (*(a as *const usize), *(b as *const usize)) };
        let (s, k) = CTX.with(|c| c.get());
        let (ma, mb, ka, kb) = unsafe { (&*s.add(ia), &*s.add(ib), &*k.add(ia), &*k.add(ib)) };
        match matchcmp(ma, ka, mb, kb) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }
    if ord.len() < 2 {
        return;
    }
    let saved = CTX.with(|c| c.replace((src.as_ptr(), keys.as_ptr())));
    unsafe {
        libc::qsort(
            ord.as_mut_ptr() as *mut libc::c_void,
            ord.len(),
            std::mem::size_of::<usize>(),
            Some(matchcmp_thunk),
        );
    }
    CTX.with(|c| c.set(saved));
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
