//! Regressions for divergences found by replaying zsh's own D- and E-series
//! `.ztst` chunks (expansion, subscripts, options) against `zsh` 5.9.2 and
//! zshrs side by side. Each expected value is what `/usr/local/bin/zsh -fc`
//! printed for the same script.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zshrs"))
}

/// Run `zshrs --zsh -f -c script` → (exit code, stdout, stderr).
fn run(script: &str) -> (i32, String, String) {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// c:Src/lex.c:1229-1247 — a command-position word whose text before `=` is
/// only a `[...]` subscript (or only `+`) is still ENVSTRING; the assignment
/// then fails isident in assignsparam (c:Src/params.c:3203-3204), which is a
/// fatal error that aborts the rest of the line. zshrs used to lex the word
/// as a glob and report `no matches found`. D04parameter.ztst chunk 193.
#[test]
fn subscript_only_lhs_is_not_an_identifier() {
    for (script, msg) in [
        ("[$key]=$val; echo after", "zsh:1: not an identifier: [$key]\n"),
        ("[a]=b echo hi", "zsh:1: not an identifier: [a]\n"),
        ("[a]+=x", "zsh:1: not an identifier: [a]\n"),
        ("[]=x", "zsh:1: not an identifier: []\n"),
        ("true; +=x; echo after", "zsh:1: not an identifier: \n"),
    ] {
        let (_ec, out, err) = run(script);
        assert_eq!(out, "", "{script:?}: the error must abort the line");
        assert_eq!(err.replacen("zshrs:", "zsh:", 1), msg, "{script:?}");
    }
    // Real identifiers with subscripts are untouched.
    let (_ec, out, _err) = run("a[2]=x; typeset -A h; h[k]=v; echo $a[2] $h[k]");
    assert_eq!(out, "x v\n");
    // As an argument (not command position) the word is still a glob.
    let (_ec, _out, err) = run("echo [a]=b");
    assert!(err.contains("no matches found: [a]=b"), "{err}");
}
