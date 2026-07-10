//! Parity gaps discovered by the differential fuzzer (`bins/parity-fuzz.rs`).
//!
//! The fuzzer generates stateful zsh programs — sequences of setopt / typeset /
//! IFS / scope / array mutations interleaved with observations — runs each
//! through `zsh -fc` and `zshrs --zsh -fc`, and delta-debugs every divergence
//! down to a minimal reproducer. Each test below is one such minimized repro,
//! reduced further by hand to the smallest standalone script that still
//! diverges, with the observed outputs recorded in the doc comment.
//!
//! Every test is `#[ignore]`d with a `zshrs gap:` note. When the underlying gap
//! is fixed, drop the `#[ignore]` — the test then flips into a regression pin
//! (same lifecycle as `discovered_gaps_2026q2_parity.rs`). Run the whole set
//! with:  `cargo test --test parity fuzz_discovered -- --ignored`.

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
/// Assert zsh and zshrs agree on stdout and exit code for `s`.
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
    assert_eq!(
        z.exit, r.exit,
        "exit divergence on:\n{s}\n  zsh={} zshrs={}",
        z.exit, r.exit
    );
}

// ─────────────────────────────────────────────────────────────────────
// A. typeset assignment values are glob-expanded after parameter expansion
//
// A `typeset name=value` word is an assignment word and its value must not
// undergo filename generation. zshrs skips globbing for a *literal* value
// (`typeset -i lv=6*2` → 12, correct) but globs the result once a parameter
// expands inside it: `$x` → 6, then `lv=6*2` is filename-generated → NOMATCH.
// ─────────────────────────────────────────────────────────────────────
mod typeset_assignment_glob {
    use super::*;

    /// Literal value already exempt from globbing — pin so a fix can't regress.
    #[test]
    fn literal_star_value_not_globbed() {
        assert_parity("typeset -i lv=6*2; print -r -- $lv");
    }

    /// zsh: `12`. FIXED: the fusevm compiler's multi-segment word-glob emit
    /// (compile_zsh.rs) now honours `assign_builtin_arg_depth`, so an
    /// assignment-builtin value combining a param expansion with a literal
    /// glob metachar is no longer filename-generated (exec.c:4246-4249).
    #[test]
    fn expanded_value_glob_expanded() {
        assert_parity("x=6; typeset -i lv=$x*2; print -r -- $lv");
    }

    /// Same fix, via a positional param inside a function (integer local).
    /// zsh: `12`.
    #[test]
    fn expanded_positional_in_function() {
        assert_parity("f() { typeset -i lv=$1*2; print -r -- $lv }; f 6");
    }
}

// ─────────────────────────────────────────────────────────────────────
// B. Subscript assignment to an undeclared parameter is too permissive
//
// `name[key]=v` on a parameter that was never declared array/assoc is a
// "assignment to invalid subscript range" error in zsh (non-numeric key on a
// non-assoc). zshrs silently accepts it.
// ─────────────────────────────────────────────────────────────────────
mod undeclared_subscript_assign {
    use super::*;

    /// zsh: `as: assignment to invalid subscript range`, exit 1. FIXED: a
    /// non-hashed param's subscript is now arithmetic-evaluated (getarg,
    /// params.c:1601) instead of auto-vivifying an assoc, so `k1`→0→error.
    #[test]
    fn nonnumeric_key_on_undeclared() {
        assert_parity("as[k1]=bb; print done");
    }

    /// Same, with a numeric-looking value — still an arithmetic *subscript*.
    #[test]
    fn nonnumeric_key_numeric_value() {
        assert_parity("as[k1]=7; print done");
    }
}

// ─────────────────────────────────────────────────────────────────────
// C. Sparse arrays: unquoted ${arr[@]} must drop empty elements
//
// Creating `arr[3]=x2` on an undeclared array yields a sparse array whose
// leading elements are empty. Unquoted `${arr[@]}` is subject to empty-word
// elision, so only `x2` survives. zshrs keeps the two empty elements.
// ─────────────────────────────────────────────────────────────────────
mod sparse_array_empty_elision {
    use super::*;

    /// zsh: one line `x2`. FIXED: the fusevm compiler's `${NAME[@]}`
    /// fast-path now appends BUILTIN_ARRAY_DROP_EMPTY in unquoted context
    /// (mirroring the $@/$*/$argv splat), so empty words are removed
    /// (subst.c:184-188 uremnode). Quoted `"${arr[@]}"` still keeps them.
    #[test]
    fn unquoted_at_drops_empty_elements() {
        assert_parity("arr[3]=x2; print -rl -- ${arr[@]}");
    }
}

// ─────────────────────────────────────────────────────────────────────
// D. (P) indirect flag on a value that is not a bare parameter name
//
// `${(P)t}` uses t's value as a parameter *expression*. With t='a,b,c' zsh
// resolves it to parameter `a`; zshrs yields empty. A clean name (t='a')
// already works, so the gap is the non-trivial value path.
// ─────────────────────────────────────────────────────────────────────
mod indirect_flag_nonname_value {
    use super::*;

    /// Bare-name indirection already works — pin it.
    #[test]
    fn bare_name_indirection() {
        assert_parity("a=(one two); ref=a; print -r -- ${(P)ref}");
    }

    /// zsh: `one two`. FIXED: the bare-name (P) branch now reparses the
    /// operand as a parameter expression (itype_end leading-identifier
    /// truncation, params.c:2216), so t="a,b,c" derefs param `a`.
    #[test]
    fn comma_value_indirection() {
        assert_parity("t='a,b,c'; a=(one two); print -r -- ${(P)t}");
    }
}

// ─────────────────────────────────────────────────────────────────────
// E. (q) quoting of control / empty values
// ─────────────────────────────────────────────────────────────────────
mod quote_flag_formatting {
    use super::*;

    /// `(#)` maps the value to a character code; `Hello` → 0 → NUL, then `(q)`
    /// quotes it. zsh: `$'\0'` (6 bytes). FIXED: quotestring now emits `\0`
    /// for a lone NUL (utils.c:6096-6104), widening to `\000` only before an
    /// octal digit.
    #[test]
    fn nul_quoting_format() {
        assert_parity("s=Hello; print -r -- ${(eq#)s}");
    }

    /// Visible+quote of the empty string. zsh: `''`. FIXED: the second-pass
    /// (V) renderer now preserves the (q) flag's internal Snull marker byte
    /// (0x9d) so the downstream stripper removes it, instead of rendering it
    /// as literal `\M-^]`.
    #[test]
    fn visible_quote_empty() {
        assert_parity("empty=''; print -r -- ${(Vq)empty}");
    }

    /// `(#)` on a bare array collapses it to one evaluated scalar (a
    /// non-numeric join → empty); a FOLLOWING flag must use that collapsed
    /// value, not re-fetch the raw array. zsh `${(#q)arr}` → `''`. FIXED:
    /// the `(#)` evalchar branch stashes the collapsed value in split_parts so
    /// `(q)`/`(V)` don't re-fetch. Bare-array-with-flag was the class behind
    /// several `${(#q…)path}` expr-fuzz divergences.
    #[test]
    fn charcode_flag_on_array_then_quote() {
        assert_parity("arr=(/a /b); print -r -- ${(#q)arr}; print -r -- ${(#qV)arr}");
    }
}

// ─────────────────────────────────────────────────────────────────────
// F. Arithmetic domain: 0 raised to a negative power
//
// zsh treats `0 ** -n` as a floating divide → `Inf` (or INT_MAX in integer
// context), exit 0. zshrs raises a "division by zero" error, exit 1.
// ─────────────────────────────────────────────────────────────────────
mod zero_to_negative_power {
    use super::*;

    /// zsh: `Inf`, exit 0.  FIXED: math.rs POWER now casts an integer base
    /// with a negative exponent to float before the zero check (c:1337-1346),
    /// so `0 ** -n` is pow(0.0,-n)=Inf, not a division-by-zero error.
    #[test]
    fn float_context() {
        assert_parity("print -r -- $(( 0 ** -4 ))");
    }

    /// Integer context. zsh: `9223372036854775807` (Inf cast to zlong). FIXED
    /// with the same POWER change.
    #[test]
    fn integer_context() {
        assert_parity("typeset -i x=\"0 ** -4\"; print -r -- $x");
    }
}

// ═════════════════════════════════════════════════════════════════════
// Tier 2 — gaps surfaced by re-fuzzing after the first seven were fixed.
// ═════════════════════════════════════════════════════════════════════

// ─────────────────────────────────────────────────────────────────────
// K. KSH_ARRAYS scalar-of-element-1 semantics on the length operator
//
// Under KSH_ARRAYS, `$arr` means `$arr[1]` (a scalar), so `${#arr}` is the
// STRING LENGTH of the first element, not the array count. `$arr` and
// `${arr[0]}` already honour this; `${#arr}` does not.
// ─────────────────────────────────────────────────────────────────────
mod ksh_arrays_length {
    use super::*;

    /// zsh: `2` (strlen of `x1`). FIXED: the `${#…}` compiler parse now keeps
    /// the `[@]`/`[*]` subscript (compile_zsh.rs) so paramsubst can tell bare
    /// `${#arr}` (scalar element 1 under KSHARRAYS) from `${#arr[@]}` (count),
    /// and the length block scalarises the bare form (subst.c:3860-3878).
    #[test]
    fn length_is_element_strlen() {
        assert_parity("arr=(x1 x2 y3); setopt KSH_ARRAYS; print -r -- ${#arr}");
    }

    /// After `arr+=(beta)` on an undeclared array under KSH_ARRAYS, `${#arr}`
    /// is strlen(`beta`)=4. zsh: `4`. FIXED with the same change.
    #[test]
    fn length_after_append() {
        assert_parity("setopt KSH_ARRAYS; arr+=(beta); print -r -- ${#arr}");
    }

    /// KSH_ARRAYS also changes assoc scalar access: a bare `$assoc` (even with
    /// (k)/(v)/(o)/(kv) flags) is a SCALAR — the bucket-first value — and the
    /// flags no-op on it. zsh `${(ko)as}` → `v1`. FIXED: paramsubst scalarizes
    /// a bare KSHARRAYS assoc before the whole-assoc key/value fold
    /// (params.c:2293-2296).
    #[test]
    fn assoc_scalar_access() {
        assert_parity(
            "typeset -A as; as=(k1 v1 k2 v2); setopt KSH_ARRAYS; \
             print -r -- ${(ko)as}; print -r -- ${(v)as}; print -r -- ${(o)as}",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// N. NO_UNSET (nounset) on the length operator
//
// `${#name}` on an unset parameter must trip nounset like a plain `$name`
// reference does. Plain `$novar` already errors; `${#arr}` does not.
// ─────────────────────────────────────────────────────────────────────
mod nounset_length {
    use super::*;

    /// zsh: `arr: parameter not set`, exit 1. FIXED: the `${#name}` length
    /// block now runs the nounset guard (subst.c:3480-3483) before it returns,
    /// instead of yielding 0 for an unset parameter.
    #[test]
    fn length_of_unset_errors() {
        assert_parity("setopt NO_UNSET; print -r -- ${#arr}");
    }
}

// ─────────────────────────────────────────────────────────────────────
// X. Arithmetic-error exit code inside an assignment-builtin
//
// A SOFT math error (`$((1/0))`, ERRFLAG_ERROR without ERRFLAG_HARD) in the RHS
// of an assignment-BUILTIN (`typeset`/`declare`/`local`/`export`/`readonly`/
// `integer`/`float`/`private`) PRESERVES the prior exit status in zsh: the
// postassign prefork (exec.c:4239-4245) breaks on errflag and exec.c:4287
// `if (!errflag) execbuiltin(...)` skips the builtin, leaving lastval unchanged.
// So a fresh shell exits 0 but `false; typeset -i x=$((1/0))` exits 1. This is
// distinct from a PLAIN assignment `x=$((1/0))` (execsimple c:1375 → 1) and from
// a NON-assign builtin `print $((1/0))` (main args-prefork c:3760 → 1), both of
// which stay 1 on both shells.
// FIXED: dispatch_builtin (fusevm_bridge.rs) now returns the prior LASTVAL when
// errflag is a soft ERRFLAG_ERROR on a BINF_ASSIGN builtin. A HARD error
// (`${var?msg}`) still aborts with status 1 (falls through the gate).
// ─────────────────────────────────────────────────────────────────────
mod typeset_arith_error_exit {
    use super::*;

    /// Fresh shell — prior status 0 is preserved when the assignment-builtin's
    /// math RHS errors and the builtin is skipped.  zsh: exit 0.
    #[test]
    fn integer_assign_divzero_exit_fresh() {
        assert_parity("typeset -i x=$(( 1/0 )); print ok");
    }

    /// After `false` — prior status 1 is preserved (not reset to 0).  zsh: exit 1.
    #[test]
    fn integer_assign_divzero_exit_after_false() {
        assert_parity("false; typeset -i x=$(( 1/0 )); print ok");
    }

    /// A PLAIN assignment (no builtin) still exits 1 — the preserve-prior rule
    /// is specific to assignment-builtins.  zsh: exit 1.
    #[test]
    fn plain_assign_divzero_exit() {
        assert_parity("x=$(( 1/0 )); print ok");
    }

    /// A HARD `${var?msg}` error in a typeset RHS aborts with status 1 even on a
    /// fresh shell — it is not preserve-prior.  zsh: exit 1.
    #[test]
    fn typeset_hard_paramerr_exit() {
        assert_parity("unset UNSETV; typeset v=${UNSETV?boom}; print ok");
    }
}

// ─────────────────────────────────────────────────────────────────────
// G. Glob-qualifier gaps (found by the fuzzer's --mode glob fixture runs).
//
// Each test builds its own temp fixture so it is self-contained and runs in
// headless CI. Both shells glob the same tree; the divergence is ordering.
// ─────────────────────────────────────────────────────────────────────
mod glob_ordering {
    use super::*;

    /// Recursive `**/*` must yield a single lexicographically-sorted path list.
    /// zsh: `d`, `d/z`, `e`, `m`.  zshrs: `d`, `e`, `m`, `d/z` — it sorts each
    /// recursion level separately and appends the recursed matches instead of
    /// merge-sorting the whole result.
    /// FIXED: `gmatchcmp` GS_NAME now strips the `./` CurDir prefix that the
    /// scanner puts on depth-0 matches (glob.rs), so the single combined sort
    /// keys on the same bare-relative path it emits (matching C's dyncat name,
    /// glob.c:424/1977). `**/*` now yields one full-path-sorted list.
    #[test]
    fn globstar_full_path_sort() {
        assert_parity(
            "setopt extendedglob; t=$(mktemp -d); cd $t; mkdir d; touch d/z e m; \
             print -rl -- **/*; cd /; rm -rf $t",
        );
    }

    // NOTE: the fuzzer also surfaced an `(oL)` size-order tie-break divergence
    // (equal-size regular file vs symlink), but it only manifests with a fuller
    // fileset and did not reduce to a stable minimal reproducer, so it is not
    // pinned here — pinning a passing case would be a misleading `#[ignore]`.
}

// ─────────────────────────────────────────────────────────────────────
// P. printf format engine (found by the fuzzer's --mode printf).
// ─────────────────────────────────────────────────────────────────────
mod printf_format {
    use super::*;

    /// A `%` directive carrying any flag/width/precision is an "invalid
    /// directive" in zsh (exit 1); zshrs printed `%`, exit 0. FIXED
    /// (builtin.c:5414-5419): only a bare `%%` (spec == "%") prints `%`.
    #[test]
    fn percent_with_modifier_is_invalid() {
        assert_parity("printf '%5%\\n'");
    }

    /// Precision on `%x`/`%o`/`%X`/`%u` is a minimum digit count (zero-pad).
    /// zsh `%.3x` of 10 → `00a`; zshrs ignored precision. FIXED in
    /// format_spec_radix/uint.
    #[test]
    fn precision_on_radix_conversions() {
        assert_parity("printf '%.3x|%.4o|%.5X\\n' 10 8 255");
    }

    /// A zero value with precision 0 yields NO digits. zsh `%.0d` of 0 → ``;
    /// zshrs printed `0`. FIXED across format_spec_int/uint/radix.
    #[test]
    fn zero_value_precision_zero_empty() {
        assert_parity("printf '[%.0d][%.0x][%.0o]\\n' 0 0 0");
    }

    /// `%q` of a MISSING argument: zsh outputs nothing (NULL curarg →
    /// nullstr). FIXED (builtin.c:5387): a missing arg is no longer coerced to
    /// an empty string and quoted as `''`; a present empty arg still quotes.
    #[test]
    fn quote_missing_arg() {
        assert_parity("printf '[%q]\\n'; printf '%q %q\\n' one");
    }

    /// The `#` flag forces a decimal point on a float conversion even when
    /// precision 0 leaves no fractional digits. zsh `%#.0f` 5 → `5.`, `%#.0e`
    /// → `5.e+00`; zshrs dropped the point. FIXED in format_spec_float_conv.
    #[test]
    fn hash_flag_forces_decimal_point() {
        assert_parity("printf '%#.0f|%#.0e|%#.0g\\n' 5 5 5");
    }

    /// A math-eval failure on a FLOAT operand is a soft error → exit 1 (like
    /// the integer path). zsh `printf '%g' '%d'` → exit 1; zshrs swallowed it
    /// (exit 0). FIXED: the float arm now flags PRINTF_MATH_ERR. Undefined
    /// vars (`abc`→0) and empty/missing args stay exit 0.
    #[test]
    fn float_math_error_exit() {
        assert_parity("printf '%g\\n' '%d'");
    }

    /// A dynamic field width via `*` reads its argument through the SAME
    /// math evaluator as a `%d` operand (builtin.c:5241
    /// `width = (int)mathevali(...)`), not a plain decimal parse. So a hex
    /// width arg `0x1f` is 31, a leading-space `' 4'` is 4, and `2+3` is 5.
    /// zshrs previously used `str::parse`, silently yielding width 0 for any
    /// non-decimal arg. FIXED: the `*` width path now calls parse_int_arg.
    #[test]
    fn star_width_math_evaluated() {
        assert_parity("printf '%*d|%-*d|%*d|\\n' 0x1f 5 0x4 5 2+3 5");
    }

    /// The `*` PRECISION arg is math-evaluated identically (builtin.c:5276).
    /// `%.*d` with a hex precision `0x3` zero-pads to 3 digits.
    #[test]
    fn star_precision_math_evaluated() {
        assert_parity("printf '%.*d|%.*f|\\n' 0x3 5 0x2 3.14159");
    }

    /// A MISSING or negative `*` precision arg leaves precision UNSET, not 0
    /// (builtin.c:5178 `prec = -1`, c:5288 `if (prec >= 0)` gates emission —
    /// unlike width whose init is 0). When args run out mid-format the `%d`
    /// keeps default precision and prints `0`; zshrs formerly forced `.0`
    /// and truncated it to empty. FIXED: the `*` precision is emitted only
    /// when its arg exists and is non-negative.
    #[test]
    fn star_precision_missing_arg_unset() {
        assert_parity("printf '] %+x%-0.*d|\\n' '12'");
    }

    /// Same rule via format recycling: a negative `*` precision (`-18`) on
    /// the second cycle is unset, so `%#g` of 0 keeps default precision.
    #[test]
    fn star_precision_negative_unset() {
        assert_parity("printf '->%-#.*g%0b|\\n' 'cafe' 'hello' '%d' -18");
    }
}

// ─────────────────────────────────────────────────────────────────────
// T. Built-in colon-tied special vars (path/fpath/cdpath) — scalar assign.
//
// A SCALAR assignment to a colon-array-tied special var must coerce to a
// 1-element array and sync the tied env var. Array assignment (`path=(...)`)
// and a user `typeset -T` tie both sync correctly; only the built-in special
// vars drop a scalar assignment. Found via the --mode heredoc preamble.
// ─────────────────────────────────────────────────────────────────────
mod special_var_scalar_tie {
    use super::*;

    /// zsh: `PATH=/aa/bb`. FIXED: a scalar assign to a tied colon-array name
    /// now re-derives its tied env scalar (colonarrsetfn reverse cascade,
    /// params.rs) — mirroring the array-assign and element-assign syncs.
    #[test]
    fn path_scalar_assign_syncs_env() {
        assert_parity("PATH=/orig; path=/aa/bb; print -r -- $PATH");
    }

    /// Same tie now works for fpath/cdpath.
    #[test]
    fn fpath_cdpath_scalar_assign_syncs_env() {
        assert_parity("FPATH=/o; fpath=/aa; print -r -- $FPATH; CDPATH=/o; cdpath=/bb; print -r -- $CDPATH");
    }
}

// ─────────────────────────────────────────────────────────────────────
// H. Bare associative-array value-join order
//
// `$assoc` (no subscript) joins its VALUES in zsh hash-BUCKET order — the
// same order `(k)`/`(v)` enumerate. The fusevm GET_VAR fast path sorted the
// keys alphabetically, so `$as` diverged from `${(v)as}` for any key set
// whose bucket order isn't alphabetical.
// ─────────────────────────────────────────────────────────────────────
mod bare_assoc_join_order {
    use super::*;

    /// zsh: `9 1 5` (bucket order, == `${(v)as}`). zshrs sorted → `1 5 9`.
    /// FIXED: GET_VAR now uses the bucket-ordered `assoc_get`, so `$as`
    /// matches `${(v)as}`. Under KSH_ARRAYS the bare form is the bucket-FIRST
    /// value (`9`), also fixed by the same change.
    #[test]
    fn bare_assoc_uses_bucket_order() {
        assert_parity(
            "typeset -A as; as=(zebra 9 apple 1 mango 5); \
             print -r -- $as; print -r -- ${(v)as}; \
             setopt KSH_ARRAYS; print -r -- $as",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// L. Logical `&&` / `||` truncate a FLOAT operand for the truth test.
//
// zsh's short-circuit prologue (math.c:1461 `bop`) computes the truth of an
// operand as `(spval->type & MN_FLOAT) ? (zlong)spval->u.d : spval->u.l` — a
// float is CAST to integer, so `0.5` → 0 → falsy (unlike `!0.5`/`0.5?:` which
// use raw float truth). The Rust port compared the float against 0.0, making
// `0.5` truthy; `0.5 || (2+3)` then short-circuited and set noeval on the RHS,
// and a COMPOUND RHS under noeval collapses to 0 → wrong result 0.
// FIXED: bop truncates a float to i64 before the truth test (math.rs).
// ─────────────────────────────────────────────────────────────────────
mod logical_float_truncation {
    use super::*;

    /// `0.5` truncates to 0 (falsy), so `||` MUST evaluate the compound RHS.
    /// zsh: `1`.  zshrs formerly short-circuited and printed `0`.
    #[test]
    fn or_float_lhs_evaluates_compound_rhs() {
        assert_parity("print -r -- $(( 0.5 || (2+3) ))");
    }

    /// A float whose truncation is nonzero (`2.5` → 2) still short-circuits;
    /// an exact-zero float (`0.0`) evaluates the RHS. Both already agreed —
    /// pinned so the fix can't over-correct.
    #[test]
    fn or_float_truncation_boundaries() {
        assert_parity("print -r -- $(( 0.5 || 0 )):$(( 2.5 || (2+3) )):$(( 0.0 || (2+3) ))");
    }

    /// `&&` uses the same truncated truth: `0.5` → 0 → short-circuit false.
    #[test]
    fn and_float_lhs_truncates() {
        assert_parity("print -r -- $(( 0.5 && (2+3) )):$(( 1.9 && (1+1) ))");
    }

    /// The full nested-ternary reproducer the fuzzer surfaced (`(-16)**-7` is a
    /// tiny nonzero float that truncates to 0 inside the `||`).  zsh: `1`.
    #[test]
    fn nested_ternary_float_power_or() {
        assert_parity(
            "neg=-7; print -r -- \"$(( (((16#56) && (16#e5)) ? ((neg) || (16#fd)) : 7) \
             ? (((-16) ** (neg)) || (2#110 ^ 0x81 + 1)) : 0 ))\"",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// N. nounset error inside a `[[ … ]]` operand preserves the cond result.
//
// Under `setopt NO_UNSET`, referencing an unset parameter (or an out-of-range
// array subscript) raises a "parameter not set" zerr that sets errflag. In a
// REGULAR command that aborts the command with status 1 (both shells agree).
// But inside `[[ … ]]`, zsh's evalcond (c:Src/cond.c:196-208 cond_subst →
// singsub, then c:254 matheval/test) NEVER checks errflag — the operand
// expands to empty, the test still evaluates (`-z ""` → true), and the `[[ ]]`
// returns 0. The errflag then aborts only the FOLLOWING commands; the shell's
// exit status is the conditional's own result (0), not a forced 1.
//
// zshrs diverged: its nounset sites call set_last_status(1) on the executor
// (extra to C — subst.c:1689 raises the zerr but never touches lastval), and
// the cond's Op::SetStatus updated only vm.last_status, so the executor lastval
// that BUILTIN_ERREXIT_CHECK reads stayed at the nounset site's transient 1.
// FIXED (fusevm_bridge.rs BUILTIN_COND_STATUS_FROM_BOOL): the conditional now
// syncs its result to the executor's lastval before the abort check, mirroring
// c:Src/exec.c:5216 `lastval = evalcond(...)`. Regular commands still yield 1
// via their own command-abort path (no cond runs), so the fix is cond-scoped.
// ─────────────────────────────────────────────────────────────────────
mod nounset_in_conditional {
    use super::*;

    /// `-z` of the empty out-of-range subscript is true → cond 0; errflag
    /// aborts nothing after it.  zsh: exit 0. zshrs formerly forced 1.
    #[test]
    fn zbracket_out_of_range_subscript_preserves_cond_status() {
        assert_parity("arr=(x1 x2 y3); setopt NO_UNSET; [[ -z ${arr[99]} ]]; print done");
    }

    /// `-n` of the empty operand is false → cond 1. Pins that the sync uses
    /// the real cond result, not a blanket 0.
    #[test]
    fn zbracket_nounset_false_cond_is_one() {
        assert_parity("arr=(x1 x2 y3); setopt NO_UNSET; [[ -n ${arr[99]} ]]; print done");
    }

    /// A bare unset scalar in `[[ ]]` behaves the same; `== ""` → true → 0.
    #[test]
    fn zbracket_unset_scalar_streq_empty() {
        assert_parity("setopt NO_UNSET; [[ ${UNSET} == \"\" ]]; echo AFTER");
    }

    /// A REGULAR command with an unset operand under NO_UNSET still aborts
    /// with status 1 (the fix is cond-scoped, not a global nounset change).
    #[test]
    fn regular_command_unset_still_aborts_one() {
        assert_parity("setopt NO_UNSET; print $UNSET; echo AFTER");
    }

    /// `${arr[@]}` on a genuinely UNSET array under NO_UNSET is a "parameter
    /// not set" error (exit 1), like `$arr` / `${arr[*]}` / `${arr[1]}`. The
    /// `[@]` splat path (BUILTIN_ARRAY_ALL) formerly returned an empty array
    /// silently (exit 0). FIXED: the truly-unset branch now fires the nounset
    /// zerr (c:Src/subst.c:3480-3485). Found by the nested-state fuzzer.
    #[test]
    fn unset_array_at_splat_errors() {
        assert_parity("setopt NO_UNSET; print -r -- \"${arr[@]}\"");
    }

    /// A DECLARED-but-empty array is set, so `${arr[@]}` splats to nothing
    /// WITHOUT erroring even under NO_UNSET — pin so the fix can't over-fire.
    #[test]
    fn declared_empty_array_at_splat_no_error() {
        assert_parity("setopt NO_UNSET; typeset -a arr; print -r -- \"${arr[@]}\"; echo AFTER");
    }
}

// ─────────────────────────────────────────────────────────────────────
// K3. KSH_ARRAYS: a bare `${(flags)arr}` collapses to element 1 before flags.
//
// Under `setopt KSH_ARRAYS`, a bare array reference with NO explicit `[@]`/`[*]`
// subscript is element 1 only — the same collapse `$arr` and `${#arr}` already
// do. A parameter-flag transform therefore operates on that SINGLE element:
//   ${(j:-:)parts}  → `a`   (join of one element, not `a-b-c`)
//   ${(o)parts}     → `a`   (sort of one element, not `a b c`)
//   ${(U)parts}     → `A`   (upcase of one element, not `A B C`)
// An explicit subscript (`${(j:-:)parts[@]}`) opts back into the whole array
// and already works, as do the flag-less `$parts` / `${#parts}` forms. zshrs
// applies the flag to the FULL array because the (j)/(o)/(U) join/sort/case
// path fetches the array independently of the KSHARRAYS bare-name collapse
// (the collapse point mirroring the ksh_bare_assoc arm in subst.rs did not
// intercept this active join path). Found by the nested-state fuzzer under a
// `setopt KSH_ARRAYS` prefix. Baselined in .github/fuzz-baseline/stateful.txt.
// ─────────────────────────────────────────────────────────────────────
mod ksh_arrays_bare_flag_collapse {
    use super::*;

    /// zsh: `a` (join of the single element-1 collapse). zshrs: `a-b-c`.
    #[test]
    #[ignore = "zshrs gap: KSH_ARRAYS bare ${(j)arr} joins full array; zsh collapses to element 1"]
    fn join_flag_collapses_to_first_element() {
        assert_parity("setopt KSH_ARRAYS; parts=(a b c); print -r -- ${(j:-:)parts}");
    }

    /// The explicit-subscript form already works — pin so a fix can't regress.
    #[test]
    fn explicit_at_subscript_keeps_full_array() {
        assert_parity("setopt KSH_ARRAYS; parts=(a b c); print -r -- ${(j:-:)parts[@]}");
    }

    /// KSH_ARRAYS + RC_EXPAND_PARAM: a bare `$acc` is still element 1 (a
    /// scalar), so RC_EXPAND_PARAM has a single value — `p1`, not the whole
    /// array. FIXED: the rc_expand whole-array shortcut in get_var_impl is
    /// gated on !KSHARRAYS so the element-1 collapse still applies. Found by
    /// the nested-state fuzzer (nested loops accumulating into `acc`).
    #[test]
    fn rc_expand_param_respects_ksh_collapse() {
        assert_parity(
            "setopt KSH_ARRAYS RC_EXPAND_PARAM; acc=(); \
             for a in p q; do for b in 1 2; do acc+=($a$b); done; done; print -r -- $acc",
        );
    }

    /// RC_EXPAND_PARAM without KSH_ARRAYS still distributes element-wise —
    /// pin so the KSH gate doesn't disable the option in the common case.
    #[test]
    fn rc_expand_param_still_distributes_without_ksh() {
        assert_parity("setopt RC_EXPAND_PARAM; a=(1 2 3); print -r -- pre${a}post");
    }
}

// ─────────────────────────────────────────────────────────────────────
// S1. Scalar subscript splice on a MULTIBYTE value must not panic.
//
// `a[i]=X` on a scalar splices at CHARACTER position i (c:params.c:2748+). The
// Rust port byte-sliced the value string at the char index, panicking (shell
// crash) when a multibyte codepoint straddled the offset — p10k's
// `_p9k_get_icon` hit it on the Powerline glyph U+E0B0. FIXED: the splice
// operates on the char sequence. (Real p10k crash, not fuzzer-found.)
// ─────────────────────────────────────────────────────────────────────
mod scalar_splice_multibyte {
    use super::*;

    #[test]
    fn splice_into_multibyte_scalar_no_panic() {
        assert_parity("a=$'\\ue0b0'; a[1]=X; print -r -- $a");
    }

    #[test]
    fn splice_replaces_correct_multibyte_char() {
        assert_parity("a=\u{3b1}\u{3b2}\u{3b3}\u{3b4}; a[2,3]=XY; print -r -- $a");
    }
}

// ─────────────────────────────────────────────────────────────────────
// S2. A RAW subscript passed to setsparam (read/sysread target) must be
// parameter-expanded before arithmetic evaluation.
//
// `read 'a[$#a+1]'` / `sysread 'pgid[$#pgid+1]'` hand assignsparam a raw,
// unexpanded subscript string (the command compiler never pre-expands the
// target). C's getarg singsub's it (c:params.c:2058/1592) before arith; the
// Rust port ran mathevalarg on the literal `$#a+1`, which yielded 0 →
// "assignment to invalid subscript range". gitstatus's daemon-pid read loop
// hit this on every prompt. FIXED: singsub the subscript first.
// ─────────────────────────────────────────────────────────────────────
mod raw_subscript_param_expansion {
    use super::*;

    /// The exact gitstatus idiom: grow a scalar one char at a time via
    /// `read` into `p[$#p+1]`.  zsh: `abc`.  zshrs: errored on the first read.
    #[test]
    fn read_into_growing_scalar_subscript() {
        assert_parity(
            "p=; for c in a b c; do print -n $c | IFS= read -r 'p[$#p+1]'; done; print -r -- $p",
        );
    }

    /// A plain `$n` subscript and a `${#a}` form both expand.
    #[test]
    fn read_into_var_subscript() {
        assert_parity("a=xx n=1; print Y | IFS= read -r 'a[$n+1]'; print -r -- $a");
    }
}

// ─────────────────────────────────────────────────────────────────────
// G2. `$~pat` / `${~pat}` with a literal PREFIX must glob the whole word.
//
// The `~` flag promotes a value's glob metachars to a pattern (c:subst.c:2596
// `globsubst = 2`). With a prefix in the same word (`$dir/$~pat`), the WHOLE
// assembled word `$dir/<pat>` is filename-generated. zshrs globbed only the
// `$~pat` segment in isolation (in the CWD), dropping the `$dir/` prefix →
// wrong matches / "no matches found". p10k's `_p9k_glob`
// (`$dir/$~2(...N:t)`) and zinit relied on this. FIXED: the sub-segment
// fast paths defer to the parent word's assembled-scalar glob.
// ─────────────────────────────────────────────────────────────────────
mod glob_subst_prefix {
    use super::*;

    /// Build a fixture dir and glob `$d/$~pat` under a relative prefix (a
    /// subdir), so the compared output is deterministic (no absolute temp
    /// path). The prefix `sub/` must survive into the glob.
    #[test]
    fn tilde_glob_flag_with_prefix() {
        assert_parity(
            "t=$(mktemp -d); cd $t; mkdir sub; touch sub/foo.txt sub/bar.txt; \
             pfx=sub pat='*.txt'; print -rl -- $pfx/$~pat | sort; cd /; rm -rf $t",
        );
    }

    /// The exact p10k `_p9k_glob` idiom: `eval` + `$dir/$~pat(N:t)`.
    #[test]
    fn p9k_glob_eval_idiom() {
        assert_parity(
            "t=$(mktemp -d); touch $t/a.md $t/b.md; d=$t pat='*.md'; \
             eval 'f=($d/$~pat(N:t))'; print -rl -- $f | sort; cd /; rm -rf $t",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// F1. `${(M)arr:#${var}}` — the (M) keep-matching flag survives a BRACED
// nested expansion in the filter pattern.
//
// The `:#pat` filter singsub's its pattern; a braced `${var}` there recurses
// through paramsubst, which resets the SHARED sub_flags — clobbering SUB_MATCH
// if it's read AFTER the singsub. So `${(M)B:#${s}}` dropped the (M) inversion
// and returned the NON-matching elements. An unbraced `$s` didn't recurse, so
// it worked. This broke vcs_info_setsys's backend dedup (`${(M)VCS_INFO_backends
// :#${sys}}`) so only 1 of 12 backends registered → "unknown backend git".
// FIXED: SUB_MATCH is captured before the pattern singsub (subst.rs).
// ─────────────────────────────────────────────────────────────────────
mod sub_match_nested_pattern {
    use super::*;

    /// zsh: `` (empty — `a` doesn't match `b`, (M) keeps only matching).
    /// zshrs formerly returned `a` (dropped the (M) inversion).
    #[test]
    fn keep_match_flag_with_braced_pattern() {
        assert_parity("B=(a); s=b; print -r -- \"[${(M)B:#${s}}]\"");
    }

    /// Array-context filter with a braced pattern keeps the matching element
    /// (this is what the SUB_MATCH-capture fix restores). zsh: `a`.
    #[test]
    fn keep_match_array_context_braced() {
        assert_parity("B=(a b c); s=a; print -rl -- ${(M)B:#${s}}");
    }

    /// The vcs_info accumulation idiom over UNIQUE values (its real input):
    /// each `${(M)arr:#${x}}` dedup check finds nothing yet, so all register.
    #[test]
    fn accumulate_unique_via_keep_match_braced() {
        assert_parity(
            "typeset -ga B; B=(); for s in bzr git hg svn; do \
             [[ -n ${(M)B:#${s}} ]] && continue; B+=($s); done; print -r -- $#B: $B",
        );
    }

    /// Plain-pattern (M):# and drop-mode :# both still work — pin the split.
    #[test]
    fn keep_and_drop_plain_patterns() {
        assert_parity("a=(one two three); print -r -- ${(M)a:#t*}:${a:#t*}");
    }
}
