//! Tests for `zshrs --ksh` drop-in mode.
//!
//! Verifies the CLI flag is parsed, the ksh option presets are
//! applied (matching `emulate ksh` from Src/options.c), and basic
//! ksh-style behaviors work (0-indexed arrays via `ksharrays`).

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Assert an option's state under `--ksh` the way the shell itself reports it.
///
/// These tests used to grep `setopt`'s listing. That cannot work: `emulate ksh`
/// sets KSH_OPTION_PRINT, which switches `setopt` from bare names to a
/// two-column `name<pad>on|off` table — real zsh does exactly the same, and
/// zshrs matches it byte-for-byte:
///
/// ```text
/// $ zsh   -fc 'emulate ksh; setopt' | head -1   ->  noaliases             off
/// $ zshrs --ksh -c 'setopt'         | head -1   ->  noaliases             off
/// ```
///
/// So `setopt | grep -x ksharrays` could never match, and the five "sets"
/// tests failed against correct behaviour. Worse, the two "unsets" tests
/// PASSED for the wrong reason — grep matched nothing for any option, so
/// `|| echo absent` fired regardless of the real state and they asserted
/// nothing at all.
///
/// `[[ -o NAME ]]` reads the option state itself, so it is independent of the
/// listing format and strictly stronger than the grep it replaces. Every
/// expectation below was verified against `zsh -fc 'emulate ksh; [[ -o X ]]'`.
fn ksh_opt(name: &str) -> String {
    let (out, _, _) = run_ksh(&format!("[[ -o {name} ]] && echo on || echo off"));
    out.trim().to_string()
}

fn run_ksh(script: &str) -> (String, String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["--ksh", "-c", script])
        .output()
        .expect("zshrs --ksh failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn ksh_mode_sets_ksharrays() {
    // ksharrays makes arrays 0-indexed (vs zsh's default 1).
    assert_eq!(ksh_opt("ksharrays"), "on", "--ksh must set ksharrays");
}

#[test]
fn ksh_mode_sets_kshglob() {
    assert_eq!(ksh_opt("kshglob"), "on", "--ksh must set kshglob");
}

#[test]
fn ksh_mode_sets_posixbuiltins() {
    assert_eq!(ksh_opt("posixbuiltins"), "on", "--ksh must set posixbuiltins");
}

#[test]
fn ksh_mode_sets_shwordsplit() {
    assert_eq!(ksh_opt("shwordsplit"), "on", "--ksh must set shwordsplit");
}

#[test]
fn ksh_mode_unsets_nomatch() {
    // Per emulate_mode_options("ksh"): nomatch is in the unset list.
    assert_eq!(ksh_opt("nomatch"), "off", "--ksh must unset nomatch");
}

#[test]
fn ksh_mode_unsets_multios() {
    assert_eq!(ksh_opt("multios"), "off", "--ksh must unset multios");
}

#[test]
fn ksh_mode_zero_indexed_arrays() {
    // Now a REAL behavioural check, not just the option bit. Under
    // KSH_ARRAYS `${a[0]}` is the first element, where zsh's default is
    // 1-based. Verified against `zsh -fc 'emulate ksh; ...'`:
    //   ${a[0]} -> x    ${a[1]} -> y    ${#a[@]} -> 3
    assert_eq!(ksh_opt("ksharrays"), "on");
    let (out, _, _) = run_ksh(r#"a=(x y z); print -r -- "${a[0]}|${a[1]}|${#a[@]}""#);
    assert_eq!(
        out.trim(),
        "x|y|3",
        "ksharrays must make subscript 0 the first element"
    );
}

#[test]
fn ksh_mode_help_lists_flag() {
    let out = Command::new(zshrs_bin())
        .arg("--help")
        .output()
        .expect("zshrs --help failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--ksh"),
        "--help output missing --ksh flag:\n{}",
        stdout
    );
}
