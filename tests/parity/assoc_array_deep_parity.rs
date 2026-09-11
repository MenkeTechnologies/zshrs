//! Associative-array deep parity:
//! `${(k)H}`, `${(v)H}`, `${(kv)H}`, sorted iteration,
//! `for k v in ${(kv)H}`, ${#H}, delete-key, nested.

#![allow(non_snake_case)]

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

mod creation {
    use super::*;

    /// `typeset -A H` declares assoc array.
    #[test]
    fn typeset_A_basic() {
        assert_parity(r#"typeset -A H; H[k]=v; echo "$H[k]""#);
    }

    /// `H=(k v k v)` bulk-init.
    #[test]
    fn bulk_init_pairs() {
        assert_parity(r#"typeset -A H=(k1 v1 k2 v2); echo "${H[k1]}|${H[k2]}""#);
    }

    /// Empty H.
    #[test]
    fn empty_assoc_count_zero() {
        assert_parity(r#"typeset -A H; echo "${#H}""#);
    }
}

mod lookup {
    use super::*;

    #[test]
    fn lookup_existing_key() {
        assert_parity(r#"typeset -A H; H[name]=jacob; echo "${H[name]}""#);
    }

    #[test]
    fn lookup_missing_key_empty() {
        assert_parity(r#"typeset -A H; H[a]=1; echo "[${H[nonexistent]}]""#);
    }

    /// Key with special chars.
    #[test]
    fn lookup_key_with_space() {
        assert_parity(r#"typeset -A H; H["a b"]=value; echo "${H["a b"]}""#);
    }
}

mod count {
    use super::*;

    #[test]
    fn count_three_entries() {
        assert_parity(r#"typeset -A H=(a 1 b 2 c 3); echo "${#H}""#);
    }

    #[test]
    fn count_after_add() {
        assert_parity(r#"typeset -A H=(a 1); H[b]=2; echo "${#H}""#);
    }

    #[test]
    fn count_after_delete() {
        assert_parity(r#"typeset -A H=(a 1 b 2 c 3); unset 'H[b]'; echo "${#H}""#);
    }
}

mod keys_values {
    use super::*;

    /// `${(k)H}` returns keys.
    #[test]
    fn flag_k_returns_keys_sorted() {
        // Use sort to make order deterministic across shells.
        assert_parity(r#"typeset -A H=(c 1 a 2 b 3); print -l "${(@k)H}" | sort"#);
    }

    /// `${(v)H}` returns values.
    #[test]
    fn flag_v_returns_values_sorted() {
        assert_parity(r#"typeset -A H=(c 30 a 10 b 20); print -l "${(@v)H}" | sort -n"#);
    }

    /// `${(kv)H}` interleaves keys+values.
    #[test]
    fn flag_kv_pairs() {
        // Sorted alternate-line output.
        assert_parity(r#"typeset -A H=(b 2 a 1); print -l "${(@kv)H}" | sort"#);
    }
}

mod iteration {
    use super::*;

    /// `for k v in ${(kv)H}` iterates pairs.
    #[test]
    fn for_loop_over_kv_pairs() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2 c 3)
for k v in "${(@kv)H}"; do
  echo "$k=$v"
done | sort
"#,
        );
    }

    /// Iterate keys only.
    #[test]
    fn for_loop_over_keys() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2)
for k in "${(@k)H}"; do
  echo "$k"
done | sort
"#,
        );
    }
}

mod delete {
    use super::*;

    /// `unset 'H[k]'` removes one key.
    #[test]
    fn unset_single_key() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2 c 3)
unset 'H[b]'
echo "${(@k)H}" | tr ' ' '\n' | sort
"#,
        );
    }

    /// `unset H` clears whole hash.
    #[test]
    fn unset_whole_hash() {
        assert_parity(r#"typeset -A H=(a 1 b 2); unset H; echo "${#H}""#);
    }
}

mod overwrite_and_extend {
    use super::*;

    /// Overwriting value.
    #[test]
    fn overwrite_existing_value() {
        assert_parity(r#"typeset -A H; H[k]=v1; H[k]=v2; echo "${H[k]}""#);
    }

    /// `H+=( k v )` add pair.
    #[test]
    fn extend_with_plus_eq() {
        assert_parity(r#"typeset -A H=(a 1); H+=(b 2 c 3); echo "${#H}""#);
    }
}

mod subscript_flags {
    use super::*;

    /// `${H[(I)pat]}` pattern-key lookup.
    #[test]
    fn flag_I_pattern_lookup() {
        assert_parity(
            r#"
typeset -A H=(apple 1 banana 2 cherry 3)
echo "${H[(I)b*]}"
"#,
        );
    }

    /// `${(M)H[(I)*a*]}` match-only modifier.
    #[test]
    fn flag_I_returns_all_matches() {
        assert_parity(
            r#"
typeset -A H=(apple 1 banana 2 cherry 3)
print -l "${(@k)H[(I)*a*]}" | sort
"#,
        );
    }
}

mod special_keys {
    use super::*;

    /// Empty string key.
    #[test]
    fn empty_string_key() {
        assert_parity(r#"typeset -A H; H[""]=emptykey; echo "[${H[""]}]""#);
    }

    /// Numeric-looking key.
    #[test]
    fn numeric_key_treated_as_string() {
        assert_parity(r#"typeset -A H; H[42]=meaning; echo "${H[42]}""#);
    }

    /// Key with $ char (literal).
    #[test]
    fn key_with_dollar_literal() {
        assert_parity(r#"typeset -A H; H['$weird']=ok; echo "${H['$weird']}""#);
    }
}

mod array_of_keys_in_subst {
    use super::*;

    /// `${H[$KEY]}` indirect via var.
    #[test]
    fn indirect_key_via_var() {
        assert_parity(r#"typeset -A H=(a 1 b 2); K=a; echo "${H[$K]}""#);
    }
}

mod assoc_in_function {
    use super::*;

    #[test]
    fn assoc_inside_function_local() {
        assert_parity(
            r#"
f() {
  typeset -A LOCAL_H
  LOCAL_H[x]=10
  echo "${LOCAL_H[x]}"
}
f
echo "outside=[${LOCAL_H[x]}]"
"#,
        );
    }
}

/// Nested subscript on a `(P)`-indirect reference to an ASSOC does key
/// lookup on the referenced param: `${${(P)n}[key]}` ≡ `${h[key]}` when
/// $n names assoc h (c:Src/subst.c (P) named-ref). The port flattened
/// the inner `${(P)n}` to its values first, so the outer string-key
/// subscript returned all values instead of indexing.
mod p_flag_indirect_assoc_subscript {
    use super::*;

    #[test]
    fn string_key_lookup() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -r - ${${(P)n}[b]}"#);
    }

    #[test]
    fn variable_key_lookup() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; k=a; print -r - ${${(P)n}[$k]}"#);
    }

    #[test]
    fn quote_flagged_outer() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - ${(q-)${(P)n}[b]}"#);
    }

    #[test]
    fn positional_ref_name() {
        assert_parity(r#"typeset -A opts=(a 1 b 2); set -- opts; print -r - ${${(P)1}[b]}"#);
    }

    /// `@`/`*` on a `(P)`-assoc still yields the values (regression guard).
    #[test]
    fn splat_yields_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - "${(@kP)n}""#);
    }

    /// Numeric subscript on a `(P)`-indexed-array still indexes (guard).
    #[test]
    fn p_array_numeric_subscript() {
        assert_parity(r#"typeset -a arr=(x y z); n=arr; print -r - ${${(P)n}[2]}"#);
    }

    /// Plain nested array subscript unaffected (regression guard).
    #[test]
    fn plain_nested_array_subscript() {
        assert_parity(r#"arr=(hello world); print -r - ${${arr}[2]}"#);
    }
}

mod assignment_subscript_expansion {
    use super::*;

    /// An assignment subscript is expanded before it is used as a key
    /// (`parsestr` + `singsub`, Src/params.c:1585-1592). The compiler
    /// recognises `$(( … ))` only in the lexer's tokenized spelling, and the
    /// assoc-assign path handed it the UNTOKENIZED subscript text, so a key
    /// that mixed literal text with arithmetic was stored verbatim. p10k hit
    /// this with `_p9k__prompt_char_saved[left14$((!_p9k__status))]`.
    #[test]
    fn arith_subscript_after_literal_text() {
        assert_parity(r#"typeset -A h; h[x$((1+1))]=v; print -r -- ${(k)h}"#);
    }

    /// Same shape with the arithmetic FIRST and literal text after.
    #[test]
    fn arith_subscript_before_literal_text() {
        assert_parity(r#"typeset -A h; h[$((1+1))x]=v; print -r -- ${(k)h}"#);
    }

    /// Arithmetic reading a parameter, wrapped in literal text on both sides.
    #[test]
    fn arith_subscript_with_param_between_text() {
        assert_parity(r#"typeset -A h; s=3; h[a$((s+1))b]=v; print -r -- ${(k)h}"#);
    }

    /// Command substitution in the same position — the other shape the
    /// untokenized text hid from the word compiler.
    #[test]
    fn cmdsubst_subscript_after_literal_text() {
        assert_parity(r#"typeset -A h; h[x$(echo 2)]=v; print -r -- ${(k)h}"#);
    }

    /// The `+=` form compiles the key twice; both sites must expand it.
    #[test]
    fn arith_subscript_append_uses_same_key() {
        assert_parity(
            r#"typeset -A h; h[x$((1+1))]=a; h[x$((1+1))]+=b; print -r -- "${(kv)h}""#,
        );
    }

    /// Guard: a subscript that is ONLY arithmetic already worked and must stay.
    #[test]
    fn bare_arith_subscript_still_expands() {
        assert_parity(r#"typeset -A h; h[$((1+1))]=v; print -r -- ${(k)h}"#);
    }

    /// Guard: a literal key with no expansion is stored verbatim, and a
    /// `(e)` flag group still forces the literal reading.
    #[test]
    fn literal_and_e_flag_keys_unchanged() {
        assert_parity(r#"typeset -A h; h[k2]=v; h[(e)*]=w; print -r -- ${(ok)h}"#);
    }
}

/// Bug #1141 — a chained `[N]` after an ASSOC pattern-scan subscript.
///
/// C never special-cases the second subscript: `paramsubst`'s
/// `while (v || …)` loop packs the first subscript's ARRAY result into a
/// temporary `PM_ARRAY` parameter (`Src/subst.c:2890-2893`) carrying the scan
/// mask (`:2897`) and runs an ordinary `getindex` on it (`:2900`). So
/// `${A[(K)pat][2]}` is an ordinary array index over the match list, and the
/// carried `SCANPM_WANTKEYS`/`SCANPM_WANTVALS` bits decide (`Src/params.c:1513-
/// 1531`) whether the chained subscript reads an ELEMENT or is an INVERSE
/// subscript that yields the index itself.
///
/// The port used to apply the second subscript only when the FIRST one was a
/// range on a plain array, so every row below returned the whole match list.
mod chained_subscript_after_scan {
    use super::*;

    /// Three keys that all match `zzqaaa` as patterns, so `(K)` returns a
    /// three-element list and the chained index has something to pick from.
    const A: &str = r#"typeset -A A; A[zzq*]=_A; A[*aaa]=_B; A[z*a]=_C; "#;

    fn p(expr: &str) {
        assert_parity(&format!(r#"{A}print -r -- {expr}"#));
    }

    /// The scan itself: the baseline both shells already agreed on.
    #[test]
    fn bare_scan_is_the_whole_match_list() {
        p(r#""${A[(K)zzqaaa]}""#);
    }

    /// `[1]`/`[2]`/`[3]` pick single matches out of the scan result.
    #[test]
    fn positive_index_picks_one_match() {
        p(r#""${A[(K)zzqaaa][1]}""#);
        p(r#""${A[(K)zzqaaa][2]}""#);
        p(r#""${A[(K)zzqaaa][3]}""#);
    }

    /// Past the end is empty, not "the whole list".
    #[test]
    fn out_of_range_index_is_empty() {
        p(r#""${A[(K)zzqaaa][4]}""#);
        p(r#""${A[(K)zzqaaa][99]}""#);
    }

    /// Negative indices count from the end of the MATCH LIST.
    #[test]
    fn negative_index_counts_from_the_end() {
        p(r#""${A[(K)zzqaaa][-1]}""#);
        p(r#""${A[(K)zzqaaa][-2]}""#);
        p(r#""${A[(K)zzqaaa][-3]}""#);
    }

    /// `[0]` is the KSH_ZERO_SUBSCRIPT empty range (`Src/params.c:2162-2171`).
    #[test]
    fn zero_index_is_empty() {
        p(r#""${A[(K)zzqaaa][0]}""#);
    }

    /// A chained RANGE slices the match list.
    #[test]
    fn range_slices_the_match_list() {
        p(r#""${A[(K)zzqaaa][1,2]}""#);
        p(r#""${A[(K)zzqaaa][2,3]}""#);
        p(r#""${A[(K)zzqaaa][2,-1]}""#);
        p(r#""${A[(K)zzqaaa][3,9]}""#);
    }

    /// `[@]`/`[*]` is the whole temp array (`Src/params.c:2048-2053`), not an
    /// index — the one chained subscript that keeps every match.
    #[test]
    fn splat_keeps_every_match() {
        p(r#""${A[(K)zzqaaa][@]}""#);
        p(r#""${A[(K)zzqaaa][*]}""#);
    }

    /// `(R)` matches VALUES and returns them all, so it chains identically.
    #[test]
    fn value_scan_chains_the_same_way() {
        p(r#""${A[(R)_*]}""#);
        p(r#""${A[(R)_*][2]}""#);
        p(r#""${A[(R)_*][-1]}""#);
    }

    /// Lowercase `(k)`/`(r)` clear SCANPM_MATCHMANY (`Src/params.c:1528-1529`),
    /// so the list holds ONE match and `[2]` is empty.
    #[test]
    fn single_match_scan_has_no_second_element() {
        p(r#""${A[(k)zzq*]}""#);
        p(r#""${A[(k)zzq*][1]}""#);
        p(r#""${A[(k)zzq*][2]}""#);
        p(r#""${A[(r)_*][1]}""#);
        p(r#""${A[(r)_*][2]}""#);
    }

    /// A scan with no match chains to empty rather than erroring.
    #[test]
    fn empty_scan_chains_to_empty() {
        p(r#""${A[(K)nomatch]}""#);
        p(r#""${A[(K)nomatch][1]}""#);
        p(r#""${A[(K)nomatch][1,2]}""#);
    }

    /// `(i)`/`(I)` set SCANPM_WANTKEYS and clear WANTVALS, so the chained
    /// subscript is INVERSE: it yields the index itself, undereferenced
    /// (`Src/params.c:2114-2118` then `:2336-2339`).
    #[test]
    fn index_scan_makes_the_chain_inverse() {
        p(r#""${A[(i)zzq*][2]}""#);
        p(r#""${A[(i)zzq*][9]}""#);
        p(r#""${A[(i)zzq*][0]}""#);
        p(r#""${A[(I)*][2]}""#);
    }

    /// A negative inverse index is folded against the match count first
    /// (`Src/subst.c:2945-2948`), so it can still land out of range.
    #[test]
    fn negative_inverse_index_folds_against_the_match_count() {
        p(r#""${A[(I)*][-1]}""#);
        p(r#""${A[(I)*][-3]}""#);
        p(r#""${A[(I)*][-9]}""#);
    }

    /// A chained FLAG subscript searches the match list: `(r)`/`(R)` return the
    /// element, `(i)`/`(I)` the position — and the carried WANTKEYS from an
    /// `(i)` first subscript flips even `(r)` to the inverse reading.
    #[test]
    fn chained_flag_subscript_searches_the_match_list() {
        p(r#""${A[(K)zzqaaa][(r)_C]}""#);
        p(r#""${A[(K)zzqaaa][(R)_*]}""#);
        p(r#""${A[(K)zzqaaa][(i)_C]}""#);
        p(r#""${A[(K)zzqaaa][(I)_C]}""#);
        p(r#""${A[(I)*][(i)z*a]}""#);
        p(r#""${A[(I)*][(I)*]}""#);
        p(r#""${A[(i)zzq*][(r)x]}""#);
    }

    /// An outer `(k)`/`(v)`/`(kv)` supplies the mask instead, overriding what
    /// the subscript flag would have set (`Src/params.c:1513-1516`).
    #[test]
    fn outer_key_value_flags_decide_the_chain_mask() {
        p(r#""${(k)A[(R)_*][1]}""#);
        p(r#""${(v)A[(I)*][1]}""#);
        p(r#""${(kv)A[(K)zzqaaa][1]}""#);
        p(r#""${(kv)A[(K)zzqaaa][2]}""#);
    }

    /// The chained subscript changes the WORD COUNT, not just the text: the
    /// pre-fix port produced three words where zsh produces one.
    #[test]
    fn chained_index_narrows_the_word_count() {
        assert_parity(&format!(
            r#"{A}b=( "${{(@)A[(K)zzqaaa][2]}}" ); print $#b"#
        ));
        assert_parity(&format!(r#"{A}b=( ${{A[(K)zzqaaa][2]}} ); print $#b"#));
        assert_parity(&format!(
            r#"{A}b=( "${{(@)A[(K)zzqaaa][1,2]}}" ); print $#b"#
        ));
        assert_parity(&format!(
            r#"{A}b=( "${{(@)A[(K)zzqaaa][@]}}" ); print $#b"#
        ));
    }

    /// Unquoted spelling takes a different arm of `paramsubst` than the
    /// double-quoted one, so both are pinned (cf. 8246d6889b).
    #[test]
    fn unquoted_spelling_chains_too() {
        p(r#"${A[(K)zzqaaa][1]}"#);
        p(r#"${A[(K)zzqaaa][-1]}"#);
        p(r#"${A[(K)zzqaaa][1,2]}"#);
        p(r#"${A[(R)_*][2]}"#);
        p(r#"${A[(i)zzq*][2]}"#);
    }

    /// The `_patcomps` shape `compinit` reads at sh:311 —
    /// `${_patcomps[(K)$svc][1]}` must be the FIRST matching handler, not the
    /// joined list of all of them.
    #[test]
    fn compinit_patcomps_first_match() {
        assert_parity(
            r#"typeset -A pc; pc[zzq*]=_A; pc[*aaa]=_B; svc=zzqaaa; print -r -- "${pc[(K)$svc][1]}""#,
        );
        assert_parity(
            r#"typeset -A pc ppc; ppc[zzq*]=_A; ppc[*aaa]=_B; svc=zzqaaa; print -r -- "${${pc[(K)$svc][1]}:-${ppc[(K)$svc][1]}}""#,
        );
    }

    /// Guards for the plain-array chaining this fix routes through: the shared
    /// path must keep answering these exactly as before.
    #[test]
    fn plain_array_chaining_unchanged() {
        let a = "a=(one two three four); ";
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,2][1]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,3][2]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,3][2,3]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1][2]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,4][(I)three]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,(r)four][(I)three]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,4][(i)zzz]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,4][(I)zzz]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,4][(r)zzz]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1,3][@]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[(R)*o*][1]}}""#));
        assert_parity(&format!(r#"{a}print -r -- "${{a[1]}}" "${{a[-1]}}" "${{a[2,4]}}""#));
    }
}
