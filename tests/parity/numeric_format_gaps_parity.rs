//! Numeric-formatting parity gaps found in a 2026-Q2 `zshrs --zsh -fc`
//! vs `/opt/homebrew/bin/zsh -fc` divergence-mining session, focused on
//! `printf` integer conversions and arithmetic integer-literal overflow.
//!
//! Each `#[ignore]`d test carries a `zshrs bug:` note plus the observed
//! divergence. They still call `assert_parity`, so a future fix flips the
//! test green and the `#[ignore]` can be dropped (turning it into a
//! regression pin). Green tests here are coverage that already holds.
//!
//! NOTE: `printf_d_int64_min_panics` documents a *crash* (the zshrs
//! process panics with "attempt to negate with overflow"), not just an
//! output mismatch — the most severe item in this file.

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

struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path()).args(["-fc", s]).output().expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{s}");
}

// ─────────────────────────────────────────────────────────────────────
// printf — `%u` unsigned conversion
// ─────────────────────────────────────────────────────────────────────
mod printf_unsigned {
    use super::*;

    /// `printf %u` on a non-negative value works — pin.
    #[test]
    fn u_positive() {
        assert_parity(r#"printf '%u\n' 42"#);
    }

    /// `printf %u -1` — zsh reinterprets as the unsigned 64-bit value
    /// `18446744073709551615`; zshrs prints `-1`.
    #[test]
    #[ignore = "zshrs bug: printf %u of a negative value prints it signed (-1) instead of the unsigned wrap (18446744073709551615)"]
    fn u_negative_one_wraps_unsigned() {
        assert_parity(r#"printf '%u\n' -1"#);
    }

    /// `printf %u -42` — same unsigned-wrap divergence.
    #[test]
    #[ignore = "zshrs bug: printf %u of -42 prints -42 instead of the unsigned wrap (18446744073709551574)"]
    fn u_negative_wraps_unsigned() {
        assert_parity(r#"printf '%u\n' -42"#);
    }

    /// `printf %+u` — `+` flag is meaningless for unsigned; zsh drops it
    /// (`5`), zshrs emits `+5`.
    #[test]
    #[ignore = "zshrs bug: printf %+u honors the + flag (+5); zsh ignores it for unsigned (5)"]
    fn u_plus_flag_ignored() {
        assert_parity(r#"printf '%+u\n' 5"#);
    }

    /// `printf '% u'` — space flag is likewise meaningless for unsigned;
    /// zsh drops it (`5`), zshrs emits ` 5`.
    #[test]
    #[ignore = "zshrs bug: printf '% u' honors the space flag ( 5); zsh ignores it for unsigned (5)"]
    fn u_space_flag_ignored() {
        assert_parity(r#"printf '% u\n' 5"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// printf — length modifiers (`l`, `h`) and `%c`
// ─────────────────────────────────────────────────────────────────────
mod printf_directives {
    use super::*;

    /// `%ld` — zsh accepts (and ignores) the `l` length modifier and
    /// prints `42`; zshrs rejects it with "invalid directive" (exit 1).
    #[test]
    #[ignore = "zshrs bug: printf rejects the 'l' length modifier (%ld → '%l: invalid directive'); zsh accepts/ignores it"]
    fn length_modifier_l() {
        assert_parity(r#"printf '%ld\n' 42"#);
    }

    /// `%lu` — same: zsh accepts the `l` modifier, zshrs rejects.
    #[test]
    #[ignore = "zshrs bug: printf rejects '%lu' length modifier; zsh accepts/ignores it"]
    fn length_modifier_lu() {
        assert_parity(r#"printf '%lu\n' 42"#);
    }

    /// `%hd` — same for the `h` length modifier.
    #[test]
    #[ignore = "zshrs bug: printf rejects the 'h' length modifier (%hd); zsh accepts/ignores it"]
    fn length_modifier_h() {
        assert_parity(r#"printf '%hd\n' 42"#);
    }

    /// `%lld` is rejected by BOTH (zsh only allows a single length
    /// modifier) — pin that both error.
    #[test]
    fn double_length_modifier_rejected_both() {
        assert_parity(r#"printf '%lld\n' 42"#);
    }

    /// `printf '%c' ''` — empty argument: zsh emits a single space,
    /// zshrs emits nothing.
    #[test]
    #[ignore = "zshrs bug: printf %c of an empty string emits nothing; zsh emits a space"]
    fn c_empty_argument() {
        assert_parity(r#"printf '%c\n' ''"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// printf — 64-bit integer overflow / truncation
// ─────────────────────────────────────────────────────────────────────
mod printf_int_overflow {
    use super::*;

    /// `printf %d -9223372036854775808` (i64::MIN as a 19-digit literal)
    /// — zsh truncates the literal after 18 digits and prints
    /// `-922337203685477580`. zshrs **PANICS** ("attempt to negate with
    /// overflow", builtin.rs:15556) and exits 101. This is a CRASH, the
    /// most severe gap in this file.
    #[test]
    #[ignore = "zshrs bug: printf %d of -9223372036854775808 PANICS (negate overflow, exit 101); zsh truncates to 18 digits"]
    fn d_int64_min_panics() {
        assert_parity(r#"printf '%d\n' -9223372036854775808"#);
    }

    /// `printf %d 9223372036854775808` (19-digit positive) — zsh
    /// truncates after 18 digits (`922337203685477580`); zshrs wraps to
    /// `-9223372036854775808`.
    #[test]
    #[ignore = "zshrs bug: printf %d of a 19-digit literal wraps to negative instead of zsh's 18-digit truncation"]
    fn d_19_digit_truncation() {
        assert_parity(r#"printf '%d\n' 9223372036854775808"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// arithmetic — integer-literal overflow
// ─────────────────────────────────────────────────────────────────────
mod arith_literal_overflow {
    use super::*;

    /// `$(( 0xFFFFFFFFFFFFFFFF ))` (16 hex digits) — zsh truncates the
    /// literal after 15 hex digits and yields `1152921504606846975`;
    /// zshrs yields `0`.
    #[test]
    #[ignore = "zshrs bug: $(( 0xFFFFFFFFFFFFFFFF )) yields 0 instead of zsh's 15-hex-digit truncation (1152921504606846975)"]
    fn hex_literal_16_digits_truncation() {
        assert_parity(r#"echo $(( 0xFFFFFFFFFFFFFFFF ))"#);
    }

    /// In-range 64-bit hex literal works — pin.
    #[test]
    fn hex_literal_in_range() {
        assert_parity(r#"echo $(( 0x7FFFFFFFFFFFFFFF ))"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// zsh/mathfunc — function set parity
// ─────────────────────────────────────────────────────────────────────
mod mathfunc_set {
    use super::*;

    /// `sqrt` and the common math functions are present — pin.
    #[test]
    fn sqrt_present() {
        assert_parity(r#"zmodload zsh/mathfunc; printf '%g\n' $(( sqrt(16) ))"#);
    }

    /// `rint(x)` (round to nearest, ties-to-even) is a zsh/mathfunc
    /// function — `rint(2.5)`→2, `rint(3.5)`→4. zshrs reports
    /// "unknown function: rint".
    #[test]
    #[ignore = "zshrs bug: zsh/mathfunc is missing rint() (round-half-to-even); zsh has it"]
    fn rint_missing() {
        assert_parity(r#"zmodload zsh/mathfunc; printf '%g\n' $(( rint(2.5) ))"#);
    }

    /// `trunc(x)` is NOT a zsh/mathfunc function — zsh reports
    /// "unknown function: trunc". zshrs added it (returns 2), so it
    /// accepts a name zsh rejects.
    #[test]
    #[ignore = "zshrs bug: zsh/mathfunc accepts trunc() (returns 2); zsh has no such function and errors"]
    fn trunc_extra() {
        assert_parity(r#"zmodload zsh/mathfunc; echo $(( trunc(2.7) ))"#);
    }
}
