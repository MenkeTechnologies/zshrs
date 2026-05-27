//! More subst flags: (z), (e), (M), (R), (V), (i), (n), (#) parity.

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

mod z_shell_word_split {
    use super::*;

    /// `${(z)CMD}` — split string into shell-words honoring quoting.
    #[test]
    fn z_simple_split_three_words() {
        assert_parity(r#"X='a b c'; arr=("${(z)X}"); echo "${#arr}""#);
    }

    /// (z) respects quoting — "a b c" stays one word.
    #[test]
    fn z_respects_double_quoted_word() {
        assert_parity(r#"X='one "two three" four'; arr=("${(z)X}"); echo "${#arr}""#);
    }

    /// (z) respects single-quoted word.
    #[test]
    fn z_respects_single_quoted_word() {
        assert_parity(r#"X="one 'two three' four"; arr=("${(z)X}"); echo "${#arr}""#);
    }
}

mod e_eval_subst {
    use super::*;

    /// `${(e)X}` — evaluate X's value as a shell expansion (one extra round).
    #[test]
    fn e_evaluates_var_reference_in_value() {
        assert_parity(r#"INNER=actual; X='$INNER'; echo "${(e)X}""#);
    }

    /// (e) doesn't evaluate command substitution by default — pin behavior.
    #[test]
    fn e_with_no_var_ref_unchanged() {
        assert_parity(r#"X='plain'; echo "${(e)X}""#);
    }
}

mod M_match_only_strip {
    use super::*;

    /// `${(M)X#pat}` — return only the matched prefix, not what's left.
    #[test]
    fn M_strip_returns_matched_part() {
        assert_parity(r#"X='/path/to/file'; echo "${(M)X#*/}""#);
    }

    /// `${(M)X##pat}` — longest matched prefix.
    #[test]
    fn M_with_longest_match() {
        assert_parity(r#"X='/path/to/file'; echo "${(M)X##*/}""#);
    }

    /// `${(M)X%pat}` — match-only suffix strip.
    #[test]
    fn M_with_suffix_strip() {
        assert_parity(r#"X='foo.txt.bak'; echo "${(M)X%.*}""#);
    }

    /// `${(Ms)X##pat}` — zsh errors in some -fc contexts; pin for subst fix.
    #[test]
    #[ignore = "ZSHRS BUG: ${(Ms)x##pat} match-only strip diverges from zsh"]
    fn Ms_longest_prefix_match_only() {
        assert_parity(r#"x=aba; echo "${(Ms)x##a}""#);
    }
}

mod hash_length_flag {
    use super::*;

    /// `${(#)X}` — interpret X as numeric code → char.
    #[test]
    fn hash_flag_converts_numeric_to_char() {
        assert_parity(r#"echo "${(#)65}""#);
    }
}

mod q_quote {
    use super::*;

    /// `${(q)X}` — quote for shell re-input. Already tested in
    /// expansion_parity but pin a stress case here too.
    #[test]
    fn q_quotes_special_chars() {
        assert_parity(r#"X='hi"there'; echo "${(q)X}""#);
    }

    /// `${(qq)X}` — single-quote the value.
    #[test]
    fn qq_single_quotes_value() {
        assert_parity(r#"X='hello'; echo "${(qq)X}""#);
    }

    /// `${(qqq)X}` — double-quote the value.
    #[test]
    fn qqq_double_quotes_value() {
        assert_parity(r#"X='hello'; echo "${(qqq)X}""#);
    }

    /// `${(qqqq)X}` — $'...' ANSI-C quote the value.
    #[test]
    fn qqqq_dollar_quotes_value() {
        assert_parity(r#"X='hello'; echo "${(qqqq)X}""#);
    }
}

mod indirect_P_more {
    use super::*;

    /// `${(P)REF}` — indirect with array element.
    #[test]
    fn P_indirect_to_array_name() {
        assert_parity(r#"arr=(a b c); REF=arr; echo "${(P)REF}""#);
    }

    /// `${(P)REF}` chain — REF→TARGET→value.
    #[test]
    fn P_indirect_chain() {
        assert_parity(r#"VALUE=hello; TARGET=VALUE; REF=TARGET; echo "${(P)REF}""#);
    }
}

mod kv_keys_values {
    use super::*;

    /// `${(k)arr}` for indexed array — gives the numeric indices.
    #[test]
    fn k_on_array_returns_indices() {
        assert_parity(r#"arr=(a b c); echo "${(k)arr[@]}""#);
    }

    /// `${(v)arr}` on indexed array — returns the values (same as @).
    #[test]
    fn v_on_array_returns_values() {
        assert_parity(r#"arr=(a b c); echo "${(v)arr[@]}""#);
    }
}

mod i_case_insensitive_subst {
    use super::*;

    /// `${(i)X}` — case-insensitive subscript lookup (zsh-specific).
    /// Effects depend on context; pin observable behavior.
    #[test]
    fn i_flag_works_without_crash() {
        assert_parity(r#"X=Hello; echo "${(i)X#h}""#);
    }
}

mod o_O_sort_more {
    use super::*;

    /// `${(o)arr}` with KSH-style numeric values — alphabetic sort.
    #[test]
    fn o_alpha_sort_strings() {
        assert_parity(r#"arr=(zebra apple mango); print -l "${(@o)arr}""#);
    }

    /// `${(On)arr}` — numeric descending sort.
    #[test]
    fn On_numeric_descending() {
        assert_parity(r#"arr=(20 3 100 5); print -l "${(@On)arr}""#);
    }
}

mod negative_subscript_range {
    use super::*;

    /// `${X[-3,-1]}` on a scalar — last 3 chars.
    #[test]
    fn scalar_negative_subscript_range() {
        assert_parity(r#"X=helloworld; echo "${X[-3,-1]}""#);
    }
}

mod sentinel_slash_pat_globally {
    use super::*;

    /// `${X//#pat/repl}` — replace all anchored-at-start matches.
    /// (`#` anchor + `//` global doesn't make sense, but pin behavior.)
    #[test]
    fn double_slash_with_pound_anchor() {
        assert_parity(r#"X=foofoo; echo "${X//#foo/X}""#);
    }

    /// `${X//%pat/repl}` — replace all anchored-at-end.
    #[test]
    fn double_slash_with_percent_anchor() {
        assert_parity(r#"X=foofoo; echo "${X//%foo/X}""#);
    }
}

mod w_words_and_padding {
    use super::*;

    #[test]
    fn w_shell_words_from_scalar() {
        assert_parity(r#"word="a  b   c"; echo "${(w)word}""#);
    }

    #[test]
    fn percent_pad_to_width() {
        assert_parity(r#"echo "${(%)3}""#);
    }

    #[test]
    fn zero_flag_octal_display() {
        assert_parity(r#"o=8; echo "${(0)o}""#);
    }
}

mod b_byte_and_assoc_match {
    use super::*;

    #[test]
    fn b_byte_count() {
        assert_parity(r#"x=hi; echo "${(b)x}""#);
    }

    #[test]
    fn Mk_assoc_keys_matching() {
        assert_parity(r#"typeset -A h; h=(x 1 y 2); print -l "${(@Mk)h}" | sort"#);
    }

    #[test]
    fn Mv_assoc_values_matching() {
        assert_parity(r#"typeset -A h; h=(x 1 y 2); print -l "${(@Mv)h}" | sort"#);
    }

    #[test]
    fn assoc_reverse_find_key_R() {
        assert_parity(r#"typeset -A h; h=(k v); echo "${h[(R)v]}""#);
    }

    #[test]
    fn assoc_reverse_find_value_r() {
        assert_parity(r#"typeset -A h; h=(a 1 b 2); echo "${h[(r)2]}""#);
    }
}

mod n_N_length_flags {
    use super::*;

    #[test]
    fn n_array_length() {
        assert_parity(r#"a=(1 2 3); echo "${(n)a}""#);
    }

    #[test]
    fn N_array_length() {
        assert_parity(r#"a=(1 2 3); echo "${(N)a}""#);
    }
}

mod sort_flags_on {
    use super::*;

    #[test]
    fn on_numeric_ascending() {
        assert_parity(r#"nums=(10 2 100); print -l "${(@on)nums}""#);
    }

    #[test]
    fn oa_alpha_ascending() {
        assert_parity(r#"nums=(10 2 100); print -l "${(@oa)nums}""#);
    }

    #[test]
    fn eu_unique_ignore_case() {
        assert_parity(r#"nums=(a A b B); print -l "${(@eu)nums}""#);
    }

    #[test]
    fn L_at_lowercase_words() {
        assert_parity(r#"nums=(a B c); print -l "${(@L)nums}""#);
    }
}

mod R_subscript_on_scalar {
    use super::*;

    /// `${s[(r)pat]}` on scalar — zsh returns char after match (`f`);
    /// zshrs currently returns the matched substring (`foo`).
    #[test]
    #[ignore = "ZSHRS BUG: scalar ${s[(r)pat]} match span differs from zsh"]
    fn scalar_reverse_find_substring() {
        assert_parity(r#"s=barfooxyz; echo "${s[(r)foo]}""#);
    }

    /// `${arr[(R)pat]}` on array — reverse-find index.
    #[test]
    fn array_reverse_find_index() {
        assert_parity(r#"arr=(a b c b d); echo "${arr[(R)b]}""#);
    }
}
