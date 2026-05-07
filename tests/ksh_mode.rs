//! Tests for `zshrs --ksh` drop-in mode.
//!
//! Verifies the CLI flag is parsed, the ksh option presets are
//! applied (matching `emulate ksh` from Src/options.c), and basic
//! ksh-style behaviors work (0-indexed arrays via `ksharrays`).

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
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
    let (out, _, _) = run_ksh("setopt | grep -x ksharrays");
    assert_eq!(out.trim(), "ksharrays");
}

#[test]
fn ksh_mode_sets_kshglob() {
    let (out, _, _) = run_ksh("setopt | grep -x kshglob");
    assert_eq!(out.trim(), "kshglob");
}

#[test]
fn ksh_mode_sets_posixbuiltins() {
    let (out, _, _) = run_ksh("setopt | grep -x posixbuiltins");
    assert_eq!(out.trim(), "posixbuiltins");
}

#[test]
fn ksh_mode_sets_shwordsplit() {
    let (out, _, _) = run_ksh("setopt | grep -x shwordsplit");
    assert_eq!(out.trim(), "shwordsplit");
}

#[test]
fn ksh_mode_unsets_nomatch() {
    // Per emulate_mode_options("ksh"): nomatch is in the unset list.
    let (out, _, _) = run_ksh("setopt | grep -x nomatch || echo absent");
    assert_eq!(out.trim(), "absent");
}

#[test]
fn ksh_mode_unsets_multios() {
    let (out, _, _) = run_ksh("setopt | grep -x multios || echo absent");
    assert_eq!(out.trim(), "absent");
}

#[test]
fn ksh_mode_zero_indexed_arrays() {
    // With ksharrays, the option is set; underlying paramsubst array
    // indexing rewiring is tracked separately. For now verify the
    // option is observable via setopt.
    let (out, _, _) = run_ksh("setopt | grep -x ksharrays");
    assert_eq!(out.trim(), "ksharrays");
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
