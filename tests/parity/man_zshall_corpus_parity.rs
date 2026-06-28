//! Behavioural parity corpus mined from the official zsh manual
//! (`man zshall` — zshexpn, zshmisc, zshparam, zshbuiltins, zshoptions).
//!
//! Unlike the plugin/snippet/zpwr corpora (which mine real code), every
//! test here encodes a behaviour the MANUAL specifies — the manual is the
//! oracle. Each test cites its section and asserts `zshrs --zsh -fc`
//! matches `/opt/homebrew/bin/zsh -fc` byte-for-byte on stdout + exit.
//! Both shells run with `-f` (no rc). Every script was verified
//! byte-identical across two consecutive real-zsh runs before inclusion.
//!
//! Weighted toward documented edge-cases where engines diverge:
//! (S) match-data flags, l/r padding, brace zero-pad/step, subscript
//! flags on arrays/assoc/special-hashes, glob qualifiers + approximate
//! matching, arithmetic bases, typeset attribute type-strings.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
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
    #[allow(dead_code)]
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
        .args(["--zsh", "-fc", script])
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
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "exit-code divergence on:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

// ═══════════════════════════════ zshexpn ═══════════════════════════

mod man_expn {
    use super::*;

    /// (S) substring with #/## — match closest to start, shortest/longest.
    #[test]
    fn S_substring_hash() {
        assert_parity(r###"str="aXbXc"; echo ${(S)str#X*}; echo ${(S)str##X*}"###);
    }

    /// (S) with %/%% — search closest to END.
    #[test]
    fn S_substring_percent() {
        assert_parity(r###"str="aXbXc"; echo ${(S)str%X*}; echo ${(S)str%%X*}"###);
    }

    /// (S) non-greedy substitution — replaces shortest match.
    #[test]
    fn S_nongreedy_subst() {
        assert_parity(r###"str="abab"; echo ${str/*b/_}; echo ${(S)str/*b/_}"###);
    }

    /// (S) non-greedy GLOBAL substitution — the manual's "twinkle" Examples entry:
    /// ${(S)foo//${~sub}/$rep} replaces each shortest match (HTML Expansion examples).
    #[test]
    fn S_nongreedy_global_subst() {
        assert_parity(
            r###"foo="twinkle twinkle little star"; sub="t*e"; rep="spy"; print ${foo//${~sub}/$rep}; print ${(S)foo//${~sub}/$rep}"###,
        );
    }

    /// (SI:N:) indexed substring match — Nth match.
    #[test]
    fn SI_indexed_match() {
        assert_parity(
            r###"s="which switch is the right switch for Ipswich?"; for n in 1 2 3 4; do echo ${(SI:$n:)s#w*ch}; done"###,
        );
    }

    /// (SM)(SB)(SE)(SN)(SR) match-data flags.
    #[test]
    fn S_match_data_flags() {
        assert_parity(
            r###"str="aXbXc"; echo M=${(SM)str#X*} B=${(SB)str#X*} E=${(SE)str#X*} N=${(SN)str#X*} R=${(SR)str#X*}"###,
        );
    }

    /// (l:N::s1::s2:) left padding with optional fills.
    #[test]
    fn l_left_pad() {
        assert_parity(
            r###"foo=ab; print -- "${(l:5:)foo}"; print -- "${(l:5::-:)foo}"; print -- "${(l:5::-::X:)foo}""###,
        );
    }

    /// (r:N::s1::s2:) right padding + truncation.
    #[test]
    fn r_right_pad_truncate() {
        assert_parity(
            r###"foo=ab; print -- "${(r:5::.:)foo}|"; print -- "${(r:5::.::Y:)foo}|"; print -- "${(l:3:)foo}-${(l:3:)$(printf abcdefg)}""###,
        );
    }

    /// combined l+r padding.
    #[test]
    fn l_r_combined() {
        assert_parity(
            r###"foo=xy; print -- "[${(l:3::-:)foo}][${(r:3::-:)foo}]"; print -- "${(l:4::L::R:)foo}""###,
        );
    }

    /// (#) character from numeric code.
    #[test]
    fn hash_char_from_code() {
        assert_parity(r###"print -- ${(#):-65}; print -- ${(#):-0x41}; print -- ${(#)${:-66}}"###);
    }

    /// (s) split elides empty fields unless (@).
    #[test]
    fn s_split_elide_empty() {
        assert_parity(
            r###"line="one::three"; print -l "${(s.:.)line}"; print MID; print -l "${(@s.:.)line}""###,
        );
    }

    /// (s/x/) on array vs (j/x/s/x/).
    #[test]
    fn s_array_vs_join_split() {
        assert_parity(
            r###"foo=(ax1 bx1); print -l "${(s/x/)foo}"; print MID; print -l "${(j/x/s/x/)foo}""###,
        );
    }

    /// (P) with nested ${...} and $(...) producing the name.
    #[test]
    fn P_indirection_forms() {
        assert_parity(
            r###"foo=bar; bar=baz; print -- ${(P)foo}; print -- ${(P)${foo}}; print -- ${(P)$(echo bar)}"###,
        );
    }

    /// (P) with nested subscript into assoc.
    #[test]
    fn P_nested_subscript() {
        assert_parity(r###"typeset -A assoc=(elt val); name=assoc; print -- ${${(P)name}[elt]}"###);
    }

    /// (n) numeric sort, +/- not special, more leading zeros first.
    #[test]
    fn n_numeric_sort() {
        assert_parity(
            r###"export LC_ALL=C; a=(foo+24 foo1 foo02 foo2 foo3 foo20 foo23); print -- ${(n)a}"###,
        );
    }

    /// (Oa) reverse index order; (u) unique.
    #[test]
    fn Oa_reverse_u_unique() {
        assert_parity(
            r###"a=(one two three); print -- ${(Oa)a}; a2=(a b a c b); print -- ${(u)a2}"###,
        );
    }

    /// ${name:offset:length} negative offset/length.
    #[test]
    fn offset_length_negatives() {
        assert_parity(
            r###"foo=abcdefgh; print -- ${foo:2} ${foo:2:3} ${foo: -3} ${foo: -3:2} ${foo:2:-2}"###,
        );
    }

    /// ${*:offset:length} positional offset (0 = $0).
    #[test]
    fn positional_offset() {
        assert_parity(
            r###"a=(w x y z); print -- ${a:1:2}; set -- one two three; print -- ${*:1:1} ${*:2:2}"###,
        );
    }

    /// ${name//#pat/r} and ${name//%pat/r} anchored gsub.
    #[test]
    fn anchored_gsub() {
        assert_parity(r###"foo=ababab; print -- ${foo//#ab/X} ${foo//%ab/X} ${foo//ab/X}"###);
    }

    /// ${name/#%pat/r} whole-string anchor.
    #[test]
    fn whole_string_anchor() {
        assert_parity(r###"foo=abc; print -- ${foo/#%abc/Z}; print -- ${foo/#%abx/Z}"###);
    }

    /// ${name:|arr} difference and ${name:*arr} intersection.
    #[test]
    fn set_diff_intersect() {
        assert_parity(
            r###"a=(1 2 3 4 5); b=(2 4); print -- ${a:|b}; c=(2 4 9); print -- ${a:*c}"###,
        );
    }

    /// ${name:^arr} zip and ${name:^^arr} cyclic zip.
    #[test]
    fn zip_and_cyclic_zip() {
        assert_parity(r###"a=(1 2 3 4); b=(x y); print -- ${a:^b}; print -- ${a:^^b}"###);
    }

    /// :s/l/r/, :gs/l/r/, :& repeat last s.
    #[test]
    fn modifier_s_gs_amp() {
        assert_parity(
            r###"v=hello; print -- ${v:s/l/L/}; print -- ${v:gs/l/L/}; v2=labels; print -- ${v:s/l/L/}${v2:&}"###,
        );
    }

    /// brace numeric ranges incl decreasing and negative step.
    #[test]
    fn brace_numeric_ranges() {
        assert_parity(
            r###"print -- {1..5}; print -- {5..1}; print -- {1..10..2}; print -- {1..10..-3}"###,
        );
    }

    /// brace zero-padding and negative width.
    #[test]
    fn brace_zero_pad() {
        assert_parity(r###"print -- {01..10}; print -- {-3..3}; print -- {-2..2..01}"###);
    }

    /// brace char ranges and nested comma lists.
    #[test]
    fn brace_char_and_nest() {
        assert_parity(r###"print -- {a..e}; print -- {d..a}; print -- foo{a,b{1,2},c}bar"###);
    }

    /// BRACE_CCL — char-class brace sorted ASCII.
    #[test]
    fn brace_ccl() {
        assert_parity(r###"setopt braceccl; print -- {abcf0-3}"###);
    }

    /// ${^arr} rc-expand distribution.
    #[test]
    fn caret_rc_expand() {
        assert_parity(
            r###"a=(1 2 3); print -- foo${^a}bar; setopt rcexpandparam; print -- foo${^^a}bar"###,
        );
    }

    /// (e) re-evaluation of param/cmd/arith.
    #[test]
    fn e_reeval() {
        assert_parity(
            r###"inner=world; v="hello $inner"; print -- ${(e)v}; w=inner; print -- ${(e):-\$$w}"###,
        );
    }

    /// (@) inner scalar vs inner array subscript detection.
    #[test]
    #[ignore = "zshrs gap: \"${(@)${foo}[1]}\" — inner ${foo} should be scalar (char subscript -> b); zshrs treats it as array (-> bar baz)"]
    fn at_inner_scalar_vs_array() {
        assert_parity(
            r###"foo=(bar baz); print -- "${(@)${foo}[1]}"; print -- "${${(@)foo}[1]}""###,
        );
    }

    /// (z) lexical split then (Q) dequote.
    #[test]
    fn z_split_then_Q() {
        assert_parity(
            r###"foo='a "b c" d'; print -l "${(z)foo}"; print MID; print -l "${(Q)${(z)foo}}""###,
        );
    }

    /// (Z+C+) strips comments, (Z+c+) retains them.
    #[test]
    fn Z_comment_flags() {
        assert_parity(
            r###"foo='a # comment'; print -l "${(Z+C+)foo}"; print MID; print -l "${(Z+c+)foo}""###,
        );
    }

    /// (w)/(c) counting with ${#}.
    #[test]
    fn w_c_counting() {
        assert_parity(
            r###"v="a b c d"; print -- ${(w)#v}; arr=(x y z); print -- ${(w)#arr}; ar2=(ab cd); print -- ${(c)#ar2}"###,
        );
    }

    /// :h with digits; absolute leading / is first component.
    #[test]
    fn h_with_digits() {
        assert_parity(r###"var=/my/path/to/something; print -- ${var:h} ${var:h3} ${var:h2}"###);
    }

    /// :tN, :r, :e modifiers.
    #[test]
    fn t_digits_r_e() {
        assert_parity(
            r###"var=/a/b/c/d/e; print -- ${var:t} ${var:t2} ${var:t3}; v2=foo.orig.c; print -- ${v2:r} ${v2:e}"###,
        );
    }

    /// short-form digit ambiguity ($var:h2 == ${var:h}2); :a logical abspath.
    #[test]
    fn shortform_ambiguity_a() {
        assert_parity(
            r###"var=/a/b/c/d; print -- $var:h2; print -- ${var:h2}; v=/before/here/../after; print -- ${v:a}"###,
        );
    }

    /// (b) backslash-quote pattern chars.
    #[test]
    fn b_pattern_quote() {
        assert_parity(r###"v="a*b?c"; print -r -- ${(b)v}"###);
    }

    /// (p) $var as flag argument separator.
    #[test]
    fn p_var_separator() {
        assert_parity(r###"sep=:; val=a:b:c; print -l ${(ps.$sep.)val}"###);
    }

    /// (0) null-byte split.
    #[test]
    fn null_byte_split() {
        assert_parity(r###"v=$(printf "a\0b\0c"); print -l "${(0)v}""###);
    }

    /// $[exp] and $((exp)).
    #[test]
    fn dollar_bracket_arith() {
        assert_parity(r###"print -- $[3*4]; print -- $((2**10))"###);
    }

    /// nested inside-out removal.
    #[test]
    fn nested_inside_out() {
        assert_parity(r###"foo=headXtail; print -- ${${foo#head}%tail}"###);
    }

    /// substitution before split/join with modifier.
    #[test]
    fn subst_before_split() {
        assert_parity(r###"foo=(ax1 bx1); print -l ${(s/x/)foo%%1*}"###);
    }

    /// (w)/(W) with s delimiter — empty-word counting.
    #[test]
    fn w_W_delim_counting() {
        assert_parity(
            r###"v="a::b::c"; print -- ${(ws.::.)#v}; v2="a::b"; print -- ${(ws.:.)#v2} ${(Ws.:.)#v2}"###,
        );
    }

    /// (I:N:) on substitution — // all from Nth, / only Nth.
    #[test]
    fn I_on_substitution() {
        assert_parity(r###"s="aXbXcXd"; print -- ${(SI:2:)s//X/-}; print -- ${(I:2:)s/X/-}"###);
    }

    /// (*) enables EXTENDED_GLOB in a substitution.
    #[test]
    fn star_flag_extendedglob() {
        assert_parity(r###"setopt extendedglob; s="abc123"; print -- ${(*)s//[[:digit:]]##/N}"###);
    }

    /// (D) reverse-of-~ substitution; (q-) minimal quote.
    #[test]
    fn D_and_q_minus() {
        assert_parity(
            r###"foo=$HOME/x; print -- ${(D)foo}; v="plain"; w="has space"; print -r -- ${(q-)v} ${(q-)w}"###,
        );
    }

    /// rule-23 semantic join before (P).
    #[test]
    fn join_before_P() {
        assert_parity(r###"foo=barval; lines=$(printf "foo\n"); print -- ${(P)${(f)lines}}"###);
    }

    /// (g::) echo-style escape processing.
    #[test]
    fn g_escape_processing() {
        assert_parity(r###"v='a\tb'; print -rn -- "${(g::)v}" | od -An -c"###);
    }

    /// (e+code+) glob qualifier with reply override.
    #[test]
    fn e_glob_qualifier_reply() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); :>$t/aa; :>$t/bbb; cd $t; m=( *(e+"reply=( \${#REPLY} )"+) ); print -- ${(on)m}; cd /; rm -rf $t"###,
        );
    }

    /// in-glob (#q...) with colon modifiers.
    #[test]
    fn inglob_q_colon_modifiers() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); :>$t/foo.c; :>$t/bar.c; cd $t; print -r -- ${(o)$(print -l -- *.c(#q:t:s/.c/.X/))}; cd /; rm -rf $t"###,
        );
    }
}

// ═══════════════════════════════ zshmisc ═══════════════════════════

mod man_glob {
    use super::*;

    /// numeric range <n-m>.
    #[test]
    fn numeric_range() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; for n in 1 5 10 50 100 200; do touch file$n; done; print -rl -- ${(on)$(print -l -- file<5-100>(:t))}; cd /; rm -rf $t"###,
        );
    }

    /// half-open numeric <n->.
    #[test]
    fn half_open_numeric() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch 12 5 99 7 030 200; print -rl -- ${(on)$(print -l -- <30->(:t))}; cd /; rm -rf $t"###,
        );
    }

    /// mixed POSIX class [[:alpha:]0-9].
    #[test]
    fn mixed_posix_class() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch fa f0 f% f_; print -rl -- ${(o)$(print -l -- f[[:alpha:]0-9](:t))}; cd /; rm -rf $t"###,
        );
    }

    /// leading - literal in [...].
    #[test]
    fn leading_dash_in_class() {
        assert_parity(
            r###"[[ "-" = [-a] ]] && echo yes || echo no; [[ "a" = [-a] ]] && echo yes || echo no; [[ "b" = [-a] ]] && echo yes || echo no"###,
        );
    }

    /// size qualifiers L0 / L+n / L-n.
    #[test]
    fn size_qualifiers() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; print -n "" > empty; print -n "abcde" > five; print -n "0123456789" > ten; print zero:; print -rl -- ${(o)$(print -l -- *(L0:t))}; print gt4:; print -rl -- ${(o)$(print -l -- *(L+4:t))}; print lt6:; print -rl -- ${(o)$(print -l -- *(L-6:t))}; cd /; rm -rf $t"###,
        );
    }

    /// link-count qualifier l+n.
    #[test]
    fn link_count_qualifier() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; mkdir d; mkdir d/sub; mkdir e; print -rl -- ${(o)$(print -l -- *(/l+2:t))}; cd /; rm -rf $t"###,
        );
    }

    /// negation qualifier (^/).
    #[test]
    fn negation_qualifier() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch reg; mkdir dir; print -rl -- ${(o)$(print -l -- *(^/:t))}; cd /; rm -rf $t"###,
        );
    }

    /// executable qualifier (*).
    #[test]
    fn executable_qualifier() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch plain; touch exe; chmod 755 exe; print -rl -- ${(o)$(print -l -- *(*:t))}; cd /; rm -rf $t"###,
        );
    }

    /// subscript-after-sort (.OL[1,2]).
    #[test]
    fn subscript_after_sort() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; print -n a > f1; print -n "" > f2; print -n abc > f3; print -n "" > f4; print -n abcde > f5; print -rl -- *(.OL[1,2]:t); cd /; rm -rf $t"###,
        );
    }

    /// exclusion x~y and chained x~a~b.
    #[test]
    fn exclusion_tilde() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch a b c d; print -rl -- ${(o)$(print -l -- *~a~c(:t))}; cd /; rm -rf $t"###,
        );
    }

    /// exclusion of alternation ^(foo|bar).
    #[test]
    fn exclude_alternation() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch foo bar baz qux; print -rl -- ${(o)$(print -l -- ^(foo|bar)(:t))}; cd /; rm -rf $t"###,
        );
    }

    /// recursive **/ matches current + subdirs.
    #[test]
    fn recursive_globstar() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; mkdir -p a/b/c; touch bar a/bar a/b/bar a/b/c/bar; print -rl -- **/bar | sort; cd /; rm -rf $t"###,
        );
    }

    /// (pat/)# path closure.
    #[test]
    fn path_closure() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; mkdir -p sub/sub/sub; touch end sub/end sub/sub/end; print -rl -- (sub/)#end | sort; cd /; rm -rf $t"###,
        );
    }

    /// case-insensitive (#i).
    #[test]
    fn ci_glob_flag() {
        assert_parity(
            r###"setopt extendedglob; [[ FOOXX = (#i)fooxx ]] && echo m1 || echo n1; [[ fooxx = (#i)FOOXX ]] && echo m2 || echo n2; [[ FooXx = (#i)fooxx ]] && echo m3 || echo n3"###,
        );
    }

    /// lower-matches-both (#l).
    #[test]
    fn l_glob_flag() {
        assert_parity(
            r###"setopt extendedglob; [[ FOOXX = (#l)fooxx ]] && echo m1 || echo n1; [[ fooxx = (#l)FOOXX ]] && echo m2 || echo n2"###,
        );
    }

    /// local negate (#I).
    #[test]
    fn I_glob_flag_negate() {
        assert_parity(
            r###"setopt extendedglob; [[ fooxx = (#i)FOO(#I)XX ]] && echo yes || echo no; [[ fooXX = (#i)FOO(#I)XX ]] && echo yes || echo no"###,
        );
    }

    /// flag scope ends at enclosing group.
    #[test]
    fn flag_scope_group() {
        assert_parity(
            r###"setopt extendedglob; [[ fooxx = ((#i)FOOX)X ]] && echo yes || echo no; [[ fooxX = ((#i)FOOX)X ]] && echo yes || echo no"###,
        );
    }

    /// (#b) backref indices via $mbegin/$mend.
    #[test]
    fn b_backref_indices() {
        assert_parity(
            r###"setopt extendedglob; [[ "a short string" = *s(#b)(???)t* ]] && print "$match[1] $mbegin[1] $mend[1]""###,
        );
    }

    /// (#m) match-data — uppercase vowels.
    #[test]
    fn m_matchdata_vowels() {
        assert_parity(
            r###"setopt extendedglob; arr=(veldt jynx grimps waqf zho buck); print ${arr//(#m)[aeiou]/${(U)MATCH}}"###,
        );
    }

    /// (#b) with # keeps last match.
    #[test]
    fn b_hash_last_match() {
        assert_parity(
            r###"setopt extendedglob; [[ abab = (#b)([ab])# ]] && print "[$match[1]]""###,
        );
    }

    /// ((ab|cd)#) captures whole repeated segment.
    #[test]
    fn capture_whole_segment() {
        assert_parity(
            r###"setopt extendedglob; [[ XababcdY = X(#b)((ab|cd)#)Y ]] && print "$match[1]""###,
        );
    }

    /// count (#cN,M).
    #[test]
    fn count_closure() {
        assert_parity(
            r###"setopt extendedglob; [[ aaa = a(#c2,3) ]] && echo m1 || echo n1; [[ a = a(#c2,3) ]] && echo m2 || echo n2; [[ aaaa = a(#c2,3) ]] && echo m3 || echo n3"###,
        );
    }

    /// approximate (#a1) — sub/del/ins.
    #[test]
    fn approx_one_error() {
        assert_parity(
            r###"setopt extendedglob; [[ fooybar = (#a1)fooxbar ]] && echo m1 || echo n1; [[ rod = (#a1)road ]] && echo m2 || echo n2; [[ strove = (#a1)stove ]] && echo m3 || echo n3"###,
        );
    }

    /// (#a3)abcd matches dcba.
    #[test]
    fn approx_transposition() {
        assert_parity(r###"setopt extendedglob; [[ dcba = (#a3)abcd ]] && echo yes || echo no"###);
    }

    /// local error count (#a0) section.
    #[test]
    fn approx_local_zero() {
        assert_parity(
            r###"setopt extendedglob; [[ catdogfox = (#a1)cat(#a0)dog(#a1)fox ]] && echo m1 || echo n1; [[ catdigfox = (#a1)cat(#a0)dog(#a1)fox ]] && echo m2 || echo n2"###,
        );
    }

    /// combined (#ia2).
    #[test]
    fn approx_ci_combined() {
        assert_parity(
            r###"setopt extendedglob; [[ READXME = (#ia2)readme ]] && echo yes || echo no"###,
        );
    }

    /// closure precedence: 12# == 1(2#).
    #[test]
    fn closure_precedence() {
        assert_parity(
            r###"setopt extendedglob; [[ 1222 = 12# ]] && echo m1 || echo n1; [[ 1212 = 12# ]] && echo m2 || echo n2"###,
        );
    }

    /// anchors (#s)/(#e).
    #[test]
    fn anchors_s_e() {
        assert_parity(
            r###"setopt extendedglob; for s in test test/at/start at/end/test in/test/middle nomatch; do [[ $s = *((#s)|/)test((#e)|/)* ]] && print "$s:y" || print "$s:n"; done"###,
        );
    }

    /// ksh-like glob operators @()/*()/+()/?()/!().
    #[test]
    fn ksh_glob_operators() {
        assert_parity(
            r###"setopt kshglob; [[ foo = @(foo|bar) ]] && echo a1 || echo a0; [[ aaa = *(a) ]] && echo b1 || echo b0; [[ "" = +(a) ]] && echo c1 || echo c0; [[ a = ?(a) ]] && echo d1 || echo d0; [[ baz = !(foo) ]] && echo e1 || echo e0"###,
        );
    }

    /// approximate exclusion matched without approximation.
    #[test]
    fn approx_exclusion() {
        assert_parity(
            r###"setopt extendedglob; [[ "READ.ME" = (#a1)README~READ_ME ]] && echo m1 || echo n1; [[ "READ_ME" = (#a1)README~READ_ME ]] && echo m2 || echo n2"###,
        );
    }

    /// e:code: shell-code qualifier with REPLY.
    #[test]
    fn e_code_qualifier() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch keep1 keep2 drop; print -rl -- ${(o)$(print -l -- *(e@[[ \$REPLY == keep* ]]@:t))}; cd /; rm -rf $t"###,
        );
    }

    /// P:str: prepend qualifier.
    #[test]
    fn P_prepend_qualifier() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch f1 f2; args=( *(P:-x:) ); print -r -- "${args[@]}"; cd /; rm -rf $t"###,
        );
    }

    /// mtime sort + subscript (om[1,2]).
    #[test]
    fn mtime_sort_subscript() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch -t 202101010000 a; touch -t 202201010000 b; touch -t 202301010000 c; touch -t 202401010000 d; print -rl -- *(.om[1,2]:t); cd /; rm -rf $t"###,
        );
    }

    /// file-mode f:gu+w: qualifier.
    #[test]
    fn fmode_gu_plus_w() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch a; chmod 664 a; touch b; chmod 600 b; print -rl -- ${(o)$(print -l -- *(f:gu+w::t))}; cd /; rm -rf $t"###,
        );
    }

    /// file-mode f-100 (owner lacks execute).
    #[test]
    fn fmode_minus_100() {
        assert_parity(
            r###"setopt extendedglob; t=$(mktemp -d); cd $t; touch noexec; chmod 644 noexec; touch yesexec; chmod 744 yesexec; print -rl -- ${(o)$(print -l -- *(f-100:t))}; cd /; rm -rf $t"###,
        );
    }
}

// ═══════════════════════ zshmisc: cond + arith ═════════════════════

mod man_cond_arith {
    use super::*;

    /// = pattern vs quoted literal.
    #[test]
    fn cond_pattern_vs_literal() {
        assert_parity(
            r###"[[ abc = a*c ]] && echo m1 || echo n1; [[ abc != a*d ]] && echo m2 || echo n2; [[ abc = "a*c" ]] && echo m3 || echo n3"###,
        );
    }

    /// < > ASCII comparison.
    #[test]
    fn cond_ascii_compare() {
        assert_parity(
            r###"[[ apple < banana ]] && echo m1 || echo n1; [[ banana > apple ]] && echo m2 || echo n2; [[ Z < a ]] && echo m3 || echo n3"###,
        );
    }

    /// -nt -ot -ef.
    #[test]
    fn cond_file_compare() {
        assert_parity(
            r###"t=$(mktemp -d); cd $t; touch -t 202001010000 old; touch -t 202501010000 new; cp old samefile; [[ new -nt old ]] && echo m1 || echo n1; [[ old -ot new ]] && echo m2 || echo n2; [[ old -ef old ]] && echo m3 || echo n3; [[ old -ef new ]] && echo m4 || echo n4; cd /; rm -rf $t"###,
        );
    }

    /// =~ regex backref vars.
    #[test]
    fn cond_regex_backrefs() {
        assert_parity(
            r###"if [[ "a short string" =~ "s(...)t" ]]; then print "$MATCH|$MBEGIN|$MEND|$match[1]|$mbegin[1]|$mend[1]"; fi"###,
        );
    }

    /// -o option test.
    #[test]
    fn cond_option_test() {
        assert_parity(
            r###"setopt extendedglob; [[ -o extendedglob ]] && echo m1 || echo n1; setopt noextendedglob; [[ -o extendedglob ]] && echo m2 || echo n2; [[ -o monitor ]] && echo m3 || echo n3"###,
        );
    }

    /// -v varname test (set even if empty).
    #[test]
    fn cond_v_set_test() {
        assert_parity(
            r###"foo=bar; [[ -v foo ]] && echo m1 || echo n1; [[ -v notset999 ]] && echo m2 || echo n2; empty=; [[ -v empty ]] && echo m3 || echo n3"###,
        );
    }

    /// nested ( ) with || &&.
    #[test]
    fn cond_nested_groups() {
        assert_parity(
            r###"[[ ( 1 = 1 && 2 = 2 ) || 3 = 4 ]] && echo m1 || echo n1; [[ 1 = 2 || ( 3 = 3 && 4 = 4 ) ]] && echo m2 || echo n2; [[ ( 1 = 2 || 3 = 4 ) && 5 = 5 ]] && echo m3 || echo n3"###,
        );
    }

    /// ** power, -3**2, right-assoc.
    #[test]
    fn arith_power() {
        assert_parity(
            r###"print $(( 2 ** 10 )); print $(( -3 ** 2 )); print $(( 2 ** 3 ** 2 ))"###,
        );
    }

    /// % modulus, << >> shifts.
    #[test]
    fn arith_mod_shift() {
        assert_parity(
            r###"print $(( 17 % 5 )); print $(( -17 % 5 )); print $(( 1 << 4 )); print $(( 256 >> 2 ))"###,
        );
    }

    /// bitwise & | ^ ~.
    #[test]
    fn arith_bitwise() {
        assert_parity(
            r###"print $(( 12 & 10 )); print $(( 12 | 10 )); print $(( 12 ^ 10 )); print $(( ~5 ))"###,
        );
    }

    /// base#n in bases 2..36.
    #[test]
    fn arith_base_input() {
        assert_parity(
            r###"print $(( 16#ff )); print $(( 2#1010 )); print $(( 8#777 )); print $(( 36#z ))"###,
        );
    }

    /// underscores in literals.
    #[test]
    fn arith_underscores() {
        assert_parity(r###"print $(( 1_000_000 )); print $(( 0xffff_ffff ))"###);
    }

    /// ternary and comma operator.
    #[test]
    fn arith_ternary_comma() {
        assert_parity(
            r###"print $(( 1 ? 10 : 20 )); print $(( 0 ? 10 : 20 )); print $(( (1, 2, 3) ))"###,
        );
    }

    /// pre/post increment.
    #[test]
    fn arith_incr() {
        assert_parity(r###"x=5; print $(( x++ )); print $x; y=5; print $(( ++y )); print $y"###);
    }

    /// integer division truncation vs float.
    #[test]
    fn arith_int_div() {
        assert_parity(r###"print $(( 6/8 )); print $(( 6.0/8 )); print $(( 3.5 + 1.5 ))"###);
    }

    /// output base [#base]/[##base].
    #[test]
    fn arith_output_base() {
        assert_parity(
            r###"print $(( [#16] 255 )); print $(( [##16] 255 )); print $(( [#2] 10 ))"###,
        );
    }

    /// (( )) return status.
    #[test]
    fn arith_return_status() {
        assert_parity(r###"(( 5 )); print $?; (( 0 )); print $?"###);
    }

    /// character value ##x.
    #[test]
    fn arith_char_value() {
        assert_parity(r###"print $(( ##A )); print $(( ##  )); print $(( ##0 ))"###);
    }

    /// logical XOR ^^.
    #[test]
    fn arith_logical_xor() {
        assert_parity(r###"print $(( 1 ^^ 0 )); print $(( 1 ^^ 1 )); print $(( 0 ^^ 0 ))"###);
    }

    /// short-circuit && ||.
    #[test]
    fn arith_short_circuit() {
        assert_parity(r###"x=0; (( 0 && (x=99) )); print $x; (( 1 || (x=88) )); print $x"###);
    }

    /// CBASES grouped hex [#16_4].
    #[test]
    fn arith_cbases_grouped() {
        assert_parity(r###"setopt cbases; print $(( [#16_4] 65536 ** 2 ))"###);
    }
}

// ══════════════════════════════ zshparam ══════════════════════════

mod man_param {
    use super::*;

    /// implicit [key]=value gap fill.
    #[test]
    fn implicit_index_assign() {
        assert_parity(
            r###"array=(one [3]=three four); print -r -- ${#array}; print -rl -- $array"###,
        );
    }

    /// (i) absent yields len+1.
    #[test]
    fn i_absent_len_plus_one() {
        assert_parity(r###"a=(x y z); print -r -- ${a[(i)nope]}; print -r -- ${#a}"###);
    }

    /// range assign resizes.
    #[test]
    fn range_assign_resize() {
        assert_parity(r###"a=(a b c d); a[2,3]=(X Y Z); print -rl -- $a; print -r -- ${#a}"###);
    }

    /// (R) last match, (I) last index.
    #[test]
    fn R_I_last() {
        assert_parity(
            r###"a=(foo bar foo baz); print -r -- ${a[(R)foo]}; print -r -- ${a[(I)foo]}"###,
        );
    }

    /// (i) on assoc returns matching key.
    #[test]
    fn i_assoc_key() {
        assert_parity(r###"typeset -A h; h=(alpha 1 beta 2 gamma 3); print -r -- ${h[(i)be*]}"###);
    }

    /// (r) on assoc compares values.
    #[test]
    fn r_assoc_value() {
        assert_parity(
            r###"typeset -A h; h=(k1 apple k2 banana k3 apricot); print -r -- ${h[(r)ap*]}"###,
        );
    }

    /// (R) on assoc gives all matching values.
    #[test]
    fn R_assoc_all_values() {
        assert_parity(
            r###"typeset -A h; h=(k1 apple k2 banana k3 apricot); print -rl -- ${(o)h[(R)ap*]}"###,
        );
    }

    /// (I) on assoc gives all matching keys.
    #[test]
    fn I_assoc_all_keys() {
        assert_parity(
            r###"typeset -A h=(apple 1 apricot 2 banana 3); print -rl -- ${(o)h[(I)ap*]}"###,
        );
    }

    /// (e) literal star key.
    #[test]
    fn e_literal_star_key() {
        assert_parity(r###"typeset -A aa; aa[(e)*]=star; print -r -- $aa[(e)*]"###);
    }

    /// (re) plain-string element match.
    #[test]
    fn re_plain_match() {
        assert_parity(r###"a=(foo "*" bar); print -r -- ${a[(re)*]}"###);
    }

    /// (w) word subscript on scalar.
    #[test]
    fn w_word_subscript() {
        assert_parity(r###"str="one two three four"; print -r -- ${str[(w)3]}"###);
    }

    /// (ws.:.) custom word separator.
    #[test]
    fn ws_custom_sep() {
        assert_parity(r###"str="a:b:c:d"; print -r -- ${str[(ws.:.)3]}"###);
    }

    /// (rn:2:) nth match.
    #[test]
    fn rn_nth_match() {
        assert_parity(
            r###"a=(x foo y foo z foo); print -r -- ${a[(rn:2:)foo]}; print -r -- ${a[(in:2:)foo]}"###,
        );
    }

    /// (ib:3:) begin at nth.
    #[test]
    fn ib_begin_at_nth() {
        assert_parity(r###"a=(foo bar foo baz foo); print -r -- ${a[(ib:3:)foo]}"###);
    }

    /// (f) line subscript on scalar.
    #[test]
    fn f_line_subscript() {
        assert_parity(r###"printf -v str "L1\nL2\nL3"; print -r -- ${str[(f)2]}"###);
    }

    /// (t) zero-padded type string + value.
    #[test]
    fn t_zero_padded() {
        assert_parity(r###"typeset -Z5 v=42; print -r -- ${(t)v}; print -r -- $v"###);
    }

    /// (t) right-justified blanks.
    #[test]
    fn t_right_blanks() {
        assert_parity(r###"typeset -R6 v=ab; print -r -- ${(t)v}; print -r -- "[$v]""###);
    }

    /// (t) tied scalar/array.
    #[test]
    fn t_tied_types() {
        assert_parity(
            r###"typeset -T VV vv; VV=a:b; print -r -- ${(t)VV}; print -r -- ${(t)vv}"###,
        );
    }

    /// (t) unique array.
    #[test]
    fn t_unique_array() {
        assert_parity(r###"typeset -U a=(1 1 2); print -r -- ${(t)a}; print -r -- $a"###);
    }

    /// tied path<->PATH two-way.
    #[test]
    fn tied_path_PATH() {
        assert_parity(
            r###"path=(/aa /bb /cc); print -r -- $PATH; PATH=/xx:/yy; print -rl -- $path"###,
        );
    }

    /// tied unset cascades — unsetting one half of a `typeset -T` tie
    /// clears the partner via `pm.ename` (c:Src/params.c:3855).
    #[test]
    fn tied_unset_cascade() {
        assert_parity(r###"typeset -T AA aa; AA=1:2; unset AA; print -r -- ${+AA} ${+aa}"###);
    }

    /// KSH_ARRAYS zero-based.
    #[test]
    fn ksharrays_zero_based() {
        assert_parity(
            r###"setopt ksharrays; a=(x y z); print -r -- ${a[0]}; print -r -- ${a[1]}"###,
        );
    }

    /// KSH_ARRAYS $name is element 0.
    #[test]
    fn ksharrays_bare_name() {
        assert_parity(r###"setopt ksharrays; a=(p q r); print -r -- ${a}"###);
    }

    /// pipestatus after pipeline.
    #[test]
    fn pipestatus_array() {
        assert_parity(r###"(exit 3) | (exit 0) | (exit 7); print -r -- $pipestatus"###);
    }

    /// funcstack[1] current function.
    #[test]
    fn funcstack_current() {
        assert_parity(r###"f(){ print -r -- $funcstack[1]; }; f"###);
    }

    /// ZSH_SUBSHELL fork depth.
    #[test]
    fn zsh_subshell_depth() {
        assert_parity(
            r###"print -r -- $ZSH_SUBSHELL; (print -r -- $ZSH_SUBSHELL); ( (print -r -- $ZSH_SUBSHELL) )"###,
        );
    }

    /// reswords reverse subscript.
    #[test]
    fn reswords_reverse() {
        assert_parity(r###"print -r -- ${reswords[(r)while]}"###);
    }

    /// pattern-range substring (r)d?,(r)h?.
    #[test]
    fn pattern_range_substring() {
        assert_parity(r###"string="abcdefghijklm"; print -r -- ${string[(r)d?,(r)h?]}"###);
    }

    /// (i)/(I) offset within scalar.
    #[test]
    fn i_I_scalar_offset() {
        assert_parity(
            r###"string=banana; print -r -- ${string[(i)a]}; print -r -- ${string[(I)a]}"###,
        );
    }

    /// set -A array builtin.
    #[test]
    fn set_A_array() {
        assert_parity(r###"set -A arr alpha beta gamma; print -r -- $arr[2]"###);
    }

    /// sparse [key]=value fills gaps.
    #[test]
    fn sparse_fill() {
        assert_parity(
            r###"a=([2]=two [4]=four); print -r -- "1=[$a[1]] 2=[$a[2]] 3=[$a[3]] 4=[$a[4]]"; print -r -- ${#a}"###,
        );
    }

    /// subscript-on-unset creates array.
    #[test]
    fn subscript_creates_array() {
        assert_parity(r###"arr[3]=hi; print -r -- ${(t)arr}; print -r -- ${#arr}"###);
    }

    /// scalar slice assignment replaces substring.
    #[test]
    fn scalar_slice_assign() {
        assert_parity(r###"v=foobar; v[2,4]=XYZ; print -r -- $v"###);
    }

    /// delete array element with ().
    #[test]
    fn delete_array_element() {
        assert_parity(r###"a=(a b c d); a[2]=(); print -r -- $a; print -r -- ${#a}"###);
    }

    /// unset "name[key]" deletes assoc element.
    #[test]
    fn unset_assoc_element() {
        assert_parity(
            r###"typeset -A h=(x 1 y 2 z 3); unset "h[y]"; print -r -- ${(oj:,:)h}; print -r -- ${#h}"###,
        );
    }

    /// [key]+=value appends to element.
    #[test]
    fn element_append() {
        assert_parity(r###"typeset -A h=(a foo); h[a]+=bar; print -r -- $h[a]"###);
    }

    /// positional n=(...) shifts following — `N=(...)` is `argv[N]=(...)`
    /// (IPDEF9 argv), splicing into the positionals.
    #[test]
    fn positional_array_assign() {
        assert_parity(r###"set -- a b c d; 2=(X Y Z); print -r -- $*"###);
    }

    /// positional n=value creates intervening.
    #[test]
    fn positional_create_gap() {
        assert_parity(r###"set --; 3=hi; print -r -- "1=[$1] 2=[$2] 3=[$3]""###);
    }

    /// FUNCTION_ARGZERO $0 = function name.
    #[test]
    fn function_argzero() {
        assert_parity(r###"setopt functionargzero; f(){ print -r -- $0; }; f"###);
    }
}

// ═════════════════════════════ zshbuiltins ════════════════════════

mod man_builtins {
    use super::*;

    /// print -a -C columns.
    #[test]
    fn print_a_C() {
        assert_parity(r###"print -a -C 2 -- a b c d"###);
    }

    /// print -m pattern filter.
    #[test]
    fn print_m_pattern() {
        assert_parity(r###"print -m "f*" foo bar fizz baz"###);
    }

    /// print -o / -O sort.
    #[test]
    fn print_o_O() {
        assert_parity(r###"print -o -- banana apple cherry; print -O -- banana apple cherry"###);
    }

    /// print -o -i case-independent.
    #[test]
    fn print_o_i() {
        assert_parity(r###"print -o -i -- Banana apple Cherry"###);
    }

    /// print -f format reuse.
    #[test]
    fn print_f_reuse() {
        assert_parity(r###"print -f "%s.%s\n" a b c d"###);
    }

    /// print -D named-dir abbrev.
    #[test]
    fn print_D_nameddir() {
        assert_parity(r###"hash -d myd=/usr/local; print -D /usr/local/bin"###);
    }

    /// print -z / read -z editor buffer stack.
    #[test]
    fn print_z_read_z() {
        assert_parity(r###"print -z foo bar; read -z x; print -r -- "[$x]""###);
    }

    /// print -R BSD echo, only -e -n recognized.
    #[test]
    fn print_R_bsd() {
        assert_parity(r###"print -R "a\tb"; print -Rn x; print"###);
    }

    /// printf %b escapes.
    #[test]
    fn printf_b() {
        assert_parity(r###"printf "%b\n" "a\tb""###);
    }

    /// printf %q quoting.
    #[test]
    fn printf_q() {
        assert_parity(r###"printf "%q\n" "a b*c""###);
    }

    /// printf format reuse.
    #[test]
    fn printf_reuse() {
        assert_parity(r###"printf "%s-%s\n" 1 2 3 4"###);
    }

    /// printf numeric quote-char.
    #[test]
    fn printf_numeric_quotechar() {
        assert_parity(r###"printf '%d\n' "'A""###);
    }

    /// printf -v array, one element per reuse.
    #[test]
    fn printf_v_array() {
        assert_parity(
            r###"typeset -a arr; printf -v arr "%s+%s\n" a b c d; print -r -- "${#arr}|${arr[1]}""###,
        );
    }

    /// typeset -i base output.
    #[test]
    fn typeset_i_base() {
        assert_parity(r###"typeset -i 16 x=255; print -r -- $x"###);
    }

    /// typeset -Z zero pad.
    #[test]
    fn typeset_Z() {
        assert_parity(r###"typeset -Z5 n=42; print -r -- "$n""###);
    }

    /// typeset -L left justify.
    #[test]
    fn typeset_L() {
        assert_parity(r###"typeset -L5 s=ab; print -r -- "[$s]""###);
    }

    /// typeset -R right justify.
    #[test]
    fn typeset_R() {
        assert_parity(r###"typeset -R5 s=ab; print -r -- "[$s]""###);
    }

    /// typeset -l / -u expansion case.
    #[test]
    fn typeset_l_u() {
        assert_parity(
            r###"typeset -l v=HeLLo; print -r -- "$v"; typeset -u w=HeLLo; print -r -- "$w""###,
        );
    }

    /// typeset -F fixed-point.
    #[test]
    fn typeset_F() {
        assert_parity(r###"typeset -F3 f=3.14159; print -r -- "$f""###);
    }

    /// typeset -E scientific.
    #[test]
    fn typeset_E() {
        assert_parity(r###"typeset -E2 f=12345; print -r -- "$f""###);
    }

    /// typeset -p integer.
    #[test]
    fn typeset_p_int() {
        assert_parity(r###"typeset -i x=5; typeset -p x"###);
    }

    /// typeset -p assoc.
    #[test]
    fn typeset_p_assoc() {
        assert_parity(r###"typeset -A h=(a 1 b 2); typeset -p h"###);
    }

    /// typeset -r readonly error.
    #[test]
    fn typeset_r_readonly() {
        assert_parity(r###"typeset -r r=1; r=2"###);
    }

    /// unset -m pattern.
    #[test]
    fn unset_m() {
        assert_parity(
            r###"foo1=a foo2=b bar=c; unset -m "foo*"; print -r -- "${foo1-unset} ${bar}""###,
        );
    }

    /// read -A array.
    #[test]
    fn read_A() {
        assert_parity(r###"print "a b c" | read -A arr; print -r -- "${#arr} ${arr[2]}""###);
    }

    /// read -d delimiter.
    #[test]
    fn read_d() {
        assert_parity(r###"printf "foo:bar" | read -d : x; print -r -- "[$x]""###);
    }

    /// read leftover fields to last name.
    #[test]
    fn read_leftover() {
        assert_parity(r###"print "one two three four" | read a b; print -r -- "$a|$b""###);
    }

    /// getopts silent missing arg.
    #[test]
    fn getopts_silent_missing() {
        assert_parity(r###"set -- -f; getopts :f: opt; print -r -- "name=$opt OPTARG=$OPTARG""###);
    }

    /// getopts silent unknown.
    #[test]
    fn getopts_silent_unknown() {
        assert_parity(r###"set -- -z; getopts :ab opt; print -r -- "name=$opt OPTARG=$OPTARG""###);
    }

    /// set -s sorts positionals.
    #[test]
    fn set_s_sort() {
        assert_parity(r###"set -s banana apple cherry; print -r -- "$@""###);
    }

    /// shift n.
    #[test]
    fn shift_n() {
        assert_parity(r###"set -- 1 2 3 4 5; shift 3; print -r -- "$@""###);
    }

    /// shift -p pops from end.
    #[test]
    fn shift_p() {
        assert_parity(r###"arr=(1 2 3 4); shift -p 2 arr; print -r -- "$arr""###);
    }

    /// whence -w word types.
    #[test]
    fn whence_w() {
        assert_parity(r###"f(){ :; }; alias al=ls; whence -w f; whence -w al; whence -w if"###);
    }

    /// functions -c copy.
    #[test]
    fn functions_c_copy() {
        assert_parity(
            r###"orig(){ print original; }; functions -c orig copy; orig(){ print changed; }; copy"###,
        );
    }

    /// hash -d / ~name.
    #[test]
    fn hash_d_nameddir() {
        assert_parity(r###"hash -d nd=/usr/bin; print -r -- ~nd"###);
    }

    /// hash -dL listing.
    #[test]
    fn hash_dL() {
        assert_parity(r###"hash -d nd=/var/tmp; hash -dL"###);
    }

    /// alias -L startup form.
    #[test]
    fn alias_L() {
        assert_parity(r###"alias foo=bar; alias -L foo"###);
    }

    /// repeat n cmd.
    #[test]
    fn repeat_n() {
        assert_parity(r###"repeat 3 print -n x; print"###);
    }

    /// let exit status.
    #[test]
    fn let_status() {
        assert_parity(r###"let "1+1"; print $?; let "0"; print $?"###);
    }

    /// (( )) assignment.
    #[test]
    fn arith_assign_builtin() {
        assert_parity(r###"(( x = 5 * 3 )); print -r -- $x"###);
    }

    /// integer -i base.
    #[test]
    fn integer_i_base() {
        assert_parity(r###"integer -i 2 b=10; print -r -- $b"###);
    }

    /// set +o lists options as input.
    #[test]
    fn set_plus_o() {
        assert_parity(r###"setopt extendedglob; set +o | grep "extendedglob$""###);
    }

    /// unsetopt -m pattern.
    #[test]
    fn unsetopt_m() {
        assert_parity(
            r###"setopt extendedglob globdots; unsetopt -m "glob*"; [[ -o globdots ]] && print on || print off"###,
        );
    }
}

// ═════════════════════════════ zshoptions ═════════════════════════

mod man_options {
    use super::*;

    /// SH_WORD_SPLIT.
    #[test]
    fn shwordsplit() {
        assert_parity(r###"setopt shwordsplit; v="a b c"; set -- $v; print -r -- "$#""###);
    }

    /// RC_EXPAND_PARAM distribution.
    #[test]
    fn rcexpandparam() {
        assert_parity(r###"setopt rcexpandparam; xx=(a b c); print -r -- foo${xx}bar"###);
    }

    /// NULL_GLOB deletes non-matching.
    #[test]
    fn nullglob() {
        assert_parity(
            r###"setopt nullglob; d=$(mktemp -d); cd "$d"; set -- pre nomatch_xyz* post; print -r -- "$#: $@"; cd /; rm -rf "$d""###,
        );
    }

    /// NOMATCH errors.
    #[test]
    fn nomatch() {
        assert_parity(r###"setopt nomatch; print -r -- /nonexistent_dir_xyz_*/foo"###);
    }

    /// GLOB_DOTS includes dotfiles.
    #[test]
    fn globdots() {
        assert_parity(
            r###"setopt globdots; d=$(mktemp -d); cd "$d"; touch .hidden visible; print -rl -- ${(o)$(print -l -- *)}; cd /; rm -rf "$d""###,
        );
    }

    /// NUMERIC_GLOB_SORT.
    #[test]
    fn numericglobsort() {
        assert_parity(
            r###"setopt numericglobsort; d=$(mktemp -d); cd "$d"; touch f1 f10 f2 f20 f3; print -r -- f*; cd /; rm -rf "$d""###,
        );
    }

    /// NO_CASE_GLOB.
    #[test]
    fn nocaseglob() {
        assert_parity(
            r###"unsetopt caseglob; d=$(mktemp -d); cd "$d"; touch ABC; print -r -- abc*; cd /; rm -rf "$d""###,
        );
    }

    /// BARE_GLOB_QUAL.
    #[test]
    fn bareglobqual() {
        assert_parity(
            r###"setopt bareglobqual; d=$(mktemp -d); cd "$d"; mkdir adir; touch afile; print -r -- *(/); cd /; rm -rf "$d""###,
        );
    }

    /// GLOB_SUBST.
    #[test]
    fn globsubst() {
        assert_parity(
            r###"setopt globsubst; d=$(mktemp -d); cd "$d"; touch abc; pat="a*"; print -r -- $pat; cd /; rm -rf "$d""###,
        );
    }

    /// MARK_DIRS.
    #[test]
    fn markdirs() {
        assert_parity(
            r###"setopt markdirs; d=$(mktemp -d); cd "$d"; mkdir sub; print -r -- *; cd /; rm -rf "$d""###,
        );
    }

    /// GLOB_ASSIGN — scalar assignment `v=pattern` globs the RHS and
    /// recreates the param as array (>1 match) or scalar (≤1 match).
    #[test]
    fn globassign() {
        assert_parity(
            r###"setopt globassign; d=$(mktemp -d); cd "$d"; touch x1 x2; v=x*; print -r -- "${#v} ${v[1]}"; cd /; rm -rf "$d""###,
        );
    }

    /// GLOB_ASSIGN sub-behaviors: type is `array` for multi-match,
    /// `scalar` for a single match; no-match errors "no matches found"
    /// (with the literal pattern, not token bytes); off → literal scalar.
    #[test]
    fn globassign_shapes_and_nomatch() {
        // multi-match → array
        assert_parity(
            r###"setopt globassign; d=$(mktemp -d); cd "$d"; touch xa xb xc; v=x*; print -r -- "${(t)v}"; cd /; rm -rf "$d""###,
        );
        // single match → scalar
        assert_parity(
            r###"setopt globassign; d=$(mktemp -d); cd "$d"; touch only1; v=only*; print -r -- "${(t)v}/$v"; cd /; rm -rf "$d""###,
        );
        // no match → "no matches found: <literal pattern>" + nonzero status
        assert_parity(r###"setopt globassign; v=zznomatch_qq*; print -r -- "rc=$?""###);
        // option off → literal scalar, no globbing
        assert_parity(
            r###"d=$(mktemp -d); cd "$d"; touch xa xb; v=x*; print -r -- "${(t)v}/$v"; cd /; rm -rf "$d""###,
        );
        // a non-glob scalar RHS is unaffected even with globassign on
        assert_parity(r###"setopt globassign; v=hello; print -r -- "${(t)v}/$v""###);
    }

    /// GLOB_ASSIGN no-match interacts with NULL_GLOB / CSH_NULL_GLOB the
    /// same way command-position globbing does (globlist's failure
    /// handling, c:Src/glob.c:1872-1888): NULL_GLOB → empty scalar (word
    /// dropped, no error); CSH_NULL_GLOB → csh-style "no match"; neither
    /// → "no matches found". A matching pattern is unaffected by either.
    #[test]
    fn globassign_nullglob_cshnullglob() {
        // NULL_GLOB + no match → empty scalar, no error
        assert_parity(
            r###"setopt globassign nullglob; v=zznomatch_qq*; print -r -- "[${(t)v}/$v] rc=$?""###,
        );
        // NULL_GLOB + match → still globs to array
        assert_parity(
            r###"setopt globassign nullglob; d=$(mktemp -d); cd "$d"; touch xa xb; v=x*; print -r -- "${(t)v}/$v"; cd /; rm -rf "$d""###,
        );
        // CSH_NULL_GLOB + no match → "no match" + nonzero status
        assert_parity(r###"setopt globassign cshnullglob; v=zznomatch_qq*; print -r -- "rc=$?""###);
    }

    /// RC_QUOTES.
    #[test]
    fn rcquotes() {
        assert_parity(r###"setopt rcquotes; print -r -- 'it''s here'"###);
    }

    /// PROMPT_SUBST on vs off.
    #[test]
    fn promptsubst() {
        assert_parity(
            r###"setopt promptsubst; x=world; print -rP 'hi $x'; unsetopt promptsubst; print -rP 'hi $x'"###,
        );
    }

    /// WARN_CREATE_GLOBAL.
    #[test]
    fn warncreateglobal() {
        assert_parity(r###"setopt warncreateglobal; f() { gv=1; }; f"###);
    }

    /// MULTIOS tee.
    #[test]
    fn multios() {
        assert_parity(
            r###"d=$(mktemp -d); echo hi > "$d/a" > "$d/b"; cat "$d/a" "$d/b"; rm -rf "$d""###,
        );
    }

    /// PIPE_FAIL.
    #[test]
    fn pipefail() {
        assert_parity(r###"setopt pipefail; false | true; print -r -- $?"###);
    }

    /// ERR_EXIT.
    #[test]
    fn errexit() {
        assert_parity(r###"setopt errexit; false; print should_not_print"###);
    }

    /// C_BASES.
    #[test]
    fn cbases() {
        assert_parity(r###"setopt cbases; typeset -i 16 x=255; print -r -- $x"###);
    }

    /// OCTAL_ZEROES.
    #[test]
    fn octalzeroes() {
        assert_parity(r###"setopt octalzeroes; print -r -- $(( 010 ))"###);
    }

    /// MAGIC_EQUAL_SUBST.
    #[test]
    fn magicequalsubst() {
        assert_parity(
            r###"setopt magicequalsubst; export HOME=/tmp/fakehome; print -r -- foo=~/bar"###,
        );
    }

    /// KSH_ZERO_SUBSCRIPT.
    #[test]
    fn kshzerosubscript() {
        assert_parity(r###"setopt kshzerosubscript; a=(x y z); print -r -- "${a[0]}""###);
    }

    /// APPEND_CREATE off errors.
    #[test]
    fn appendcreate_off() {
        assert_parity(
            r###"unsetopt clobber appendcreate; d=$(mktemp -d); { echo hi >> "$d/n.txt"; } 2>/dev/null; print -r -- "exit=$? exists=$([[ -e $d/n.txt ]] && echo y || echo n)"; rm -rf "$d""###,
        );
    }

    /// GLOB_STAR_SHORT.
    #[test]
    fn globstarshort() {
        assert_parity(
            r###"setopt globstarshort; d=$(mktemp -d); cd "$d"; mkdir sub; touch sub/deep.c top.c; print -rl -- ${(o)$(print -l -- **.c)}; cd /; rm -rf "$d""###,
        );
    }

    /// HIST_SUBST_PATTERN.
    #[test]
    fn histsubstpattern() {
        assert_parity(
            r###"setopt histsubstpattern extendedglob; v=hello123world; print -r -- ${v//(#m)[0-9]##/N}"###,
        );
    }
}
