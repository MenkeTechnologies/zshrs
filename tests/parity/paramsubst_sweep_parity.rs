//! Parameter-expansion divergences found by the 2026-10 differential sweep
//! against the upstream-master oracle (flags, replace anchors, flag-parse
//! diagnostics). Each case is checked against the oracle when it is
//! available AND against the pinned upstream output, so a missing oracle
//! cannot turn a regression green.

use std::path::PathBuf;
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

/// stdout, stderr with the `zsh:N:` / `zshrs:N:` tag dropped, exit status.
fn run(cmd: &mut Command, script: &str) -> (String, String, i32) {
    let o = cmd.arg(script).output().expect("spawn shell");
    let err = String::from_utf8_lossy(&o.stderr)
        .lines()
        .map(|l| {
            let l = l
                .strip_prefix("zshrs:")
                .or_else(|| l.strip_prefix("zsh:"))
                .unwrap_or(l);
            match l.split_once(": ") {
                Some((n, rest)) if n.bytes().all(|b| b.is_ascii_digit()) => rest.to_string(),
                _ => l.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        err,
        o.status.code().unwrap_or(-1),
    )
}

fn check(script: &str, out: &str, err: &str, status: i32) {
    let mut rs = Command::new(zshrs_bin());
    rs.args(["--zsh", "-f", "-c"]).env_remove("ZSHRS_CACHE");
    let got = run(&mut rs, script);
    let zsh = crate::oracle::zsh_path();
    if Command::new(zsh).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let want = run(Command::new(zsh).arg("-fc"), script);
        assert_eq!(got, want, "zshrs vs oracle on:\n{script}");
    }
    assert_eq!(got, (out.to_string(), err.to_string(), status), "on:\n{script}");
}

// c:Src/subst.c:3120-3134 — the replace anchors are read from the source
// text right after the `/`; an escaped or quoted `#`/`%` there is a token
// (Bnull/Snull), not an anchor, and stays pattern text.
#[test]
fn replace_escaped_percent_is_not_an_end_anchor() {
    check(r"y='a%b'; print -r -- ${y/\%/P} ${y/'%'/P}", "aPb aPb\n", "", 0);
    check(r"y='a%b'; print -r -- ${y/\%b/P} ${y/\%*/Z}", "aP aZ\n", "", 0);
    check(r"y='%ab'; print -r -- ${y/#\%/P}", "Pab\n", "", 0);
    check(r"x='%a%'; print -r -- ${x/\%%/V} ${x/%\%/Z}", "%a% %aZ\n", "", 0);
    check(r#"y='a%b%'; print -r -- "${y/\%/P}""#, "aPb%\n", "", 0);
}

// c:Src/subst.c:2504-2528 flagerr — every flag-parse failure reports the
// 1-based offset of `s` from the `$`. These paths either printed a bare
// "error in flags" / "bad substitution", accepted the flags, or counted the
// position from the wrong delimiter.
#[test]
fn flag_parse_errors_report_the_c_position() {
    for (expr, pos) in [
        ("${(qq-)v}", 5),          // c:2242 q- after q
        ("${(q-q)v}", 6),          // c:2250 q after q-
        ("${(qqqqq)v}", 8),        // c:2238 q after QT_DOLLARS
        ("${(bq)v}", 5),           // c:2238 q after b
        ("${(qb)v}", 5),           // c:2256 b after q
        ("${(bb)v}", 5),           // c:2256 b after b
        ("${(!k)v}", 5),           // c:2391 k after !
        ("${(k!)v}", 5),           // c:2387 ! after k
        ("${(v!)v}", 5),           // c:2387 ! after v
        ("${(!v)v}", 5),           // c:2395 v after !
        ("${(g:z:)v}", 6),         // c:2429 unknown (g) sub-flag
        ("${(g:e)v}", 5),          // c:2414 (g) delimiter never closed
        ("${(g)v}", 5),            // c:2436 (g) without an argument
        ("${(Z:c)v}", 5),          // c:2445 (Z) delimiter never closed
        ("${(}", 4),               // c:2504 flag block runs into the `}`
        ("${(Q}", 5),              //   ... after a valid flag
        ("${(l:3::\\::)v}", 11),   // c:2361 (l) STR2 opened, never closed
        ("${(l:3::xyz::abc)v}", 13),
        ("${(lj:3:)v}", 5),        // c:1436 get_intarg: no second `j`
        ("${(I:12)v}", 5),         // c:1436 (I) delimiter never closed
    ] {
        check(
            &format!("v=a; print -r -- {expr}"),
            "",
            &format!("error in flags near position {pos} in '{expr}'"),
            1,
        );
    }
}

// c:Src/glob.c:424 — a match's name is the pathbuf text plus the entry, so a
// typed `./`, `.//` or `././` prefix is kept, including in $REPLY for (e).
#[test]
fn glob_keeps_a_typed_dot_slash_prefix_verbatim() {
    check(
        "d=$(mktemp -d) && cd $d && : > a.txt && print -r -- .//*.txt ././*.txt .///*.txt \
         ./*.txt(e:'REPLY=R$REPLY':) .//*.txt(:h); cd / && rm -r -- $d",
        ".//a.txt ././a.txt .///a.txt R./a.txt .\n",
        "",
        0,
    );
}

// c:Src/subst.c:4744-4747 — modify() leaves a `:` with no modifier letter
// after it unparsed, and c:3797-3800 reports it; `imeta(s[1])` (the NUL after
// a trailing `:`, or a lexer token) selects the message without a character.
#[test]
fn braced_trailing_colon_is_an_unrecognized_modifier() {
    for expr in ["${x:h:}", "${x:u:}", "${x:s/a/b/:}", "${x:gh:}", "${x:h:=}", "${x:h:*}", "${x:h:-q}"] {
        check(&format!("x=/a/b; print -r -- {expr}"), "", "unrecognized modifier", 1);
    }
    check("x=/a/b; print -r -- ${x:h:Z}", "", "unrecognized modifier `Z'", 1);
    check("x=/a/b; print -r -- $x:h: $x:t::", "/a: b::\n", "", 0);
}

// c:Src/utils.c:722 / c:5721 — `WC_ISPRINT` and `WCWIDTH` are the wcwidth9
// tables (Src/ztype.h:77, Src/zsh.h:3302): format characters, a BOM and an
// unassigned code point are not printable, so `(q+)` writes them `$'\u…'`
// and `(m)` gives them no width.
#[test]
fn q_plus_and_width_use_the_unicode9_printable_table() {
    check(
        r"for c in $'­' $'​' $'﻿' $'͸'; do print -r -- ${(q+)c} ${(m)#c}; done",
        "$'\\M--' 0\n$'\\u200b' 0\n$'\\ufeff' 0\n$'\\u0378' 0\n",
        "",
        0,
    );
}

// c:Src/subst.c:3054 / c:3783-3818 — an unbraced reference applies the
// history modifiers that follow its subscript.
#[test]
fn unbraced_subscript_takes_modifiers() {
    check(
        r#"a=(/x/one /y/two); print -r -- $a[-1]:u $a[1]:h "$a[2]:t:r" $a[1]:s/o/0/ $a[1]: $a[1]:x"#,
        "/Y/TWO /x two /x/0ne /x/one: /x/one:x\n",
        "",
        0,
    );
}

// c:Src/glob.c:947-972 — GS_DEPTH compares the match NAMES, which carry no
// `./` for a top-level match (c:424), so `(od)` puts deeper matches first.
#[test]
fn glob_depth_sort_compares_match_names() {
    check(
        "d=$(mktemp -d) && cd $d && mkdir -p sub/deep && : > a.txt > b.txt > sub/s.txt > sub/deep/d.txt \
         && print -r -- **/*(.od) && print -r -- **/*(.Od); cd / && rm -r -- $d",
        "sub/deep/d.txt sub/s.txt a.txt b.txt\na.txt b.txt sub/s.txt sub/deep/d.txt\n",
        "",
        0,
    );
}
