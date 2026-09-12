//! `strmetasort` unmetafy parity: `${(o)}` and friends must collate the
//! BYTES an element stands for, not the Meta encoding's spelling.
//!
//! `Src/sort.c:293-315` builds each element's compare key by unmetafying
//! the original — `while ((*t = *metaptr++)) { if (*t++ == Meta) t[-1] =
//! *metaptr++ ^ 32; }` — so `strcoll` at c:134 sees the raw bytes. The
//! Rust port used to skip that step and hand `strcoll` the metafied form,
//! where the byte `0xe9` is spelled `\u{83}\u{c9}`. A UTF-8 locale then
//! collated the payload `\u{c9}` as É and `\u{c8}` as È — accented Latin
//! letters whose relative order is the REVERSE of the bytes 0xe9/0xe8
//! they encode — so `${(o)}` over `$'a\xe9' $'a\xe8'` came back backwards.
//!
//! The tell that the operands were being read as text: the same input was
//! ordered correctly under `LC_ALL=C`, where `strcoll` degenerates to a
//! byte compare and the Meta payload's letter identity stops mattering.
//! Both locales are therefore pinned below, and they legitimately give
//! DIFFERENT answers for some inputs (`b \xe9 a \xe8 Z` puts `Z` first
//! under C and third under en_US.UTF-8).
//!
//! Every expectation is the oracle's own stdout, captured byte-for-byte
//! from `/opt/homebrew/bin/zsh -f -c` and written here as hex, because
//! these strings are not valid UTF-8 and a lossy `String` comparison
//! would mask exactly the bytes under test.

#![allow(non_snake_case)]

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

/// Hex → bytes. The expectations are written as hex so a non-UTF-8 byte
/// survives the source file intact.
fn unhex(h: &str) -> Vec<u8> {
    assert!(h.len() % 2 == 0, "hex literal has an odd length: {h}");
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn run_zsh(locale: &str, script: &str) -> Vec<u8> {
    let o = Command::new(zsh_path())
        .args(["-f", "-c", script])
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env("LC_ALL", locale)
        .output()
        .expect("zsh");
    o.stdout
}

fn run_zshrs(locale: &str, script: &str) -> Vec<u8> {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env("LC_ALL", locale)
        .output()
        .expect("zshrs");
    o.stdout
}

/// Assert zshrs reproduces the pinned oracle bytes, and — when a real zsh
/// is installed — that the pin still matches what that zsh emits today.
fn assert_sort_bytes(locale: &str, script: &str, expect_hex: &str) {
    let want = unhex(expect_hex);
    if zsh_available() {
        let z = run_zsh(locale, script);
        assert_eq!(
            hex(&z),
            hex(&want),
            "pinned bytes no longer match the oracle\n  LC_ALL={locale}\n  script: {script}"
        );
    }
    let got = run_zshrs(locale, script);
    assert_eq!(
        hex(&got),
        hex(&want),
        "zshrs diverged from zsh\n  LC_ALL={locale}\n  script: {script}"
    );
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The original report: two elements differing only in a high byte, where
/// the Meta payloads spell É and È and collate the opposite way round.
#[test]
fn metasort_high_byte_pair_orders_by_byte_not_by_meta_payload() {
    let s = r#"msA=($'a\xe9' $'a\xe8'); print -r -- ${(o)msA}"#;
    // a\xe8 a\xe9
    assert_sort_bytes("en_US.UTF-8", s, "61e82061e90a");
    assert_sort_bytes("C", s, "61e82061e90a");
}

/// `(O)` is the same comparator with `sortdir = -1` (c:402), so it has to
/// flip with it rather than independently.
#[test]
fn metasort_high_byte_pair_reverse() {
    let s = r#"msA=($'a\xe9' $'a\xe8'); print -r -- ${(O)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "61e92061e80a");
    assert_sort_bytes("C", s, "61e92061e80a");
}

/// High bytes mixed with ASCII. The two locales disagree here and BOTH
/// answers are zsh's: under `C` the raw byte order puts `Z` (0x5a) first,
/// under en_US.UTF-8 `strcoll` sorts the letters a, b, Z before the two
/// undecodable bytes.
#[test]
fn metasort_high_bytes_mixed_with_ascii() {
    let s = r#"msA=(b $'\xe9' a $'\xe8' Z); print -r -- ${(o)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "612062205a20e820e90a");
    assert_sort_bytes("C", s, "5a2061206220e820e90a");
}

/// `(i)` folds case in the prep pass (c:328-373) — which C runs on the
/// UNMETAFIED bytes, so the fold and the unmetafy have to compose.
#[test]
fn metasort_ignoring_case_over_high_bytes() {
    let s = r#"msA=($'A\xe9' $'a\xe8' B a); print -r -- ${(oi)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "612061e82041e920420a");
    assert_sort_bytes("C", s, "612061e82041e920420a");
}

/// 0x80 and 0xff bracket the Meta-escaped range from both ends.
#[test]
fn metasort_full_high_byte_range() {
    let s = r#"msA=($'a\xe9' $'a\xe8' $'a\xff' $'a\x80' a b); print -r -- ${(o)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "612061802061e82061e92061ff20620a");
    assert_sort_bytes("C", s, "612061802061e82061e92061ff20620a");
}

/// Associative-array KEYS reach the same comparator via `${(ko)}`.
#[test]
fn metasort_assoc_keys_with_high_bytes() {
    let s = r#"typeset -A msH=($'k\xe9' 1 $'k\xe8' 2); print -r -- ${(ko)msH}"#;
    assert_sort_bytes("en_US.UTF-8", s, "6be8206be90a");
    assert_sort_bytes("C", s, "6be8206be90a");
}

/// …and associative-array VALUES via `${(o)}`.
#[test]
fn metasort_assoc_values_with_high_bytes() {
    let s = r#"typeset -A msH=(k1 $'a\xe9' k2 $'a\xe8' k3 z); print -r -- ${(o)msH}"#;
    assert_sort_bytes("en_US.UTF-8", s, "61e82061e9207a0a");
    assert_sort_bytes("C", s, "61e82061e9207a0a");
}

/// `print -o` is the `unmetalenp != NULL` caller (c:builtin.c:4792). C
/// unmetafies its arguments at c:builtin.c:4736 BEFORE that call; zshrs
/// defers the decode until after the sort, so the prep loop has to do it.
#[test]
fn print_o_sorts_high_bytes_by_byte() {
    let s = r#"print -o $'a\xe9' $'a\xe8' a"#;
    assert_sort_bytes("en_US.UTF-8", s, "612061e82061e90a");
    assert_sort_bytes("C", s, "612061e82061e90a");
}

#[test]
fn print_O_sorts_high_bytes_by_byte() {
    let s = r#"print -O $'a\xe9' $'a\xe8' a"#;
    assert_sort_bytes("en_US.UTF-8", s, "61e92061e820610a");
    assert_sort_bytes("C", s, "61e92061e820610a");
}

/// No-regression guards: pure ASCII (locale-sensitive, and the two
/// answers differ), numeric `(n)`, and the embedded-NUL path that depends
/// on `sortelt.len` being set from the DECODED bytes.
#[test]
fn metasort_plain_ascii_unchanged() {
    let s = r#"msA=(foo Bar BAZ qux Zeta apple); print -r -- ${(o)msA}"#;
    assert_sort_bytes(
        "en_US.UTF-8",
        s,
        "6170706c65204261722042415a20666f6f20717578205a6574610a",
    );
    assert_sort_bytes("C", s, "42415a20426172205a657461206170706c6520666f6f207175780a");
}

#[test]
fn metasort_numeric_unchanged() {
    let s = r#"msA=(10 9 2 100 1); print -r -- ${(on)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "3120322039203130203130300a");
    assert_sort_bytes("C", s, "3120322039203130203130300a");
}

#[test]
fn metasort_embedded_nul_unchanged() {
    let s = r#"msA=($'a\0c' $'a\0b' $'a'); print -rl -- ${(o)msA}"#;
    assert_sort_bytes("en_US.UTF-8", s, "610a6100620a6100630a");
    assert_sort_bytes("C", s, "610a6100620a6100630a");
}
