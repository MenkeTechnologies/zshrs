//! Two families that both come down to state paramsubst was dropping.
//!
//! ## 1. A chained subscript after a slice whose bound is a pattern SEARCH
//!
//! `getindex` evaluates BOTH range bounds through `getarg`
//! (c:Src/params.c:2058 / :2133), so either may be `(r)pat` / `(R)pat` rather
//! than arithmetic. The port's chained-subscript arm parsed its bounds with a
//! plain integer/math parse, so `(r)pat` fell back to the 1 / len defaults and
//! `${a[1,(r)b][…]}` silently searched the WHOLE array.
//!
//! A search bound also raises `SCANPM_WANTVALS` (c:Src/params.c:1523), and
//! c:Src/subst.c:2896 `v->isarr = isarr` copies the WHOLE scanflags mask onto
//! the temporary Value the chained subscript indexes — so the bit survives the
//! chain. c:Src/params.c:1515 then forces `*inv = 0`, and a chained `(i)`/`(I)`
//! returns the matched ELEMENT instead of its index. That is the whole reason
//! `${a[1,(r)d][(I)C]}` is `C` while `${a[1,4][(I)C]}` is `3`.
//!
//! Named victim: `Completion/Base/Completer/_approximate:66-67` picks its group
//! option with `${argv[1,(r)-(-|)][(R)-*[JV]]}`.
//!
//! ## 2. `ssub` — c:Src/subst.c:1759 `pf_flags & PREFORK_SINGLE`
//!
//! The RHS of a scalar assignment is preforked with PREFORK_SINGLE
//! (c:Src/exec.c:2546). Inside paramsubst that bit does two things:
//!
//!   * c:3913 `int force_split = !ssub && (spbreak || spsep);` — a `(s:X:)` /
//!     `(f)` / `(0)` split does NOT run, so `r=${(s.c.)v[2,-1]}` keeps `bcdef`
//!     instead of splitting and re-joining on IFS as `b def`.
//!   * c:3916 `if (nojoin == 0 || sep) { val = sepjoin(aval, sep, 1);
//!     isarr = 0; }` — an array value is joined to ONE scalar before the
//!     c:4041 quote block, so `v=${(qq)a}` quotes the joined string once
//!     (`'x y z'`) rather than each element (`'x' 'y z'`). `(@)` (nojoin == 2,
//!     c:2165) is the documented exception and still quotes per element.
//!
//! Callers: `Completion/Base/Widget/_complete_debug:7` (`${(qq)…}` into a
//! scalar) and `Completion/Base/Widget/_complete_help:53`.
//!
//! Half the rows below are cases where the CURRENT answer is already right
//! (print context, double-quoted expansion, `(@)`, a plain numeric slice).
//! They are here deliberately: a fix that just suppresses splitting everywhere,
//! or joins on every path, passes the bug rows and breaks these.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// stdout + exit-status parity against the reference `zsh`.
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");

    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );
    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Chained subscript after a search-flag slice bound.
// ═══════════════════════════════════════════════════════════════════════════

mod search_flag_slice_bound {
    use super::*;

    const A: &str = r#"a=(Alpha beta Gamma delta); "#;

    /// The reported repro, all three shapes in one word.
    #[test]
    fn reported_repro_three_chains() {
        assert_parity(
            r#"a=(A b C d); print "[${a[1,(r)b][(R)C]}][${a[1,(r)d][(I)C]}][${a[(r)d,4][(I)C]}]""#,
        );
    }

    /// The END bound is a forward search: the sub-array stops at `beta`, so
    /// element 3 of it does not exist.
    #[test]
    fn end_bound_search_bounds_the_sub_array() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)beta][3]}}]""#));
    }

    /// …and element 2 of that same sub-array does.
    #[test]
    fn end_bound_search_keeps_in_range_element() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)beta][2]}}]""#));
    }

    /// The START bound is a reverse search.
    #[test]
    fn start_bound_search_offsets_the_sub_array() {
        assert_parity(&format!(r#"{A}print -r "[${{a[(R)beta,4][1]}}]""#));
    }

    /// c:1523 WANTVALS: a chained `(I)` after an `(r)` bound returns the
    /// ELEMENT, not the index.
    #[test]
    fn wantvals_makes_chained_I_return_the_element() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)Gamma][(I)beta]}}]""#));
    }

    /// Same for lowercase `(i)`.
    #[test]
    fn wantvals_makes_chained_i_return_the_element() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)Gamma][(i)beta]}}]""#));
    }

    /// A forward miss under WANTVALS lands past the end → empty, not `len+1`.
    #[test]
    fn wantvals_forward_miss_is_empty_not_len_plus_one() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)Gamma][(i)zz]}}]""#));
    }

    /// A reverse miss under WANTVALS is empty, not `0`.
    #[test]
    fn wantvals_reverse_miss_is_empty_not_zero() {
        assert_parity(&format!(r#"{A}print -r "[${{a[1,(r)Gamma][(I)zz]}}]""#));
    }

    /// `k` degrades to `r` on a non-hash (c:1400/1405 gate `keymatch` on
    /// `ishash`), so it raises WANTVALS the same way.
    #[test]
    fn k_bound_on_array_also_raises_wantvals() {
        assert_parity(&format!(r#"{A}print -r "[${{a[(k)beta,4][(i)delta]}}]""#));
    }

    /// CONTROL: with NUMERIC bounds no WANTVALS is raised, so a chained
    /// `(I)` still returns the INDEX. A fix that always returns the element
    /// breaks this row.
    #[test]
    fn numeric_bounds_keep_the_index_answer() {
        assert_parity(r#"a=(A b C d); print -r "[${a[1,3][(I)C]}][${a[1,4][(I)C]}]""#);
    }

    /// CONTROL: a non-range search subscript clears scanflags (c:2158, `com`
    /// is 0), so the chain indexes CHARACTERS of the matched element.
    #[test]
    fn non_range_search_chains_into_characters() {
        assert_parity(&format!(r#"{A}print -r "[${{a[(r)Gamma][1]}}][${{a[(r)Gamma][2]}}]""#));
    }

    /// CONTROL: numeric slice, numeric chain — unchanged.
    #[test]
    fn numeric_slice_then_numeric_chain() {
        assert_parity(r#"a=(A b C d); print -r "[${a[1,3][2,3]}][${a[1,3][2]}]""#);
    }

    /// The `_approximate` shape verbatim (c:Completion/Base/Completer/_approximate:66).
    #[test]
    fn approximate_group_option_shape() {
        assert_parity(
            r#"argv=(-J -group -V -other); print -r "[${argv[1,(r)-(-|)][(R)-*[JV]]}]""#,
        );
    }

    /// `_print`'s `->prompt` decision (unquoted scalar-assignment RHS).
    #[test]
    fn print_completer_no_match_stays_empty() {
        assert_parity(r#"w=(print -); C=2; v=${w[1,C][(r)zz]}; print -r -- "[$v]""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Chained RANGE subscript inside a nested expansion (`Comma` token).
// ═══════════════════════════════════════════════════════════════════════════

mod chained_range_in_nested_expansion {
    use super::*;

    /// The reported repro. The inner `[2,-1]` arrives with `,` lexed to the
    /// `Comma` token (c:Src/zsh.h:181) because the nested body is unquoted, so
    /// the range went unrecognised and the chain read character 1.
    #[test]
    fn reported_repro_split_then_join() {
        assert_parity(r#"a=(abcdef); r=${(j:,:)${(s.c.)a[1][2,-1]}}; print -r -- "[$r]""#);
    }

    #[test]
    fn nested_chained_range_without_flags() {
        assert_parity(r#"a=(abcdef ghij); r=${${a[2][3,4]}}; print -r -- "[$r]""#);
    }

    #[test]
    fn nested_chained_range_under_case_flag() {
        assert_parity(r#"a=(abcdef ghij); r=${(U)${a[2][3,4]}}; print -r -- "[$r]""#);
    }

    #[test]
    fn nested_chained_range_negative_end() {
        assert_parity(r#"a=(abcdef); r=${(j:,:)${a[1][2,-1]}}; print -r -- "[$r]""#);
    }

    /// CONTROL: a chained SINGLE index in the same position already worked.
    #[test]
    fn nested_chained_single_index() {
        assert_parity(r#"a=(abcdef ghij); r=${${a[2][2]}}; print -r -- "[$r]""#);
    }

    /// CONTROL: the same expression in print context and in double quotes.
    #[test]
    fn print_context_and_quoted_agree() {
        assert_parity(
            r#"a=(abcdef ghij); print -r -- "[${${a[2][3,4]}}]"; r="${${a[2][3,4]}}"; print -r -- "[$r]""#,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. ssub: c:3913 force_split is off on a scalar-assignment RHS.
// ═══════════════════════════════════════════════════════════════════════════

mod ssub_suppresses_split {
    use super::*;

    /// The reported repro: a `(s:X:)` split on a SUBSCRIPTED scalar.
    #[test]
    fn s_flag_on_scalar_slice_does_not_split() {
        assert_parity(r#"v=abcdef; r=${(s.c.)v[2,-1]}; print -r -- "[$r]""#);
    }

    #[test]
    fn s_flag_on_scalar_slice_positive_bounds() {
        assert_parity(r#"v=abcdef; r=${(s.c.)v[2,6]}; print -r -- "[$r]""#);
    }

    #[test]
    fn s_flag_on_array_element_slice() {
        assert_parity(r#"a=(abcdef); r=${(s.c.)a[1,1]}; print -r -- "[$r]""#);
    }

    #[test]
    fn s_flag_on_bare_scalar_does_not_split() {
        assert_parity(r#"v=abcdef; r=${(s.c.)v}; print -r -- "[$r]""#);
    }

    /// Double quotes do not change it — `ssub` comes from the assignment, not
    /// from the quoting.
    #[test]
    fn quoted_rhs_also_does_not_split() {
        assert_parity(r#"v=abcdef; r="${(s.c.)v[2,-1]}"; print -r -- "[$r]""#);
    }

    #[test]
    fn f_flag_on_scalar_assignment_keeps_newlines() {
        assert_parity(r#"p=$(printf 'a\nb\n'); r=${(f)p}; print -r -- "[$r]""#);
    }

    /// CONTROL: with no `ssub` the split DOES run — one word per field.
    #[test]
    fn print_context_still_splits() {
        assert_parity(r#"v=abcdef; print -rl -- ${(s.c.)v[2,-1]}"#);
    }

    /// CONTROL: an ARRAY assignment is not `ssub` (c:2546 passes only
    /// PREFORK_ASSIGN), so the split survives into the elements.
    #[test]
    fn array_assignment_still_splits() {
        assert_parity(r#"v="a:b:c"; x=(${(s.:.)v}); print -r -- "$#x""#);
    }

    /// CONTROL: a quoted word in ARGV keeps its split words (c:3317 `!spsep`).
    #[test]
    fn quoted_argv_word_keeps_split_words() {
        assert_parity(r#"v="a:b:c"; set -- "${(s.:.)v}"; print -r -- "$#""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. ssub: c:3916 joins the array before the c:4041 quote block.
// ═══════════════════════════════════════════════════════════════════════════

mod ssub_joins_before_quoting {
    use super::*;

    const A: &str = r#"a=(x "y z"); "#;

    /// The reported repro.
    #[test]
    fn qq_quotes_the_joined_scalar() {
        assert_parity(&format!(r#"{A}v=${{(qq)a}}; print -r -- "[$v]""#));
    }

    #[test]
    fn q_quotes_the_joined_scalar() {
        assert_parity(&format!(r#"{A}v=${{(q)a}}; print -r -- "[$v]""#));
    }

    #[test]
    fn q_minus_quotes_the_joined_scalar() {
        assert_parity(&format!(r#"{A}v=${{(q-)a}}; print -r -- "[$v]""#));
    }

    #[test]
    fn qqq_quotes_the_joined_scalar() {
        assert_parity(&format!(r#"{A}v=${{(qqq)a}}; print -r -- "[$v]""#));
    }

    /// A `[@]` / `[*]` subscript sets SCANPM_ISVAR_AT but leaves `nojoin` at
    /// 0, so c:3916 still joins.
    #[test]
    fn at_and_star_subscripts_still_join() {
        assert_parity(&format!(
            r#"{A}v=${{(qq)a[@]}}; w=${{(qq)a[*]}}; print -r -- "[$v][$w]""#
        ));
    }

    #[test]
    fn positional_params_join() {
        assert_parity(r#"set -- a "b c"; v=${(qq)@}; print -r -- "[$v]""#);
    }

    /// The join uses `sepjoin`'s separator rules, i.e. `$IFS[1]`.
    #[test]
    fn join_honours_ifs_first_char() {
        assert_parity(r#"IFS=,; a=(x y); v=${(qq)a}; print -r -- "[$v]""#);
    }

    /// A bare association expands to its VALUE list, which joins the same way.
    #[test]
    fn assoc_values_join_before_quoting() {
        assert_parity(r#"typeset -A h=(k1 v1 k2 v2); v=${(qq)h}; print -r -- "[$v]""#);
    }

    #[test]
    fn local_declaration_is_also_ssub() {
        assert_parity(&format!(r#"{A}local v=${{(qq)a}}; print -r -- "[$v]""#));
    }

    #[test]
    fn typeset_declaration_is_also_ssub() {
        assert_parity(&format!(r#"{A}typeset v=${{(qq)a}}; print -r -- "[$v]""#));
    }

    /// CONTROL: `(@)` sets nojoin = 2 (c:2165) so c:3916 does NOT join and the
    /// quoting stays per element.
    #[test]
    fn at_flag_keeps_per_element_quoting() {
        assert_parity(&format!(r#"{A}v=${{(@qq)a}}; print -r -- "[$v]""#));
    }

    /// CONTROL: an explicit `(j:X:)` separator already forced the join, and it
    /// must keep using ITS separator rather than the IFS one.
    #[test]
    fn explicit_join_separator_wins() {
        assert_parity(&format!(r#"{A}v=${{(qqj:-:)a}}; print -r -- "[$v]""#));
    }

    /// CONTROL: in ARGV (no ssub) `(qq)` quotes per element.
    #[test]
    fn argv_context_quotes_per_element() {
        assert_parity(&format!(r#"{A}print -rl -- ${{(qq)a}}"#));
    }

    /// CONTROL: the other per-element flag arms that share C's `if (isarr)`
    /// gate must agree too.
    #[test]
    fn V_and_Q_flags_agree_under_ssub() {
        assert_parity(&format!(
            r#"{A}v=${{(V)a}}; w=${{(Q)a}}; print -r -- "[$v][$w]""#
        ));
    }
}
