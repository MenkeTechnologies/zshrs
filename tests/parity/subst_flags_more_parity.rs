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

    /// `${(Ms)X##pat}` — zsh rejects the `M` + `s` + `##` combo with
    /// a "bad flag" parse error. zshrs's paramsubst also rejects it
    /// (different error wording, same empty stdout + exit 1) so the
    /// test passes on the assert_parity stdout/exit comparison.
    #[test]
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
    fn scalar_reverse_find_substring() {
        assert_parity(r#"s=barfooxyz; echo "${s[(r)foo]}""#);
    }

    /// `${arr[(R)pat]}` on array — reverse-find index.
    #[test]
    fn array_reverse_find_index() {
        assert_parity(r#"arr=(a b c b d); echo "${arr[(R)b]}""#);
    }
}

mod zero_length_quantifier_replace {
    //! c:Src/glob.c:3008-3061 igetmatch SUB_SUBSTR arms — `pattrylen`
    //! can succeed with `patmatchlen() == 0` (extendedglob `pat#`
    //! matches the empty string). The global loop records the empty
    //! span and bumps one char (`if (mpos == t) mpos += ...`,
    //! c:3060-3061); the non-global shortest arm has a head pre-check
    //! that replaces the empty span at offset 0 (c:3008-3015).
    use super::*;

    /// Zero-length match at every position; bump char kept.
    #[test]
    fn global_zero_length_between_chars() {
        assert_parity(r#"setopt extendedglob; v=bbb; echo "[${v//a#/X}]""#);
    }

    /// Mix of real and zero-length matches.
    #[test]
    fn global_zero_length_mixed_with_real_match() {
        assert_parity(r#"setopt extendedglob; v=baaab; echo "[${v//a#/X}]""#);
    }

    /// Empty input: c:3029 `t <= send` tries the empty span once.
    #[test]
    fn global_zero_length_on_empty_string() {
        assert_parity(r#"setopt extendedglob; v=""; echo "[${v//a#/X}]""#);
    }

    /// Greedy longest still wins before the zero-length fallback.
    #[test]
    fn global_longest_first_then_zero_length() {
        assert_parity(r#"setopt extendedglob; v=aaabbb; echo "[${v//a#/X}]""#);
    }

    /// Single replace: longest at position 0 is the empty span.
    #[test]
    fn single_zero_length_head_match() {
        assert_parity(r#"setopt extendedglob; v=abc; echo "[${v/b#/X}]""#);
    }

    /// (S) shortest pre-check replaces empty span at offset 0 even
    /// when a one-char match exists right there (c:3008-3015).
    #[test]
    fn single_shortest_precheck_beats_real_match() {
        assert_parity(r#"setopt extendedglob; v=abc; echo "[${(S)v/a#/X}]""#);
    }

    /// (S) global: zero-length first at every position.
    #[test]
    fn global_shortest_zero_length_every_position() {
        assert_parity(r#"setopt extendedglob; v=abc; echo "[${(S)v//a#/X}]""#);
    }

    /// Start-anchored global with zero-length-only match.
    #[test]
    fn global_start_anchor_zero_length() {
        assert_parity(r#"setopt extendedglob; v=ab; echo "[${v//#c#/X}]""#);
    }

    /// Group quantifier `(ab)#` zero-length on non-matching input.
    #[test]
    fn global_group_quantifier_zero_length() {
        assert_parity(r#"setopt extendedglob; v=xy; echo "[${v//(ab)#/X}]""#);
    }

    /// Empty pattern behaves as a zero-length matcher.
    #[test]
    fn global_empty_pattern() {
        assert_parity(r#"setopt extendedglob; v=ab; echo "[${v///X}]""#);
    }

    /// Per-element array replace with an empty element.
    #[test]
    fn array_element_empty_string() {
        assert_parity(r#"setopt extendedglob; a=(aa b ""); echo "[${(@)a//a#/X}]""#);
    }

    /// `*` matches whole string in one bite — zero-length fallback
    /// must not fragment it.
    #[test]
    fn global_star_single_replacement() {
        assert_parity(r#"setopt extendedglob; v=abc; echo "[${v//*/X}]""#);
    }
}

/// `(S)` substring-mode strip combined with the `(#m)` whole-match and
/// `(#b)` backreference flags. These require EXTENDED_GLOB for the
/// `(#...)` glob-flag syntax; with it off, `(#m)` is literal text and no
/// match/strip happens (also pinned). The `(S)` flag floats the pattern
/// as a substring instead of anchoring it to the head/tail, and `%%`/`##`
/// select longest, `%`/`#` shortest.
mod S_substring_match_flag_refs {
    use super::*;

    /// `${(S)v%%(#m)M*H}` strips the longest floating substring matching
    /// `M*H` (case-sensitive → only "MATCH", not spanning to lowercase
    /// "h" in "here") and sets $MATCH to it.
    #[test]
    fn s_longest_substring_strip_sets_match() {
        assert_parity(
            r#"setopt extendedglob; v="and look for a MATCH in here"; r=${(S)v%%(#m)M*H}; print "[$r] M=$MATCH""#,
        );
    }

    /// Without EXTENDED_GLOB, `(#m)` is literal — no strip, $MATCH empty.
    #[test]
    fn pound_m_literal_without_extendedglob() {
        assert_parity(
            r#"v="and look for a MATCH in here"; r=${(S)v%%(#m)M*H}; print "[$r] M=$MATCH""#,
        );
    }

    /// `(#m)` in a global substitution exposes $MATCH/$MBEGIN/$MEND per
    /// replacement (1-based positions).
    #[test]
    fn pound_m_global_subst_position_refs() {
        assert_parity(
            r#"setopt extendedglob; v="this is a string"; print ${v//(#m)s/$MATCH-$MBEGIN-$MEND}"#,
        );
    }

    /// Plain `(S)%%` (no match flag) still strips the longest floating
    /// substring, case-sensitively.
    #[test]
    fn s_plain_longest_and_shortest() {
        assert_parity(r#"v="abc XYZ def xyz"; print ${(S)v%%X*Z}"#);
        assert_parity(r#"v="aXbXc"; print ${(S)v%X*X}"#);
        assert_parity(r#"v="aMxHbMyH"; print ${(S)v##M*H}"#);
        assert_parity(r#"v="aMxHbMyH"; print ${(S)v#M*H}"#);
    }

    /// `(#b)` backreferences populate $match[] on a `[[ == ]]` test.
    #[test]
    fn pound_b_backrefs_in_match_array() {
        assert_parity(
            r#"setopt extendedglob; v="key=val"; [[ $v == (#b)(*)=(*) ]] && print "$match[1]/$match[2]""#,
        );
    }
}

mod spliced_backslash_in_replace_pattern {
    //! A parameter value spliced into a `${x//pat}` pattern is LITERAL
    //! (GLOBSUBST off) — including a trailing `\`. The literalize pass
    //! treated every raw `\` as a source escape prefix, so a spliced
    //! backslash swallowed the source class-close `]` (a token char by
    //! then) and patcompile rejected the class: `bad pattern: [ab\`.
    //! Broke history-search-multi-word's `[$specch]` (specch ends in
    //! `\\`) at _hsmw_main:45 — every ^R search errored.
    use super::*;

    /// Class from a var whose value ends in a literal backslash.
    #[test]
    fn class_from_var_with_trailing_backslash() {
        assert_parity(r#"c="ab\\"; s="x\\y"; print -r -- "${s//[$c]/_}""#);
    }

    /// The exact hsmw specch shape (all glob metas + trailing `\`).
    #[test]
    fn hsmw_specch_class_escapes_metas() {
        assert_parity(
            r#"setopt extendedglob
specch="][*?|#~^()><\\"
b="foo bar[baz]*"
b="${b//(#b)((\[?##\])|([$specch]))/${${match[2]:+$match[2]}:-\\${match[3]}}}"
print -r -- "$b""#,
        );
    }

    /// Spliced backslash as the entire pattern value.
    #[test]
    fn spliced_lone_backslash_pattern() {
        assert_parity(r#"c="\\"; s="a\\b"; print -r -- "${s//$c/_}""#);
    }
}

mod default_alternate_words_keep_arrays {
    //! c:Src/subst.c:3203-3233 / 3296-3313 — the default/alternate word
    //! of `${var-word}` `${var:-word}` `${var+word}` `${var:+word}` goes
    //! through multsub, so a nested array-producing expansion
    //! (`${(z)buf}`, `$arr`, `${arr[@]}`) stays an ARRAY. The port
    //! singsub'd (joined) it: fast-syntax-highlighting's tokenizer
    //! `for __arg in ${interactive_comments-${(z)__buf}}` received the
    //! whole buffer as ONE word and painted every line unknown-token
    //! red (0 N fg=red,bold instead of per-token styles).
    use super::*;

    /// The f-sy-h shape: (z)-split inside an unset-var default.
    #[test]
    fn z_split_in_default_arm_stays_split() {
        assert_parity(r#"unset u; v="a b c"; a=(${u-${(z)v}}); print $#a $a[2]"#);
    }

    #[test]
    fn z_split_in_colon_default_arm_stays_split() {
        assert_parity(r#"unset u; v="a b c"; a=(${u:-${(z)v}}); print $#a $a[2]"#);
    }

    /// Nested array parameter in the default arm.
    #[test]
    fn array_in_default_arm_stays_array() {
        assert_parity(r#"unset u; arr=(x y z); a=(${u-$arr}); b=(${u-${arr[@]}}); print $#a $#b $b[3]"#);
    }

    /// Alternate arms (`+` / `:+`) with the var set.
    #[test]
    fn array_in_alternate_arm_stays_array() {
        assert_parity(
            r#"u=set; v="a b c"; arr=(x y z); a=(${u+${(z)v}}); b=(${u:+$arr}); print $#a $#b $a[3]"#,
        );
    }

    /// Scalar defaults still emit one word.
    #[test]
    fn scalar_default_still_single_word() {
        assert_parity(r#"unset u; a=(${u-hello}); b=(${u:-one}); print $#a $#b $a $b"#);
    }
}

mod quoted_empty_subscript_word_survives {
    //! c:Src/subst.c:4437 + 1650-1656 — a word CONTAINING a quoted span
    //! never drops to zero args, even under RC_EXPAND_PARAM. zpwr sets
    //! `rc_expand_param` globally; autopair's `_ap-can-delete-p` runs
    //! `local lchar="${LBUFFER[-1]}"` — with an EMPTY prompt the whole
    //! `lchar=…` word vanished and the ARGLESS `local` dumped the full
    //! parameter table on every backspace.
    use super::*;

    #[test]
    fn local_assign_from_quoted_empty_subscript() {
        assert_parity(
            r#"setopt rcexpandparam; v=""; f(){ local c="${v[-1]}"; print -r -- "c=[$c] ok"; }; f"#,
        );
    }

    #[test]
    fn word_with_quoted_empty_subscript_survives() {
        assert_parity(r#"setopt rcexpandparam; v=""; print -rl -- A x"${v[-1]}"y B"#);
    }

    #[test]
    fn standalone_quoted_empty_subscript_is_one_arg() {
        assert_parity(r#"setopt rcexpandparam; v=""; set -- x${v[-1]}y "${v[-1]}"; print -r -- argc=$#"#);
    }

    #[test]
    fn typeset_assign_quoted_empty_subscript() {
        assert_parity(
            r#"setopt rcexpandparam; v=""; typeset tv="${v[-1]}"; print -r -- "t=[$tv] ${(t)tv}""#,
        );
    }

    /// Unquoted empty expansions still drop under rcexpandparam
    /// (plan9 word-removal is real for genuine arrays).
    #[test]
    fn unquoted_empty_array_still_drops() {
        assert_parity(r#"setopt rcexpandparam; e=(); print -rl -- A x${e}y B"#);
    }
}
