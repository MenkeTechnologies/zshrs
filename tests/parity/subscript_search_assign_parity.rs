//! Writes through a subscript SEARCH (`a[(r)pat]`, `a[(i)pat]`, …) and
//! through a subscript on a numeric parameter.
//!
//! C resolves every subscripted write the same way: `assignsparam` calls
//! `getvalue(&vbuf, &t, 1)` (c:Src/params.c:3251), whose `getindex` runs the
//! flag search and leaves `v->start` / `v->end` on the matched element, and
//! `assignstrvalue` (c:3343) writes through those bounds. Three port gaps
//! broke that chain:
//!
//! * `${a[(r)y]::=Q}` (paramsubst → `setsparam(idbeg, …)`, c:Src/subst.c:3313)
//!   and `(( a[(i)3] = 7 ))` handed the raw `(r)y` to arithmetic, so the
//!   write either errored or landed on element one;
//! * getindex's array `(r)`/`(R)` arm cached the matched VALUE with bounds
//!   `0..n` instead of naming the element, and assignstrvalue's PM_ARRAY arm
//!   spliced with its own bounds instead of calling setarrvalue (c:2818);
//! * a subscript on an integer/float (`typeset -i j=5; j[1]=3`) was stored as
//!   an array element; C's PM_INTEGER / PM_FFLOAT arms ignore the bounds and
//!   assign the evaluated value (c:2777-2810).
//!
//! Expected output captured from zsh 5.9.2 and the development tree
//! (zsh-5.9.0.3-test-357); both agree on every case here. No oracle needed at
//! test time.

use std::path::PathBuf;
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

/// Run `script` under `zshrs --zsh -fc` and assert stdout and exit status.
/// `stderr_has` names a fragment of the diagnostic when one is expected.
fn check(script: &str, stdout: &str, status: i32, stderr_has: Option<&str>) {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", script])
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    let got = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(got, stdout, "stdout for {script:?} (stderr: {err})");
    assert_eq!(out.status.code(), Some(status), "status for {script:?} (stderr: {err})");
    match stderr_has {
        Some(frag) => assert!(err.contains(frag), "stderr for {script:?}: {err}"),
        None => assert!(err.is_empty(), "unexpected stderr for {script:?}: {err}"),
    }
}

#[test]
fn assign_default_through_value_search_lands_on_the_match() {
    check("a=(x y z); : ${a[(r)y]::=Q}; print -r -- $a", "x Q z\n", 0, None);
    // (R) is the LAST match, not the first.
    check("a=(x y z); : ${a[(R)*]::=L}; print -r -- $a", "x y L\n", 0, None);
    // (e) literal match and (n:N:) Nth match.
    check(
        "a=(x y x z); : ${a[(re)x]::=E}; : ${a[(rn:2:)x]::=W}; print -r -- $a",
        "E y x z W\n",
        0,
        None,
    );
    // k on an array acts as r (c:1401 `keymatch = ishash`).
    check("a=(x y z); : ${a[(k)y]::=K}; print -r -- $a", "x K z\n", 0, None);
}

#[test]
fn assign_default_through_index_search_replaces_one_element() {
    check("b=(x y z); : ${b[(i)y]::=Q}; : ${b[(I)z]::=W}; print -r -- $b", "x Q W\n", 0, None);
}

#[test]
fn search_miss_appends_or_reports_the_empty_range() {
    // A forward miss is index len+1: the value is appended.
    check("a=(x y z); : ${a[(r)nomatch]::=N}; print -r -- $a", "x y z N\n", 0, None);
    // A reverse miss is index 0: invalid for setting (c:2141-2150) ...
    check(
        "a=(x y z); : ${a[(R)nomatch]::=N}; print -r -- $a",
        "",
        1,
        Some("a: assignment to invalid subscript range"),
    );
    // ... unless KSH_ZERO_SUBSCRIPT makes it the first element (c:2134-2140).
    check(
        "setopt kshzerosubscript; a=(x y z); : ${a[(R)nomatch]::=N}; print -r -- $a",
        "N y z\n",
        0,
        None,
    );
}

#[test]
fn search_writes_under_ksharrays() {
    check(
        "setopt ksharrays; a=(x y z); : ${a[(r)y]::=Q}; : ${a[(R)x]::=W}; print -r -- ${a[@]}",
        "W Q z\n",
        0,
        None,
    );
    check(
        "setopt ksharrays; a=(x y z); a[(i)y]=Q; a[(r)z]=R; print -r -- ${a[@]}",
        "x Q R\n",
        0,
        None,
    );
}

#[test]
fn arithmetic_assignment_through_a_search_subscript() {
    check("a=(1 2 3); (( a[(i)3] = 7 )); (( a[(r)2] += 5 )); print -r -- $a", "1 7 7\n", 0, None);
}

#[test]
fn subscripted_write_to_a_numeric_parameter_replaces_its_value() {
    check(
        "typeset -i j=5; j[1]=3; print -r -- $j ${(t)j}; typeset -i i; i[2]=4+4; print -r -- $i",
        "3 integer\n8\n",
        0,
        None,
    );
    check(
        "typeset -F f=2; f[1]=3; typeset -E e=1; e[1]=2.5; print -r -- $f $e ${(t)f}",
        "3.0000000000 2.500000000e+00 float\n",
        0,
        None,
    );
    check(
        "typeset -i i=5; i[0]=3; print -r -- notreached",
        "",
        1,
        Some("i: assignment to invalid subscript range"),
    );
}

#[test]
fn assign_default_through_a_range_replaces_the_slice() {
    // c:2111-2113 — the range's second getarg; the single-index store took
    // the comma expression as one arithmetic index.
    check("a=(x y z); : ${a[2,3]::=Q}; print -r -- $a", "x Q\n", 0, None);
    check("s=abcdef; : ${s[2,4]::=X}; print -r -- $s", "aXef\n", 0, None);
    // Each range operand may itself be a search.
    check("b=(a b c d e); : ${b[(r)b,(r)d]::=M}; print -r -- $b", "a M e\n", 0, None);
    check(
        "setopt ksharrays; c=(a b c d e); : ${c[(r)b,(r)d]::=M}; print -r -- ${c[@]}",
        "a M e\n",
        0,
        None,
    );
}
