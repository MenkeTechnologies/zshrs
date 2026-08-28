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

    /// `${(P)name[sub]}` — the subscript binds to the NAMED parameter and
    /// is resolved BEFORE the (P) dereference (subst.c:2800: the first
    /// fetchvalue processes `name[sub]`, then aspar refetches the result
    /// as a parameter name). zshrs previously deref'd `name` first and
    /// then char-subscripted the result.
    #[test]
    fn p_indirect_array_subscript_resolves_before_deref() {
        // arr[1]="foo" → (P) → $foo
        assert_parity("foo=bar; arr=(foo); print ${(P)arr[1]}");
        // arr[2]="a2" → (P) → $a2
        assert_parity("a1=x a2=y; arr=(a1 a2); print ${(P)arr[2]}");
        // negative index
        assert_parity("a1=x a2=y; arr=(a1 a2); print ${(P)arr[-1]}");
        // quoted context
        assert_parity(r#"foo=bar; arr=(foo); print "${(P)arr[1]}""#);
        // combined with a case-mod flag: (P) then (U)
        assert_parity("foo=bar; arr=(foo); print ${(PU)arr[1]}");
    }

    /// Scalar operand `${(P)scalar[N]}` char-subscripts the scalar FIRST,
    /// then derefs that single char as a name. Discriminator: n="ab",
    /// n[1]="a" → $a ("AVAL"); the old deref-first bug yielded ${ab}[1]
    /// = "A".
    #[test]
    fn p_indirect_scalar_char_subscript_before_deref() {
        assert_parity("a=AVAL b=BVAL ab=ABVAL; n=ab; print ${(P)n[1]}");
        assert_parity("a=AVAL b=BVAL ab=ABVAL; n=ab; print ${(P)n[2]}");
        assert_parity(r#"a=AVAL b=BVAL ab=ABVAL; n=ab; print "[${(P)n[1]}]""#);
        // deref target unset (unquoted) → empty
        assert_parity("foo=bar; n=foo; print ${(P)n[1]}");
        // assoc key operand: h[k]="foo" → (P) → $foo
        assert_parity("foo=bar; typeset -A h=(k foo); print ${(P)h[k]}");
    }

    /// A multi-word slice operand derefs only the FIRST resolved name
    /// (subst.c:2800 itype_end stops at whitespace); an element whose
    /// value itself carries a subscript (`"foo2[2]"`) flows through to
    /// the embedded-bracket dereference.
    #[test]
    fn p_indirect_slice_and_embedded_subscript_edges() {
        // slice "v1 v2" → name "v1" → $v1
        assert_parity("v1=A v2=B; arr=(v1 v2); print ${(P)arr[1,2]}");
        // element value "foo2[2]" → deref foo2[2]
        assert_parity(r#"foo2=(x y z); arr=("foo2[2]"); print ${(P)arr[1]}"#);
        // spacey element "hello world" → name "hello" → $hello
        assert_parity(r#"arr=("hello world"); hello=H; print "[${(P)arr[1]}]""#);
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

// ═══════════════════════════════════════════════════════════════════════════
// Parameter-expansion flag ARGUMENTS delimited by bracket pairs.
//
// `get_strarg` (Src/subst.c:1348) reads the char after the flag as the
// opening delimiter and maps the four bracket families to their closing
// partner (c:1366-1391) — in BOTH the raw-ASCII form (c:1367-1378) and the
// TOKENIZED form (c:1379-1390). Which form the flag parser sees depends on
// whether the expansion went through the lexer: `${(s(X))var}` arrives raw,
// but `${(s(X))#var}` arrives with the parens already tokenized because the
// `#`/`^`/`=` after the flag block forces the lexed path. A raw-ASCII-only
// map therefore passed the first shape and failed the second.
// ═══════════════════════════════════════════════════════════════════════════

mod flag_arg_delimiters {
    use super::*;

    #[test]
    fn split_paren_delim() {
        assert_parity("s=aXbXc; print -rl -- ${(s(X))s}");
    }

    #[test]
    fn split_paren_delim_under_length() {
        assert_parity("s=aXbXc; print -r -- ${(s(X))#s}");
    }

    #[test]
    fn join_bracket_delim_under_length() {
        assert_parity("a=(x y); print -r -- ${(j[X])#a}");
    }

    #[test]
    fn wordcount_split_brace_delim_under_length() {
        assert_parity("s=aXbXc; print -r -- ${(ws{X})#s}");
    }

    #[test]
    fn pad_paren_delim_under_length() {
        assert_parity("foo=ab; print -r -- \"${(l(5))#foo}\"");
    }

    #[test]
    fn pad_paren_delim_two_args_under_length() {
        assert_parity("foo=ab; print -r -- \"${(l(5)(y))#foo}\"");
    }

    /// `(Z...)` reads its sub-flag list with `get_strarg` (c:2207), so a
    /// bracket delimiter closes with its partner.
    #[test]
    #[allow(non_snake_case)]
    fn Z_flag_paren_delim() {
        assert_parity("v='a b'; print -rl -- ${(Z(c))v}");
    }

    #[test]
    #[allow(non_snake_case)]
    fn Z_flag_bracket_delim() {
        assert_parity("v='a b'; print -rl -- ${(Z[c])v}");
    }

    #[test]
    #[allow(non_snake_case)]
    fn Z_flag_brace_delim() {
        assert_parity("v='a b'; print -rl -- ${(Z{c})v}");
    }

    /// C's `Z` arm (c:2206-2237) ONLY ORs the sub-flag bits — it never sets
    /// `LEXFLAGS_ACTIVE` (that is the `z` arm, c:2203) — and the split test
    /// downstream is `if (shsplit)` (c:3906). So an EMPTY sub-flag list
    /// leaves shsplit == 0 and does not split at all.
    #[test]
    #[allow(non_snake_case)]
    fn Z_flag_empty_subflags_does_not_split() {
        assert_parity("v='a b'; print -rl -- ${(Z::)v}");
    }

    #[test]
    #[allow(non_snake_case)]
    fn Z_flag_comment_subflag_splits() {
        assert_parity("v='a b'; print -rl -- ${(Z:c:)v}");
    }

    /// `(g...)` also reads its sub-flags via `get_strarg` (c:2173).
    #[test]
    fn g_flag_paren_delim() {
        assert_parity("v='a\\tb'; print -r -- ${(g(o))v}");
    }

    #[test]
    fn g_flag_bracket_delim() {
        assert_parity("v='a\\tb'; print -r -- ${(g[o])v}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// `${(l:N:)#var}` — the `getlen` block (c:3584-3615) turns the value into
// the decimal length and then FALLS THROUGH to the padding blocks
// (c:4061/4109/4128/4148/4187), which pad that decimal string. Returning
// early from the length path skipped the pad entirely.
// ═══════════════════════════════════════════════════════════════════════════

mod length_with_padding {
    use super::*;

    #[test]
    fn left_pad_scalar_length() {
        assert_parity("foo=ab; print -r -- \"${(l:5:)#foo}\"");
    }

    #[test]
    fn right_pad_scalar_length() {
        assert_parity("foo=ab; print -r -- \"${(r:5:)#foo}\"");
    }

    #[test]
    fn left_pad_with_fill_scalar_length() {
        assert_parity("foo=ab; print -r -- \"${(l:5::y:)#foo}\"");
    }

    #[test]
    fn left_pad_array_element_count() {
        assert_parity("a=(x y z); print -r -- \"${(l:5:)#a}\"");
    }

    #[test]
    fn left_pad_word_count() {
        assert_parity("s='a b c'; print -r -- \"${(wl:5:)#s}\"");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// `=cmd` on the RHS of a magic-equals word.
//
// c:Src/subst.c:667 `filesub` — after the leading `filesubstr` pass, the
// PREFORK_TYPESET arm finds `strchr(*namptr + 1, Equals)` (c:678) and at
// c:680 tests `(sub[1] == Tilde || sub[1] == Equals) && filesubstr(&str,
// assign)`. The `Tilde` half gives `print -r -- a=~/x`; the `Equals` half
// gives `print -r -- a==ls`, which routes through
// c:Src/subst.c:799 (`*str == Equals && isset(EQUALS)`) → c:715
// `equalsubstr` → `findcmd`. The `:`-walk at c:688-698 applies the same
// pair of triggers to every `:`-separated component, which is what makes
// `a=x:=ls` and the plain assignment `kv=a:=ls` expand too.
//
// The `~` spellings already pass; the `=` spellings are the gap. Kept as
// live parity assertions (not `#[ignore]`) so the divergence stays visible.
// ═══════════════════════════════════════════════════════════════════════════

mod magic_equals_cmd {
    use super::*;

    // c:Src/subst.c:680 — `sub[1] == Equals` half of the trigger pair.
    #[test]
    fn magicequals_rhs_equals_cmd() {
        assert_parity("setopt magicequalsubst; print -r -- a==ls");
    }

    // c:Src/subst.c:678 — `strchr(*namptr + 1, Equals)` has NO identifier
    // test, so a non-identifier LHS (`x:y`) still qualifies.
    #[test]
    fn magicequals_nonident_lhs_rhs_equals_cmd() {
        assert_parity("setopt magicequalsubst; print -r -- x:y==ls");
    }

    // c:Src/subst.c:688-698 — the `:`-component walk, `=` flavour.
    #[test]
    fn magicequals_colon_component_equals_cmd() {
        assert_parity("setopt magicequalsubst; print -r -- a=x:=ls");
    }

    // c:Src/exec.c addvars → prefork(PREFORK_ASSIGN) → filesub's `:`-walk.
    // No MAGIC_EQUAL_SUBST needed: a real assignment is always assign-context.
    #[test]
    fn assign_colon_component_equals_cmd() {
        assert_parity("kv=a:=ls; print -r -- $kv");
    }

    // ── controls that already agree; they must not regress ──────────────

    // c:Src/subst.c:680 — `sub[1] == Tilde` half.
    #[test]
    fn magicequals_rhs_tilde() {
        assert_parity("setopt magicequalsubst; print -r -- a=~/x");
    }

    // c:Src/subst.c:799 — leading `=cmd`, no assignment shape.
    #[test]
    fn leading_equals_cmd() {
        assert_parity("print -r -- =ls");
    }

    // c:Src/exec.c:3353 — without MAGIC_EQUAL_SUBST there is no
    // PREFORK_TYPESET, so the RHS stays literal.
    #[test]
    fn no_magicequals_rhs_stays_literal() {
        assert_parity("print -r -- a==ls");
    }

    // c:Src/subst.c:799 — `isset(EQUALS)` gate.
    #[test]
    fn noequals_option_leaves_literal() {
        assert_parity("unsetopt equals; print -r -- =ls");
    }

    // ── over-firing regression (see src/ported/subst.rs:2375-2390) ───────
    //
    // c:Src/subst.c:799 keys on the Equals TOKEN (`\u{8d}`), which only the
    // LEXER writes for a source-level `=`. A `=` that arrives from a
    // parameter substitution is a raw byte and must stay literal — else
    // `${kv#a}` of `a=1` is read as `=1` → "1: not found".
    #[test]
    fn substituted_leading_equals_is_literal() {
        assert_parity("setopt magicequalsubst; kv=a=1; print -r -- ${kv#a}");
    }

    #[test]
    fn substituted_inner_equals_is_literal() {
        assert_parity("setopt magicequalsubst; kv=a=1=2; print -r -- ${kv#a}");
    }

    #[test]
    fn quoted_word_never_equals_expands() {
        assert_parity("setopt magicequalsubst; v=\"a==ls\"; print -r -- $v");
    }
}
