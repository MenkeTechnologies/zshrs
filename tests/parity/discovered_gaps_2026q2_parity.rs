//! Parity gaps discovered in a focused `zshrs --zsh -fc` vs reference
//! `/opt/homebrew/bin/zsh -fc` divergence-mining session (2026-Q2).
//!
//! Two areas surfaced concrete, reproducible divergences:
//!
//!   1. **Multibyte from `$'\x..'` / `\M-` / octal escapes.** Bytes injected
//!      via `$'...'` numeric escapes are not recombined into multibyte
//!      characters for character-counting (`${#x}`), case-modification
//!      (`${(U)x}`), char-indexed substring (`${x: -n}`), single-char
//!      pattern match (`?`), and char-splitting (`${(s::)x}`). Literal
//!      UTF-8 written directly in the source (`$'é'`) works — only the
//!      numeric-escape path diverges.
//!   2. **`(t)` type flag on non-parameter / array-valued operands.** When
//!      `(t)` is applied to a command substitution or an array-valued
//!      nested expansion (not a bare parameter name), zsh yields the value
//!      unchanged; zshrs reports a synthesized type string instead.
//!   3. **Unquoted command-substitution `$*`/`${arr[*]}` join.** With a
//!      non-default IFS set *inside* an unquoted `$(...)`, the join uses the
//!      outer shell's IFS instead of the inner one. Quoted/assigned forms
//!      (`x=$(...)`) are correct.
//!   4. **`(I:n:)` substitution match index.** `${(SI:2:)s/a/X}` replaces
//!      the 1st match in zshrs, the 2nd in zsh.
//!
//! Each `#[ignore]`d test carries a `zshrs bug:` note and the observed
//! divergence. They still call `assert_parity`, so once a bug is fixed the
//! test passes and the `#[ignore]` can be dropped (turning it into a
//! regression pin). Green tests in this file are coverage that already
//! holds — added so the working sub-cases stay pinned.

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
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
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
// 1. Multibyte characters from `$'\x..'` / octal / `\M-` numeric escapes
// ─────────────────────────────────────────────────────────────────────
mod multibyte_numeric_escapes {
    use super::*;

    /// Literal UTF-8 in the source counts as one character — this path
    /// already works; pin it so a fix to the escape path can't regress it.
    #[test]
    fn literal_utf8_in_source_len_one() {
        assert_parity(r#"x=$'é'; echo ${#x}"#);
    }

    /// ASCII hex escapes decode correctly — pin.
    #[test]
    fn ascii_hex_escapes_decode() {
        assert_parity(r#"echo $'\x41\x42'"#);
    }

    /// The raw bytes round-trip on output — pin (echo demetafies fine).
    #[test]
    fn escaped_utf8_bytes_echo_roundtrip() {
        assert_parity(r#"x=$'caf\xc3\xa9'; echo $x"#);
    }

    /// `${#x}` over `$'\xc3\xa9'` (UTF-8 é) — FIXED. `mb_metastrlenend`
    /// (utils.c:5655) is now a faithful port (demetafy → mbrtowc-count)
    /// instead of `chars().count()`, and the `${#}` scalar arm routes
    /// through it (c:3879 `MB_METASTRLEN2`), so the metafied é counts 1.
    #[test]
    fn hex_escape_multibyte_char_length() {
        assert_parity(r#"x=$'\xc3\xa9'; echo ${#x}"#);
    }

    /// Two-character CJK string `$'\xe4\xb8\xad\xe6\x96\x87'` — FIXED
    /// by the same `mb_metastrlenend` port (zsh: 2).
    #[test]
    fn hex_escape_multibyte_two_char_length() {
        assert_parity(r#"s=$'\xe4\xb8\xad\xe6\x96\x87'; echo ${#s}"#);
    }

    /// `\M-` meta escape — FIXED. The lone invalid byte 0xe9 demetafies
    /// to one byte that `mbrtowc` treats as a single character (c:5712).
    #[test]
    fn meta_escape_char_length() {
        assert_parity(r#"x=$'\M-i'; echo ${#x}"#);
    }

    /// `${(U)x}` uppercase over escaped é — zsh: É, zshrs: mojibake.
    /// The case-mapping runs over metafied bytes and corrupts them.
    /// `${(U)x}` uppercase over escaped é — FIXED. The `(U)`/`(L)`/`(C)`
    /// case transform now demetafies to the logical-char form first
    /// (subst.c:3937 casemodify is `MB_METACHARLENCONV`-based), so the
    /// metafied é uppercases to É instead of mangling its bytes.
    #[test]
    fn case_modify_escaped_multibyte() {
        assert_parity(r#"x=$'\xc3\xa9'; echo ${(U)x}"#);
    }

    /// Char-indexed substring `${x: -1}` over `$'caf\xc3\xa9'` — FIXED.
    /// The `${x:off:len}` substring arm demetafies before counting/
    /// slicing characters (subst.c:3766 MB_METACHARLEN), so `-1` lands
    /// on the trailing é.
    #[test]
    fn negative_substring_escaped_multibyte() {
        assert_parity(r#"x=$'caf\xc3\xa9'; echo ${x: -1}"#);
    }

    /// Single-char glob `?` against escaped é — FIXED. P_ANY now
    /// advances one metafied character (METACHARINC), so `?` matches
    /// the whole é.
    #[test]
    fn single_char_pattern_escaped_multibyte() {
        assert_parity(r#"x=$'\xc3\xa9'; [[ $x == ? ]] && echo onechar || echo multi"#);
    }

    /// Char-split `${(s::)x}` over escaped é — FIXED. The empty-separator
    /// split demetafies before iterating characters (subst.c:581
    /// MB_METACHARLENCONV), so a metafied multibyte char is one element.
    #[test]
    fn char_split_escaped_multibyte() {
        assert_parity(r#"x=$'\xc3\xa9'; arr=( ${(s::)x} ); echo ${#arr}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// 2. `(t)` type flag on non-parameter operands
// ─────────────────────────────────────────────────────────────────────
mod type_flag_non_parameter {
    use super::*;

    /// `(t)` on a bare scalar parameter — works. Pin.
    #[test]
    fn t_flag_scalar_parameter() {
        assert_parity(r#"x=hi; echo ${(t)x}"#);
    }

    /// `(t)` on a bare array parameter — works. Pin.
    #[test]
    fn t_flag_array_parameter() {
        assert_parity(r#"a=(1 2); echo ${(t)a}"#);
    }

    /// `(t)` on an unquoted command substitution — zsh yields the value
    /// (`x`, no parameter to introspect), zshrs reports `array`.
    #[test]
    #[ignore = "zshrs bug: ${(t)$(echo x)} reports 'array' instead of yielding the value 'x'"]
    fn t_flag_command_substitution() {
        assert_parity(r#"echo ${(t)$(echo x)}"#);
    }

    /// `(t)` on an array-valued nested expansion — zsh yields the value
    /// (`1 2`), zshrs reports `array`.
    #[test]
    #[ignore = "zshrs bug: ${(t)${a}} on an array yields 'array' instead of the value '1 2'"]
    fn t_flag_array_valued_nested() {
        assert_parity(r#"a=(1 2); echo ${(t)${a}}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// 3. Unquoted command-substitution `$*` / `${arr[*]}` IFS join
// ─────────────────────────────────────────────────────────────────────
mod cmdsubst_ifs_join {
    use super::*;

    /// Assigned command substitution honours the inner IFS — works. Pin.
    #[test]
    fn assigned_cmdsubst_star_join() {
        assert_parity(r#"x=$(IFS=:; set -- a b c; echo "$*"); echo $x"#);
    }

    /// Bare subshell honours the inner IFS — works. Pin.
    #[test]
    fn subshell_star_join() {
        assert_parity(r#"(IFS=:; set -- a b c; echo "$*")"#);
    }

    /// Unquoted command substitution in command-argument position — zsh
    /// joins `$*` with the inner IFS (`a:b:c`), zshrs uses the outer
    /// IFS (`a b c`).
    #[test]
    #[ignore = "zshrs bug: echo $(IFS=:; set -- a b c; echo \"$*\") joins with outer IFS (space) not inner (:)"]
    fn unquoted_cmdsubst_positional_star_join() {
        assert_parity(r#"echo $(IFS=:; set -- a b c; echo "$*")"#);
    }

    /// Same divergence via `${arr[*]}`.
    #[test]
    #[ignore = "zshrs bug: echo $(IFS=:; arr=(a b c); echo \"${arr[*]}\") joins with outer IFS not inner"]
    fn unquoted_cmdsubst_array_star_join() {
        assert_parity(r#"echo $(IFS=:; arr=(a b c); echo "${arr[*]}")"#);
    }

    /// Same divergence via backticks.
    #[test]
    #[ignore = "zshrs bug: backtick cmdsubst `IFS=:; set -- a b c; echo \"$*\"` joins with outer IFS not inner"]
    fn unquoted_backtick_star_join() {
        assert_parity(r#"echo `IFS=:; set -- a b c; echo "$*"`"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// 4. `(I:n:)` substitution match index
// ─────────────────────────────────────────────────────────────────────
mod subst_match_index {
    use super::*;

    /// `${(SI:2:)s/a/X}` — replace the *2nd* match. FIXED: the substr
    /// replace loop now threads `flnum` (the `(I:N:)` index), counting one
    /// match per leftmost start and replacing only the Nth (glob.c:3057
    /// `if (!--n …)`, default 1 per c:3095-3096). zsh: `aXa`.
    #[test]
    fn search_index_two_replaces_second_match() {
        assert_parity(r#"s=aaa; echo ${(SI:2:)s/a/X}"#);
    }
}
