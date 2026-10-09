//! `test` / `[` builtin parity — argv is compiled by `parse_cond` in
//! test mode (`testlex`, c:Src/builtin.c:7200; `par_cond*`,
//! c:Src/parse.c:2409-2731) and run through `evalcond` (c:Src/cond.c:70).
//! Every case runs under real zsh and zshrs; stdout, stderr (shell-name
//! prefix normalised) and `$?` must match.
//!
//! No-ops silently when `zsh` is not on PATH.
#![allow(non_snake_case)]

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

fn zsh_available() -> bool {
    Command::new(crate::oracle::zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Drop the leading `zsh:` / `zshrs:` / `zsh:N:` shell-name prefix so only the
/// builtin name and message are compared.
fn norm(stderr: &str) -> String {
    stderr
        .lines()
        .map(|l| {
            let rest = l
                .strip_prefix("zshrs")
                .or_else(|| l.strip_prefix("zsh"))
                .and_then(|r| r.strip_prefix(':'));
            match rest {
                Some(r) => {
                    let r = r.trim_start_matches(|c: char| c.is_ascii_digit());
                    r.strip_prefix(':').unwrap_or(r).trim_start().to_string()
                }
                None => l.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Run {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run(bin: &str, args: &[&str], script: &str) -> Run {
    let o = Command::new(bin)
        .args(args)
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell");
    Run {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: norm(&String::from_utf8_lossy(&o.stderr)),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Run every case in `cases` as `<case>; echo "rc=$?"` under both shells and
/// fail with the full list of divergent cases.
fn assert_cases(cases: &[&str]) {
    if !zsh_available() {
        return;
    }
    let zshrs = zshrs_bin();
    let zshrs = zshrs.to_str().expect("utf8 path");
    let mut bad = Vec::new();
    for case in cases {
        let script = format!("{case}; echo \"rc=$?\"");
        let z = run(crate::oracle::zsh_path(), &["-fc"], &script);
        let r = run(zshrs, &["--zsh", "-f", "-c"], &script);
        if z.stdout != r.stdout || z.stderr != r.stderr || z.exit != r.exit {
            bad.push(format!(
                "{case}\n  zsh  : out={:?} err={:?} exit={}\n  zshrs: out={:?} err={:?} exit={}",
                z.stdout, z.stderr, z.exit, r.stdout, r.stderr, r.exit
            ));
        }
    }
    assert!(bad.is_empty(), "test/[ divergences:\n{}", bad.join("\n"));
}

/// Prefix creating a scratch tree: file `f`, empty file `e`, dir `d`, link `l`
/// -> `f`, hard link `h` -> `f`, `old` older than `f`, executable `x`.
const TREE: &str = "t=$(mktemp -d) || exit 9; touch $t/e; echo data > $t/f; mkdir $t/d; \
ln -s f $t/l; ln $t/f $t/h; touch -t 200001010000 $t/old; \
echo '#!/bin/sh' > $t/x; chmod 755 $t/x";

#[test]
fn argument_counts() {
    assert_cases(&[
        "test",
        "test a",
        "test ''",
        "[ ]",
        "[ a ]",
        "[ '' ]",
        "[ a b ]",
        "[ a = a ]",
        "[ a = b ]",
        "[ a != b ]",
        "[ a == a ]",
        "[ a b c ]",
        "[ a b c d ]",
        "[ a b c d e ]",
        "[ -n a b c d ]",
        "[ a b c d e f g ]",
        "[ -f ]",
        "[ -t ]",
    ]);
}

#[test]
fn negation() {
    assert_cases(&[
        "[ ! ]",
        "[ ! a ]",
        "[ ! '' ]",
        "[ ! ! a ]",
        "[ ! ! ! a ]",
        "[ ! a = a ]",
        "[ ! a = b ]",
        "[ ! = x ]",
        "[ ! -n a ]",
        "[ ! -z a ]",
        "[ ! -a ]",
        "[ ! a -a b ]",
        "[ ! -a -a b ]",
        "[ ! -o -o b ]",
        "[ ! a -o '' ]",
        "[ ! ! ]",
    ]);
}

#[test]
fn and_or_connectives() {
    assert_cases(&[
        "[ a -a b ]",
        "[ '' -a b ]",
        "[ a -a '' ]",
        "[ '' -o b ]",
        "[ '' -o '' ]",
        "[ -n a -a -n b ]",
        "[ -n a -a -z b ]",
        "[ -z a -o -n b ]",
        "[ -z a -o -z b ]",
        "[ -n a -a -n b -o -z c ]",
        "[ -z a -a -n b -o -n c ]",
        "[ -a ]",
        "[ -o ]",
        "[ -a -a -a ]",
        "[ -o -o -o ]",
        "[ a -o ]",
        "[ a -a ]",
        "[ -a a ]",
        "[ -o a ]",
        "[ -a a b ]",
        "[ a -a b -a c ]",
        "[ a -o b -a '' ]",
    ]);
}

#[test]
fn parentheses() {
    assert_cases(&[
        "[ \\( a \\) ]",
        "[ \\( '' \\) ]",
        "[ \\( -n a \\) ]",
        "[ \\( -z a \\) ]",
        "[ \\( -n a -o -z a \\) -a -n b ]",
        "[ \\( -n a -o -z a \\) -a -z b ]",
        "[ \\( a ]",
        "[ \\) ]",
        "[ \\( ]",
        "[ \\( \\) ]",
        "[ \\( = \\) ]",
        "[ \\( = = \\) ]",
        "[ '(' = '(' ]",
        "[ '(' = ')' ]",
        "[ \\( \\( a \\) \\) ]",
        "[ ! \\( a \\) ]",
        "[ ! \\( '' \\) ]",
        "[ \\( a \\) b ]",
        "[ \\( a = a \\) ]",
        "[ \\( a = b \\) -o \\( b = b \\) ]",
    ]);
}

#[test]
fn unary_string_tests() {
    assert_cases(&[
        "[ -n ]",
        "[ -n a ]",
        "[ -n '' ]",
        "[ -z ]",
        "[ -z '' ]",
        "[ -z a ]",
        "[ -n -n ]",
        "[ -z -z ]",
        "[ -n a b ]",
        "[ -z a b ]",
        "[ -n = ]",
        "[ -n = x ]",
    ]);
}

#[test]
fn numeric_comparisons() {
    assert_cases(&[
        "[ 1 -eq 1 ]",
        "[ 1 -eq 2 ]",
        "[ 1 -ne 2 ]",
        "[ 1 -lt 2 ]",
        "[ 2 -lt 1 ]",
        "[ 2 -gt 1 ]",
        "[ 1 -le 1 ]",
        "[ 1 -ge 2 ]",
        "[ 10 -gt 9 ]",
        "[ -1 -lt 0 ]",
        "[ 1.5 -eq 1 ]",
        "[ a -eq 1 ]",
        "[ 1 -eq a ]",
        "[ '' -eq 0 ]",
        "[ 0x10 -eq 16 ]",
        "[ 1 -eq 1 -a 2 -lt 3 ]",
        "[ 1 -eq 1 -a 2 -gt 3 ]",
        "[ 1 -eq ]",
        "[ -eq 1 ]",
        "[ 1 -eq 1 -a ]",
    ]);
}

#[test]
fn string_comparisons() {
    assert_cases(&[
        "[ a = a ]",
        "[ a == a ]",
        "[ a = b ]",
        "[ a != a ]",
        "[ a != b ]",
        "[ a '<' b ]",
        "[ b '<' a ]",
        "[ b '>' a ]",
        "[ a '>' b ]",
        "[ 'a*' = 'a*' ]",
        "[ abc = 'a*' ]",
        "[ abc = abc* ]",
        "[ abc != 'a?c' ]",
        "[ = = = ]",
        "[ -a = -a ]",
        "[ '' = '' ]",
        "[ a = ]",
        "[ = ]",
        "[ a '<' ]",
        "[ '<' ]",
        "[ '<' '<' '<' ]",
        "[ a =~ a ]",
    ]);
}

#[test]
fn file_tests() {
    let cases: Vec<String> = [
        "[ -e $t/f ]",
        "[ -e $t/nope ]",
        "[ -a $t/f ]",
        "[ -f $t/f ]",
        "[ -f $t/d ]",
        "[ -d $t/d ]",
        "[ -d $t/f ]",
        "[ -s $t/f ]",
        "[ -s $t/e ]",
        "[ -r $t/f ]",
        "[ -w $t/f ]",
        "[ -x $t/x ]",
        "[ -x $t/f ]",
        "[ -L $t/l ]",
        "[ -h $t/l ]",
        "[ -L $t/f ]",
        "[ -f $t/l ]",
        "[ $t/f -nt $t/old ]",
        "[ $t/old -nt $t/f ]",
        "[ $t/old -ot $t/f ]",
        "[ $t/f -ot $t/old ]",
        "[ $t/f -nt $t/nope ]",
        "[ $t/nope -ot $t/f ]",
        "[ $t/f -ef $t/h ]",
        "[ $t/f -ef $t/l ]",
        "[ $t/f -ef $t/e ]",
        "[ -f $t/f -a -d $t/d ]",
        "[ -f $t/f -a -d $t/f ]",
        "[ -f $t/nope -o -d $t/d ]",
        "[ ! -e $t/nope ]",
        "[ -e ]",
        "[ -d ]",
    ]
    .iter()
    .map(|c| format!("{TREE}; {c}; r=$?; rm -rf $t; (exit $r)"))
    .collect();
    let refs: Vec<&str> = cases.iter().map(|s| s.as_str()).collect();
    assert_cases(&refs);
}

#[test]
fn syntax_errors() {
    assert_cases(&[
        "[ a -foo b ]",
        "[ -foo a ]",
        "[ -foo ]",
        "[ a b c d e f ]",
        "[ \\( a b ]",
        "[ a \\) ]",
        "[ a \\( ]",
        "[ a -a \\( ]",
        "[ a b -a c ]",
        "[ a b = c ]",
        "[ a = b c ]",
        "[ a -o b c ]",
        "[ -n a -a ]",
        "[ \\( -n a ]",
        "[ \\( -n a \\) \\) ]",
        "test a b",
        "test a b c d e",
        "test \\( ",
        "test a = ",
        "test -n a -a",
        "test ! ! ! ! !",
    ]);
}

#[test]
fn missing_closing_bracket() {
    assert_cases(&[
        "[ a",
        "[ a = a",
        "[",
        "[ ]]",
        "[ a ] b ]",
        "[ a ] b",
        "[ -n",
        "[ ! ",
    ]);
}

#[test]
fn emulate_sh_ksh_edge_cases() {
    let mut cases: Vec<String> = Vec::new();
    for emu in ["sh", "ksh", "bash"] {
        for body in [
            "[ ! ]",
            "[ -a ]",
            "[ -n ]",
            "[ -z ]",
            "[ -t ]",
            "[ ! -a ]",
            "[ ! -n ]",
            "[ -a x ]",
            "[ -n = ]",
            "[ ! a ]",
            "[ ! a = b ]",
            "[ a = a -a b = b ]",
            "[ -e /nonexistent -o -d / ]",
            "[ -d / -a -e /nonexistent ]",
            "[ \\( -d / \\) ]",
            "[ \\( -d / -o -e /x \\) ]",
            "test ! -a -a x",
        ] {
            cases.push(format!("emulate {emu} -c '{body}; echo inner=$?'"));
        }
    }
    let refs: Vec<&str> = cases.iter().map(|s| s.as_str()).collect();
    assert_cases(&refs);
}
