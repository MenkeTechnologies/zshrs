//! `setopt KSH_ARRAYS` — the SECOND subscript of a chain is 0-based too.
//!
//! Every subscript in C is read by `getarg`, and c:Src/params.c:1618-1620 is the
//! one place KSH_ARRAYS 0-bases it:
//!
//! ```c
//! } else {
//!     r = mathevalarg(s, &s);
//!     if (isset(KSHARRAYS) && r >= 0)
//!         r++;
//! }
//! ```
//!
//! A chained subscript is not exempt. c:Src/subst.c:2890-2900 wraps whatever the
//! first subscript produced in a throwaway
//! `createparam(nulstring, isarr ? PM_ARRAY : PM_SCALAR)` and calls `getindex`
//! on it, which calls that same `getarg`. zshrs read the chain with a separate
//! hand-rolled parser that never applied the rule, so only the FIRST subscript
//! of an expression honoured the option:
//!
//! ```text
//! % setopt ksharrays; a=(hello world)
//! ${a[1][1]}      zsh o        zshrs was w
//! ${a[1][2]}      zsh r        zshrs was o
//! ${a[0,1][1]}    zsh world    zshrs was hello
//! ```
//!
//! It reaches shapes with no array at all: the same parser indexes the
//! CHARACTERS of a scalar (`ks=hello; ${ks[1,3][1]}` is `l` in zsh, not `e`),
//! and the value side of an association (`${kh[k1][1]}`).
//!
//! Two neighbouring C rules are pinned alongside it, because the chain reaches
//! both and the fix had to route through them.
//!
//! c:Src/params.c:2112-2113 — `if (start > 0 && (isset(KSHARRAYS) ||
//! (v->pm->node.flags & PM_HASHED))) start--;`. An `(i)`/`(I)` chained onto a
//! slice answers a POSITION, and under the option that position comes down by
//! one: `kb=(one two three four); ${kb[1,4][(i)two]}` is `0`. The same line runs
//! on a chained NUMERIC index in the inverse arm, where it cancels the `r++`
//! exactly — `${kh[(i)k1][2]}` is `2` under either option state — and it runs
//! BEFORE c:Src/subst.c:2945-2948 folds a negative index against the temp
//! array's length, so the order of the two is load-bearing.
//!
//! c:Src/params.c:1619 again, on the `hi` bound of a chained RANGE. The bound 0
//! is a real bound in C, not a stand-in for "to the end": `${ks[1,0]}` is empty
//! in zsh. zshrs spelled its unparsed-bound fallback as 0 and so swallowed a
//! written one, which the option then made visible (`${a[1][0,0]}` is `w`).
//!
//! Every case is asserted under BOTH option states; `unsetopt ksharrays` is the
//! default and must not move.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run(bin: &str, args: &[&str], s: &str) -> (Vec<u8>, i32) {
    let o = Command::new(bin)
        .args(args)
        .arg(s)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell");
    (o.stdout, o.status.code().unwrap_or(-1))
}

/// `ka`'s elements are five characters wide so that an element index and a
/// character index cannot land on the same text by accident, and `kb` has four
/// elements so a one-off in either direction is visible from both ends.
const SETUP: &str = "ka=(hello world); kb=(one two three four); ks=hello; kn=kb; \
                     typeset -A kh; kh=(k1 v1 k2 v2 k3 v3); set -- p1 p2 p3";

/// Assert byte parity for one expansion under BOTH option states.
fn both_states(expr: &str) {
    if !zsh_available() {
        return;
    }
    let rs = zshrs_bin();
    let rs = rs.to_str().expect("utf-8 path");
    for setopt in ["setopt", "unsetopt"] {
        let s = format!("{setopt} ksharrays; {SETUP}\nprint -rl -- {expr}");
        let (zo, zx) = run(zsh_path(), &["-f", "-c"], &s);
        let (ro, rx) = run(rs, &["--zsh", "-f", "-c"], &s);
        assert_eq!(
            zo,
            ro,
            "stdout diverges under `{setopt} ksharrays`:\n{s}\n--- zsh   --- {:?}\n--- zshrs --- {:?}",
            String::from_utf8_lossy(&zo),
            String::from_utf8_lossy(&ro)
        );
        assert_eq!(zx, rx, "exit status diverges under `{setopt} ksharrays`:\n{s}");
    }
}

/// `${a[N][M]}` — the first subscript picks an element, the second picks one of
/// its characters, and c:1619 applies to both.
mod element_then_character {
    use super::*;

    #[test]
    fn first_character_of_the_indexed_element() {
        both_states("${ka[1][1]}");
    }

    #[test]
    fn second_character_of_the_indexed_element() {
        both_states("${ka[1][2]}");
    }

    #[test]
    fn element_zero_then_character() {
        both_states("${ka[0][1]}");
    }

    /// The shift shortens the reachable text by one from the front, so the last
    /// WRITTEN index that still lands is one lower under the option.
    #[test]
    fn character_past_the_end() {
        both_states("${ka[1][5]}");
    }

    /// c:1619's `r >= 0` leaves a negative subscript alone, on either side of
    /// the chain.
    #[test]
    fn negative_character_index() {
        both_states("${ka[1][-1]}");
    }

    #[test]
    fn negative_element_then_character() {
        both_states("${ka[-1][1]}");
    }

    /// The bound is arithmetic, not a literal, and goes through the same
    /// `mathevalarg` c:1618 calls.
    #[test]
    fn arithmetic_character_index() {
        both_states("${ka[1][1+0]}");
    }
}

/// `${a[lo,hi][M]}` — the first subscript is a RANGE, so the chain indexes the
/// sub-array ELEMENT-wise. c:1619 applies to all three bounds.
mod slice_then_element {
    use super::*;

    #[test]
    fn first_element_of_the_slice() {
        both_states("${ka[0,1][1]}");
    }

    #[test]
    fn element_zero_of_the_slice() {
        both_states("${ka[0,1][0]}");
    }

    /// The slice bounds shift too, so a slice written from 1 starts one element
    /// later under the option.
    #[test]
    fn slice_written_from_one() {
        both_states("${ka[1,2][1]}");
    }

    #[test]
    fn middle_of_a_four_element_slice() {
        both_states("${kb[1,3][2]}");
    }

    #[test]
    fn negative_index_into_a_shifted_slice() {
        both_states("${kb[0,3][-1]}");
    }

    #[test]
    fn arithmetic_index_into_a_slice() {
        both_states("${kb[0,2][1+1]}");
    }

    /// A splat as the second subscript is not an index (c:Src/params.c:2048-2053
    /// `v->start = 0; v->end = -1`), so only the FIRST subscript's bounds move.
    ///
    /// Unquoted only: a QUOTED splat of a chain loses its word boundaries in
    /// zshrs (`"${kb[1,3][@]}"` is one space-joined word, not three), which is
    /// a separate defect — it diverges identically under `unsetopt ksharrays`.
    #[test]
    fn splat_of_a_shifted_slice() {
        both_states("${kb[1,3][@]}");
    }
}

/// `${a[N][lo,hi]}` — a chained RANGE over the element's characters.
mod element_then_character_range {
    use super::*;

    #[test]
    fn two_characters() {
        both_states("${ka[1][1,2]}");
    }

    #[test]
    fn range_written_from_zero() {
        both_states("${ka[1][0,1]}");
    }

    /// The `hi` bound 0 is a real bound: empty under either option state
    /// without the shift, and one character with it.
    #[test]
    fn range_to_bound_zero() {
        both_states("${ka[1][1,0]}");
    }

    #[test]
    fn range_zero_to_zero() {
        both_states("${ka[1][0,0]}");
    }

    #[test]
    fn range_to_negative_bound() {
        both_states("${ka[1][1,-1]}");
    }

    #[test]
    fn range_between_negative_bounds() {
        both_states("${ka[1][-3,-1]}");
    }
}

/// `${a[lo,hi][lo,hi]}` — a range of a range, four bounds, all shifted.
mod slice_then_slice {
    use super::*;

    #[test]
    fn slice_of_a_slice() {
        both_states("${kb[0,2][1,2]}");
    }

    #[test]
    fn slice_written_from_one_of_a_slice() {
        both_states("${kb[1,3][1,2]}");
    }

    #[test]
    fn tail_of_a_slice() {
        both_states("${kb[0,3][2,3]}");
    }

    #[test]
    fn negative_slice_of_a_slice() {
        both_states("${kb[1,4][-2,-1]}");
    }
}

/// A SCALAR subject — the same parser, with no array anywhere in the
/// expression. c:Src/subst.c:2890's carrier is PM_SCALAR and the chain indexes
/// characters twice over.
mod scalar_subject {
    use super::*;

    #[test]
    fn character_of_a_character() {
        both_states("${ks[1][1]}");
    }

    #[test]
    fn character_zero_of_character_zero() {
        both_states("${ks[0][0]}");
    }

    #[test]
    fn second_character_of_a_character() {
        both_states("${ks[1][2]}");
    }

    #[test]
    fn character_of_a_later_character() {
        both_states("${ks[2][1]}");
    }

    #[test]
    fn character_of_a_character_range() {
        both_states("${ks[1,3][1]}");
    }

    #[test]
    fn character_of_a_range_written_from_zero() {
        both_states("${ks[0,2][1]}");
    }

    #[test]
    fn character_of_the_last_character() {
        both_states("${ks[-1][1]}");
    }

    #[test]
    fn negative_character_of_a_character() {
        both_states("${ks[1][-1]}");
    }
}

/// An ASSOCIATION subject — the key lookup is not a shifted subscript
/// (c:Src/params.c:1616 hands a hash its own `r`), but the chain over the
/// VALUE's characters is.
mod association_subject {
    use super::*;

    #[test]
    fn character_of_a_value() {
        both_states("${kh[k1][1]}");
    }

    /// Not pinned here: `${kh[k1][0]}`. A chained single index that resolves
    /// below the first character answers the first character in zshrs instead
    /// of nothing, which is a separate defect in the shared bound-clamp — it
    /// diverges identically under `unsetopt ksharrays`, where c:1619 never
    /// fires at all.
    #[test]
    fn character_range_of_a_value() {
        both_states("${kh[k2][1,2]}");
    }

    /// c:Src/params.c:2112-2113 cancels c:1619 exactly in the inverse arm: an
    /// `(i)` on a hash answers the KEY and a chained numeric index is echoed
    /// unchanged under either option state.
    #[test]
    fn numeric_index_chained_onto_a_key_search() {
        both_states("${kh[(i)k1][1]}");
    }

    #[test]
    fn numeric_index_chained_onto_a_backward_key_search() {
        both_states("${kh[(I)k1][2]}");
    }

    #[test]
    fn character_of_a_pattern_matched_value() {
        both_states("${kh[(K)k1][1]}");
    }

    #[test]
    fn character_of_a_reverse_matched_key() {
        both_states("${kh[(R)v1][1]}");
    }
}

/// The inverse subscript flags as EITHER half of the chain. c:2112-2113 shifts
/// the position the `(i)`/`(I)` arms answer; `(r)`/`(R)` answer a value and are
/// untouched by it, but the slice they search was itself shifted.
mod inverse_flags {
    use super::*;

    #[test]
    fn value_search_over_a_shifted_slice() {
        both_states("${kb[1,4][(r)two]}");
    }

    #[test]
    fn value_search_over_a_slice_written_from_zero() {
        both_states("${kb[0,3][(r)two]}");
    }

    #[test]
    fn backward_value_search_over_a_shifted_slice() {
        both_states("${kb[1,4][(R)t*]}");
    }

    #[test]
    fn index_search_over_a_shifted_slice() {
        both_states("${kb[1,4][(i)two]}");
    }

    #[test]
    fn index_search_over_a_slice_written_from_zero() {
        both_states("${kb[0,3][(i)two]}");
    }

    #[test]
    fn backward_index_search_over_a_shifted_slice() {
        both_states("${kb[1,4][(I)t*]}");
    }

    #[test]
    fn backward_index_search_over_a_slice_written_from_zero() {
        both_states("${kb[0,3][(I)t*]}");
    }

    #[test]
    fn character_of_a_value_search() {
        both_states("${kb[(r)two][1]}");
    }

    #[test]
    fn character_of_a_backward_value_search() {
        both_states("${kb[(R)t*][1]}");
    }

    /// The position `(i)` answers is one digit, so a chained `[1]` reads past
    /// it under the option and is empty.
    #[test]
    fn character_of_an_index_search() {
        both_states("${kb[(i)two][1]}");
    }

    #[test]
    fn character_of_a_backward_index_search() {
        both_states("${kb[(I)t*][1]}");
    }

    /// c:Src/params.c:1801 — a forward miss answers `len + 1`, which c:2112
    /// then shifts like any other position.
    #[test]
    fn character_of_a_missed_index_search() {
        both_states("${kb[(i)nope][1]}");
    }

    #[test]
    fn character_of_a_missed_value_search() {
        both_states("${kb[(r)nope][1]}");
    }
}

/// Out-of-range on either half: the shift moves where "out of range" begins.
mod out_of_range {
    use super::*;

    #[test]
    fn element_past_the_array() {
        both_states("${ka[9][1]}");
    }

    #[test]
    fn character_far_past_the_element() {
        both_states("${ka[1][99]}");
    }

    #[test]
    fn slice_past_the_array_then_index() {
        both_states("${ka[0,9][3]}");
    }

    #[test]
    fn slice_entirely_past_the_array() {
        both_states("${kb[9,20][1]}");
    }
}

/// The positional parameters are an array like any other, reached by name and
/// by splat.
mod positionals {
    use super::*;

    #[test]
    fn character_of_a_positional() {
        both_states("${argv[1][1]}");
    }

    #[test]
    fn element_of_a_positional_slice() {
        both_states("${argv[0,1][1]}");
    }

    #[test]
    fn character_of_an_at_positional() {
        both_states("${@[1][1]}");
    }
}
