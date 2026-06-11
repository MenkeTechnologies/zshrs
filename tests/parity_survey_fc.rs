//! Broad parity survey of `zshrs --zsh -c` vs `zsh -fc`.
//!
//! Each behaviour is its own `#[test]` so a failed run pinpoints exactly
//! one feature divergence. All scripts run with `-fc` (zsh) and
//! `--zsh -c` (zshrs) — startup files are bypassed, so the surface
//! exercised here is the language/builtin core only.
//!
//! This file is purely diagnostic: zero production code changes. Add
//! more tests here whenever you suspect an under-tested corner of the
//! parameter-expansion / builtin / glob / lexer surface.

#![allow(non_snake_case)]
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

fn zsh_path() -> &'static str {
    use std::path::Path;
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

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(s: &str) -> ShellResult {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(s: &str) -> ShellResult {
    let o = Command::new(zshrs_bin())
        // `-fc` to MATCH run_zsh above (the suite name says fc): the
        // zsh side skips rc files, so the zshrs side must too —
        // plain `-c` loaded the user's full zshrc per test, which is
        // slow, environment-dependent, and wedged whole sweep runs.
        .args(["--zsh", "-fc", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// stdout-only parity: typical case where stderr formatting may differ
/// but the success surface is identical.
#[track_caller]
fn assert_parity(s: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{}\n--- zsh stdout ---\n{:?}\n--- zshrs stdout ---\n{:?}\n--- zsh stderr ---\n{}\n--- zshrs stderr ---\n{}",
        s, z.stdout, r.stdout, z.stderr, r.stderr
    );
    assert_eq!(
        z.exit, r.exit,
        "exit divergence on:\n{}\nzsh={} zshrs={}",
        s, z.exit, r.exit
    );
}

// ───────── parameter-expansion flags: ${(X)var} ─────────

mod paren_flags {
    use super::*;

    #[test]
    fn paren_C_capitalize() {
        assert_parity(r#"a="hello world"; print -- ${(C)a}"#);
    }
    #[test]
    fn paren_L_lowercase() {
        assert_parity(r#"a="HeLLo"; print -- ${(L)a}"#);
    }
    #[test]
    fn paren_U_uppercase() {
        assert_parity(r#"a="HeLLo"; print -- ${(U)a}"#);
    }
    #[test]
    fn paren_o_sort_array() {
        assert_parity(r#"a=(c a b); print -- ${(o)a}"#);
    }
    #[test]
    fn paren_O_reverse() {
        assert_parity(r#"a=(c a b); print -- ${(O)a}"#);
    }
    #[test]
    fn paren_n_numeric() {
        assert_parity(r#"a=(10 2 1 20); print -- ${(n)a}"#);
    }
    #[test]
    fn paren_oi_case_insens() {
        assert_parity(r#"a=(Banana apple Cherry); print -- ${(oi)a}"#);
    }
    #[test]
    fn paren_u_unique() {
        assert_parity(r#"a=(a b a c b); print -- ${(u)a}"#);
    }
    #[test]
    fn paren_q_quote() {
        assert_parity(r#"a="he llo"; print -- ${(q)a}"#);
    }
    #[test]
    fn paren_qq_quote() {
        assert_parity(r#"a="he'llo"; print -- ${(qq)a}"#);
    }
    #[test]
    fn paren_qqq_quote() {
        assert_parity(r#"a='he"llo'; print -- ${(qqq)a}"#);
    }
    #[test]
    fn paren_Q_unquote() {
        assert_parity(r#"a='"hello"'; print -- ${(Q)a}"#);
    }
    #[test]
    fn paren_e_eval_inner() {
        assert_parity(r#"x=hi; a='$x'; print -- ${(e)a}"#);
    }
    #[test]
    fn paren_P_namref() {
        assert_parity(r#"foo=bar; bar=baz; print -- ${(P)foo}"#);
    }
    #[test]
    fn paren_t_type_int() {
        assert_parity(r#"typeset -i n=5; print -- ${(t)n}"#);
    }
    #[test]
    fn paren_t_type_array() {
        assert_parity(r#"a=(1 2); print -- ${(t)a}"#);
    }
    #[test]
    fn paren_t_type_assoc() {
        assert_parity(r#"typeset -A m=(k v); print -- ${(t)m}"#);
    }
    #[test]
    fn paren_k_assoc_keys() {
        assert_parity(r#"typeset -A m=(a 1 b 2); print -- ${(ko)m}"#);
    }
    #[test]
    fn paren_v_assoc_vals() {
        assert_parity(r#"typeset -A m=(a 1 b 2); print -- ${(vo)m}"#);
    }
    #[test]
    fn paren_kv_flat() {
        assert_parity(r#"typeset -A m=(a 1 b 2); print -l -- "${(kv)m[@]}" | sort"#);
    }
    #[test]
    fn paren_s_split_colon() {
        assert_parity(r#"s="a:b:c"; print -l -- "${(s.:.)s}""#);
    }
    #[test]
    fn paren_s_split_dash() {
        assert_parity(r#"s="a-b-c"; print -l -- "${(s/-/)s}""#);
    }
    #[test]
    fn paren_j_join_colon() {
        assert_parity(r#"a=(a b c); print -- "${(j.:.)a}""#);
    }
    #[test]
    fn paren_pj_with_esc() {
        assert_parity(r#"a=(a b c); print -- "${(pj.\n.)a}" | wc -l | tr -d ' '"#);
    }
    #[test]
    fn paren_f_split_lines() {
        assert_parity(r#"s=$'a\nb\nc'; print -- ${#${(f)s}}"#);
    }
    #[test]
    fn paren_F_join_lines() {
        assert_parity(r#"a=(x y z); print -- "${(F)a}" | wc -l | tr -d ' '"#);
    }
    #[test]
    fn paren_at_keep_array() {
        assert_parity(r#"a=(a "" b); print -- ${#${(@)a}}"#);
    }
    #[test]
    fn paren_A_assign_assoc() {
        assert_parity(r#"typeset -A m; m=("${(@)$(print a 1 b 2)}"); print -- ${(t)m}"#);
    }
    #[test]
    fn paren_z_shell_split() {
        assert_parity(r#"s='a "b c" d'; print -l -- "${(z)s}""#);
    }
    #[test]
    fn paren_w_word_count() {
        assert_parity(r#"s="one two three"; print -- ${(w)#s}"#);
    }
    #[test]
    fn paren_g_oct_chars() {
        assert_parity(r#"a=$'\\t'; print -- ${(g::)a} | od -c | head -1"#);
    }
    #[test]
    fn paren_p_print_esc() {
        assert_parity(r#"a=(x y); print -- "${(pj:\t:)a}" | od -c | head -1"#);
    }
    #[test]
    fn paren_M_keep_matched() {
        assert_parity(r#"s=foobar; print -- ${(M)s##foo}"#);
    }
    #[test]
    fn paren_R_remove_match() {
        assert_parity(r#"s=foobar; print -- ${(R)s##foo}"#);
    }
    #[test]
    fn paren_B_begin_offset() {
        assert_parity(r#"s=foobar; print -- ${(B)s##foo}"#);
    }
    #[test]
    fn paren_E_end_offset() {
        assert_parity(r#"s=foobar; print -- ${(E)s##foo}"#);
    }
    #[test]
    fn paren_I_match_index() {
        assert_parity(r#"s=foobar; print -- ${(I)s##foo}"#);
    }
    #[test]
    fn paren_N_match_length() {
        assert_parity(r#"s=foobar; print -- ${(N)s##foo}"#);
    }
    #[test]
    fn paren_combined_oU() {
        assert_parity(r#"a=(c a B); print -- ${(oU)a}"#);
    }
    #[test]
    fn paren_combined_LCs() {
        assert_parity(r#"s="HELLO WORLD"; print -- ${(LC)s}"#);
    }
}

// ───────── history-style modifiers: var:h, var:t, var:r, var:e, … ─────────

mod modifiers {
    use super::*;

    #[test]
    fn mod_h_dirname() {
        assert_parity(r#"f=/a/b/c.txt; print -- ${f:h}"#);
    }
    #[test]
    fn mod_h_repeat() {
        assert_parity(r#"f=/a/b/c.txt; print -- ${f:h:h}"#);
    }
    #[test]
    fn mod_t_basename() {
        assert_parity(r#"f=/a/b/c.txt; print -- ${f:t}"#);
    }
    #[test]
    fn mod_r_root() {
        assert_parity(r#"f=/a/b/c.txt; print -- ${f:r}"#);
    }
    #[test]
    fn mod_e_ext() {
        assert_parity(r#"f=/a/b/c.txt; print -- ${f:e}"#);
    }
    #[test]
    fn mod_l_lower() {
        assert_parity(r#"f=AbC; print -- ${f:l}"#);
    }
    #[test]
    fn mod_u_upper() {
        assert_parity(r#"f=AbC; print -- ${f:u}"#);
    }
    #[test]
    fn mod_q_quote() {
        assert_parity(r#"f="a b"; print -- ${f:q}"#);
    }
    #[test]
    fn mod_Q_unquote() {
        assert_parity(r#"f='"hi"'; print -- ${f:Q}"#);
    }
    #[test]
    fn mod_s_substitute() {
        assert_parity(r#"f=foofoo; print -- ${f:s/foo/bar}"#);
    }
    #[test]
    fn mod_gs_subst_all() {
        assert_parity(r#"f=foofoo; print -- ${f:gs/foo/bar}"#);
    }
    #[test]
    fn mod_a_absolute() {
        assert_parity(r#"cd /tmp; f=./x; print -- ${f:a}"#);
    }
    #[test]
    fn mod_h_array() {
        assert_parity(r#"a=(/a/b /c/d); print -l -- ${a:h}"#);
    }
    #[test]
    fn mod_t_array() {
        assert_parity(r#"a=(/a/b /c/d); print -l -- ${a:t}"#);
    }
    #[test]
    fn mod_chained() {
        assert_parity(r#"f=/A/B/C.TXT; print -- ${${f:t}:l}"#);
    }
}

// ───────── ${var:#pat}, ${var:|arr}, ${var:*arr}, subscripted patterns ─────────

mod parameter_filters {
    use super::*;

    #[test]
    fn hash_filter_strip() {
        assert_parity(r#"a=(foo bar baz); print -l -- ${a:#b*} | sort"#);
    }
    #[test]
    fn hash_filter_keep_pipe() {
        assert_parity(r#"a=(foo bar baz); print -l -- ${(M)a:#b*} | sort"#);
    }
    #[test]
    fn pipe_diff_arrays() {
        assert_parity(r#"a=(1 2 3); b=(2); print -l -- ${a:|b} | sort"#);
    }
    #[test]
    fn star_intersect_arrays() {
        assert_parity(r#"a=(1 2 3); b=(2 3 4); print -l -- ${a:*b} | sort"#);
    }
    #[test]
    fn subscript_r_first_match() {
        assert_parity(r#"a=(foo bar baz); print -- ${a[(r)b*]}"#);
    }
    #[test]
    fn subscript_R_last_match() {
        assert_parity(r#"a=(foo bar baz); print -- ${a[(R)b*]}"#);
    }
    #[test]
    fn subscript_i_first_index() {
        assert_parity(r#"a=(foo bar baz); print -- ${a[(i)b*]}"#);
    }
    #[test]
    fn subscript_I_last_index() {
        assert_parity(r#"a=(foo bar baz); print -- ${a[(I)b*]}"#);
    }
    #[test]
    fn subscript_n_nth_match() {
        assert_parity(r#"a=(foo bar baz bin); print -- ${a[(n:2:)b*]}"#);
    }
}

// ───────── parameter expansion patterns: ##, %%, /, // ─────────

mod parameter_substitutions {
    use super::*;

    #[test]
    fn hash_strip_short() {
        assert_parity(r#"s=fooXbar; print -- ${s#*o}"#);
    }
    #[test]
    fn hash_strip_long() {
        assert_parity(r#"s=fooXbar; print -- ${s##*o}"#);
    }
    #[test]
    fn percent_strip_short() {
        assert_parity(r#"s=fooXbar; print -- ${s%a*}"#);
    }
    #[test]
    fn percent_strip_long() {
        assert_parity(r#"s=fooXbarXbaz; print -- ${s%%X*}"#);
    }
    #[test]
    fn slash_replace_first() {
        assert_parity(r#"s=foofoo; print -- ${s/foo/X}"#);
    }
    #[test]
    fn slash_replace_all() {
        assert_parity(r#"s=foofoo; print -- ${s//foo/X}"#);
    }
    #[test]
    fn slash_anchor_head() {
        assert_parity(r#"s=foofoo; print -- ${s/#foo/X}"#);
    }
    #[test]
    fn slash_anchor_tail() {
        assert_parity(r#"s=foofoo; print -- ${s/%foo/X}"#);
    }
    #[test]
    fn slash_with_extglob() {
        assert_parity(r#"setopt extendedglob; s=abc123; print -- ${s//[0-9]##/X}"#);
    }
    #[test]
    fn slash_array_elements() {
        assert_parity(r#"a=(foo foobar baz); print -l -- ${a/foo/X} | sort"#);
    }
    #[test]
    fn slash_empty_pattern() {
        assert_parity(r#"s=abc; print -- ${s/b/}"#);
    }
    #[test]
    fn double_hash_glob() {
        assert_parity(r#"s=aaabbbccc; print -- ${s##a##}"#);
    }
}

// ───────── brace expansion edge cases ─────────

mod brace_expansion {
    use super::*;

    #[test]
    fn brace_zero_pad() {
        assert_parity(r#"print -- {01..05}"#);
    }
    #[test]
    fn brace_step_int() {
        assert_parity(r#"print -- {1..10..2}"#);
    }
    #[test]
    fn brace_neg_step() {
        assert_parity(r#"print -- {10..1..-2}"#);
    }
    #[test]
    fn brace_letter_step() {
        assert_parity(r#"print -- {a..z..3}"#);
    }
    #[test]
    fn brace_descending_alpha() {
        assert_parity(r#"print -- {z..a}"#);
    }
    #[test]
    fn brace_nested() {
        assert_parity(r#"print -- {a,b{1,2},c}"#);
    }
    #[test]
    fn brace_concat_prefix() {
        assert_parity(r#"print -- pre{a,b}post"#);
    }
    #[test]
    fn brace_with_param() {
        assert_parity(r#"x=z; print -- ${x}{1,2}"#);
    }
    #[test]
    fn brace_single_elem_noexp() {
        assert_parity(r#"print -- {a}"#);
    }
    #[test]
    fn brace_empty_alt() {
        assert_parity(r#"print -- a{,b,c}d"#);
    }
}

// ───────── glob qualifiers under -fc ─────────

mod glob_qualifiers {
    use super::*;

    fn with_tmp(setup: &str, glob_test: &str) -> (ShellResult, ShellResult) {
        // One tempdir PER SHELL: with a shared dir, zsh ran the setup
        // first (creating files + `mkdir d`), so zshrs's `mkdir d`
        // failed on the existing dir and the `&&` chain skipped the
        // glob entirely — every with_tmp test compared zsh's real
        // output against "" regardless of glob behavior.
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_r = tempfile::tempdir().expect("tempdir");
        let dp = dir.path().to_str().unwrap().to_string();
        let dp_r = dir_r.path().to_str().unwrap().to_string();
        let script = format!("cd {} && {} && {}", dp, setup, glob_test);
        let script_r = format!("cd {} && {} && {}", dp_r, setup, glob_test);
        let z = run_zsh(&script);
        let r = run_zshrs(&script_r);
        (z, r)
    }

    #[test]
    fn qual_dot_regular_files() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a b; mkdir d", "print -l -- *(.) | sort");
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_slash_dirs_only() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a; mkdir d1 d2", "print -l -- *(/) | sort");
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_at_symlinks_only() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a; ln -s a sa", "print -l -- *(@) | sort");
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_size_minus_lt_5b() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp(
            "echo abc > small; printf '%.0s_' {1..100} > big",
            "print -l -- *(L-50) | sort",
        );
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_exec_pred() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a b; chmod +x a", "print -l -- *(*) | sort");
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_om_modtime_sort() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp(
            "touch -t 202001010000 old; sleep 0.05; touch new",
            "print -l -- *(om)",
        );
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_combined_dot_om_first() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp(
            "touch a; sleep 0.05; touch b; sleep 0.05; touch c",
            "print -l -- *(om[1])",
        );
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_e_glob_excl() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp(
            "touch keep skip",
            r#"print -l -- *(e:'[[ $REPLY = keep ]]':) | sort"#,
        );
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_N_nullglob_no_match() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a", "print -l -- nope_*(N) ; print after");
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn qual_D_dot_files_included() {
        if !zsh_available() {
            return;
        }
        let (z, r) = with_tmp("touch a .b", "print -l -- *(D) | sort");
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────── prompt expansion via print -P ─────────

mod print_dash_P {
    use super::*;

    #[test]
    fn prompt_percent_n_user() {
        assert_parity(r#"print -P "%n" | wc -c | tr -d ' '"#);
    }
    #[test]
    fn prompt_percent_d_pwd() {
        assert_parity(r#"cd /tmp; print -P "%d""#);
    }
    #[test]
    fn prompt_percent_slash_pwd() {
        assert_parity(r#"cd /tmp; print -P "%/""#);
    }
    #[test]
    fn prompt_percent_tilde() {
        assert_parity(r#"cd $HOME; print -P "%~""#);
    }
    #[test]
    fn prompt_percent_pct() {
        assert_parity(r#"print -P "100%%""#);
    }
    #[test]
    fn prompt_percent_capF_color() {
        assert_parity(r#"print -P "%F{red}x%f" | sed 's/[^x]//g'"#);
    }
    #[test]
    fn prompt_percent_question() {
        assert_parity(r#"true; print -P "%?""#);
    }
    #[test]
    fn prompt_percent_braces() {
        assert_parity(r#"print -P "%(?.OK.NO)""#);
    }
    #[test]
    fn prompt_percent_hash() {
        assert_parity(r#"print -P "%#""#);
    }
    #[test]
    fn prompt_pwd_truncate() {
        assert_parity(r#"cd /tmp; print -P "%2~""#);
    }
}

// ───────── print / printf / echo edge cases ─────────

mod print_family {
    use super::*;

    #[test]
    fn print_dash_n_no_newline() {
        assert_parity(r#"print -n hi; print done"#);
    }
    #[test]
    fn print_dash_l_one_per_line() {
        assert_parity(r#"print -l a b c"#);
    }
    #[test]
    fn print_dash_r_no_escape() {
        assert_parity(r#"print -r -- '\n\t'"#);
    }
    #[test]
    fn print_dash_R_raw_n() {
        assert_parity(r#"print -R -n hello; print"#);
    }
    #[test]
    fn print_dash_v_to_var() {
        assert_parity(r#"print -v X hello; print -- $X"#);
    }
    #[test]
    fn print_dash_o_sort() {
        assert_parity(r#"print -o b a c"#);
    }
    #[test]
    fn print_dash_O_revsort() {
        assert_parity(r#"print -O a b c"#);
    }
    #[test]
    fn print_dash_u2_stderr() {
        assert_parity(r#"print -u2 hi 2>&1"#);
    }
    #[test]
    fn printf_pct_q_quote() {
        assert_parity(r#"printf '%q\n' 'a b'"#);
    }
    #[test]
    fn printf_pct_d_int() {
        assert_parity(r#"printf '%d\n' 42"#);
    }
    #[test]
    fn printf_pct_x_hex() {
        assert_parity(r#"printf '%x\n' 255"#);
    }
    #[test]
    fn printf_pct_o_octal() {
        assert_parity(r#"printf '%o\n' 8"#);
    }
    #[test]
    fn printf_pct_b_escape() {
        assert_parity(r#"printf '%b\n' 'a\nb'"#);
    }
    #[test]
    fn printf_repeat_args() {
        assert_parity(r#"printf '%s,' a b c; print"#);
    }
    #[test]
    fn echo_dash_e_expands() {
        assert_parity(r#"echo 'a\tb'"#);
    }
    #[test]
    fn echo_dash_E_no_expand() {
        assert_parity(r#"echo -E 'a\tb'"#);
    }
}

// ───────── typeset / declare flag matrix ─────────

mod typeset_flags {
    use super::*;

    #[test]
    fn typeset_dash_i_int() {
        assert_parity(r#"typeset -i n=10; n=$((n+5)); print -- $n"#);
    }
    #[test]
    fn typeset_dash_i16_base() {
        assert_parity(r#"typeset -i 16 n=255; print -- $n"#);
    }
    #[test]
    fn typeset_dash_F_float() {
        assert_parity(r#"typeset -F 2 f=1.5; f=$((f*2)); print -- $f"#);
    }
    #[test]
    fn typeset_dash_l_lower() {
        assert_parity(r#"typeset -l s=HELLO; print -- $s"#);
    }
    #[test]
    fn typeset_dash_u_upper() {
        assert_parity(r#"typeset -u s=hello; print -- $s"#);
    }
    #[test]
    fn typeset_dash_U_uniq_arr() {
        assert_parity(r#"typeset -U a; a=(x y x z y); print -- $a"#);
    }
    #[test]
    fn typeset_dash_a_array() {
        assert_parity(r#"typeset -a a=(1 2 3); print -- ${(t)a}"#);
    }
    #[test]
    fn typeset_dash_A_assoc() {
        assert_parity(r#"typeset -A m=(k v); print -- ${(t)m}"#);
    }
    #[test]
    fn typeset_dash_r_readonly() {
        let z = run_zsh("typeset -r n=5; n=10 2>/dev/null; print -- $n");
        let r = run_zshrs("typeset -r n=5; n=10 2>/dev/null; print -- $n");
        if !zsh_available() {
            return;
        }
        assert_eq!(z.stdout, r.stdout);
    }
    #[test]
    fn typeset_dash_p_show() {
        assert_parity(r#"typeset -i n=5; typeset -p n"#);
    }
    #[test]
    fn typeset_dash_g_global() {
        assert_parity(r#"f() { typeset -g X=hi }; f; print -- $X"#);
    }
}

// ───────── here-docs and here-strings ─────────

mod heredocs {
    use super::*;

    #[test]
    fn heredoc_basic() {
        assert_parity(
            r#"cat <<EOF
hello
EOF"#,
        );
    }
    #[test]
    fn heredoc_param_expand() {
        assert_parity(
            r#"x=world; cat <<EOF
hi $x
EOF"#,
        );
    }
    #[test]
    fn heredoc_no_expand_quoted() {
        assert_parity(
            r#"x=world; cat <<'EOF'
hi $x
EOF"#,
        );
    }
    #[test]
    fn heredoc_dash_strip_tabs() {
        assert_parity("cat <<-EOF\n\thi\n\tEOF");
    }
    #[test]
    fn herestring_basic() {
        assert_parity(r#"cat <<< "hello""#);
    }
    #[test]
    fn herestring_with_param() {
        assert_parity(r#"x=hi; cat <<< "v:$x""#);
    }
}

// ───────── arithmetic / math edge cases ─────────

mod arith_edge {
    use super::*;

    #[test]
    fn arith_pow_float() {
        assert_parity(r#"print -- $((2.0**10))"#);
    }
    #[test]
    fn arith_unary_plus() {
        assert_parity(r#"print -- $((+5))"#);
    }
    #[test]
    fn arith_bitwise_xor() {
        assert_parity(r#"print -- $((5 ^ 3))"#);
    }
    #[test]
    fn arith_bitwise_not() {
        assert_parity(r#"print -- $((~0))"#);
    }
    #[test]
    fn arith_octal_input() {
        assert_parity(r#"print -- $((010))"#);
    }
    #[test]
    fn arith_base_hash() {
        assert_parity(r#"print -- $((16#ff))"#);
    }
    #[test]
    fn arith_comma_ret_last() {
        assert_parity(r#"print -- $((1,2,3))"#);
    }
    #[test]
    fn arith_assign_in_expr() {
        assert_parity(r#"print -- $((x=5, x*2))"#);
    }
    #[test]
    fn arith_logical_short() {
        assert_parity(r#"x=0; (( x || (x=5) )); print -- $x"#);
    }
    #[test]
    fn arith_func_sqrt() {
        assert_parity(r#"zmodload zsh/mathfunc 2>/dev/null; print -- $((sqrt(16)))"#);
    }
}

// ───────── command-misc: emulate, builtin, command, noglob ─────────

mod command_misc {
    use super::*;

    #[test]
    fn builtin_bypass_function() {
        assert_parity(r#"echo() { print -r REDEF }; builtin echo hi; echo bye"#);
    }
    #[test]
    fn command_bypass_function() {
        assert_parity(r#"true() { echo OOPS }; command true; echo $?"#);
    }
    #[test]
    fn noglob_disables_globbing() {
        assert_parity(r#"noglob print -- *.nonexist123"#);
    }
    #[test]
    fn eval_double_expand() {
        assert_parity(r#"x='print hi'; eval $x"#);
    }
    #[test]
    fn whence_dash_w_word_kind() {
        assert_parity(r#"alias foo=bar; whence -w foo"#);
    }
    #[test]
    fn whence_dash_a_all() {
        assert_parity(r#"whence -a true | head -2"#);
    }
}

// ───────── parameter defaults and alternates ─────────

mod param_defaults {
    use super::*;

    #[test]
    fn colon_dash_default() {
        assert_parity(r#"unset x; print -- ${x:-fallback}"#);
    }
    #[test]
    fn dash_default_only_unset() {
        assert_parity(r#"x=; print -- ${x-fallback}"#);
    }
    #[test]
    fn colon_eq_assign_default() {
        assert_parity(r#"unset x; print -- ${x:=set}; print -- $x"#);
    }
    #[test]
    fn colon_plus_alt_set() {
        assert_parity(r#"x=val; print -- ${x:+alt}"#);
    }
    #[test]
    fn plus_alt_only_set() {
        assert_parity(r#"x=; print -- ${x+alt}"#);
    }
    #[test]
    fn colon_question_err() {
        assert_parity(r#"unset x; (print -- ${x:?missing}) 2>/dev/null; print after"#);
    }
    #[test]
    fn nested_default_with_subst() {
        assert_parity(r#"unset x; print -- ${x:-${y:-deepfallback}}"#);
    }
    #[test]
    fn pound_length_string() {
        assert_parity(r#"x=hello; print -- ${#x}"#);
    }
    #[test]
    fn pound_length_array() {
        assert_parity(r#"a=(a b c d); print -- ${#a}"#);
    }
}

// ───────── redirection / multios ─────────

mod redirection {
    use super::*;

    #[test]
    fn multios_two_outs() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dp = dir.path().to_str().unwrap();
        let script = format!("cd {dp} && print hi >a >b && cat a b");
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn append_redirect() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dp = dir.path().to_str().unwrap();
        let script = format!("cd {dp} && print one > f; print two >> f; cat f");
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn fd_dup_2to1() {
        assert_parity(r#"(print stdout; print -u2 stderr) 2>&1 | wc -l | tr -d ' '"#);
    }

    #[test]
    fn pipe_amp_combines_stderr() {
        assert_parity(r#"(print -u2 e; print o) |& cat | wc -l | tr -d ' '"#);
    }

    #[test]
    fn numbered_fd_braces() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dp = dir.path().to_str().unwrap();
        let script = format!("cd {dp} && print hi {{outfd}}>f; cat f");
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────── special parameters ─────────

mod special_params {
    use super::*;

    #[test]
    fn dollar_dash_options() {
        assert_parity(r#"print -- ${(t)-} >/dev/null; print -- ${#-}"#);
    }
    #[test]
    fn dollar_pipestatus() {
        assert_parity(r#"true | false | true; print -- $pipestatus"#);
    }
    #[test]
    fn dollar_status_zero() {
        assert_parity(r#"true; print -- $?"#);
    }
    #[test]
    fn dollar_status_one() {
        assert_parity(r#"false; print -- $?"#);
    }
    #[test]
    fn dollar_funcstack() {
        assert_parity(r#"f() { print -- ${#funcstack} }; f"#);
    }
    #[test]
    fn dollar_zsh_name() {
        assert_parity(r#"print -- $ZSH_NAME"#);
    }
    #[test]
    fn dollar_random_is_int() {
        assert_parity(r#"print -- $(( RANDOM >= 0 ))"#);
    }
    #[test]
    fn dollar_seconds_is_int() {
        assert_parity(r#"print -- $(( SECONDS >= 0 ))"#);
    }
    #[test]
    fn dollar_options_is_assoc() {
        assert_parity(r#"print -- ${(t)options}"#);
    }
    #[test]
    fn dollar_aliases_assoc() {
        assert_parity(r#"print -- ${(t)aliases}"#);
    }
    #[test]
    fn dollar_path_array() {
        assert_parity(r#"print -- ${(t)path}"#);
    }
}

// ───────── control flow: for/while/until/repeat/select/case ─────────

mod control_flow {
    use super::*;

    #[test]
    fn cstyle_for_loop() {
        assert_parity(r#"for (( i=0; i<3; i++ )); do print -- $i; done"#);
    }
    #[test]
    fn for_word_list() {
        assert_parity(r#"for w in a b c; do print -- $w; done"#);
    }
    #[test]
    fn for_array_in() {
        assert_parity(r#"a=(x y z); for w in $a; do print -- $w; done"#);
    }
    #[test]
    fn for_no_in_uses_argv() {
        assert_parity(r#"set -- p q r; for x; do print -- $x; done"#);
    }
    #[test]
    fn while_loop_count() {
        assert_parity(r#"i=0; while (( i<3 )); do print -- $i; (( i++ )); done"#);
    }
    #[test]
    fn until_loop_count() {
        assert_parity(r#"i=0; until (( i>=3 )); do print -- $i; (( i++ )); done"#);
    }
    #[test]
    fn repeat_n_times() {
        assert_parity(r#"repeat 3 print hi"#);
    }
    #[test]
    fn repeat_block() {
        assert_parity(r#"repeat 3 do print hi; done"#);
    }
    #[test]
    fn break_from_for() {
        assert_parity(r#"for i in 1 2 3 4; do (( i==3 )) && break; print -- $i; done"#);
    }
    #[test]
    fn continue_skips() {
        assert_parity(r#"for i in 1 2 3; do (( i==2 )) && continue; print -- $i; done"#);
    }
    #[test]
    fn break_n_levels() {
        assert_parity(
            r#"for i in a b; do for j in 1 2; do print $i$j; (( j==1 )) && break 2; done; done"#,
        );
    }
    #[test]
    fn case_basic() {
        assert_parity(r#"case foo in foo) print Y;; *) print N;; esac"#);
    }
    #[test]
    fn case_pipes_alt() {
        assert_parity(r#"case bar in foo|bar) print Y;; *) print N;; esac"#);
    }
    #[test]
    fn case_glob_pattern() {
        assert_parity(r#"case abc in a*) print Y;; *) print N;; esac"#);
    }
    #[test]
    fn case_semisemiamp_fall() {
        assert_parity(r#"case foo in foo) print one;& bar) print two;; esac"#);
    }
    #[test]
    fn case_semisemicolon() {
        assert_parity(r#"case foo in foo) print one;| f*) print two;; esac"#);
    }
    #[test]
    fn if_elif_else() {
        assert_parity(
            r#"x=2; if (( x==1 )); then print A; elif (( x==2 )); then print B; else print C; fi"#,
        );
    }
    #[test]
    fn if_compound_with_pipe() {
        assert_parity(r#"if echo hi | grep -q hi; then print Y; fi"#);
    }
}

// ───────── functions, anonymous functions, locals ─────────

mod functions {
    use super::*;

    #[test]
    fn func_braces() {
        assert_parity(r#"f() { print hi }; f"#);
    }
    #[test]
    fn func_function_kw() {
        assert_parity(r#"function f { print hi }; f"#);
    }
    #[test]
    fn anon_function_inline() {
        assert_parity(r#"() { print hi from anon } "#);
    }
    #[test]
    fn anon_function_args() {
        assert_parity(r#"() { print -- $1 $2 } a b"#);
    }
    #[test]
    fn local_scoping() {
        assert_parity(r#"x=outer; f() { local x=inner; print -- $x }; f; print -- $x"#);
    }
    #[test]
    fn local_array() {
        assert_parity(r#"a=(o); f() { local -a a=(i); print -- $a }; f; print -- $a"#);
    }
    #[test]
    fn private_scoping() {
        assert_parity(
            r#"x=outer; f() { private x=inner; g; print -- $x }; g() { print -- ${x:-unset} }; f"#,
        );
    }
    #[test]
    fn func_return_value() {
        assert_parity(r#"f() { return 7 }; f; print -- $?"#);
    }
    #[test]
    fn nested_funcs_seestack() {
        assert_parity(r#"f() { g }; g() { print -- ${#funcstack} }; f"#);
    }
    #[test]
    fn func_param_dollar1() {
        assert_parity(r#"f() { print -- "$1-$2" }; f hi there"#);
    }
    #[test]
    fn func_param_count() {
        assert_parity(r#"f() { print -- $# }; f a b c"#);
    }
    #[test]
    fn func_argv_array() {
        assert_parity(r#"f() { print -l -- "$@" }; f a b c | wc -l | tr -d ' '"#);
    }
}

// ───────── traps and signals (non-interactive only) ─────────

mod traps {
    use super::*;

    #[test]
    fn trap_exit_runs() {
        assert_parity(r#"trap 'print bye' EXIT; print hi"#);
    }
    #[test]
    fn trap_zerr_no_fire_succ() {
        assert_parity(r#"trap 'print ERR' ZERR; true; print done"#);
    }
    #[test]
    fn trap_zerr_fires_on_fail() {
        assert_parity(r#"trap 'print ERR' ZERR; false; print done"#);
    }
    #[test]
    fn TRAPDEBUG_function() {
        assert_parity(r#"TRAPEXIT() { print bye }; print hi"#);
    }
    #[test]
    fn trap_minus_clears() {
        assert_parity(r#"trap 'print bye' EXIT; trap - EXIT; print hi"#);
    }
}

// ───────── alias: regular, global (-g), suffix (-s) ─────────

mod aliases {
    use super::*;

    #[test]
    fn alias_define_use() {
        assert_parity(r#"alias hi='print hello'; hi"#);
    }
    #[test]
    fn alias_global_anywhere() {
        assert_parity(r#"alias -g X='|wc -l|tr -d " "'; print -l a b c X"#);
    }
    #[test]
    fn alias_show_one() {
        assert_parity(r#"alias x=y; alias x"#);
    }
    #[test]
    fn unalias_removes() {
        assert_parity(r#"alias hi='print hi'; unalias hi; alias hi 2>/dev/null; print done"#);
    }
    #[test]
    fn alias_chain() {
        assert_parity(r#"alias a='b'; alias b='print BB'; a"#);
    }
}

// ───────── read builtin: -A, -d, -k, -p, -r, -s, -t, -u, -E ─────────

mod read_builtin {
    use super::*;

    #[test]
    fn read_simple() {
        assert_parity(r#"print -n hello | { read x; print -- $x }"#);
    }
    #[test]
    fn read_dash_A_array() {
        assert_parity(r#"print "a b c" | { read -A arr; print -- $arr }"#);
    }
    #[test]
    fn read_dash_d_delim() {
        assert_parity(r#"printf 'a:b:c' | { read -d : x; print -- $x }"#);
    }
    #[test]
    fn read_dash_r_raw() {
        assert_parity(r#"print -r '\\n' | { read -r x; print -- $x }"#);
    }
    #[test]
    fn read_dash_k_n_chars() {
        assert_parity(r#"print -n hello | { read -k 3 x; print -- $x }"#);
    }
    #[test]
    fn read_dash_E_echo() {
        assert_parity(r#"print hi | { read -E x } | wc -l | tr -d ' '"#);
    }
    #[test]
    fn read_into_two_vars() {
        assert_parity(r#"print "a b c" | { read x y; print -- "$x|$y" }"#);
    }
}

// ───────── conditional [[ ]] operators ─────────

mod conditional_ops {
    use super::*;

    #[test]
    fn dq_string_eq() {
        assert_parity(r#"[[ foo == foo ]] && print Y"#);
    }
    #[test]
    fn dq_string_neq() {
        assert_parity(r#"[[ foo != bar ]] && print Y"#);
    }
    #[test]
    fn dq_glob_match() {
        assert_parity(r#"[[ foobar == foo* ]] && print Y"#);
    }
    #[test]
    fn dq_regex_match() {
        assert_parity(r#"[[ abc123 =~ '^[a-z]+[0-9]+$' ]] && print Y"#);
    }
    #[test]
    fn dq_arith_lt() {
        assert_parity(r#"[[ 5 -lt 10 ]] && print Y"#);
    }
    #[test]
    fn dq_or_chain() {
        assert_parity(r#"[[ a == b || x == x ]] && print Y"#);
    }
    #[test]
    fn dq_and_chain() {
        assert_parity(r#"[[ a == a && b == b ]] && print Y"#);
    }
    #[test]
    fn dq_neg_with_bang() {
        assert_parity(r#"[[ ! a == b ]] && print Y"#);
    }
    #[test]
    fn dq_dash_n_nonempty() {
        assert_parity(r#"x=hi; [[ -n $x ]] && print Y"#);
    }
    #[test]
    fn dq_dash_z_empty() {
        assert_parity(r#"x=; [[ -z $x ]] && print Y"#);
    }
    #[test]
    fn dq_dash_v_set_var() {
        assert_parity(r#"x=hi; [[ -v x ]] && print Y"#);
    }
    #[test]
    fn dq_dash_o_option_on() {
        assert_parity(r#"setopt extendedglob; [[ -o extendedglob ]] && print Y"#);
    }
    #[test]
    fn dq_lex_lt() {
        assert_parity(r#"[[ apple < banana ]] && print Y"#);
    }
    #[test]
    fn dq_lex_gt() {
        assert_parity(r#"[[ banana > apple ]] && print Y"#);
    }

    #[test]
    fn dq_dash_e_exists() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dp = dir.path().to_str().unwrap();
        let s = format!("touch {dp}/f; [[ -e {dp}/f ]] && print Y");
        assert_eq!(run_zsh(&s).stdout, run_zshrs(&s).stdout);
    }
}

// ───────── extended-glob patterns ─────────

mod extended_glob {
    use super::*;

    #[test]
    fn star_brace_alternation() {
        assert_parity(r#"setopt extendedglob; [[ foo == (foo|bar) ]] && print Y"#);
    }
    #[test]
    fn pat_neg_caret() {
        assert_parity(r#"setopt extendedglob; [[ baz == ^foo ]] && print Y"#);
    }
    #[test]
    fn pat_repeat_pound() {
        assert_parity(r#"setopt extendedglob; [[ aaab == a##b ]] && print Y"#);
    }
    #[test]
    fn pat_one_or_more_pound() {
        assert_parity(r#"setopt extendedglob; [[ ab == a#b ]] && print Y"#);
    }
    #[test]
    fn pat_capture_paren() {
        assert_parity(r#"setopt extendedglob; [[ abc == (a)(b)(c) ]] && print -- "$match[2]""#);
    }
    #[test]
    fn pat_kshglob_at() {
        assert_parity(r#"setopt kshglob; [[ foo == @(foo|bar) ]] && print Y"#);
    }
    #[test]
    fn pat_kshglob_plus() {
        assert_parity(r#"setopt kshglob; [[ foofoo == +(foo) ]] && print Y"#);
    }
    #[test]
    fn pat_kshglob_qmark() {
        assert_parity(r#"setopt kshglob; [[ foo == ?(foo) ]] && print Y"#);
    }
    #[test]
    fn pat_kshglob_excl() {
        assert_parity(r#"setopt kshglob; [[ baz == !(foo|bar) ]] && print Y"#);
    }
}

// ───────── process substitution =() <() >() ─────────

mod proc_subst {
    use super::*;

    #[test]
    fn lt_paren_pipe_input() {
        assert_parity(r#"cat <(print hello)"#);
    }
    #[test]
    fn eq_paren_temp_file() {
        assert_parity(r#"cat =(print "via tempfile")"#);
    }

    #[test]
    fn gt_paren_output_to_proc() {
        if !zsh_available() {
            return;
        }
        // both must produce same trimmed stdout (output proc may reorder relative to outer)
        let s = r#"print hello > >(cat); sleep 0.1"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        let mut zl: Vec<&str> = z.stdout.lines().collect();
        zl.sort();
        let mut rl: Vec<&str> = r.stdout.lines().collect();
        rl.sort();
        assert_eq!(zl, rl);
    }

    #[test]
    fn lt_paren_into_diff() {
        assert_parity(r#"diff <(print -l a b c) <(print -l a b c) && print same"#);
    }
}

// ───────── modules: zsh/datetime, zsh/mathfunc, zsh/zutil, zsh/stat ─────────

mod modules {
    use super::*;

    #[test]
    fn datetime_epochseconds() {
        assert_parity(r#"zmodload zsh/datetime; print -- $(( EPOCHSECONDS > 0 ))"#);
    }
    #[test]
    fn datetime_strftime() {
        assert_parity(r#"zmodload zsh/datetime; output_strftime "%Y" 0"#);
    }
    #[test]
    fn mathfunc_sin_zero() {
        assert_parity(r#"zmodload zsh/mathfunc; print -- $(( sin(0) ))"#);
    }
    #[test]
    fn mathfunc_abs_neg() {
        assert_parity(r#"zmodload zsh/mathfunc; print -- $(( abs(-5.0) ))"#);
    }
    #[test]
    fn mathfunc_log_e() {
        assert_parity(r#"zmodload zsh/mathfunc; print -- $(( int(exp(log(7))) ))"#);
    }
    #[test]
    fn zutil_zformat_basic() {
        assert_parity(r#"zmodload zsh/zutil; zformat -f out "hi %n" n:world; print -- $out"#);
    }
    #[test]
    fn zutil_zparseopts_simple() {
        assert_parity(
            r#"zmodload zsh/zutil; set -- -a 1 -b 2 rest; zparseopts a:=A b:=B; print -- "$A $B"; print -- "$@""#,
        );
    }
    #[test]
    fn parameter_module_assocs() {
        assert_parity(r#"zmodload zsh/parameter; alias x=y; print -- ${aliases[x]}"#);
    }

    #[test]
    fn stat_module_size() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dp = dir.path().to_str().unwrap();
        let s = format!("cd {dp} && print abc > f && zmodload zsh/stat && zstat -L +size f");
        assert_eq!(run_zsh(&s).stdout, run_zshrs(&s).stdout);
    }
}

// ───────── job control (non-interactive subset) ─────────

mod jobs_subset {
    use super::*;

    // Background + wait: portable, deterministic in -fc.
    #[test]
    fn bg_then_wait() {
        assert_parity(r#"sleep 0.05 & wait; print done"#);
    }
    #[test]
    fn bg_dollar_bang_pid() {
        assert_parity(r#"sleep 0.05 & p=$!; wait $p; print done"#);
    }
    #[test]
    fn jobs_dash_p_lists() {
        assert_parity(r#"sleep 0.05 & jobs -p | wc -l | tr -d ' '; wait"#);
    }
    #[test]
    fn disown_then_jobs_empty() {
        assert_parity(r#"sleep 0.05 & disown %1; jobs | wc -l | tr -d ' '; wait 2>/dev/null"#);
    }
}

// ───────── set / setopt subset ─────────

mod setopt_subset {
    use super::*;

    #[test]
    fn setopt_no_ksh_arrays() {
        assert_parity(r#"a=(x y z); print -- $a[1]"#);
    }
    #[test]
    fn setopt_ksh_arrays_zero() {
        assert_parity(r#"setopt ksharrays; a=(x y z); print -- $a[0]"#);
    }
    #[test]
    fn setopt_nullglob_no_match() {
        assert_parity(r#"setopt nullglob; print -l -- nope_*; print done"#);
    }
    #[test]
    fn setopt_extendedglob_on() {
        assert_parity(r#"setopt extendedglob; [[ -o extendedglob ]] && print Y"#);
    }
    #[test]
    fn setopt_dash_o_form() {
        assert_parity(r#"set -o extendedglob; [[ -o extendedglob ]] && print Y"#);
    }
    #[test]
    fn unsetopt_clears() {
        assert_parity(
            r#"setopt extendedglob; unsetopt extendedglob; [[ -o extendedglob ]] || print N"#,
        );
    }
    #[test]
    fn setopt_minus_dashes() {
        assert_parity(r#"setopt no_extended_glob; [[ -o extendedglob ]] || print N"#);
    }
    #[test]
    fn setopt_aliases_match() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o extendedglob ]] && print Y"#);
    }
}

// ───────── command substitution edge cases ─────────

mod cmd_subst_edges {
    use super::*;

    #[test]
    fn nested_3_deep() {
        assert_parity(r#"print -- $(echo $(echo $(echo deep)))"#);
    }
    #[test]
    fn cmd_subst_strip_trailing() {
        assert_parity(r#"x=$(printf "hi\n\n\n"); print -- "[$x]""#);
    }
    #[test]
    fn backtick_basic() {
        assert_parity(r#"print -- `echo hi`"#);
    }
    #[test]
    fn cmd_subst_in_arith() {
        assert_parity(r#"print -- $(( $(echo 5) + 7 ))"#);
    }
    #[test]
    fn cmd_subst_in_arr() {
        assert_parity(r#"a=( $(print -l a b c) ); print -- ${#a}"#);
    }
    #[test]
    fn cmd_subst_word_split() {
        assert_parity(r#"a=( $(print "a b c") ); print -- ${#a}"#);
    }
    #[test]
    fn dq_cmd_subst_no_split() {
        assert_parity(r#"x="$(print -l a b c)"; print -- ${#${(f)x}}"#);
    }
    #[test]
    fn cmd_subst_dollar_paren() {
        assert_parity(r#"print -- $(true; print -- second)"#);
    }
}

// ───────── time / timing builtins ─────────

mod time_builtin {
    use super::*;

    #[test]
    fn time_pipeline_smoke() {
        if !zsh_available() {
            return;
        }
        // We can't compare real timing; verify both produce SOME timing line.
        let z = run_zsh("(time (sleep 0.01)) 2>&1 | wc -l | tr -d ' '");
        let r = run_zshrs("(time (sleep 0.01)) 2>&1 | wc -l | tr -d ' '");
        // Both should print at least one line
        assert!(z.stdout.trim().parse::<i32>().unwrap_or(0) >= 1);
        assert!(r.stdout.trim().parse::<i32>().unwrap_or(0) >= 1);
    }
}

// ───────── set -- positional manipulation ─────────

mod positional_params {
    use super::*;

    #[test]
    fn set_dash_dash_args() {
        assert_parity(r#"set -- a b c; print -- $#"#);
    }
    #[test]
    fn shift_one() {
        assert_parity(r#"set -- a b c; shift; print -- $1"#);
    }
    #[test]
    fn shift_n() {
        assert_parity(r#"set -- a b c d; shift 2; print -- $1"#);
    }
    #[test]
    fn dollar_star_join() {
        assert_parity(r#"set -- a b c; IFS=,; print -- "$*""#);
    }
    #[test]
    fn dollar_at_words() {
        assert_parity(r#"set -- a b c; print -l -- "$@""#);
    }
    #[test]
    fn dollar_argv() {
        assert_parity(r#"set -- a b c; print -- $argv"#);
    }
    #[test]
    fn argv_assign() {
        assert_parity(r#"argv=(p q r); print -- "$@""#);
    }
}

// ───────── here-doc/string with redirections in fn body ─────────

mod heredoc_in_func {
    use super::*;

    #[test]
    fn herestring_into_grep() {
        assert_parity(r#"grep -c o <<< "foo bar boo""#);
    }
    #[test]
    fn heredoc_in_func() {
        assert_parity(
            r#"f() { cat <<EOF
inside
EOF
}; f"#,
        );
    }
}
