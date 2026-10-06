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
