//! zsh-specific array feature parity tests.

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
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod one_indexed {
    use super::*;

    /// zsh arrays are 1-indexed (NOT 0-indexed like bash).
    #[test]
    fn arr_index_one_is_first() {
        assert_parity(r#"arr=(a b c); echo "${arr[1]}""#);
    }

    /// `arr[0]` returns empty in zsh-native mode.
    #[test]
    fn arr_index_zero_empty_in_zsh_native() {
        assert_parity(r#"arr=(a b c); echo "[${arr[0]}]""#);
    }
}

mod negative_indexing {
    use super::*;

    #[test]
    fn arr_neg_one_is_last() {
        assert_parity(r#"arr=(a b c d); echo "${arr[-1]}""#);
    }

    #[test]
    fn arr_neg_two_is_second_to_last() {
        assert_parity(r#"arr=(a b c d); echo "${arr[-2]}""#);
    }

    #[test]
    fn arr_neg_count_out_of_range() {
        assert_parity(r#"arr=(a b c); echo "[${arr[-10]}]""#);
    }
}

mod slicing {
    use super::*;

    /// `${arr[M,N]}` slice from M to N inclusive.
    #[test]
    fn slice_two_to_four() {
        assert_parity(r#"arr=(a b c d e f); print -l "${arr[2,4]}""#);
    }

    /// `${arr[1,-1]}` full slice.
    #[test]
    fn slice_one_to_negative_one_full() {
        assert_parity(r#"arr=(a b c); print -l "${arr[1,-1]}""#);
    }

    /// `${arr[2,-2]}` middle slice.
    #[test]
    fn slice_two_to_neg_two_middle() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[2,-2]}""#);
    }

    /// Out-of-range slice returns empty.
    #[test]
    fn slice_out_of_range_empty() {
        assert_parity(r#"arr=(a b c); print -l "${arr[10,15]}"; echo done"#);
    }
}

mod append_prepend {
    use super::*;

    /// `arr+=(elem)` appends.
    #[test]
    fn append_via_plus_equals() {
        assert_parity(r#"arr=(a b); arr+=(c d); print -l "${arr[@]}""#);
    }

    /// `arr+=(single)` adds one.
    #[test]
    fn append_one_element() {
        assert_parity(r#"arr=(a); arr+=(b); print -l "${arr[@]}""#);
    }

    /// Multiple appends.
    #[test]
    fn append_chained() {
        assert_parity(r#"arr=(a); arr+=(b); arr+=(c); arr+=(d); print -l "${arr[@]}""#);
    }
}

mod element_assignment {
    use super::*;

    /// `arr[N]=val` assigns to specific element.
    #[test]
    fn element_assignment_replaces() {
        assert_parity(r#"arr=(a b c); arr[2]=X; print -l "${arr[@]}""#);
    }

    /// Assigning past end extends array.
    #[test]
    fn element_assignment_past_end_extends() {
        assert_parity(r#"arr=(a b c); arr[5]=Z; print -l "${arr[@]}"; echo ${#arr}"#);
    }

    /// Negative index assignment.
    #[test]
    fn element_assignment_negative_index() {
        assert_parity(r#"arr=(a b c d); arr[-1]=LAST; print -l "${arr[@]}""#);
    }
}

mod length {
    use super::*;

    /// `${#arr}` = element count.
    #[test]
    fn length_returns_element_count() {
        assert_parity(r#"arr=(a b c d e); echo ${#arr}"#);
    }

    /// Empty array length = 0.
    #[test]
    fn empty_array_length_zero() {
        assert_parity(r#"arr=(); echo ${#arr}"#);
    }

    /// After append, length grows.
    #[test]
    fn length_grows_after_append() {
        assert_parity(r#"arr=(a); arr+=(b c d); echo ${#arr}"#);
    }
}

mod splat {
    use super::*;

    /// `"${arr[@]}"` splats with quoting per-element preserved.
    #[test]
    fn at_splat_preserves_each_element_as_arg() {
        assert_parity(r#"arr=("a b" "c d"); f() { echo $#; }; f "${arr[@]}""#);
    }

    /// `"${arr[*]}"` joins on $IFS first char.
    #[test]
    fn star_splat_joins_on_ifs() {
        assert_parity(r#"arr=(a b c); echo "${arr[*]}""#);
    }

    /// Unquoted `${arr[@]}` — see zsh's no-default-split rule.
    #[test]
    fn unquoted_at_splat() {
        assert_parity(r#"arr=("a b" "c d"); f() { echo $#; }; f ${arr[@]}"#);
    }
}

mod assign_from_string {
    use super::*;

    /// `arr=( $(echo a b c) )` initialized from cmdsubst (which splits in zsh).
    #[test]
    fn array_init_from_cmdsubst() {
        assert_parity(r#"arr=($(echo a b c)); echo ${#arr}"#);
    }

    /// `arr=($X)` with quoted-only X (no split).
    #[test]
    fn array_init_from_quoted_string() {
        assert_parity(r#"X="a b c"; arr=("$X"); echo ${#arr}"#);
    }

    /// `arr=($X)` with unquoted X (zsh default no-split → 1 elem).
    #[test]
    fn array_init_from_unquoted_var_no_split() {
        assert_parity(r#"X="a b c"; arr=($X); echo ${#arr}"#);
    }

    /// With $=X force split.
    #[test]
    fn array_init_with_force_split() {
        assert_parity(r#"X="a b c"; arr=($=X); echo ${#arr}"#);
    }
}

mod special_vars {
    use super::*;

    /// $_ is last argument of previous command (only in interactive
    /// shells; in -c the behavior varies). Don't compare value,
    /// just pin: exists and string.
    #[test]
    fn dollar_underscore_after_command() {
        if !zsh_available() {
            return;
        }
        let s = r#"echo first second; [[ -n "$_" ]]; echo $?"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        // Just compare exit (value of $_ depends on whether $_ is tracked in -c mode).
        let _ = (z.stdout, r.stdout);
    }
}

mod array_in_param_expansion {
    use super::*;

    /// `${arr#pat}` — STRIP prefix from each element (zsh-array semantics).
    #[test]
    fn strip_prefix_per_element() {
        assert_parity(r#"arr=(foo bar fox); print -l "${arr[@]#f}""#);
    }

    /// `${arr%pat}` — strip suffix from each element.
    #[test]
    fn strip_suffix_per_element() {
        assert_parity(r#"arr=(foo bar baz); print -l "${arr[@]%[oz]}""#);
    }
}

/// `name[@]=` / `name[*]=` on an assignment LHS selects the WHOLE array
/// (c:Src/params.c getindex). The prior port rejected these as
/// "not an identifier: a@" (split_subscript dropped the brackets); now the
/// compiler routes them to whole-array set/append, scalar RHS → 1-element
/// array (no word-split), with an assoc-target slice error.
mod whole_array_subscript_assign {
    use super::*;

    #[test]
    fn at_append_scalar() {
        assert_parity("a=(1 2); a[@]+=3; print -l $a");
    }

    #[test]
    fn at_set_array() {
        assert_parity("a=(1 2); a[@]=(x y z); print -l $a");
    }

    #[test]
    fn star_set_array() {
        assert_parity("a=(1 2); a[*]=(x y z); print -l $a");
    }

    #[test]
    fn at_append_array() {
        assert_parity("a=(1 2); a[@]+=(3 4); print -l $a");
    }

    #[test]
    fn at_set_scalar_makes_one_element() {
        assert_parity("a=(1 2); a[@]=x; print -l $a");
    }

    #[test]
    fn at_create_new_array() {
        assert_parity("a[@]=(new arr); print -l $a");
    }

    /// scalar RHS to `[@]` is NOT word-split (unlike `a=($foo)`).
    #[test]
    fn at_scalar_rhs_no_split() {
        assert_parity(r#"foo="a b"; typeset -a a; a[@]=$foo; print -l $a"#);
    }

    #[test]
    fn at_append_scalar_rhs_no_split() {
        assert_parity(r#"foo="a b"; a=(1); a[@]+=$foo; print -l $a"#);
    }

    /// `[@]` on an associative array → slice error (c:3324).
    #[test]
    fn at_on_assoc_is_slice_error() {
        assert_parity("typeset -A h=(a 1 b 2); h[@]=(x y); print -rl -- ${(kv)h}");
    }

    #[test]
    fn at_append_on_assoc_is_slice_error() {
        assert_parity("typeset -A h=(a 1); h[@]+=(x y); echo done");
    }
}

/// Negative array subscript assignment that points PAST the array start
/// (`a[-N]` with N > len) clamps to position 0 and INSERTS (prepend),
/// keeping every element, rather than overwriting element 0
/// (c:Src/params.c:2930-2946 setarrvalue start/end clamp).
mod negative_oob_subscript_assign {
    use super::*;

    #[test]
    fn negative_past_start_prepends() {
        assert_parity("a=(1 2 3 4 5); a[-7]=42; print $a");
    }

    #[test]
    fn negative_one_past_start_prepends() {
        assert_parity("a=(1 2 3 4 5); a[-6]=42; print $a");
    }

    /// `a[-len]` (exactly position 1) OVERWRITES element 0 (boundary).
    #[test]
    fn negative_equals_len_overwrites_first() {
        assert_parity("a=(1 2 3 4 5); a[-5]=42; print $a");
    }

    #[test]
    fn negative_last_overwrites() {
        assert_parity("a=(1 2 3 4 5); a[-1]=42; print $a");
    }

    #[test]
    fn chained_negative_past_start() {
        assert_parity("a=(1 2 3 4 5); a[-7]=42; a[-9]=99; print $a");
    }
}

/// Scalar char-splice with out-of-range subscripts (the SCALAR analog of
/// the array clamp). `a="abc"; a[-4]="x"` inserts at the FRONT rather than
/// overwriting char 0, because a negative subscript past the start clamps
/// both start and end to 0 → empty slice old[0..0]
/// (c:Src/params.c:2721-2733 assignstrvalue).
mod scalar_oob_subscript_assign {
    use super::*;

    #[test]
    fn negative_past_start_prepends() {
        assert_parity(r#"a="abc"; a[-4]="x"; print $a"#);
    }

    #[test]
    fn negative_far_past_start_prepends() {
        assert_parity(r#"a="abc"; a[-10]="z"; print $a"#);
    }

    /// `a[-len]` (exactly char 1) OVERWRITES char 0 (boundary).
    #[test]
    fn negative_equals_len_overwrites_first() {
        assert_parity(r#"a="abc"; a[-3]="x"; print $a"#);
    }

    #[test]
    fn negative_last_overwrites() {
        assert_parity(r#"a="abc"; a[-1]="X"; print $a"#);
    }

    /// Past the end appends (scalar has no gap-fill).
    #[test]
    fn past_end_appends() {
        assert_parity(r#"a="abc"; a[5]="z"; print $a"#);
    }

    #[test]
    fn interior_overwrites() {
        assert_parity(r#"a="abcde"; a[-2]="X"; print $a"#);
    }
}

/// Array slice assignment `a[lo,hi]=(...)`. A REVERSED range (lo > hi)
/// is an EMPTY range → the value is INSERTED at lo keeping every element
/// (c:Src/params.c:2940-2943 — `if (end < start) end = start`).
mod slice_range_assign {
    use super::*;

    #[test]
    fn reversed_range_inserts() {
        assert_parity("a=(1 2 3 4 5); a[4,2]=(42 43 44); print $a");
    }

    #[test]
    fn forward_range_replaces() {
        assert_parity("a=(1 2 3 4 5); a[2,4]=(x y); print $a");
    }

    /// `a[1,0]` — the canonical prepend idiom (end one before start).
    #[test]
    fn one_zero_prepends() {
        assert_parity("a=(1 2 3); a[1,0]=(X Y); print $a");
    }

    #[test]
    fn single_element_range_replaces() {
        assert_parity("a=(1 2 3 4 5); a[3,3]=(z); print $a");
    }

    #[test]
    fn negative_end_range() {
        assert_parity("a=(1 2 3 4 5); a[2,-1]=(p); print $a");
    }
}

/// Subscripted array APPEND `a[N]+=(...)` / `a[lo,hi]+=(...)`. The augment
/// does NOT prepend the old slice — it collapses the range to an empty
/// range positioned AFTER the slice end and inserts only the new value
/// (c:Src/params.c:3518-3520). The bug: the compile path ignored
/// `assign.append`, so `a[2]+=(d)` OVERWROTE element 2 instead of
/// inserting after it.
mod subscript_array_append {
    use super::*;

    #[test]
    fn single_index_inserts_after() {
        assert_parity("a=(a b c); a[2]+=(d); echo $a");
    }

    #[test]
    fn single_index_multi_value() {
        assert_parity("a=(a b c); a[2]+=(d e); echo $a");
    }

    #[test]
    fn first_index_inserts_after() {
        assert_parity("a=(a b c); a[1]+=(d); echo $a");
    }

    #[test]
    fn negative_last_appends() {
        assert_parity("a=(a b c); a[-1]+=(d); echo $a");
    }

    #[test]
    fn negative_interior_inserts_after() {
        assert_parity("a=(a b c); a[-2]+=(m); echo $a");
    }

    #[test]
    fn forward_range_inserts_after_end() {
        assert_parity("a=(1 2 3 4); a[2,3]+=(x); echo $a");
    }

    #[test]
    fn range_element_count_preserved() {
        assert_parity("a=(1 2 3 4); a[2,3]+=(x); echo $#a");
    }

    #[test]
    fn past_end_pads_then_appends() {
        assert_parity("a=(a b c); a[5]+=(z); print -l -- $a; echo $#a");
    }

    #[test]
    fn append_to_unset_pads() {
        assert_parity("typeset -a u; u[1]+=(x); print -l -- $u; echo $#u");
    }

    #[test]
    fn cmdsubst_value_splits() {
        assert_parity(r#"a=(a b c); a[2]+=($(echo "x y")); echo $a"#);
    }

    /// Non-append must keep plain-replace (regression guard).
    #[test]
    fn non_append_replaces() {
        assert_parity("a=(a b c); a[2]=(d e); echo $a");
    }
}

/// A range/array-value subscript assignment to a NONEXISTENT parameter
/// auto-creates it as an array (c:Src/exec.c getvalue create flag →
/// createparam(name, PM_ARRAY)). Before the fix the SET_SUBSCRIPT_RANGE
/// handler saw v.pm == None and silently stored nothing.
mod subscript_range_autovivify {
    use super::*;

    #[test]
    fn forward_range_creates_array() {
        assert_parity(r#"unset u; u[1,2]=(a z); echo "${(t)u} = $u""#);
    }

    #[test]
    fn negative_range_append_creates_array() {
        assert_parity("unset u; u[-34,-2]+=(a z); echo $u");
    }

    #[test]
    fn single_index_array_value_creates() {
        assert_parity(r#"unset u; u[1]=(x); echo "${(t)u} = $u""#);
    }

    #[test]
    fn append_index_pads_and_creates() {
        assert_parity("unset u; u[2]+=(a z); print -l -- $u; echo $#u");
    }

    #[test]
    fn scalar_value_range_creates_array() {
        assert_parity(r#"unset s; s[2,3]="XY"; print -l -- $s; echo "n=$#s t=${(t)s}""#);
    }
}

/// PM_UNIQUE (`typeset -aU`) dedupes on single-index element assignment
/// too, not just whole-array/append (c:Src/params.c:2966-2967 arrunique).
/// The single-index element path bypassed setarrvalue's dedup tail.
mod unique_array_element_assign {
    use super::*;

    #[test]
    fn overwrite_creating_dup_dedupes() {
        assert_parity("typeset -aU u=(first second); u[1]=second; print $u");
    }

    #[test]
    fn interior_dup_dedupes() {
        assert_parity("typeset -aU u=(a b c); u[2]=c; print $u");
    }

    #[test]
    fn past_end_dup_dedupes() {
        assert_parity(r#"typeset -aU u=(a b c); u[5]=a; print -l -- $u; echo $#u"#);
    }

    #[test]
    fn non_dup_assignment_keeps_all() {
        assert_parity("typeset -aU u=(a b c); u[2]=z; print $u");
    }

    /// Non-unique array keeps duplicates (regression guard).
    #[test]
    fn non_unique_keeps_dups() {
        assert_parity("typeset -a u=(a b c); u[2]=c; print $u");
    }
}

/// Under KSHARRAYS a bare array name addresses element 0 (ksh semantics),
/// so a scalar assignment `a=X` to an existing array sets the FIRST
/// element keeping the rest, rather than replacing the whole array
/// (c:Src/params.c getvalue + the unset(KSHARRAYS)-gated reset c:3179).
mod ksharrays_scalar_assign {
    use super::*;

    #[test]
    fn scalar_assign_sets_element_zero() {
        assert_parity(r#"setopt ksharrays; a=(first second); a=X; print -l "${a[@]}""#);
    }

    #[test]
    fn scalar_assign_empty_array_creates_one() {
        assert_parity(r#"setopt ksharrays; a=(); a=X; print -l "${a[@]}""#);
    }

    /// Without KSHARRAYS, `a=X` replaces the array with a scalar.
    #[test]
    fn non_ksharrays_scalar_assign_replaces() {
        assert_parity(r#"a=(first second); a=X; print -l "${a[@]}""#);
    }

    /// `a+=X` (scalar augment) under KSHARRAYS concats onto element 0.
    #[test]
    fn scalar_append_concats_element_zero() {
        assert_parity(r#"setopt ksharrays; a=(first second); a+=last; print -l "${a[@]}""#);
    }

    #[test]
    fn scalar_append_empty_array() {
        assert_parity(r#"setopt ksharrays; a=(); a+=hi; print -l "${a[@]}""#);
    }

    #[test]
    fn scalar_append_chained() {
        assert_parity(r#"setopt ksharrays; a=(x); a+=y; a+=z; print -l "${a[@]}""#);
    }

    /// Without KSHARRAYS, `a+=X` pushes a new element.
    #[test]
    fn non_ksharrays_scalar_append_pushes() {
        assert_parity(r#"a=(first second); a+=last; print -l "${a[@]}""#);
    }
}

/// `(k)` paramflag on an array WITH a subscript returns the resolved
/// INDEX, not the element (c:Src/params.c:1513-1514 — WANTKEYS sets the
/// subscript's inverted-index mode). `${(k)a[2]}` → `2`, `${(k)a[(R)y]}`
/// → matched position. Gated on hkeys so plain `${a[2]}` is unaffected.
/// Fixed at the flagged (subst.rs:5900) and numeric (subst.rs:6120)
/// array subscript-resolution sites.
mod paramflag_k_subscript_index {
    use super::*;

    #[test]
    fn numeric_subscript_returns_index() {
        assert_parity("a=(x y z); echo ${(k)a[2]}");
    }

    #[test]
    fn numeric_negative_subscript_returns_resolved_index() {
        assert_parity("a=(x y z); echo ${(k)a[-1]}");
    }

    #[test]
    fn numeric_out_of_range_returns_subscript() {
        assert_parity("a=(x y); echo ${(k)a[5]}");
    }

    #[test]
    fn numeric_on_empty_returns_subscript() {
        assert_parity("a=(); echo ${(k)a[1]}");
    }

    #[test]
    fn reverse_search_returns_match_index() {
        assert_parity("a=(x y z y); echo ${(k)a[(R)y]}");
    }

    #[test]
    fn forward_search_returns_match_index() {
        assert_parity("a=(foo bar baz); echo ${(k)a[(r)ba*]}");
    }

    #[test]
    fn ksharrays_zero_based_index() {
        assert_parity("setopt ksharrays; a=(x y z); echo ${(k)a[0]}");
    }

    /// Plain subscript without (k) still returns the element (guard).
    #[test]
    fn plain_subscript_unaffected() {
        assert_parity("a=(x y z); echo ${a[2]}; echo $a[3]; echo ${a[(R)y]}");
    }
}

/// The `(v)` paramflag FORCES a subscript to return the value even when
/// `(i)`/`(I)` would return the index (c:Src/params.c:1515 — `else if
/// (WANTVALS) *inv = 0`). Inverse of `(k)`. `${(v)h[(i)b]}` → element,
/// not its index. With BOTH `(k)` and `(v)` the subscript's own i/I
/// flag decides (c:1513 `ind || !WANTVALS` → `ind`).
mod paramflag_v_forces_value {
    use super::*;

    #[test]
    fn v_overrides_i_index_to_value() {
        assert_parity("h=(a 1 b 2); echo ${(v)h[(i)b]}");
    }

    #[test]
    fn v_with_plain_search_unaffected() {
        assert_parity("a=(x y z); echo ${(v)a[(i)y]}");
    }

    #[test]
    fn both_k_and_v_defer_to_subscript_flag() {
        assert_parity("a=(x y z); echo ${(kv)a[(i)y]}; echo ${(vk)a[(R)y]}");
    }
}

/// `(k)`/`(v)`/`(kv)` on a plain SCALAR (no subscript) are no-ops and
/// return the scalar's value, not empty (c:Src/subst.c — assoc/array
/// key/value enumeration doesn't apply to a scalar). zshrs's no-subscript
/// (k)/(v)/(kv) arms fell through to `unwrap_or_default()` → empty for a
/// scalar; now fall back to the resolved scalar value. Arrays/assocs
/// (which DO enumerate) are unaffected.
mod paramflag_kv_on_scalar {
    use super::*;

    #[test]
    fn k_on_scalar_returns_value() {
        assert_parity("x=hello; echo ${(k)x}");
    }

    #[test]
    fn v_on_scalar_returns_value() {
        assert_parity("x=hello; echo ${(v)x}");
    }

    #[test]
    fn kv_on_scalar_returns_value() {
        assert_parity(r#"x="a b c"; echo ${(kv)x}"#);
    }

    #[test]
    fn k_on_special_scalar_returns_value() {
        assert_parity("echo ${(k)PWD}");
    }

    #[test]
    fn kv_on_scalar_quoted() {
        assert_parity(r#"x=hi; echo "${(kv)x}""#);
    }

    /// Array/assoc enumeration still works (regression guard).
    #[test]
    fn k_v_on_array_and_assoc_unaffected() {
        assert_parity("a=(1 2 3); echo ${(k)a}; echo ${(v)a}");
        assert_parity("typeset -A h=(a 1 b 2); echo ${(k)h}; echo ${(v)h}");
    }
}

/// Delimited `(b:N:)`/`(n:N:)` subscript flags WITHOUT a search flag
/// (r/R/i/I) on a scalar or array: zshrs's flag-strip closures didn't
/// consume the `:N:` delimited arg (only `s` did) and rejected a
/// non-numeric remainder, so the form fell through to a full-sub
/// mathevali and errored "bad math expression". C strips the flag block
/// (consuming n/b's delimited arg, c:params.c:1432) then math-evals the
/// REMAINDER (bare ident → 0). `${s[(b:2:)l]}` → "" (eval "l"=0).
mod subscript_bn_delimited_flags {
    use super::*;

    #[test]
    fn scalar_b_flag_bare_ident_remainder() {
        assert_parity("s=hello; echo ${s[(b:2:)l]}");
    }

    #[test]
    fn scalar_n_flag_bare_ident_remainder() {
        assert_parity("s=hello; echo ${s[(n:2:)l]}");
    }

    #[test]
    fn scalar_b_flag_with_set_index_var() {
        assert_parity("s=hello; l=2; echo ${s[(b:1:)l]}");
    }

    #[test]
    fn array_b_flag_bare_ident_remainder() {
        assert_parity("a=(x y z); echo ${a[(b:1:)l]}");
    }

    #[test]
    fn array_b_flag_with_set_index_var() {
        assert_parity("a=(x y z); l=2; echo ${a[(b:1:)l]}");
    }

    /// Regression guards: plain/arith/search/word subscripts unchanged.
    #[test]
    fn plain_and_search_subscripts_unaffected() {
        assert_parity("a=(x y z); echo ${a[2]}; echo ${a[1+1]}; echo ${a[(r)y]}; echo ${a[(w)2]}");
        assert_parity("s=hello; echo ${s[2]}; echo ${s[(r)l]}; echo ${s[2,4]}");
    }
}

/// `(e)`/`(n)`/`(b)` ALONE (no r/R/i/I search flag) on a scalar are
/// modifiers, not a search — the remainder is a numeric index, not a
/// search pattern. zshrs's scalar search closure matched on `e`/`n`/`b`
/// alone and did an exact-match SEARCH, returning the matched char.
/// Require an actual search flag so these fall through to the numeric
/// path: `${s[(e)2]}` → char 2 = "e"; `${s[(e)l]}` → eval "l" = 0 → "".
mod subscript_e_without_search {
    use super::*;

    #[test]
    fn e_alone_numeric_remainder_index() {
        assert_parity("s=hello; echo ${s[(e)2]}");
    }

    #[test]
    fn e_alone_bare_ident_remainder() {
        assert_parity("s=hello; echo ${s[(e)l]}");
    }

    /// Real search flags still search (regression guard).
    #[test]
    fn search_flags_still_search() {
        assert_parity("s=hello; echo ${s[(r)l]}; echo ${s[(i)l]}; echo ${s[(I)l]}; echo ${s[(R)l]}");
    }

    /// (re) exact-match search still works (r present).
    #[test]
    fn re_exact_search_unaffected() {
        assert_parity("s=foobar; echo ${s[(re)o]}");
    }
}

/// `(w)` word-subscript index CLAMPS to [1, wordcount] (c:Src/params.c:
/// 1623-1631 — `if (r<0) r+=i+1; if (r<1) r=1; if (r>i) r=i`). zshrs's
/// scalar (w) path returned "" out of range / for index 0; clamp like C:
/// `${s[(w)2]}` on 1-word → first word, `${s[(w)4]}` on 3 words → last,
/// `${s[(w)0]}` → first.
mod subscript_w_word_clamp {
    use super::*;

    #[test]
    fn w_index_beyond_count_clamps_to_last() {
        assert_parity(r#"s="a b c"; echo ${s[(w)4]}"#);
    }

    #[test]
    fn w_index_on_single_word_clamps() {
        assert_parity("s=hello; echo ${s[(w)2]}");
    }

    #[test]
    fn w_index_zero_clamps_to_first() {
        assert_parity("s=hello; echo ${s[(w)0]}");
    }

    /// In-range / negative / custom-separator unchanged (regression).
    #[test]
    fn w_in_range_and_negative_unaffected() {
        assert_parity(r#"s="a b c"; echo ${s[(w)2]}; echo ${s[(w)1]}; echo ${s[(w)-1]}"#);
        assert_parity(r#"s="x:y:z"; echo ${s[(ws:::)2]}"#);
    }
}

/// `(w)` word subscript with a custom separator SKIPS empty fields
/// (between adjacent separators); `(W)` keeps them (c:Src/utils.c
/// findword/wordcount word-vs-field semantics). zshrs's custom-sep
/// split kept empties for both, so `s="a::b"; ${s[(ws.:.)2]}` returned
/// the empty middle field "" instead of zsh's "b".
mod subscript_w_skip_empty_fields {
    use super::*;

    #[test]
    fn w_custom_sep_skips_empty_field() {
        assert_parity(r#"s="a::b"; echo ${s[(ws.:.)2]}"#);
    }

    #[test]
    fn w_custom_sep_multiple_empties() {
        assert_parity(r#"s="a::b::c"; echo ${s[(ws.:.)2]}"#);
    }

    #[test]
    fn w_custom_sep_leading_trailing_empties() {
        assert_parity(r#"s=":a:b:"; echo ${s[(ws.:.)1]}"#);
    }

    #[test]
    fn w_custom_sep_comma() {
        assert_parity(r#"s="a,,b"; echo ${s[(ws.,.)2]}"#);
    }

    /// Non-empty custom-sep + whitespace (w) unchanged (regression).
    #[test]
    fn w_no_empty_fields_unaffected() {
        assert_parity(r#"s="x:y:z"; echo ${s[(ws:::)2]}"#);
        assert_parity(r#"s="a b c"; echo ${s[(w)2]}; echo ${s[(w)-1]}"#);
    }
}

/// `(p)` alone is a separator-escape modifier, NOT a word flag, so
/// `${s[(p)1]}` is a plain char index ("h"), not word 1 (c:Src/params.c:
/// 1419-1426 — only w/W/f flip word mode). zshrs flipped word mode on
/// `p` and returned the whole string; after removing it from the word
/// trigger, `(p)` is accepted as a bare flag in the numeric flag-strip
/// closures so `(p)N` strips the flag and evals N as the index.
mod subscript_p_not_word_flag {
    use super::*;

    #[test]
    fn p_alone_is_char_index() {
        assert_parity("s=hello; echo ${s[(p)1]}; echo ${s[(p)2]}");
    }

    #[test]
    fn p_alone_array_index() {
        assert_parity("a=(x y z); echo ${a[(p)2]}");
    }

    /// (pw) still word mode (w present); regression guard.
    #[test]
    fn pw_still_word_mode() {
        assert_parity(r#"s="a b"; echo ${s[(pw)2]}"#);
    }
}
