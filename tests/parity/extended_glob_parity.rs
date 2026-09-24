//! EXTENDED_GLOB pattern parity — (#i), (#l), (#a), (#b), (#m),
//! `^pat` negation, `~pat` exclusion. Uses [[ str == ${~pat} ]] form.

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

mod hash_i_case_insensitive {
    use super::*;

    #[test]
    fn hash_i_matches_uppercase_against_lowercase_pat() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#i)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_i_matches_lowercase_against_uppercase_pat() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#i)FOO ]]; echo $?"#);
    }

    #[test]
    fn hash_i_matches_mixed_case() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FoO" == (#i)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_i_rejects_different_letters() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "bar" == (#i)foo ]]; echo $?"#);
    }
}

mod hash_l_lowercase_pat_matches_uppercase_text {
    use super::*;

    /// `(#l)foo` — lowercase letter in pattern matches lowercase or
    /// uppercase in text (asymmetric: pattern→text only).
    #[test]
    fn hash_l_lowercase_pat_matches_uppercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#l)foo ]]; echo $?"#);
    }

    #[test]
    fn hash_l_lowercase_pat_matches_lowercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#l)foo ]]; echo $?"#);
    }

    /// `(#l)FOO` — uppercase in pat REQUIRES uppercase in text.
    #[test]
    fn hash_l_uppercase_pat_requires_uppercase_text() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == (#l)FOO ]]; echo $?"#);
    }
}

mod hash_a_approximate_match {
    use super::*;

    /// `(#a1)` — 1 substitution distance allowed.
    #[test]
    fn hash_a1_one_substitution() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fop" == (#a1)foo ]]; echo $?"#);
    }

    /// `(#a2)` — 2 substitutions.
    #[test]
    fn hash_a2_two_substitutions() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fxy" == (#a2)foo ]]; echo $?"#);
    }

    /// `(#a1)` rejects 2 substitutions.
    #[test]
    fn hash_a1_rejects_two_substitutions() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "fxy" == (#a1)foo ]]; echo $?"#);
    }
}

mod hash_m_match_var {
    use super::*;

    /// `(#m)pat` populates $MATCH on success.
    #[test]
    fn hash_m_populates_match_var() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "hello" == (#m)he* ]] && echo "MATCH=$MATCH""#);
    }

    #[test]
    fn hash_m_match_var_full_match() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "abc123" == (#m)*[0-9]* ]] && echo "MATCH=$MATCH""#,
        );
    }
}

mod hash_b_backref {
    use super::*;

    /// `(#b)pat` populates $match[] array with capture groups.
    #[test]
    fn hash_b_captures_into_match_array() {
        assert_parity(
            r#"
setopt EXTENDED_GLOB
[[ "hello-world" == (#b)(*)-(*) ]] && echo "1=${match[1]} 2=${match[2]}"
"#,
        );
    }

    #[test]
    fn hash_b_three_captures() {
        assert_parity(
            r#"
setopt EXTENDED_GLOB
[[ "a:b:c" == (#b)(*):(*):(*) ]] && echo "${match[1]}|${match[2]}|${match[3]}"
"#,
        );
    }

    /// c:Src/pattern.c:775 — `(#b)` after a quoted literal prefix must
    /// still number the following group. The flag activates wherever it
    /// appears, not only at pattern start. Gap #1 2026-06-12.
    #[test]
    fn hash_b_after_literal_prefix() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "%test" = "%"(#b)(test)* ]] && print -r "[$match[1]] $mbegin[1] $mend[1]""#,
        );
    }

    /// `(#b)` after a `?` class prefix.
    #[test]
    fn hash_b_after_question_prefix() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "xtesty" = ?(#b)(test)* ]] && print -r "[$match[1]] $mbegin[1] $mend[1]""#,
        );
    }

    /// `(#b)` mid-pattern with multiple groups — indices count from the
    /// flag onward (c:775 parno = patnpar++ only while GF_BACKREF live).
    #[test]
    fn hash_b_mid_pattern_multiple_groups() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "abctestxyz" = abc(#b)(test)(x)* ]] && print -r "[$match[1]][$match[2]] $mbegin[*] $mend[*]""#,
        );
    }

    /// Unmatched alternation branch: match[i]="" and mbegin/mend = -1
    /// (c:Src/pattern.c:2607-2613).
    #[test]
    fn hash_b_unmatched_branch_minus_one() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ ab = (#b)((x)|ab) ]] && print -r "m1=[$match[1]] m2=[$match[2]] b=$mbegin[*] e=$mend[*] n=$#match""#,
        );
    }

    /// `(#B)` switches backreferences back off — groups after it are
    /// plain P_OPEN+0 and record nothing (c:Src/pattern.c:1088-1091).
    #[test]
    fn hash_capital_b_disables_captures() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "%test" = "%"(#b)(#B)(test)* ]] && print -r "n=$#match""#,
        );
    }

    /// Groups past NSUBEXP (9) are unnumbered, not a compile error
    /// (c:Src/pattern.c:777-780 "we just use P_OPEN on its own").
    #[test]
    fn hash_b_more_than_nine_groups() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ abcdefghijk = (#b)(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k) ]] && print -r "n=$#match last=[$match[9]]""#,
        );
    }

    /// `(#b)` in a case arm.
    #[test]
    fn hash_b_in_case_arm() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; case "%test" in ("%"(#b)(test)*) print -r "[$match[1]]";; esac"#,
        );
    }
}

mod caret_negation {
    use super::*;

    /// `^pat` — matches anything NOT matching pat (zsh ext-glob).
    #[test]
    fn caret_negation_matches_non_matching() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "bar" == ^foo ]]; echo $?"#);
    }

    #[test]
    fn caret_negation_rejects_matching() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo" == ^foo ]]; echo $?"#);
    }
}

mod tilde_exclusion {
    use super::*;

    /// `pat~excl` — match pat but exclude excl.
    /// e.g. `*~*.bak` = all files except those ending in `.bak`.
    #[test]
    fn tilde_excludes_subset() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo.txt" == *~*.bak ]]; echo $?"#);
    }

    #[test]
    fn tilde_exclusion_rejects_excluded() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "foo.bak" == *~*.bak ]]; echo $?"#);
    }
}

mod alternation {
    use super::*;

    /// `(a|b|c)` — alternation (works even without EXTENDED_GLOB).
    #[test]
    fn alternation_matches_first() {
        assert_parity(r#"[[ "foo" == (foo|bar|baz) ]]; echo $?"#);
    }

    #[test]
    fn alternation_matches_middle() {
        assert_parity(r#"[[ "bar" == (foo|bar|baz) ]]; echo $?"#);
    }

    #[test]
    fn alternation_rejects_outside() {
        assert_parity(r#"[[ "qux" == (foo|bar|baz) ]]; echo $?"#);
    }
}

mod combined {
    use super::*;

    /// `(#i)(foo|bar)` case-insensitive alternation.
    #[test]
    fn hash_i_with_alternation() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "FOO" == (#i)(foo|bar) ]]; echo $?"#);
    }

    /// `(#b)(*).log` capture with extension match.
    #[test]
    fn hash_b_with_extension_match() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; [[ "access.log" == (#b)(*).log ]] && echo "name=${match[1]}""#,
        );
    }
}

/// zsh-specific named character classes that track $IFS / $WORDCHARS
/// (c:Src/pattern.c:3707-3719). `[[:IFS:]]`, `[[:IFSSPACE:]]`, `[[:WORD:]]`
/// were unhandled (empty class → never matched).
mod zsh_named_classes {
    use super::*;

    #[test]
    fn ifs_matches_space() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ " " = [[:IFS:]] ]]; echo $?"#);
    }

    #[test]
    fn ifs_matches_tab() {
        assert_parity("setopt EXTENDED_GLOB; [[ $'\\t' = [[:IFS:]] ]]; echo $?");
    }

    #[test]
    fn ifs_rejects_letter() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ x = [[:IFS:]] ]]; echo $?"#);
    }

    #[test]
    fn ifsspace_matches_space() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ " " = [[:IFSSPACE:]] ]]; echo $?"#);
    }

    #[test]
    fn word_matches_slash() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ / = [[:WORD:]] ]]; echo $?"#);
    }

    #[test]
    fn word_matches_alnum() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ a = [[:WORD:]] ]]; echo $?"#);
    }

    #[test]
    fn ident_still_works() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ _ = [[:IDENT:]] ]]; echo $?"#);
    }
}

mod number_qualifier_quantifier {
    use super::*;

    /// `x##` — one or more x's.
    #[test]
    fn double_hash_one_or_more() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "aaa" == a## ]]; echo $?"#);
    }

    /// `x##` rejects zero.
    #[test]
    fn double_hash_rejects_zero() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "" == a## ]]; echo $?"#);
    }

    /// `x#` — zero or more.
    #[test]
    fn single_hash_zero_or_more() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ "" == a# ]]; echo $?"#);
    }
}

mod quest_two_hash_closure {
    //! `?##` (one-or-more of any character). c:Src/pattern.c:1710-1722 —
    //! patcomppiece special-cases a `#`/`##` closure over a P_ANY atom:
    //! `?#` collapses to a single P_STAR, and `?##` becomes the two-node
    //! chain `?*` via `pattail(starter, patnode(P_STAR))`. The port emitted
    //! the P_STAR but never ran the `pattail`, so the star was unreachable
    //! and `?##` matched EXACTLY ONE character instead of one-or-more.
    //!
    //! Every case here asserts zsh's own answer. Pre-fix, 38 of the 49
    //! cases in the bisect matrix diverged; the ones below are the
    //! representative shapes.
    use super::*;

    /// The single smallest divergence: `[[ abc == ?## ]]` was false.
    #[test]
    fn quest_two_hash_matches_more_than_one_char() {
        assert_parity(r#"setopt extendedglob; v="abc"; [[ $v == ?## ]] && print yes || print no"#);
    }

    /// One-or-more, not zero-or-more: the empty string must NOT match.
    #[test]
    fn quest_two_hash_rejects_empty_but_quest_hash_accepts() {
        assert_parity(r#"setopt extendedglob; v=""; [[ $v == ?## ]] && print yes || print no"#);
        assert_parity(r#"setopt extendedglob; v=""; [[ $v == ?# ]] && print yes || print no"#);
    }

    /// The closure has to stay greedy when a literal tail follows it.
    #[test]
    fn quest_two_hash_greedy_with_literal_tail() {
        assert_parity(
            r#"setopt extendedglob; v="abbbc"; [[ $v == a?##c ]] && print yes || print no"#,
        );
        assert_parity(r#"setopt extendedglob; v="abc"; [[ $v == ?##c ]] && print yes || print no"#);
    }

    /// A `(#b)` group wrapping the closure captures the WHOLE run, and
    /// $mbegin/$mend must agree — not just the replacement text.
    #[test]
    fn backref_group_captures_whole_run_with_mbegin_mend() {
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == (#b)(?##) ]] && print -r -- "$match[1]|$mbegin|$mend" || print no"#,
        );
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == (#b)(?##)(?) ]] && print -r -- "$match[1]/$match[2]|$mbegin|$mend" || print no"#,
        );
    }

    /// The originally-reported shape: an escaped literal bracket around
    /// the closure, inside a `(#b)` group, driving `${x//…}`.
    #[test]
    fn escaped_brackets_around_closure_in_backref_group() {
        assert_parity(
            r#"setopt extendedglob; v="[baz]"; print -r -- ${v//(#b)(\[?##\])/<$match[1]>}"#,
        );
        assert_parity(
            r#"setopt extendedglob; v="x[baz]y"; print -r -- ${v//(#b)(\[?##\])/<$match[1]>[$mbegin,$mend]}"#,
        );
    }

    /// `(#B)` (match, do not capture) over the same pattern.
    #[test]
    fn no_capture_backref_group_over_closure() {
        assert_parity(r#"setopt extendedglob; v="[baz]"; print -r -- ${v//(#B)(\[?##\])/Y}"#);
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == (#B)(?##) ]] && print -r -- "cap=${match:-none}" || print no"#,
        );
    }

    /// Every substitution form that drives the same compiled pattern:
    /// `//`, `/`, and the four strip operators.
    #[test]
    fn closure_across_substitution_forms() {
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v//?##/X}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v/?##/X}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v#?##}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v##?##}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v%?##}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; print -r -- ${v%%?##}"#);
    }

    /// `case` dispatches through the same compiler.
    #[test]
    fn closure_in_case_pattern() {
        assert_parity(
            r#"setopt extendedglob; v="[baz]"; case $v in ((#b)(\[?##\])) print -r -- "c:$match[1]";; (*) print c-no;; esac"#,
        );
    }

    /// Two closures in one pattern, and a repeated-group replacement —
    /// pre-fix these split the subject into single characters.
    #[test]
    fn two_closures_and_repeated_replacement() {
        assert_parity(
            r#"setopt extendedglob; v="foo.bar.baz"; print -r -- ${v//(#b)(?##).(?##)/<$match[1]|$match[2]>}"#,
        );
        assert_parity(r#"setopt extendedglob; v="aXbXc"; print -r -- ${v//(#b)(?##)X/[$match[1]]}"#);
        assert_parity(r#"setopt extendedglob; v="abcd"; [[ $v == ?##?## ]] && print yes || print no"#);
    }

    /// Anchors and exclusion still bind to the whole `?*` chain, not to
    /// the leading P_ANY alone.
    #[test]
    fn closure_with_anchors_and_exclusion() {
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == (?##)(#e) ]] && print yes || print no"#,
        );
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == (#s)?## ]] && print yes || print no"#,
        );
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == ?##~abcd ]] && print yes || print no"#,
        );
        assert_parity(
            r#"setopt extendedglob; v="abcd"; [[ $v == ?##~abc ]] && print yes || print no"#,
        );
    }

    /// `(#m)` MATCH and the `(M)` flag read the same run.
    #[test]
    fn closure_under_match_flags() {
        assert_parity(r#"setopt extendedglob; v="abc"; print -r -- ${(M)v##?##}"#);
        assert_parity(r#"setopt extendedglob; v="abc"; print -r -- ${v//(#m)?##/[$MATCH]}"#);
    }

    /// Was misfiled as
    /// `subst_flags_more_parity::spliced_backslash_in_replace_pattern::hsmw_specch_class_escapes_metas`.
    /// No parameter splice is involved in its failure: the `\[?##\]`
    /// alternative never matched because of the `?##` compilation bug
    /// above. Moved here verbatim, assertion unchanged.
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
}

mod ksh_bang_negation_after_prefix {
    use super::*;

    /// `!(pat)` compiled by patcompnot(1) must hook every branch of the
    /// inner switch to its closing node (c:Src/pattern.c:902-911). A
    /// trailing `*` in the negated group used to re-run the whole
    /// pattern from the split point, so a literal prefix made these
    /// match. globtests.ksh: `[[ mad.moo.cow = !(*.*).!(*.*) ]]` is 1.
    #[test]
    fn negated_group_ending_in_star_after_literal() {
        assert_parity(
            r#"setopt kshglob
for s p in mad.moo.cow '!(*.*).!(*.*)' mad.cow '!(*.*).!(*.*)' \
           xmc 'x!(m*)' xm.c 'x!(m.*)' xmc 'x!(*)' xq 'x!(m*)' \
           mad.moo.cow 'mad.!(*.*)' xmc 'x!(mc|q*)'; do
  [[ $s = ${~p} ]]; print -r -- "$s $p $?"
done"#,
        );
    }

    /// `(#b)` live at `!(`: patcompswitch numbers the group (c:775-783).
    #[test]
    fn negated_group_under_backref_keeps_later_numbering() {
        assert_parity(
            r#"setopt kshglob extendedglob
[[ xab = (#b)x!(q*)(b) ]] && print -r -- "${match[1]}|${match[2]}|$#match""#,
        );
    }
}

/// D07multibyte "Invalid characters in pattern matching": a byte that is
/// not valid UTF-8 is `WCHAR_INVALID` (0xDC00 + byte, c:Src/pattern.c:241)
/// on BOTH sides of a bracket (c:1979-1984 charrefinc, c:384 metacharinc,
/// compared at c:3634), so it matches only itself — never the character
/// whose code point equals the byte — and belongs to no POSIX class.
mod invalid_byte_in_bracket {
    use super::*;

    #[test]
    fn invalid_byte_matches_itself_not_the_same_code_point() {
        assert_parity(
            r#"x=$'\xcc' u=$'\ucc'
[[ $x = [$x] ]]; print -n $?
[[ $x = [$u] ]]; print -n $?
[[ $u = [$u] ]]; print -n $?
[[ $x = [$'\xcb'-$'\xcd'] ]]; print -n $?
[[ $x = [^$x] ]]; print -n $?
[[ $x = [[:alpha:]] ]]; print -n $?
[[ a${x}b = a[$x]b ]]; print -n $?
case $x in [$x]) print -n y;; *) print -n n;; esac
(unsetopt multibyte; [[ bj$'\xf6'rn = *[$'\xe5\xe4\xf6\xc5\xc4\xd6']* ]]; print $?)"#,
        );
    }
}
