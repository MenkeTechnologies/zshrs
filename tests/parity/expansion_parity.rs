//! Parameter / variable expansion parity tests — pin every common
//! `${var ...}` form against real zsh 5.9 via `zsh -fc` vs `zshrs --zsh -f -c`.
//!
//! Anchored to zsh behavior: where zshrs diverges, the test FAILS and
//! the failure is the actionable bug. Some forms here are known-buggy
//! from the unit-test round; they're left active so we see the full
//! picture under `cargo test --test expansion_parity`.
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

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stderr, r.stderr
    );
    assert_eq!(z.exit, r.exit, "exit divergence on script:\n{script}");
}

/// Only assert stdout matches (skip stderr/exit). Use for cases where
/// zsh and zshrs may emit different diagnostic prefixes but produce
/// the same user-visible value.
#[allow(dead_code)]
fn assert_stdout_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Bare expansion
// ═══════════════════════════════════════════════════════════════════════════

mod bare {
    use super::*;

    #[test]
    fn dollar_var_basic() {
        assert_parity("X=hello; echo $X");
    }

    #[test]
    fn braced_basic() {
        assert_parity("X=hello; echo ${X}");
    }

    #[test]
    fn unset_var_empty() {
        assert_parity("echo [$UNSET_PARAM]");
    }

    #[test]
    fn empty_string_var() {
        assert_parity("X=; echo [$X]");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Default / alternative / assign / error operators
// ═══════════════════════════════════════════════════════════════════════════

mod defaults {
    use super::*;

    #[test]
    fn colon_dash_unset() {
        assert_parity("echo ${UNSET:-default}");
    }

    #[test]
    fn colon_dash_empty() {
        assert_parity("X=; echo ${X:-default}");
    }

    #[test]
    fn colon_dash_set() {
        assert_parity("X=value; echo ${X:-default}");
    }

    #[test]
    fn dash_only_empty_keeps_empty() {
        assert_parity("X=; echo [${X-default}]");
    }

    #[test]
    fn dash_only_unset_uses_default() {
        assert_parity("echo ${UNSET-default}");
    }

    #[test]
    fn colon_plus_set() {
        assert_parity("X=value; echo ${X:+alt}");
    }

    #[test]
    fn colon_plus_empty() {
        assert_parity("X=; echo [${X:+alt}]");
    }

    #[test]
    fn colon_plus_unset() {
        assert_parity("echo [${UNSET:+alt}]");
    }

    #[test]
    fn colon_equals_assigns_if_empty() {
        assert_parity("X=; echo ${X:=assigned}; echo $X");
    }

    #[test]
    fn colon_equals_noop_if_set() {
        assert_parity("X=keep; echo ${X:=assigned}; echo $X");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Length / substring
// ═══════════════════════════════════════════════════════════════════════════

mod length_and_substring {
    use super::*;

    #[test]
    fn length_simple() {
        assert_parity(r#"X=hello; echo ${#X}"#);
    }

    #[test]
    fn length_empty_is_zero() {
        assert_parity("X=; echo ${#X}");
    }

    #[test]
    fn length_multibyte_counts_chars() {
        assert_parity("X=日本語; echo ${#X}");
    }

    #[test]
    fn substring_offset_only() {
        assert_parity("X=helloworld; echo ${X:5}");
    }

    #[test]
    fn substring_offset_and_length() {
        assert_parity("X=helloworld; echo ${X:0:5}");
    }

    #[test]
    fn substring_negative_offset() {
        assert_parity("X=helloworld; echo ${X:(-3)}");
    }

    #[test]
    fn substring_negative_length() {
        assert_parity("X=helloworld; echo ${X:0:-1}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Prefix / suffix strip — # ## % %%
// ═══════════════════════════════════════════════════════════════════════════

mod strip_prefix_suffix {
    use super::*;

    #[test]
    fn strip_shortest_prefix() {
        assert_parity("X=/a/b/c.txt; echo ${X#*/}");
    }

    #[test]
    fn strip_longest_prefix() {
        assert_parity("X=/a/b/c.txt; echo ${X##*/}");
    }

    #[test]
    fn strip_shortest_suffix() {
        assert_parity("X=foo.txt.bak; echo ${X%.*}");
    }

    #[test]
    fn strip_longest_suffix() {
        assert_parity("X=foo.txt.bak; echo ${X%%.*}");
    }

    #[test]
    fn strip_literal_suffix() {
        assert_parity("X=foo.txt.bak; echo ${X%.bak}");
    }

    #[test]
    fn strip_no_match_unchanged() {
        assert_parity("X=foo; echo ${X#bar}");
    }

    #[test]
    fn strip_empty_pattern_noop() {
        assert_parity("X=foo; echo ${X#}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern substitution — / // /# /%
// ═══════════════════════════════════════════════════════════════════════════

mod substitution {
    use super::*;

    #[test]
    fn replace_first_literal() {
        assert_parity("X=foobarfoo; echo ${X/foo/baz}");
    }

    #[test]
    fn replace_all_literal() {
        assert_parity("X=foobarfoo; echo ${X//foo/baz}");
    }

    #[test]
    fn replace_anchored_start() {
        assert_parity("X=foobarfoo; echo ${X/#foo/baz}");
    }

    #[test]
    fn replace_anchored_end() {
        assert_parity("X=foobarfoo; echo ${X/%foo/baz}");
    }

    #[test]
    fn replace_no_match_unchanged() {
        assert_parity("X=foo; echo ${X/bar/baz}");
    }

    #[test]
    fn replace_with_empty_deletes() {
        assert_parity("X=foobarbaz; echo ${X/bar/}");
    }

    #[test]
    fn replace_glob_pattern() {
        assert_parity("X=foobarbaz; echo ${X/ba?/XX}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Case-conversion flags (L) (U) (C)
// ═══════════════════════════════════════════════════════════════════════════

mod case_flags {
    use super::*;

    #[test]
    fn lower_flag() {
        assert_parity("X=HelloWorld; echo ${(L)X}");
    }

    #[test]
    fn upper_flag() {
        assert_parity("X=HelloWorld; echo ${(U)X}");
    }

    /// Note: `X=hello world` is an inline-env-prefix invocation of the
    /// command `world`, NOT an assignment of "hello world" to X. Both
    /// shells emit "command not found: world" via the standard
    /// scriptname prefix and exit 127.
    #[test]
    fn capitalize_flag() {
        assert_parity("X=hello world; echo ${(C)X}");
    }

    #[test]
    fn lower_chained_with_substring() {
        assert_parity("X=HELLOWORLD; echo ${(L)X:0:5}");
    }

    #[test]
    fn upper_chained_with_strip() {
        assert_parity("X=foobar; echo ${(U)X#foo}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Filename modifiers — :h :t :r :e :s/x/y/ :gs/x/y/ :q
// ═══════════════════════════════════════════════════════════════════════════

mod modifiers {
    use super::*;

    #[test]
    fn head() {
        assert_parity("X=/a/b/c.txt; echo ${X:h}");
    }

    #[test]
    fn tail() {
        assert_parity("X=/a/b/c.txt; echo ${X:t}");
    }

    #[test]
    fn root() {
        assert_parity("X=/a/b/c.txt; echo ${X:r}");
    }

    #[test]
    fn extension() {
        assert_parity("X=/a/b/c.txt; echo ${X:e}");
    }

    #[test]
    fn chain_h_t() {
        assert_parity("X=/a/b/c.txt; echo ${X:h:t}");
    }

    #[test]
    fn chain_r_t() {
        assert_parity("X=/a/b/c.txt.bak; echo ${X:r:t}");
    }

    #[test]
    fn chain_r_e() {
        assert_parity("X=/a/b/c.txt.bak; echo ${X:r:e}");
    }

    #[test]
    fn subst_single() {
        assert_parity("X=foobar; echo ${X:s/foo/X/}");
    }

    #[test]
    fn subst_global() {
        assert_parity("X=fooBarFooBaz; echo ${X:gs/o/0/}");
    }

    #[test]
    fn quote_modifier() {
        assert_parity(r#"X='hi there'; echo ${X:q}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Special flags — (q) (q-) (q+) (P) (s/x/)
// ═══════════════════════════════════════════════════════════════════════════

mod special_flags {
    use super::*;

    #[test]
    fn q_backslash_quote() {
        assert_parity(r#"X='hi there'; echo ${(q)X}"#);
    }

    #[test]
    fn q_dash_single_quote() {
        assert_parity(r#"X='hi there'; echo ${(q-)X}"#);
    }

    #[test]
    fn p_indirect() {
        assert_parity("TARGET=value; REF=TARGET; echo ${(P)REF}");
    }

    #[test]
    fn split_on_colon_scalar() {
        assert_parity(r#"S=a:b:c:d; print -l "${(@s/:/)S}""#);
    }

    #[test]
    fn split_on_x_scalar() {
        assert_parity(r#"S=aXbXcXd; print -l "${(@s/X/)S}""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Arrays
// ═══════════════════════════════════════════════════════════════════════════

mod arrays {
    use super::*;

    #[test]
    fn array_index_one() {
        assert_parity("arr=(a b c d e); echo ${arr[1]}");
    }

    #[test]
    fn array_index_last_negative() {
        assert_parity("arr=(a b c d e); echo ${arr[-1]}");
    }

    #[test]
    fn array_slice() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[1,3]}""#);
    }

    #[test]
    fn array_slice_to_end_negative() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[2,-1]}""#);
    }

    #[test]
    fn array_length() {
        assert_parity("arr=(a b c d e); echo ${#arr}");
    }

    #[test]
    fn array_empty_length() {
        assert_parity("arr=(); echo ${#arr}");
    }

    #[test]
    fn array_at_splat() {
        assert_parity(r#"arr=(a b c); print -l "${arr[@]}""#);
    }

    #[test]
    fn array_star_splat() {
        assert_parity(r#"arr=(a b c); print -l "${arr[*]}""#);
    }

    #[test]
    fn array_out_of_range_empty() {
        assert_parity("arr=(a b c); echo [${arr[99]}]");
    }

    /// `${(j/_/)arr}` — fixed by subst.rs:5924 (j/F flag clears isarr
    /// after sepjoin); see commit ffa01c7233.
    #[test]
    fn array_join_underscore() {
        assert_parity("arr=(a b c d); echo ${(j/_/)arr}");
    }

    /// `${(F)arr}` — fixed by subst.rs:5924; same root as (j) above.
    #[test]
    fn array_join_newlines_via_F() {
        assert_parity(r#"arr=(a b c d); print -r -- "${(F)arr}""#);
    }

    /// `${arr[@]:#pat}` — fixed by subst.rs:4562 (:#pat array filter
    /// fires for [@]/[*]/range subscripts); see commit b677c95d32.
    #[test]
    fn array_filter_hash_removes_match() {
        assert_parity(r#"arr=(foo bar baz qux); print -l "${arr[@]:#bar}""#);
    }

    /// `${arr[@]:#glob}` — same fix as literal above.
    #[test]
    fn array_filter_hash_with_glob() {
        assert_parity(r#"arr=(foo bar baz qux); print -l "${arr[@]:#ba*}""#);
    }

    /// `${arr[(N)]}` paren-wrapped subscript — fixed by subst.rs:3737
    /// (or_else paren-strip retry); see commit 3ab947a915.
    #[test]
    fn array_paren_subscript() {
        assert_parity("arr=(a b c d e); echo ${arr[(1)]}");
    }

    #[test]
    fn array_sort_ascending() {
        assert_parity(r#"arr=(charlie alpha bravo); print -l "${(@o)arr}""#);
    }

    #[test]
    fn array_sort_descending() {
        assert_parity(r#"arr=(charlie alpha bravo); print -l "${(@O)arr}""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Hashes (associative arrays)
// ═══════════════════════════════════════════════════════════════════════════

mod hashes {
    use super::*;

    #[test]
    fn hash_index_key() {
        assert_parity("typeset -A h; h[a]=1; h[b]=2; echo ${h[a]}");
    }

    #[test]
    fn hash_missing_key_empty() {
        assert_parity("typeset -A h; h[a]=1; echo [${h[missing]}]");
    }

    #[test]
    fn hash_element_count() {
        assert_parity("typeset -A h; h[a]=1; h[b]=2; h[c]=3; echo ${#h}");
    }

    /// Keys come back in unspecified order; sort for stable compare.
    #[test]
    fn hash_keys_via_k_flag_sorted() {
        assert_parity(r#"typeset -A h; h[a]=1; h[b]=2; h[c]=3; print -l "${(@k)h}" | sort"#);
    }

    #[test]
    fn hash_values_via_v_flag_sorted() {
        assert_parity(r#"typeset -A h; h[a]=1; h[b]=2; h[c]=3; print -l "${(@v)h}" | sort"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Nested expansion ${${X}}
// ═══════════════════════════════════════════════════════════════════════════

mod nested {
    use super::*;

    /// `${${X%:*}#*:}` — fixed by subst.rs:3481 (var-name walk skips
    /// when subexp_value in flight); see commit 979b4eb401.
    #[test]
    fn nested_strip_prefix_suffix() {
        assert_parity("X=alpha:beta:gamma; echo ${${X%:*}#*:}");
    }

    /// `${${X#*:}#*:}` — same fix as nested_strip_prefix_suffix.
    #[test]
    fn nested_double_strip_prefix() {
        assert_parity("X=alpha:beta:gamma; echo ${${X#*:}#*:}");
    }

    #[test]
    fn nested_double_strip_suffix() {
        assert_parity("X=alpha:beta:gamma; echo ${${X%:*}%:*}");
    }

    #[test]
    fn nested_length_of_uppercase() {
        assert_parity("X=hello; echo ${#${(U)X}}");
    }

    #[test]
    fn nested_length_of_strip() {
        assert_parity("X=FOOBARBAZ; echo ${#${X##*B}}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Array subscript search flags — (I) (i) (R) (r) (Ie), ranges
// ═══════════════════════════════════════════════════════════════════════════

mod array_subscript_search {
    use super::*;

    #[test]
    fn reverse_find_value_r() {
        assert_parity(r#"arr=(alpha beta gamma); echo "${arr[(r)beta]}""#);
    }

    #[test]
    fn reverse_find_index_R() {
        assert_parity(r#"arr=(alpha beta gamma); echo "${arr[(R)beta]}""#);
    }

    #[test]
    fn forward_find_index_I() {
        assert_parity(r#"arr=(alpha beta gamma); echo "${arr[(I)beta]}""#);
    }

    #[test]
    fn forward_find_value_i() {
        assert_parity(r#"arr=(alpha beta gamma); echo "${arr[(i)beta]}""#);
    }

    #[test]
    fn exact_match_index_Ie() {
        assert_parity(r#"arr=(1 2 3); echo "${arr[(Ie)2]}""#);
    }

    #[test]
    fn range_subscript_slice() {
        assert_parity(r#"arr=(a b c d e); echo "${arr[2,4]}""#);
    }

    #[test]
    fn range_subscript_through_end() {
        assert_parity(r#"arr=(1 2 3); echo "${arr[1,-1]}""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Assign-default and nested ${${...}}}
// ═══════════════════════════════════════════════════════════════════════════

mod scalar_assign_default {
    use super::*;

    #[test]
    fn assign_default_colon_equals() {
        assert_parity(r#"unset y; : ${y::=default}; echo "$y""#);
    }

    #[test]
    fn nested_default_expansion() {
        assert_parity(r#"unset x; echo "${${x:-fallback}}""#);
    }

    #[test]
    fn nested_plus_and_default() {
        assert_parity(r#"unset x; echo "${${x:+set}:-unset}""#);
    }

    #[test]
    fn scalar_root_extension_on_dotted_name() {
        assert_parity(r#"str=abc.def; echo "${str:r}:${str:e}""#);
    }

    #[test]
    fn array_elem_plus_assign() {
        assert_parity(r#"arr=(1); arr[1]+=2; echo "${arr[1]}""#);
    }

    #[test]
    fn array_slice_from_offset() {
        assert_parity(r#"arr=(a b c); echo "${arr[@]:1}""#);
    }

    #[test]
    fn array_negative_range() {
        assert_parity(r#"arr=(9 8 7); echo "${arr[-3,-2]}""#);
    }

    #[test]
    fn replace_with_var_pattern() {
        assert_parity(r#"x=a1a2; pat=a; echo "${x//pat/repl}""#);
    }

    #[test]
    fn replace_prefix_with_var() {
        assert_parity(r#"x=abc; pat=a; echo "${x/#pat/repl}""#);
    }

    #[test]
    fn replace_suffix_with_var() {
        assert_parity(r#"x=abc; pat=c; echo "${x/%pat/repl}""#);
    }

    #[test]
    fn join_empty_delim() {
        assert_parity(r#"a=(x y); echo "${(j::)a}""#);
    }

    #[test]
    fn word_count_subscript_w() {
        assert_parity(r#"arr=(a b c); echo "${arr[(w)2]}""#);
    }

    #[test]
    fn collapse_words_W_on_scalar() {
        assert_parity(r#"word="  hi  "; echo "${(W)word}""#);
    }

    #[test]
    fn exact_ie_subscript() {
        assert_parity(r#"arr=(a b c); echo "${arr[(ie)b]}""#);
    }

    #[test]
    fn at_lines_from_scalar() {
        assert_parity(r#"word=$'l1\nl2'; print -l "${(@f)word}""#);
    }

    #[test]
    fn z_words_from_scalar() {
        assert_parity(r#"word="a b c"; print -l "${(z)word}""#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// $? exit status, $$ pid, $! background, $- options
// ═══════════════════════════════════════════════════════════════════════════

mod special_params {
    use super::*;

    #[test]
    fn dollar_question_after_true() {
        assert_parity("true; echo $?");
    }

    #[test]
    fn dollar_question_after_false() {
        assert_parity("false; echo $?");
    }

    #[test]
    fn dollar_question_after_exit_code() {
        assert_parity("(exit 42); echo $?");
    }

    #[test]
    fn dollar_hash_positional_count_zero() {
        assert_parity("echo $#");
    }
}

mod round_pins {
    use super::*;

    #[test]
    fn substring_offset_length() {
        assert_parity("x=abc; print -r ${x:1:2}");
    }

    #[test]
    fn array_reverse_match() {
        assert_parity("a=(1 2 3); print -r ${a[(r)2]}");
    }

    /// `:a` modifier is LEXICAL (chabspath, c:subst.c:4744) — collapses
    /// `.`/`..` and makes absolute WITHOUT resolving symlinks or
    /// requiring existence. Was using xsymlinks (the `:A` physical walk).
    #[test]
    fn modifier_a_collapses_dotdot_absolute() {
        assert_parity("x=/a/b/../c; print -r ${x:a}");
    }

    #[test]
    fn modifier_a_collapses_nonexistent() {
        assert_parity("x=/nonexistent/../foo; print -r ${x:a}");
    }

    #[test]
    fn modifier_a_relative_made_absolute() {
        assert_parity("cd /tmp; x=./x/../y; print -r ${x:a}");
    }
}
