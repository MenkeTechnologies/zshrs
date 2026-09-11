//! End-to-end pins for `zbanner` and `zshrs --banner`
//! (`src/extensions/banner.rs`), driven through the real shell binary.
//!
//! The unit tests in `banner.rs` cover the formatting (box widths, plural
//! counts, daemon states). These cover the half that only exists once the
//! builtin is wired in: a literal and an indirect `zbanner` both reaching
//! it, its counts reading the shell's LIVE tables rather than a snapshot,
//! and the two refusals.
//!
//! stdout is a pipe here, so every run is colourless, and `-f` keeps the
//! developer's rc files out of the counts.

use std::collections::HashMap;
use std::process::Command;

fn zshrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_zshrs"))
}

/// Run `script` under `zshrs -f -c` → (stdout, stderr, exit code).
fn run(script: &str) -> (String, String, i32) {
    let out = zshrs()
        .args(["-f", "-c", script])
        .output()
        .expect("spawn zshrs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// The shell counts on every live line in `out`, keyed by plural noun
/// (`1 alias` and `2 aliases` both land under `aliases`).
fn live_counts(out: &str) -> Vec<HashMap<&'static str, usize>> {
    const NOUNS: [(&str, &str); 4] = [
        ("function", "functions"),
        ("alias", "aliases"),
        ("parameter", "parameters"),
        ("job", "jobs"),
    ];
    out.lines()
        .filter(|l| l.starts_with(" daemon "))
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let mut counts = HashMap::new();
            for pair in words.windows(2) {
                for (one, many) in NOUNS {
                    if pair[1] == one || pair[1] == many {
                        counts.insert(many, pair[0].parse().expect("a count precedes the noun"));
                    }
                }
            }
            counts
        })
        .collect()
}

#[test]
fn counts_follow_the_shell_they_are_drawn_in() {
    // Same process, before and after: a count that did not move would mean
    // the builtin read a snapshot, or some table other than the live one.
    let (out, err, code) = run("zbanner; f1() { :; }; f2() { :; }; alias zb_a=ls; zb_p=1; \
         sleep 30 & sleep 31 & zbanner; kill %1 %2");
    assert_eq!(code, 0, "stderr: {err}");
    let lines = live_counts(&out);
    assert_eq!(lines.len(), 2, "two banners expected: {out}");
    let (before, after) = (&lines[0], &lines[1]);
    assert_eq!(after["functions"], before["functions"] + 2);
    assert_eq!(after["aliases"], before["aliases"] + 1);
    assert_eq!(after["jobs"], before["jobs"] + 2);
    assert_eq!(after["parameters"], before["parameters"] + 1);
}

#[test]
fn disabled_functions_and_aliases_are_not_counted() {
    // `$functions` / `$aliases` leave out what `disable` turned off
    // (Src/Modules/parameter.c:470); the banner counts the same thing.
    let (out, err, code) =
        run("f1() { :; }; alias zb_a=ls; zbanner; disable -f f1; disable -a zb_a; zbanner");
    assert_eq!(code, 0, "stderr: {err}");
    let lines = live_counts(&out);
    assert_eq!(lines.len(), 2, "two banners expected: {out}");
    assert_eq!(lines[1]["functions"], lines[0]["functions"] - 1);
    assert_eq!(lines[1]["aliases"], lines[0]["aliases"] - 1);
}

#[test]
fn indirect_and_builtin_invocations_reach_it_too() {
    // `zbanner` has no fusevm opcode, so a literal name and a `$var` name
    // take different routes to the same dispatch arm.
    let (out, err, code) = run("x=zbanner; $x; builtin zbanner; whence -w zbanner");
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.matches(" ZSHRS // v").count(), 2, "{out}");
    assert!(out.trim_end().ends_with("zbanner: builtin"), "{out}");
}

#[test]
fn the_flag_prints_the_daemon_line_without_shell_counts() {
    let out = zshrs().arg("--banner").output().expect("spawn zshrs");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(concat!(" ZSHRS // v", env!("CARGO_PKG_VERSION"), " // ")));
    let lines = live_counts(&stdout);
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(
        lines[0].is_empty(),
        "no shell ran, so nothing to count: {stdout}"
    );
}

#[test]
fn an_argument_and_the_zsh_emulation_are_refused() {
    let (out, err, code) = run("zbanner now");
    assert_eq!(code, 2);
    assert!(out.is_empty(), "nothing is drawn on a usage error: {out}");
    assert_eq!(err.trim_end(), "zshrs: zbanner: bad argument: now");

    let out = zshrs()
        .args(["--zsh", "--banner"])
        .output()
        .expect("spawn zshrs");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}
