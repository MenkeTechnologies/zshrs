//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C cuts a subscripted scalar once, inline, at the tail of `getstrvalue`
//! (`Src/params.c:2510-2532`). The port needs the same cut at three sites:
//! `ported::params::getstrvalue` itself, and two `ported::subst::paramsubst`
//! paths that hold a character slice rather than a `value` struct. C has no
//! function to port for it, so the shared body lives here, with each line
//! citing the C it comes from.

/// The `[lo, hi)` index range `getstrvalue`'s subscripted-scalar tail keeps
/// of a value `len` units long (`Src/params.c:2510-2532`), or `None` when
/// the cut is empty. `start` / `end` are C's `v->start` / `v->end`.
fn range_bounds(len: i64, mut start: i64, mut end: i64) -> Option<(usize, usize)> {
    if start < 0 {
        // c:2511-2515
        start += len;
        if start < 0 {
            start = 0;
        }
    }
    if end < 0 {
        // c:2516-2522
        end += len;
        if end >= 0 && end < len {
            end += 1; // c:2520 `v->end += MB_METACHARLEN(eptr)`
        }
    }
    if start > len || end <= start {
        return None; // c:2524-2529
    }
    Some((start as usize, end.min(len) as usize)) // c:2530-2531
}

/// Cut a scalar to the range `[start, end)` exactly as `getstrvalue`'s
/// subscripted-scalar tail does (`Src/params.c:2510-2532`).
///
/// `start` is C's `v->start` (0-based; a negative value counts from the
/// end and clamps to 0, so `${s[-10,2]}` is the first two characters,
/// not empty). `end` is C's `v->end` (exclusive; negative counts from the
/// end, `-1` is the whole tail). Offsets are characters — the port's
/// stand-in for C's `MB_METACHARLEN` byte walk.
pub fn str_range_cut(chars: &[char], start: i64, end: i64) -> String {
    match range_bounds(chars.len() as i64, start, end) {
        Some((lo, hi)) => chars[lo..hi].iter().collect(),
        None => String::new(),
    }
}

/// [`str_range_cut`] over the value's character UNITS — one string per
/// `MB_METACHARLEN` step (c:2520), as `mb_metacharlenconv` splits it. In a
/// single-byte locale a non-ASCII byte is one unit but more than one `char`
/// of the port's metafied string, so cutting the flattened `char`s put the
/// bounds in the wrong place (`LC_ALL=C; s=xé<ab; ${s[2,3]}`).
pub fn units_range_cut(units: &[String], start: i64, end: i64) -> String {
    match range_bounds(units.len() as i64, start, end) {
        Some((lo, hi)) => units[lo..hi].concat(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{str_range_cut, units_range_cut};

    fn cut(s: &str, start: i64, end: i64) -> String {
        let cs: Vec<char> = s.chars().collect();
        str_range_cut(&cs, start, end)
    }

    #[test]
    fn positive_bounds_are_half_open() {
        // `${s[2,3]}` reaches the cut as start=1, end=3.
        assert_eq!(cut("hello", 1, 3), "el");
        assert_eq!(cut("hello", 0, 5), "hello");
        assert_eq!(cut("hello", 0, 1), "h");
    }

    #[test]
    fn negative_start_clamps_to_zero_not_empty() {
        // `${s[-10,2]}` is the first two characters (c:2511-2515).
        assert_eq!(cut("hello", -10, 2), "he");
        assert_eq!(cut("hello", -2, 5), "lo");
    }

    #[test]
    fn negative_end_is_inclusive_of_the_counted_char() {
        // c:2516-2522: -1 is the whole tail, -2 drops the last char.
        assert_eq!(cut("hello", 0, -1), "hello");
        assert_eq!(cut("hello", 0, -2), "hell");
        assert_eq!(cut("hello", -3, -1), "llo");
    }

    #[test]
    fn negative_end_past_the_front_yields_empty() {
        assert_eq!(cut("hello", 0, -10), "");
    }

    #[test]
    fn empty_and_out_of_range_cuts() {
        assert_eq!(cut("hello", 3, 3), "");
        assert_eq!(cut("hello", 4, 2), "");
        assert_eq!(cut("hello", 6, 9), "");
        assert_eq!(cut("hello", 5, 9), "");
        assert_eq!(cut("", 0, -1), "");
    }

    #[test]
    fn end_past_length_clamps() {
        assert_eq!(cut("hello", 2, 100), "llo");
    }

    #[test]
    fn counts_chars_not_bytes() {
        assert_eq!(cut("aéb", 1, 2), "é");
    }

    #[test]
    fn units_cut_keeps_multichar_units_whole() {
        // One unit per MB_METACHARLEN step; a unit may be several chars.
        let units: Vec<String> = ["x", "\u{e9}\u{301}", "<", "a", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(units_range_cut(&units, 1, 3), "\u{e9}\u{301}<");
        assert_eq!(units_range_cut(&units, 0, -1), "x\u{e9}\u{301}<ab");
        assert_eq!(units_range_cut(&units, -2, -1), "ab");
        assert_eq!(units_range_cut(&units, 9, 10), "");
    }
}
