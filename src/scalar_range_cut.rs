//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C cuts a subscripted scalar once, inline, at the tail of `getstrvalue`
//! (`Src/params.c:2510-2532`). The port needs the same cut at three sites:
//! `ported::params::getstrvalue` itself, and two `ported::subst::paramsubst`
//! paths that hold a character slice rather than a `value` struct. C has no
//! function to port for it, so the shared body lives here, with each line
//! citing the C it comes from.

/// Cut a scalar to the range `[start, end)` exactly as `getstrvalue`'s
/// subscripted-scalar tail does (`Src/params.c:2510-2532`).
///
/// `start` is C's `v->start` (0-based; a negative value counts from the
/// end and clamps to 0, so `${s[-10,2]}` is the first two characters,
/// not empty). `end` is C's `v->end` (exclusive; negative counts from the
/// end, `-1` is the whole tail). Offsets are characters — the port's
/// stand-in for C's `MB_METACHARLEN` byte walk.
pub fn str_range_cut(chars: &[char], mut start: i64, mut end: i64) -> String {
    let len = chars.len() as i64; // c:2510
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
        return String::new(); // c:2524-2529
    }
    let hi = end.min(len); // c:2530-2531
    chars[start as usize..hi as usize].iter().collect()
}
