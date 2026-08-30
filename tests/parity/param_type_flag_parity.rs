//! `${(t)…}` parity — the type-flag, including on SUBSCRIPTED values.
//!
//! `(t)` reports a parameter's type as `scalar` / `array` /
//! `association`, plus its attribute suffixes. Completion functions and
//! plugins branch on it constantly (`[[ ${(t)opt} == array ]]`), so the
//! answer for a value that is NOT a whole parameter — an element picked
//! out with a subscript — matters as much as the answer for the
//! parameter itself.
//!
//! zsh's answer for an association element is EMPTY: `$h[a]` is a value,
//! not a parameter, so it has no type of its own. Array elements are
//! their own case and both shells already agree on them, which is what
//! makes the association behaviour a bug rather than a design choice.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

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

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-f", "-c", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let zo = String::from_utf8_lossy(&z.stdout).into_owned();
    let ro = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        zo, ro,
        "stdout divergence on:\n{script}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&z.stderr),
        String::from_utf8_lossy(&r.stderr),
        "stderr divergence on:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Whole parameters, and array subscripts — already in agreement
// ═══════════════════════════════════════════════════════════════════════

/// The plain forms every `[[ ${(t)x} == array ]]` test in a completer
/// depends on.
#[test]
fn whole_parameters_report_their_type() {
    assert_parity(
        r#"typeset s=v; typeset -a arr=(x y); typeset -A h=(a 1)
print "[${(t)s}]" "[${(t)arr}]" "[${(t)h}]""#,
    );
}

/// An unset name has no type at all.
#[test]
fn an_unset_name_has_no_type() {
    assert_parity(r#"print "[${(t)nosuchvar_zzz}]""#);
}

/// Array subscripts: an in-range element reports its own type, an
/// out-of-range one reports nothing. Both shells agree here, which is
/// the control for the association cases below.
#[test]
fn array_subscripts_report_the_element_type() {
    assert_parity(r#"typeset -a arr=(x y); print "[${(t)arr[1]}]" "[${(t)arr[9]}]""#);
}

/// Attribute suffixes ride along on the type string.
#[test]
fn attributes_are_appended_to_the_type() {
    assert_parity(
        r#"typeset -i n=1; typeset -r ro=v; typeset -U -a u=(a a b)
print "[${(t)n}]" "[${(t)ro}]" "[${(t)u}]""#,
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Association subscripts — zshrs answers for the whole parameter
// ═══════════════════════════════════════════════════════════════════════

/// zshrs gap: `${(t)assoc[key]}` reports the type of the WHOLE
/// association instead of nothing.
///
///     typeset -A h=(a 1); print "[${(t)h[a]}]" "[${(t)h[nope]}]"
///     zsh     [] []
///     zshrs   [association] [association]
///
/// An association element is a value, not a parameter, so it has no
/// type of its own — which is exactly how the array case above already
/// behaves in both shells for an OUT-OF-RANGE index. Anything branching
/// on `${(t)h[k]}` to decide "is this an association?" gets the wrong
/// answer, and gets it for a key that does not even exist.
#[test]
#[ignore = "zshrs gap: ${(t)assoc[key]} reports the whole association's type"]
fn an_association_element_has_no_type_of_its_own() {
    assert_parity(r#"typeset -A h=(a 1); print "[${(t)h[a]}]" "[${(t)h[nope]}]""#);
}

/// Same gap through a magic hash, where the wrong answer is louder:
/// the full attribute string of the special parameter comes back for
/// what should be an empty result.
///
///     zsh     []
///     zshrs   [association-hide-hideval-special]
#[test]
#[ignore = "zshrs gap: ${(t)assoc[key]} reports the whole association's type"]
fn a_magic_hash_element_has_no_type_of_its_own() {
    assert_parity(
        r#"zmodload zsh/parameter
f(){ :; }
print "[${(t)functions[f]}]" "[${(t)functions[nosuch_zzz]}]""#,
    );
}

/// And through `$commands`, the one a completer is most likely to
/// probe.
#[test]
#[ignore = "zshrs gap: ${(t)assoc[key]} reports the whole association's type"]
fn a_commands_element_has_no_type_of_its_own() {
    assert_parity(
        r#"zmodload zsh/parameter
print "[${(t)commands[ls]}]" "[${(t)commands[nosuchcmd_zzz]}]""#,
    );
}
