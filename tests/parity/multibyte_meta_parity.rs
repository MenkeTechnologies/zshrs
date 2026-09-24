//! Multibyte / metafication parity: a value holding a high byte or a
//! multibyte character must be measured, subscripted and scanned in
//! METAFIED CHARACTERS, the unit zsh's C source uses throughout
//! (`MB_METACHARLEN`, `MB_METASTRLEN`, `itype_end`).
//!
//! Expectations are the oracle's stdout from `/opt/homebrew/bin/zsh -f -c`
//! under `LC_ALL=en_US.UTF-8`, written as hex because several of them are
//! not valid UTF-8 (a lone `0xdc` byte) and a lossy `String` compare would
//! hide exactly the bytes under test. When a zsh is installed the pin is
//! also re-checked against it.

use std::path::{Path, PathBuf};
use std::process::Command;

const LOCALE: &str = "en_US.UTF-8";

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> Option<&'static str> {
    ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

fn run(cmd: &mut Command, script: &str) -> Vec<u8> {
    cmd.args(["-f", "-c", script])
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("ZSHRS_CACHE")
        .env("LC_ALL", LOCALE)
        .output()
        .expect("spawn shell")
        .stdout
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn assert_bytes(script: &str, expect_hex: &str) {
    if let Some(z) = zsh_path() {
        let got = run(&mut Command::new(z), script);
        assert_eq!(hex(&got), expect_hex, "pin no longer matches zsh\n  script: {script}");
    }
    let mut c = Command::new(zshrs_bin());
    c.arg("--zsh");
    let got = run(&mut c, script);
    assert_eq!(hex(&got), expect_hex, "zshrs diverged from zsh\n  script: {script}");
}

/// `$#c` inside `$(( ))` counts characters with `MB_METASTRLEN`
/// (c:Src/subst.c:4490 singsub → paramsubst's length arm), so the invalid
/// byte `0xdc` — stored as a `Meta` pair — is ONE character, not two.
#[test]
fn arith_length_of_invalid_high_byte_is_one() {
    let s = r#"c=$'\M-\\'; print $(( $#c )) $[ $#c ]; (( n = $#c )); print $n"#;
    assert_bytes(s, "3120310a310a");
}

/// c:Src/params.c:1634-1663 — a scalar subscript walks `MB_METACHARLEN`
/// units. The unbraced `$c[$i]` / `$c[1,$i]` arm and the nested
/// `${${c}[$i]}` arm split the METAFIED text instead and returned the bare
/// `Meta` byte (`c2 83`).
#[test]
fn dynamic_subscript_keeps_meta_pair_whole() {
    let s = r#"c=$'\M-\\'; i=1; print -r -- $c[$i] $c[1,$i] ${${c}[$i]} ${${c}[1,$i]}"#;
    assert_bytes(s, "dc20dc20dc20dc0a");
}

/// A03quoting "$'-style quote with metafied backslash": both halves of
/// the loop — the `$#chars` bound and `$chars[$i]` — must see seven units.
#[test]
fn metafied_backslash_loop_matches_a03() {
    let s = r#"chars=$(print -r $'BS\\MBS\M-\\'); for (( i = 1; i <= $#chars; i++ )); do char=$chars[$i]; print -n $(( [#16] #char )) ""; done"#;
    assert_bytes(
        s,
        "313623343220313623353320313623354320313623344420313623343220313623353320313623444320",
    );
}

/// With MULTIBYTE unset a unit is one BYTE (c:Src/utils.c:5613), so the
/// second unit of `héllo` is the lead byte `0xc3` on both the unbraced and
/// the nested subscript arms.
#[test]
fn nomultibyte_dynamic_subscript_is_one_byte() {
    let s = r#"s=héllo; i=2; print -r -- $s[$i] $s[2,$i+2] ${${s}[$i]} ${${s}[-1]}; unsetopt multibyte; print -r -- $s[$i] ${${s}[$i]}"#;
    assert_bytes(s, "c3a920c3a96c6c20c3a9206f0ac320c30a");
}

