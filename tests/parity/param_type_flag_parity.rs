//! `${(t)…}` parity — the type-flag, including on SUBSCRIPTED values.
//!
//! `(t)` reports a parameter's type as `scalar` / `array` /
//! `association`, plus its attribute suffixes. Completion functions and
//! plugins branch on it constantly (`[[ ${(t)opt} == array ]]`), so the
//! answer for a value that is NOT a whole parameter — an element picked
//! out with a subscript — matters as much as the answer for the
//! parameter itself.
//!
//! The subscript does not go to the parameter at all. `Src/subst.c:2803`
//! calls `fetchvalue` with bracket parsing INHIBITED under `(t)`, and
//! `c:2858-2859` then discards the value outright, so the `[…]` left over
//! is applied by the `c:2868` loop to a temporary PM_SCALAR holding the
//! TYPE STRING. `${(t)arr[1]}` is therefore `a` — character 1 of
//! "array", not the type of element 1 — and `${(t)h[k]}` is empty,
//! because `k` is an unset name that evaluates to the arithmetic 0.
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

/// The array case is the control that gives the whole game away: `[1]`
/// answers `a` and `[9]` answers nothing, which is character 1 and
/// character 9 of the five-character string "array" — not element 1 and
/// element 9 of the array. Both shells already agreed here.
#[test]
fn array_subscripts_index_the_type_string() {
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
// Association subscripts — the subscript indexes the TYPE STRING
// ═══════════════════════════════════════════════════════════════════════

/// An association element is a value, not a parameter, so it has no
/// type of its own. `Src/subst.c:2858-2859` throws the fetched value
/// away (`v = NULL; isarr = 0;`) once the tag is built, so the `[a]`
/// that follows is an ARITHMETIC index into the string "association"
/// (c:Src/params.c:1618 `mathevalarg`) — `a` and `nope` are both unset
/// names, both evaluate to 0, and index 0 is c:Src/params.c:2168-2169's
/// empty range. Regression pin: answering `association` here means the
/// subscript went to the parameter instead of to the tag.
#[test]
fn an_association_element_has_no_type_of_its_own() {
    assert_parity(r#"typeset -A h=(a 1); print "[${(t)h[a]}]" "[${(t)h[nope]}]""#);
}

/// Same through a magic hash, where a wrong answer is louder: the full
/// attribute string of the special parameter would come back for what
/// must be an empty result.
#[test]
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
fn a_commands_element_has_no_type_of_its_own() {
    assert_parity(
        r#"zmodload zsh/parameter
print "[${(t)commands[ls]}]" "[${(t)commands[nosuchcmd_zzz]}]""#,
    );
}

// ═══════════════════════════════════════════════════════════════════════
// The full index vocabulary, applied to the tag
// ═══════════════════════════════════════════════════════════════════════

/// Every index shape `Src/params.c:getindex` accepts on a PM_SCALAR, run
/// against the six-character tag "scalar" and the five-character "array":
/// a single position, a negative position, a `[N,M]` range, a range with
/// a negative end, and the zero index (c:Src/params.c:2168-2169 — empty
/// unless KSH_ZERO_SUBSCRIPT). Pins the direction the association fix
/// must not be bought at the expense of: a change that made `[k]` empty
/// by ignoring subscripts wholesale would break every line here.
#[test]
fn the_index_shapes_all_slice_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset s=v
print "[${(t)arr[2]}]" "[${(t)arr[-1]}]" "[${(t)arr[2,4]}]" "[${(t)arr[2,-1]}]" "[${(t)arr[0]}]"
print "[${(t)s[1,3]}]" "[${(t)s[-2,-1]}]""#,
    );
}

/// KSH_ZERO_SUBSCRIPT flips index 0 from "empty range" to "first
/// character" (c:Src/params.c:2160-2161 `end = startnextlen`).
#[test]
fn ksh_zero_subscript_makes_index_zero_the_first_character() {
    assert_parity(
        r#"setopt kshzerosubscript; typeset -a arr=(x y); print "[${(t)arr[0]}]""#,
    );
}

/// `[*]` / `[@]` are not arithmetic: c:Src/params.c:2027-2031 sets the
/// range to the whole value, so the tag comes back intact.
#[test]
fn a_splat_subscript_keeps_the_whole_type() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(a 1)
print "[${(t)arr[@]}]" "[${(t)arr[*]}]" "[${(t)h[@]}]""#,
    );
}

/// Text after the closing brace must still be concatenated — the
/// spelling that showed the old bug at its plainest, since the wrong
/// answer ran the whole word together as `associationX`.
#[test]
fn text_after_the_expansion_is_kept() {
    assert_parity(
        r#"typeset -A h=(k v); typeset -a arr=(x y)
print -r -- ${(t)h[k]}X
print -r -- ${(t)arr[1,2]}X"#,
    );
}

/// The subscript is arithmetic, so a name whose VALUE will not parse as
/// math is a hard error: `$parameters[PATH]` substitutes a colon-list
/// and c:Src/math.c:1534 reports it, aborting the expansion. An operand
/// that expands to nothing at all is the other error
/// (c:Src/math.c:1530-1532, "empty string"), and both must reach stderr
/// rather than silently substituting the tag.
#[test]
fn a_subscript_that_is_not_a_math_expression_is_an_error() {
    assert_parity(r#"zmodload zsh/parameter; print -r -- "[${(t)parameters[PATH]}]""#);
    assert_parity(r#"unset u_zzz; typeset -a arr=(x y); print -r -- "[${(t)arr[$u_zzz]}]""#);
}
