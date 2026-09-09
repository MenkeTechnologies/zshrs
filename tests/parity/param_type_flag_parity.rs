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

// ═══════════════════════════════════════════════════════════════════════
// Search subscripts — getarg's reverse arm, run over the tag
// ═══════════════════════════════════════════════════════════════════════

/// `(r)` is a PATTERN search over the carrier, and the carrier is a
/// PM_SCALAR (c:Src/subst.c:2890 with `isarr` 0), so `v->scanflags` is 0
/// and c:Src/params.c:1731/1782 both fall through to c:1819's "Searching
/// characters" arm. c:1698-1704 appends an implicit `*` to the pattern,
/// c:1985-1992 returns the raw offset just PAST the matching character
/// and c:2144-2145 backs it off by one, so the answer is the single
/// CHARACTER the match started on: `array` matches at position 1 of the
/// tag "array", giving `a`. A pattern that never matches returns
/// c:2001's `slen + 1`, which c:2525-2531 reads back as the empty
/// string. Regression pin: answering the whole tag means the `(…)`
/// subscript was dropped on the floor instead of being applied.
#[test]
fn search_subscripts_match_against_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr[(r)array]}]" "[${(t)arr[(r)nomatch]}]" "[${(t)arr[(r)a*]}]""#,
    );
}

/// `(i)` sets `ind` (c:Src/params.c:1432-1435), which c:2061-2119 turns
/// into VALFLAG_INV, and c:2336-2340 then renders `v->start` as a
/// DECIMAL POSITION rather than reading the string: the 1-based
/// character index of the match. A miss walks off the end and reports
/// `len + 1` — 6 for the five-character "array".
#[test]
fn index_search_subscripts_report_a_position_in_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr[(i)array]}]" "[${(t)arr[(i)nomatch]}]" "[${(t)arr[(i)r]}]""#,
    );
}

/// The upper-case spellings search BACKWARDS (c:Src/params.c:1418-1421
/// / 1436-1439 set `down`), so `(I)a` finds the LAST `a` in "array" —
/// position 4 — and `(R)a` returns the character there. `k`/`K` gate
/// their key-matching on `ishash` (c:1423/1428), which is 0 for the
/// scalar carrier, so they collapse onto `r`/`R`.
#[test]
fn reverse_and_key_search_spellings_on_the_carrier() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr[(I)a]}]" "[${(t)arr[(R)a]}]" "[${(t)arr[(k)array]}]" "[${(t)arr[(K)a]}]""#,
    );
}

/// `(n:N:)` picks the Nth match (c:Src/params.c:1452-1463) and `(b:N:)`
/// starts the scan at an offset (c:1464-1475); both are counted in
/// characters of the tag.
#[test]
fn match_count_and_begin_offset_flags_apply_to_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr[(n:2:i)r]}]" "[${(t)arr[(i)r]}]" "[${(t)arr[(R)r]}]""#,
    );
}

/// A flag set with NO search direction never sets `rev`, so
/// c:Src/params.c:1596-1621 takes the ARITHMETIC branch over the text
/// that follows the flag block: `(e)ar` evaluates the unset name `ar` as
/// 0, and index 0 is c:2168-2169's empty range. Regression pin for the
/// carrier path forwarding only the `getarg` search result and falling
/// back to the tag whole.
#[test]
fn a_flag_block_without_a_search_direction_falls_back_to_arithmetic() {
    assert_parity(r#"typeset -a arr=(x y); print "[${(t)arr[(e)ar]}]" "[${(t)arr[(e)3]}]""#);
}

/// The same search vocabulary against the eleven-character
/// "association" tag, so a wrong answer cannot hide behind the shorter
/// "array".
#[test]
fn search_subscripts_on_an_association_tag() {
    assert_parity(
        r#"typeset -A h=(k v)
print "[${(t)h[(r)assoc]}]" "[${(t)h[(i)assoc]}]" "[${(t)h[(i)ation]}]" "[${(t)h[(I)i]}]""#,
    );
}

// ═══════════════════════════════════════════════════════════════════════
// The postmodifiers run AFTER the tag is built, and see the tag
// ═══════════════════════════════════════════════════════════════════════

/// C's order is `wantt` tag (c:Src/subst.c:2808-2861) → carrier
/// subscript (c:2868-2985) → operators (c:3081+), so c:3188-3191's colon
/// NULL test — `vunset = (isarr) ? !*aval : !*val` — is evaluated
/// against the SUBSCRIPTED TAG. `${(t)h[k]}` and `${(t)a[9]}` are both
/// empty, so `:-` supplies its default; `${(t)a[1]}` is `a`, so it does
/// not. Regression pin for the ordering: running the type arm after the
/// operators makes every one of these read the parameter's own value,
/// which is non-empty, and the default never fires.
#[test]
fn the_colon_default_tests_the_subscripted_tag() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(k v)
print "[${(t)h[k]:-D}]" "[${(t)arr[9]:-D}]" "[${(t)arr[1]:-D}]" "[${(t)arr:-D}]""#,
    );
}

/// `:+` is the same test read the other way (c:3194-3201): the
/// alternate word appears only when the tag is non-null.
#[test]
fn the_colon_alternate_tests_the_subscripted_tag() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(k v)
print "[${(t)h[k]:+P}]" "[${(t)arr[1]:+P}]" "[${(t)arr[9]:+P}]" "[${(t)arr[2,3]:+P}]""#,
    );
}

/// WITHOUT the colon the test is c:3205's `vunset`, which c:2855 already
/// cleared for any parameter that had a type: `${(t)arr[9]-D}` stays
/// empty even though the tag indexed by `[9]` is, while a name with no
/// type at all still takes the default. Pins that the ordering fix did
/// not turn the plain `-` into the colon form.
#[test]
fn the_plain_default_still_tests_whether_the_name_has_a_type() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr[9]-D}]" "[${(t)arr[1]-D}]" "[${(t)nosuchvar_zzz-D}]" "[${(t)nosuchvar_zzz:-D}]""#,
    );
}

/// `:=` (c:3246-3322) fires on the same null test and substitutes its
/// word.
#[test]
fn the_colon_assign_fires_on_an_empty_tag() {
    assert_parity(r#"typeset -a arr=(x y); print "[${(t)arr[9]:=D}]" "[${(t)arr[1]:=D}]""#);
}

/// The pattern operators are downstream of the tag too (c:3081 is one
/// block for the whole family), so `#`/`##`/`%`/`%%` strip from the type
/// string, not from the parameter.
#[test]
fn the_strip_operators_run_on_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(k v)
print "[${(t)arr#a}]" "[${(t)arr##a}]" "[${(t)arr%y}]" "[${(t)arr%%r*}]" "[${(t)h#ass}]""#,
    );
}

/// So are `/` and `//` (c:3107-3167) and the `:#` filter (c:3540).
#[test]
fn the_replace_operators_run_on_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y)
print "[${(t)arr/rr/XX}]" "[${(t)arr//a/Z}]" "[${(t)arr:#array}]" "[${(t)arr:#nope}]""#,
    );
}

/// And the history-style colon modifiers (c:3761-3776). c:2858-2859's
/// `isarr = 0` means c:4533's per-element leg is dead, so `:u` upcases
/// the TAG once — answering `X Y` would mean the modifier found the
/// parameter's array behind the tag's back.
#[test]
fn the_colon_modifiers_run_on_the_type_string() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(k v)
print "[${(t)arr:u}]" "[${(t)arr:t}]" "[${(t)arr:s/r/R/}]" "[${(t)arr:q}]" "[${(t)h:l}]""#,
    );
    assert_parity(r#"typeset -a arr=(x y); print -r -- ${(t)arr:u} ${(t)arr:t} ${(t)arr:h}"#);
}

/// A `(t)` with no operator at all must still be one word, quoted or
/// not: c:2858-2859 cleared `isarr`, so nothing splats the parameter's
/// elements after the tag replaced them.
#[test]
fn a_bare_type_flag_stays_one_word_in_either_context() {
    assert_parity(
        r#"typeset -a arr=(x y); typeset -A h=(k v)
print -r -- ${(t)arr} ${(t)arr[@]} ${(t)h} ${(t)h[@]}
print -r -- "${(t)arr}" "${(t)arr[@]}"
set -- ${(t)arr}; print $#"#,
    );
}
