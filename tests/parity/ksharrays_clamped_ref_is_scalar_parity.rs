//! `setopt KSH_ARRAYS` — a clamped bare reference is a SCALAR, not a
//! one-element array.
//!
//! The clamp is c:Src/params.c:2286-2288, inside `fetchvalue`:
//!
//! ```c
//! } else if (!(scanflags & SCANPM_ASSIGNING) && v->scanflags &&
//!            itype_end(t, INAMESPC, 1) != t && isset(KSHARRAYS))
//!     v->end = 1, v->scanflags = 0;
//! ```
//!
//! It does TWO things. `v->end = 1` narrows the Value to element 0 — that half
//! was already in place, and `ksharrays_nested_subst_parity` pins the shapes it
//! must NOT reach. `v->scanflags = 0` is the other half: it drops the array
//! SHAPE, and c:Src/subst.c:2916 reads that straight back —
//!
//! ```c
//! if ((isarr = (v->scanflags & SCANPM_ISVAR_AT) ? -1 : v->scanflags ? 1 : 0))
//!     aval = getarrvalue(v);
//! else
//!     ... val = getstrvalue(v);
//! ```
//!
//! — so C takes the SCALAR arm, never assigns `aval`, and every consumer
//! downstream sees a string. zshrs answered with a one-element array instead,
//! so an outer length counted elements where zsh counts characters and an outer
//! subscript picked the element where zsh picks a character:
//!
//! ```text
//! % setopt ksharrays; a=(aa bb cc)
//! ${#${a}}      zsh 2   (the characters of `aa`)      zshrs was 1
//! ${${a}[0]}    zsh a   (character 0 of `aa`)         zshrs was aa
//! ```
//!
//! Two further C rules ride on the same line and are pinned here too.
//!
//! c:Src/params.c:1617-1620 — every subscript is read by `getarg`, and that is
//! the one place KSH_ARRAYS 0-bases it (`if (isset(KSHARRAYS) && r >= 0) r++`).
//! A subexp result is not exempt: c:Src/subst.c:2890 wraps the nested value in
//! a throwaway `createparam(nulstring, isarr ? PM_ARRAY : PM_SCALAR)` and
//! c:2900 calls `getindex` on it. So `${${s}[0]}` on a plain SCALAR is its
//! first character, with no array anywhere in the expression.
//!
//! c:Src/subst.c:2945-2954 — the scalar arm the clamp lands in sets
//! `vunset = 1` when `v->start` is past the end, which for an EMPTY array is
//! always. A clamped reference to `a=()` is therefore UNSET, not
//! empty-but-set: `${a-D}` is `D` and `${+a}` is 0.
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

/// Elements are two characters wide on purpose. With single-character elements
/// an element COUNT and a string LENGTH are the same number and a subscript
/// picks the same text either way, so the whole divergence would be invisible.
const SETUP: &str = "ka=(aa bb cc); ko=(zz); ke=(); ks=hello; kn=ka; \
                     typeset -A kh; kh=(k1 v1 k2 v2)";

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

/// An outer LENGTH over a clamped reference counts characters, because the
/// clamped reference is a scalar. c:Src/subst.c:3849's element-count branch is
/// gated on `isarr`, which c:2916 has just answered 0.
mod outer_length_counts_characters {
    use super::*;

    #[test]
    fn nested_bare_array() {
        both_states("${#${ka}}");
    }

    #[test]
    fn nested_bare_one_element_array() {
        both_states("${#${ko}}");
    }

    #[test]
    fn nested_twice() {
        both_states("${#${${ka}}}");
    }

    /// `[@]` on the clamped SCALAR is not a splat of the parameter — the
    /// parameter is gone by then, and c:Src/subst.c:2890's carrier is
    /// PM_SCALAR, so the length is still the element's characters.
    #[test]
    fn nested_bare_array_at_splat() {
        both_states("${#${ka}[@]}");
    }

    #[test]
    fn nested_bare_array_star_splat() {
        both_states("${#${ka}[*]}");
    }

    /// c:Src/subst.c:2707-2709 — the `(P)` splice hands the OUTER expansion a
    /// real parameter name, so `subexp` is off again and the fetch clamps.
    #[test]
    fn indirect_name() {
        both_states("${#${(P)kn}}");
    }

    #[test]
    fn indirect_name_at_splat() {
        both_states("${#${(P)kn}[@]}");
    }

    #[test]
    fn nested_sorted_array() {
        both_states("${#${(o)ka}}");
    }
}

/// An outer SUBSCRIPT over a clamped reference indexes CHARACTERS, and
/// c:Src/params.c:1619-1620 makes those indices 0-based.
mod outer_subscript_indexes_characters {
    use super::*;

    #[test]
    fn index_zero_is_first_character() {
        both_states("${${ka}[0]}");
    }

    #[test]
    fn index_one_is_second_character() {
        both_states("${${ka}[1]}");
    }

    #[test]
    fn index_two_is_past_a_two_character_element() {
        both_states("${${ka}[2]}");
    }

    /// c:Src/params.c:1619's `r >= 0` leaves a negative subscript alone, so the
    /// last character is `[-1]` under either option state.
    #[test]
    fn negative_index_is_unmoved() {
        both_states("${${ka}[-1]}");
    }

    #[test]
    fn range_both_bounds_shift() {
        both_states("${${ka}[0,1]}");
    }

    #[test]
    fn range_one_two() {
        both_states("${${ka}[1,2]}");
    }

    #[test]
    fn nested_twice_then_index() {
        both_states("${${${ka}}[0]}");
    }

    #[test]
    fn quoted_index() {
        both_states("\"${${ka}[0]}\"");
    }

    #[test]
    fn one_element_array_index() {
        both_states("${${ko}[0]}");
    }

    /// No array anywhere: c:2890's carrier is PM_SCALAR and c:2900's `getindex`
    /// still routes through `getarg`, so the 0-basing applies to a plain string
    /// too. This one reproduces with KSH_ARRAYS and a scalar alone.
    #[test]
    fn scalar_subject_index_zero() {
        both_states("${${ks}[0]}");
    }

    #[test]
    fn scalar_subject_index_one() {
        both_states("${${ks}[1]}");
    }

    #[test]
    fn scalar_subject_range() {
        both_states("${${ks}[0,1]}");
    }

    #[test]
    fn scalar_subject_negative_index() {
        both_states("${${ks}[-1]}");
    }

    /// c:Src/params.c:2270-2276 stamps `v->scanflags` for PM_HASHED as well, so
    /// a bare hash clamps the same way and its one value is then a string.
    #[test]
    fn hash_subject_index_zero() {
        both_states("${${kh}[0]}");
    }

    #[test]
    fn hash_subject_index_one() {
        both_states("${${kh}[1]}");
    }

    #[test]
    fn indirect_name_index_zero() {
        both_states("${${(P)kn}[0]}");
    }
}

/// The clamped reference's TYPE, and the one shape that keeps the array: an
/// explicit `[@]`/`[*]` on the INNER reference means `getindex` ran, so c:2281
/// takes the bracket branch and c:2286's `else if` never fires.
mod shape {
    use super::*;

    #[test]
    fn inner_at_splat_keeps_the_array() {
        both_states("${${ka[@]}}");
    }

    #[test]
    fn inner_at_splat_keeps_the_count() {
        both_states("${#${ka[@]}}");
    }

    #[test]
    fn inner_at_splat_index_is_an_element() {
        both_states("${${ka[@]}[1]}");
    }

    #[test]
    fn nested_bare_array_splats_one_word() {
        both_states("${${ka}[@]}");
    }

    #[test]
    fn nested_bare_array_value() {
        both_states("${${ka}}");
    }

    #[test]
    fn nested_bare_array_quoted() {
        both_states("\"${${ka}}\"");
    }

    #[test]
    fn default_operator_sees_the_element() {
        both_states("${ka:-D}");
    }

    #[test]
    fn zip_sees_the_element() {
        both_states("${ka:^ko}");
    }
}

/// c:Src/subst.c:2945-2954 — the clamp on a ZERO-element array lands past the
/// end, so the reference is UNSET rather than set-and-empty.
mod empty_array_is_unset {
    use super::*;

    #[test]
    fn dash_default_fires() {
        both_states("${ke-D}");
    }

    #[test]
    fn colon_dash_default_fires() {
        both_states("${ke:-D}");
    }

    #[test]
    fn plus_alternate_does_not_fire() {
        both_states("${ke+S}");
    }

    #[test]
    fn set_test_is_zero() {
        both_states("${+ke}");
    }

    /// An unset scalar quotes to `''`, which is a real word; an empty ARRAY
    /// would have quoted to nothing at all.
    #[test]
    fn quote_flag_emits_one_word() {
        both_states("${(q)ke}");
    }

    #[test]
    fn quote_dash_flag_emits_one_word() {
        both_states("${(q-)ke}");
    }

    /// c:Src/subst.c:3480 — an unset LHS skips the zip entirely (c:3486), so
    /// c:3498's `aval = hmkarray(val)` never promotes the empty scalar.
    #[test]
    fn zip_skips_entirely() {
        both_states("${ke:^ka}");
    }

    #[test]
    fn long_zip_skips_entirely() {
        both_states("${ke:^^ka}");
    }

    #[test]
    fn length_is_zero() {
        both_states("${#ke}");
    }

    #[test]
    fn type_is_still_array() {
        both_states("${(t)ke}");
    }

    #[test]
    fn nested_length_of_slice() {
        both_states("${#${ke}[1,2]}");
    }
}

/// The shapes `b0518269e4` fixed — a nested substitution never performs a
/// parameter fetch (c:Src/subst.c:2764), so no clamp can reach its array.
/// These must stay correct.
mod split_carriers_must_not_clamp {
    use super::*;

    const SPLIT_SETUP: &str = "kv='p q r'; kf=$'l1\\nl2\\nl3'";

    fn split_states(expr: &str) {
        if !zsh_available() {
            return;
        }
        let rs = zshrs_bin();
        let rs = rs.to_str().expect("utf-8 path");
        for setopt in ["setopt", "unsetopt"] {
            let s = format!("{setopt} ksharrays; {SPLIT_SETUP}\nprint -rl -- {expr}");
            let (zo, _) = run(zsh_path(), &["-f", "-c"], &s);
            let (ro, _) = run(rs, &["--zsh", "-f", "-c"], &s);
            assert_eq!(
                zo,
                ro,
                "stdout diverges under `{setopt} ksharrays`:\n{s}\n--- zsh   --- {:?}\n--- zshrs --- {:?}",
                String::from_utf8_lossy(&zo),
                String::from_utf8_lossy(&ro)
            );
        }
    }

    #[test]
    fn z_flag_count() {
        split_states("${#${(z)kv}}");
    }

    #[test]
    fn s_flag_count() {
        split_states("${#${(s: :)kv}}");
    }

    #[test]
    fn f_flag_count() {
        split_states("${#${(f)kf}}");
    }

    #[test]
    fn z_flag_element() {
        split_states("${${(z)kv}[1]}");
    }

    #[test]
    fn f_flag_element() {
        split_states("${${(f)kf}[1]}");
    }

    #[test]
    fn s_flag_sorted() {
        split_states("${(o)${(s: :)kv}}");
    }

    #[test]
    fn f_flag_uppercased() {
        split_states("${(U)${(f)kf}}");
    }

    #[test]
    fn z_flag_joined() {
        split_states("${(j:-:)${(z)kv}}");
    }

    #[test]
    fn cmdsubst_line_split_count() {
        split_states("${#${(f)$(print -l x y)}}");
    }

    #[test]
    fn cmdsubst_line_split_element() {
        split_states("${${(f)$(print -l x y)}[1]}");
    }
}
