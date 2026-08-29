//! Bug #1111 — `${arr[i]:-DEF}` with a BARE-NAME (or arithmetic) subscript.
//!
//! `getindex` → `getarg` math-evaluates the subscript text
//! (c:Src/params.c:1419-1484) and stores the result in `v->start`; paramsubst
//! then derives `vunset` from THAT SAME index against the array length
//! (c:Src/subst.c:2944-2954). The port split those into two independent
//! expressions and the set-ness side only accepted a digit literal, so a bare
//! name (`${arr[i]}`) or an arith expression (`${arr[i+j]}`) resolved fine for
//! the VALUE but read as "unset" for the operator, and every default-family
//! operator (`:-` `-` `:+` `+` `:=` `=` `:?` `?`) took its unset branch.
//!
//! The real-world shape is `${funcfiletrace[i]:-(unknown)}` in a loop over
//! `$funcstack`, which printed `(unknown)` for every frame.
//!
//! Half of these cases are rows where the DEFAULT is the CORRECT answer
//! (out-of-range index, empty element under `:-`, unset array, index 0). They
//! are here deliberately: the cheap "just treat any subscript as set" fix
//! passes the bug rows and breaks every one of these.
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

/// stdout + exit-status parity. stderr is compared only after stripping the
/// leading shell name, which differs by construction (`zsh:` vs `zshrs:`).
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

    // `zshrs --zsh` reports itself as `zsh:` in diagnostics, but a build that
    // does not emulate would say `zshrs:` — normalize both spellings on both
    // sides so the test pins the MESSAGE, not the shell's own name.
    let norm = |s: &str| s.replace("zshrs:", "SHELL:").replace("zsh:", "SHELL:");
    let z_err = norm(&String::from_utf8_lossy(&z.stderr));
    let r_err = norm(&String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        z_err, r_err,
        "stderr divergence on script:\n{script}\n--- zsh ---\n{z_err:?}\n--- zshrs ---\n{r_err:?}"
    );

    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The bug: bare-name subscript under every default-family operator.
// zsh answers with the ELEMENT in all of these.
// ═══════════════════════════════════════════════════════════════════════════

mod bare_name_subscript {
    use super::*;

    #[test]
    fn colon_dash_takes_the_element_not_the_default() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn colon_plus_sees_the_element_as_set() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]:+SET}]""#);
    }

    #[test]
    fn colon_assign_does_not_overwrite() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]:=DEF}]" "[${arr[1]}]""#);
    }

    #[test]
    fn colon_question_does_not_abort() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]:?msg}]""#);
    }

    #[test]
    fn plain_dash_takes_the_element() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]-DEF}]""#);
    }

    #[test]
    fn plain_plus_sees_the_element_as_set() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]+SET}]""#);
    }

    #[test]
    fn plain_assign_does_not_overwrite() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i]=DEF}]" "[${arr[1]}]""#);
    }

    #[test]
    fn arithmetic_expression_subscript() {
        assert_parity(r#"arr=(x y z); i=1; j=1; print "[${arr[i+j]:-DEF}]""#);
    }

    #[test]
    fn arithmetic_expression_subscript_no_colon() {
        assert_parity(r#"arr=(x y z); i=1; j=1; print "[${arr[i+j]-DEF}]""#);
    }

    #[test]
    fn scalar_character_subscript() {
        assert_parity(r#"s=abc; i=1; print "[${s[i]:-DEF}]""#);
    }

    #[test]
    fn negative_bare_name_subscript() {
        assert_parity(r#"arr=(x y z); i=-1; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn nested_arith_subscript() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[$((i+1))]:-DEF}]""#);
    }

    /// The shape that made the bug visible: a loop over `$funcstack`
    /// indexing `$funcfiletrace` with the loop counter.
    #[test]
    fn funcfiletrace_shape_loop_index() {
        assert_parity(
            r#"trace=(fileA:1 fileB:2 fileC:3)
for i in 1 2 3 4; do print "$i=${trace[i]:-(unknown)}"; done"#,
        );
    }

    /// A bare name that is NOT a set variable math-evaluates to 0, so the
    /// index is 0 and the default IS correct. A fix that special-cased
    /// "subscript is an identifier ⇒ set" would break this.
    #[test]
    fn unset_name_subscript_evaluates_to_zero_and_takes_default() {
        assert_parity(r#"arr=(x y z); print "[${arr[nosuchvar]:-DEF}]""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Rows where DEF is the CORRECT answer — the guardrails on the fix.
// ═══════════════════════════════════════════════════════════════════════════

mod default_is_correct {
    use super::*;

    #[test]
    fn index_past_end_takes_default() {
        assert_parity(r#"arr=(x y z); i=9; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn index_past_end_no_colon_takes_default() {
        assert_parity(r#"arr=(x y z); i=9; print "[${arr[i]-DEF}]""#);
    }

    #[test]
    fn index_zero_takes_default() {
        assert_parity(r#"arr=(x y z); i=0; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn negative_index_past_start_takes_default() {
        assert_parity(r#"arr=(x y z); i=-9; print "[${arr[i]:-DEF}]""#);
    }

    /// `:-` MUST still substitute for a set-but-EMPTY element.
    #[test]
    fn empty_element_colon_dash_takes_default() {
        assert_parity(r#"arr=(x '' z); i=2; print "[${arr[i]:-DEF}]""#);
    }

    /// `-` (no colon) MUST NOT substitute for a set-but-empty element.
    #[test]
    fn empty_element_plain_dash_keeps_the_empty_element() {
        assert_parity(r#"arr=(x '' z); i=2; print "[${arr[i]-DEF}]""#);
    }

    #[test]
    fn empty_element_colon_plus_is_unset() {
        assert_parity(r#"arr=(x '' z); i=2; print "[${arr[i]:+SET}]""#);
    }

    #[test]
    fn empty_element_plain_plus_is_set() {
        assert_parity(r#"arr=(x '' z); i=2; print "[${arr[i]+SET}]""#);
    }

    #[test]
    fn entirely_unset_array_takes_default() {
        assert_parity(r#"i=1; print "[${nosucharr[i]:-DEF}]""#);
    }

    #[test]
    fn scalar_index_past_end_colon_dash_takes_default() {
        assert_parity(r#"s=abc; i=9; print "[${s[i]:-DEF}]""#);
    }

    /// A scalar's set-ness is the PARAMETER's, so `-` (no colon) yields the
    /// empty out-of-range character rather than the default.
    #[test]
    fn scalar_index_past_end_plain_dash_is_empty() {
        assert_parity(r#"s=abc; i=9; print "[${s[i]-DEF}]""#);
    }

    #[test]
    fn unset_scalar_with_bare_name_subscript_takes_default() {
        assert_parity(r#"unset s; i=1; print "[${s[i]:-DEF}]""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The subscript is evaluated EXACTLY ONCE (C: one `getarg`, one `v->start`).
// Re-running mathevali to answer the set-ness question would double a
// side-effecting subscript.
// ═══════════════════════════════════════════════════════════════════════════

mod single_evaluation {
    use super::*;

    #[test]
    fn pre_increment_subscript_runs_once_under_colon_dash() {
        assert_parity(r#"arr=(x y z); i=0; print "[${arr[++i]:-DEF}]"; print "i=$i""#);
    }

    #[test]
    fn post_increment_subscript_runs_once_under_colon_dash() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[i++]:-DEF}]"; print "i=$i""#);
    }

    #[test]
    fn plain_read_still_runs_the_subscript_once() {
        assert_parity(r#"arr=(x y z); i=0; print "[${arr[++i]}]"; print "i=$i""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// `${+name[sub]}` reports SLOT existence off the same resolved index.
// ═══════════════════════════════════════════════════════════════════════════

mod set_test_operator {
    use super::*;

    #[test]
    fn plus_form_in_range() {
        assert_parity(r#"arr=(x y z); i=2; print "[${+arr[i]}]""#);
    }

    #[test]
    fn plus_form_out_of_range() {
        assert_parity(r#"arr=(x y z); i=9; print "[${+arr[i]}]""#);
    }

    #[test]
    fn plus_form_index_zero() {
        assert_parity(r#"arr=(x y z); i=0; print "[${+arr[i]}]""#);
    }

    #[test]
    fn plus_form_scalar() {
        assert_parity(r#"s=abc; i=2; print "[${+s[i]}]""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The index-option matrix — the resolved index still goes through the
// KSH_ARRAYS / KSH_ZERO_SUBSCRIPT slot mapping (c:Src/params.c:2110-2150).
// ═══════════════════════════════════════════════════════════════════════════

mod index_options {
    use super::*;

    #[test]
    fn ksh_arrays_zero_is_the_first_element() {
        assert_parity(r#"setopt kshArrays; arr=(x y z); i=0; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn ksh_arrays_length_index_is_past_the_end() {
        assert_parity(r#"setopt kshArrays; arr=(x y z); i=3; print "[${arr[i]:-DEF}]""#);
    }

    #[test]
    fn ksh_zero_subscript_zero_is_the_first_element() {
        assert_parity(r#"setopt kshZeroSubscript; arr=(x y z); i=0; print "[${arr[i]:-DEF}]""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Neighbouring subscript shapes that must not regress: associative keys are
// LITERAL (never math), slices and search flags keep their own set-ness rules.
// ═══════════════════════════════════════════════════════════════════════════

mod neighbouring_shapes {
    use super::*;

    #[test]
    fn assoc_dollar_key() {
        assert_parity(r#"typeset -A h; h[k]=v; k=k; print "[${h[$k]:-DEF}]""#);
    }

    /// An assoc subscript is a literal KEY, not math — `h[i]` is the key
    /// "i" even when a variable `i` exists.
    #[test]
    fn assoc_bare_key_is_literal_not_math() {
        assert_parity(r#"typeset -A h; h[i]=V; i=1; print "[${h[i]:-DEF}]""#);
    }

    #[test]
    fn assoc_missing_key_takes_default() {
        assert_parity(r#"typeset -A h; h[k]=v; print "[${h[nope]:-DEF}]""#);
    }

    #[test]
    fn assoc_empty_value_colon_dash_takes_default() {
        assert_parity(r#"typeset -A h; h[k]=''; print "[${h[k]:-DEF}]""#);
    }

    #[test]
    fn assoc_empty_value_plain_dash_keeps_it() {
        assert_parity(r#"typeset -A h; h[k]=''; print "[${h[k]-DEF}]""#);
    }

    #[test]
    fn slice_with_bare_name_bounds() {
        assert_parity(r#"arr=(x y z); i=1; j=2; print "[${arr[i,j]:-DEF}]""#);
    }

    #[test]
    fn search_flag_subscript_hit() {
        assert_parity(r#"arr=(x y z); print "[${arr[(r)y]:-DEF}]""#);
    }

    #[test]
    fn search_flag_subscript_miss_takes_default() {
        assert_parity(r#"arr=(x y z); print "[${arr[(r)q]:-DEF}]""#);
    }

    #[test]
    fn at_splat_on_empty_array_takes_default() {
        assert_parity(r#"arr=(); print "[${arr[@]:-DEF}]""#);
    }

    /// A subscript that is not valid math is a fatal `bad math expression`
    /// even under `:-` — the default never gets a chance to fire.
    #[test]
    fn unparseable_subscript_is_a_math_error() {
        assert_parity(r#"arr=(x y z); print "[${arr[b*]:-DEF}]""#);
    }

    #[test]
    fn dollar_subscript_still_works() {
        assert_parity(r#"arr=(x y z); i=1; print "[${arr[$i]:-DEF}]""#);
    }

    #[test]
    fn literal_index_still_works() {
        assert_parity(r#"arr=(x y z); print "[${arr[2]:-DEF}]""#);
    }
}
