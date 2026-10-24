//! ksh93 pattern syntax zsh's pattern compiler has no spelling for.
//!
//! !!! RUST-ONLY MODULE — no zsh C counterpart !!!
//!
//! ksh93(1) "File Name Generation" counts a group with a PREFIX:
//! `{n}(pattern-list)` matches exactly n occurrences, `{m,n}(…)` m to n,
//! `{m,}(…)` at least m, `{,n}(…)` at most n. zsh's equivalent is the
//! postfix extended-glob closure `(…)(#cm,n)`, so `patcompbranch`
//! (src/ported/pattern.rs) reads the bounds here and wraps the group it
//! compiles next in the same `P_COUNT` node.

/// Parses a ksh93 count prefix at the start of `bytes` that is immediately
/// followed by the group-open byte `inpar`. Returns `(min, max, consumed)`,
/// where `consumed` stops just before the group; `None` leaves the brace
/// literal, as ksh93 does for a `{…}` not followed by `(`. An omitted lower
/// bound is 0 and an omitted upper bound is unbounded; `{,}` and `{}` are
/// not counts.
pub fn ksh93_count_prefix(bytes: &[u8], inpar: u8) -> Option<(i64, i64, usize)> {
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let digits = |from: usize| -> (Option<i64>, usize) {
        let mut j = from;
        let mut n: i64 = 0;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add((bytes[j] - b'0') as i64);
            j += 1;
        }
        (if j > from { Some(n) } else { None }, j)
    };
    let (lo, mut j) = digits(1);
    let (min, max) = if bytes.get(j) == Some(&b',') {
        let (hi, k) = digits(j + 1);
        j = k;
        if lo.is_none() && hi.is_none() {
            return None;
        }
        (lo.unwrap_or(0), hi.unwrap_or(i64::MAX))
    } else {
        let n = lo?;
        (n, n)
    };
    if bytes.get(j) != Some(&b'}') || bytes.get(j + 1) != Some(&inpar) || min > max {
        return None;
    }
    Some((min, max, j + 1))
}

#[cfg(test)]
mod tests {
    use super::ksh93_count_prefix;

    #[test]
    fn reads_each_bound_shape() {
        assert_eq!(ksh93_count_prefix(b"{3}(a)", b'('), Some((3, 3, 3)));
        assert_eq!(ksh93_count_prefix(b"{2,4}(a)", b'('), Some((2, 4, 5)));
        assert_eq!(ksh93_count_prefix(b"{2,}(a)", b'('), Some((2, i64::MAX, 4)));
        assert_eq!(ksh93_count_prefix(b"{,2}(a)", b'('), Some((0, 2, 4)));
    }

    #[test]
    fn a_brace_that_is_not_a_count_stays_literal() {
        // No group after it, no digits, a reversed range, or a non-digit.
        assert_eq!(ksh93_count_prefix(b"{3}a", b'('), None);
        assert_eq!(ksh93_count_prefix(b"{,}(a)", b'('), None);
        assert_eq!(ksh93_count_prefix(b"{}(a)", b'('), None);
        assert_eq!(ksh93_count_prefix(b"{4,2}(a)", b'('), None);
        assert_eq!(ksh93_count_prefix(b"{a}(a)", b'('), None);
        assert_eq!(ksh93_count_prefix(b"x{3}(a)", b'('), None);
    }
}
