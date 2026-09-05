//! Subscript RANGE-BOUND and colon-NULL parity against `zsh -f`.
//!
//! Companion to `tests/bugs_md_regression.rs`, split out because these two
//! entries land in `src/ported/subst.rs` + `src/subscript_escape.rs` while
//! that file is being extended concurrently for a different fix.
//!
//! * BUGS.md #1133 — a range bound whose math evaluation FAILS must `zerr`
//!   and raise errflag (c:`Src/math.c:1541`/`1546`, c:`Src/utils.c:184`,
//!   c:`Src/exec.c:1443`), not silently substitute the arm's default bound.
//! * BUGS.md #1132 residual — the COLON null test is shape-dependent,
//!   c:`Src/subst.c:3189` `vunset = (isarr) ? !*aval : !*val`.
//!
//! Same pattern as its companion: spawn the freshly built `target/debug/zshrs`
//! with `--zsh` and assert stdout + exit code against the zsh reference.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ]
    .into_iter()
    .find(|cand| cand.exists())
}

fn run_zshrs(script: &str) -> (i32, String, String) {
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => {
            eprintln!("skip: zshrs binary not built");
            return (0, String::new(), String::new());
        }
    };
    let out = Command::new(&bin)
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #1133 — a range bound whose math evaluation FAILS must
// `zerr` and raise errflag, not silently substitute the arm's default
// bound.
// Fix: src/ported/subst.rs (`eval_idx` / `bound_idx` / `eval_bound`),
//      src/subscript_escape.rs (`subscript_bound_classify`)
// c:Src/math.c:1541/1546 — `mathevalarg` -> `mathevall(…, MPREC_ARG, …)`
// zerrs on a parse failure and hands back the ZERO value; `zerr` raises
// ERRFLAG_ERROR (c:Src/utils.c:184) and c:Src/exec.c:1443 ends the list.
// ════════════════════════════════════════════════════════════════════

#[test]
fn subscript_range_bound_math_failure_errors_instead_of_defaulting() {
    // Each script must abort with rc 1, print NOTHING on stdout (the
    // trailing `print AFTER` never runs, exactly as `zsh -f` does), and
    // name the failure on stderr with zsh's own wording.
    let cases: &[(&str, &str)] = &[
        // `(zz)` and `(W)` both take the c:Src/params.c:1498-1503 `flagerr`
        // rewind, so the WHOLE group is the math text. END bound, array.
        (
            "a=(x y z); print -r -- ${a[1,(zz)2]}; print AFTER",
            "bad math expression: operator expected at `2'",
        ),
        // END bound, scalar char slice.
        (
            "s='alpha beta'; print -r -- ${s[1,(W)2]}; print AFTER",
            "bad math expression: operator expected at `2'",
        ),
        // START bound — c:2058's getarg call, the other half of the range.
        (
            "s='alpha beta'; print -r -- ${s[(zz)1,2]}; print AFTER",
            "bad math expression: operator expected at `1'",
        ),
        (
            "a=(x y z); print -r -- ${a[(W)2,3]}; print AFTER",
            "bad math expression: operator expected at `2'",
        ),
        // c:1618 again, reached through the WORD branch: the number after a
        // `(w)` flag is still `mathevalarg`, so its failure errors too.
        (
            "s='a b c'; print -r -- ${s[(w)qq*,2]}; print AFTER",
            "bad math expression: operand expected at end of string",
        ),
        // c:1618 reached through a flag group that PARSED but ran no search:
        // getarg is left at `mathevalarg` over the text AFTER the group.
        (
            "s='alpha beta'; print -r -- ${s[1,(e)qq*]}; print AFTER",
            "bad math expression: operand expected at end of string",
        ),
        // The `(@)` splat path's own bound evaluator.
        (
            "a=(x y z); print -r -- ${(@)a[1,(zz)2]}; print AFTER",
            "bad math expression: operator expected at `2'",
        ),
    ];
    for (script, want_err) in cases {
        let (code, out, err) = run_zshrs(script);
        assert_eq!(code, 1, "script: {script}\nstdout: {out}\nstderr: {err}");
        assert_eq!(out, "", "script: {script}\nstderr: {err}");
        assert!(
            err.contains(want_err),
            "script: {script}\nwant stderr to contain: {want_err}\ngot: {err}"
        );
    }
}

#[test]
fn subscript_empty_flag_group_parses_and_is_stripped() {
    // c:Src/params.c:1412 — `for (s++; *s != ')' && … ; s++)` never enters
    // the body for `()`, so `flagerr` is not reached; c:1506-1507
    // `if (s != *str) s++;` steps past the `)` and c:1618 evaluates the text
    // AFTER it. An empty group is therefore stripped like `(e)` or `(s.X.)`,
    // NOT handed to the math evaluator. Masked until the bound evaluators
    // stopped swallowing a failed `mathevali`.
    let cases: &[(&str, &str)] = &[
        ("a=(x y z); print -r -- \"[${a[1,()2]}]\"", "[x y]\n"),
        ("a=(x y z); print -r -- \"[${a[()2,3]}]\"", "[y z]\n"),
        ("s='alpha beta'; print -r -- \"[${s[1,()2]}]\"", "[al]\n"),
        // An UNKNOWN letter still rewinds (c:1498-1503), so it stays math and
        // still errors — the two cases must not collapse into one another.
        (
            "a=(x y z); ( print -r -- ${a[1,(q)2]} ) 2>/dev/null; print -r -- done",
            "done\n",
        ),
    ];
    for (script, want) in cases {
        let (code, out, err) = run_zshrs(script);
        assert_eq!(
            (code, out.as_str()),
            (0, *want),
            "script: {script}\nstderr: {err}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #1132 residual — the colon NULL test in the default family is
// SHAPE-dependent (c:Src/subst.c:3189 `vunset = (isarr) ? !*aval : !*val`),
// so an array is null only when it has NO ELEMENTS.
// Fix: src/ported/subst.rs — the `:-` / `:+` / `:?` arms
// ════════════════════════════════════════════════════════════════════

#[test]
fn colon_default_family_null_test_is_array_shape_aware() {
    // The `w=` form is the scalar-assign (`ssub`) context, one of the four
    // that bypass the compiler's own `BUILTIN_PARAM_DEFAULT_FAMILY` name
    // lowering and reach `paramsubst` directly.
    //
    // The `v=('')` / `set -- ''` cases are the ones the c:3189 shape gate
    // fixes; they were measured RED on a binary built from this tree with
    // only `src/ported/subst.rs` + `src/subscript_escape.rs` reverted. The
    // `set -- x y` / `argv` cases at the top belong to the SEPARATE getter
    // fix covered by `bug1132_argv_is_pparams_for_the_colon_default_family`
    // and are green on that control — they are kept here as the parity
    // statement for the neighbouring shapes, not as this fix's evidence.
    let cases: &[(&str, &str)] = &[
        // c:Src/params.c:428-430 — `argv`, `*` and `@` are ONE
        // IPDEF9(&pparams) parameter.
        ("set -- x y; w=${argv:-nope}; print -r -- \"[$w]\"", "[x y]\n"),
        (
            "set -- x y; [[ ${argv:-nope} == 'x y' ]] && print HIT || print MISS",
            "HIT\n",
        ),
        (
            "set -- x y; case ${argv:-nope} in 'x y') print HIT;; *) print MISS;; esac",
            "HIT\n",
        ),
        ("set -- x y; cat <<< ${argv:-nope}", "x y\n"),
        ("set -- x y; w=${argv:+Y}; print -r -- \"[$w]\"", "[Y]\n"),
        // c:3189 `!*aval` — an array of ONE EMPTY element has an element, so
        // it is NOT null even though it joins to the empty string.
        ("v=(''); w=${v:-N}; print -r -- \"[$w]\"", "[]\n"),
        ("v=(''); w=${v:+Y}; print -r -- \"[$w]\"", "[Y]\n"),
        ("v=(''); w=${v[@]:-N}; print -r -- \"[$w]\"", "[]\n"),
        ("v=(''); w=${v:?E}; print -r -- \"[$w]\"", "[]\n"),
        ("set -- ''; w=${@:-N}; print -r -- \"[$w]\"", "[]\n"),
        ("set -- ''; w=${argv:+Y}; print -r -- \"[$w]\"", "[Y]\n"),
        // The other half of c:3189 must keep disagreeing: NO elements is
        // null, and so is an empty scalar.
        ("v=(); w=${v:-N}; print -r -- \"[$w]\"", "[N]\n"),
        ("v=''; w=${v:-N}; print -r -- \"[$w]\"", "[N]\n"),
        ("set --; w=${argv:-N}; print -r -- \"[$w]\"", "[N]\n"),
        // c:Src/params.c:2175-2179 — a SINGLE index clears the Value's
        // scanflags (SCANPM_ISVAR_AT with them), so c:2916 gives isarr 0 and
        // the SCALAR test applies: the empty element IS null.
        ("set -- ''; w=${@[1]:-N}; print -r -- \"[$w]\"", "[N]\n"),
        ("v=(x y); w=${v[2]:-N}; print -r -- \"[$w]\"", "[y]\n"),
        // An assoc has no `arrays_get` row; its map is what `!*aval` reads.
        ("typeset -A v; v[k]=q; w=${v:-N}; print -r -- \"[$w]\"", "[q]\n"),
        ("typeset -A v; w=${v:-N}; print -r -- \"[$w]\"", "[N]\n"),
    ];
    for (script, want) in cases {
        let (code, out, err) = run_zshrs(script);
        assert_eq!(
            (code, out.as_str()),
            (0, *want),
            "script: {script}\nstderr: {err}"
        );
    }
}
