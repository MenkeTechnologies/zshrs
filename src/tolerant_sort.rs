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

/// Bottom-up **stable** merge sort that TOLERATES a comparator which is not a
/// strict weak ordering — the faithful stand-in for C's `qsort`. O(n log n),
/// ties keep their input order, never panics on an inconsistent `cmp`.
///
/// The sort runs over indices so a non-transitive `cmp` can only affect the
/// final order — it can never cause an out-of-bounds access or a non-terminating
/// loop.
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
}
