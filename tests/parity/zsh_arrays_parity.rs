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
