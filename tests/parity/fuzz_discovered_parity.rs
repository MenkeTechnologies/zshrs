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

// ─────────────────────────────────────────────────────────────────────
// L. `$((…))` output radix survives a NESTED evaluation
//
// c:Src/math.c:1486 — `if (!mlevel) outputradix = outputunderscore = 0;`.
// The radix is cleared only at TOP level; the comment above it in the C source
// says the values are deliberately "maintain[ed] … across levels of
// evaluation". The port had the reset at the top of `mathevall`, which is
// re-entered for EVERY nesting level — including the recursive re-evaluation of
// a scalar parameter whose value is itself a math expression. So reading `j`
// (value `8#62`) wiped the `[#36]` the caller had just set.
// ─────────────────────────────────────────────────────────────────────
mod output_radix_across_eval_levels {
    use super::*;

    /// The gap: the operand's value is a based literal, so evaluating it
    /// recurses into the evaluator. zsh: `36#1E`; zshrs printed `50`.
    #[test]
    fn radix_survives_param_whose_value_is_a_based_literal() {
        assert_parity(r#"j=8#62; print -r -- "$(( [#36] j ))""#);
    }

    /// Same shape reached through an arithmetic assignment.
    #[test]
    fn radix_survives_after_arith_assignment_of_based_literal() {
        assert_parity(
            r#"print -r -- "$(( j = 8#62 ))"; print -r -- "$(( [#36] 0 ** ((j & big) & 3) ))""#,
        );
    }

    /// The reset must still happen at TOP level: a `[#16]` must not leak into
    /// the NEXT `$((…))`. This is the behaviour the misplaced reset protected,
    /// so pin it alongside the fix.
    #[test]
    fn radix_does_not_leak_between_top_level_expressions() {
        assert_parity(r#"print -r -- "$(( [#16] 255 ))"; print -r -- "$(( 255 ))""#);
    }

    /// A subscript evaluation between the two must not disturb it either.
    #[test]
    fn radix_reset_survives_an_intervening_subscript() {
        assert_parity(
            r#"print -r -- "$(( [#16] 255 ))"; a=(x y); print -r -- "$a[2]"; print -r -- "$(( 12 ))""#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// M. An out-of-range ARITHMETIC subscript must yield an EMPTY array, not the
//    whole backing array
//
// c:Src/params.c:2548 `getarrvalue` ALWAYS returns an array; an out-of-range or
// inverted slice returns an EMPTY one (c:2573-2578 `arrdup_max(nular, 0)`).
// zshrs resolved the bounds correctly but, on the empty result, returned a bare
// empty string without setting `split_parts` (C's `aval`) — indistinguishable
// downstream from "no subscript at all", so a `(j:…:)` sepjoin re-fetched and
// joined the WHOLE array. Only a NON-literal bound took that path, which is why
// the shape real code writes — `"${(j:,:)argv[OPTIND,-1]}"` — was the one that
// broke.
// ─────────────────────────────────────────────────────────────────────
mod oob_arithmetic_subscript_is_empty {
    use super::*;

    /// Slice whose start is past the end, via a variable. zsh: empty.
    #[test]
    fn slice_past_end_via_variable_is_empty() {
        assert_parity(r#"a=(x y); i=3; print -r -- "(${(j:,:)a[i,-1]})""#);
    }

    /// The getopts idiom this actually broke: after consuming every option,
    /// `$@[OPTIND,-1]` is the (empty) remainder.
    #[test]
    fn getopts_remainder_slice_is_empty_when_all_args_consumed() {
        assert_parity(
            "set -- -c\nwhile getopts ab:c opt; do print -r -- \"opt=$opt\"; done\n\
             print -r -- \"rest=(${(j:,:)@[OPTIND,-1]})\"",
        );
    }

    /// A single out-of-range ELEMENT subscript is empty too, not element 1.
    #[test]
    fn element_past_end_via_variable_is_empty() {
        assert_parity(r#"a=(x y z); i=9; print -r -- "(${(j:,:)a[i]})""#);
    }

    /// `(j:…:)` over a single-element subscript joins THAT element — it must not
    /// re-fetch the backing array. zsh: `y`; zshrs printed `x,y,z`.
    #[test]
    fn join_flag_over_single_element_subscript() {
        assert_parity(r#"a=(x y z); print -r -- "(${(j:,:)a[2]})""#);
    }

    /// In-range slices/elements and the whole-array joins must be unchanged.
    #[test]
    fn in_range_subscripts_and_whole_array_joins_unchanged() {
        assert_parity(
            r#"a=(x y z); i=2; print -r -- "(${(j:,:)a[i,-1]})(${(j:,:)a[1,2]})(${(j:,:)a})(${(j:,:)a[@]})""#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// N. Special parameters backed by a process GLOBAL must not leak out of a
//    subshell
//
// `IFS`, `WORDCHARS`, `HOME`, `HISTSIZE`, … live in a C global reached through a
// GSU (`Src/params.c`'s `char *ifs`), not in the parameter table. C forks for
// `( … )`, so a child's writes to those globals die with it. zshrs runs
// subshells IN-PROCESS and snapshots the param table — which restores the param
// NODE but not the global behind it. So `(IFS=,; :)` left the PARENT's IFS as
// `,` and every later word-split in the parent silently used it.
// ─────────────────────────────────────────────────────────────────────
mod subshell_special_param_isolation {
    use super::*;

    /// The gap. zsh restores the default `space/tab/newline/nul`.
    #[test]
    fn ifs_assignment_does_not_escape_a_subshell() {
        assert_parity(r#"(IFS=,; :); printf "%q\n" "$IFS""#);
    }

    /// The leak was FUNCTIONAL, not cosmetic: the parent's splitting changed.
    #[test]
    fn parent_word_splitting_unaffected_by_subshell_ifs() {
        assert_parity("x=$'a\\nb'; (IFS=,; :); print -rl -- ${=x}; print END");
    }

    /// Same class, other globals.
    #[test]
    fn wordchars_and_histsize_do_not_escape_a_subshell() {
        assert_parity(r#"(WORDCHARS=xyz; HISTSIZE=99; :); print -r -- "$WORDCHARS $HISTSIZE""#);
    }

    /// IFS must still take effect INSIDE the subshell.
    #[test]
    fn ifs_still_applies_within_the_subshell() {
        assert_parity(r#"(IFS=,; s="a,b,c"; print -rl -- ${=s}); print END"#);
    }

    /// An UNSET special must not be resurrected by the restore.
    #[test]
    fn unset_special_stays_unset_after_subshell() {
        assert_parity(r#"unset IFS; (IFS=,; :); print -r -- "[${IFS-UNSET}]""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// O. `[[ … ]]` operands are NOT brace-expanded
//
// c:Src/subst.c:170 — `if (unset(IGNOREBRACES) && !(flags & PREFORK_SINGLE))`
// gates `xpandbraces`. Cond operands reach prefork with PREFORK_SINGLE
// (cond.c:53 `singsub` → subst.c:520 `prefork(&foo, PREFORK_SINGLE, NULL)`), so
// C never brace-expands them. zshrs did: an ERE bound `a{2,3}` became `a2 a3`
// (so the regex stopped matching), and `[[ -n a{2,3} ]]` split ONE operand into
// two words.
// ─────────────────────────────────────────────────────────────────────
mod cond_operands_are_not_brace_expanded {
    use super::*;

    /// The gap: an ERE interval on the `=~` RHS. zsh: rc=0.
    #[test]
    fn ere_bound_survives_on_regex_rhs() {
        assert_parity("[[ aaa =~ a{2,3} ]]; print rc=$?");
    }

    /// Open-ended bound, and an anchored one.
    #[test]
    fn ere_open_bound_and_anchored() {
        assert_parity("[[ aaa =~ a{2,} ]]; print rc=$?; [[ aaa =~ ^a{2,3}$ ]]; print rc=$?");
    }

    /// A braced word stays ONE operand — as a unary operand and on both sides
    /// of a comparison.
    #[test]
    fn braced_word_stays_one_operand() {
        assert_parity("[[ -n a{2,3} ]]; print rc=$?; [[ a{2,3} == a{2,3} ]]; print rc=$?");
    }

    /// Capture groups with counted repeats still populate $match.
    #[test]
    fn ere_bounds_with_capture_groups() {
        assert_parity(
            r#"[[ 2024-01-31 =~ ([0-9]{4})-([0-9]{2}) ]]; print -r -- "$match[1]/$match[2]""#,
        );
    }

    /// Brace expansion everywhere ELSE must be untouched.
    #[test]
    fn brace_expansion_outside_cond_is_unchanged() {
        assert_parity("a=(x{1,2}); print -r -- ${a[@]}; print -r -- pre{x,y}post; echo {1..3}");
    }
}

// ─────────────────────────────────────────────────────────────────────
// P. `read -d DELIM` — EOF status and backslash handling
//
// C has ONE read loop; `-d` only swaps the `delim` variable, so the backslash
// rules (c:7041-7057) and the EOF status (c:7116 `else if (c == EOF) return 1`)
// apply to `-d` exactly as to a plain `read`. zshrs had a separate `-d` loop
// that returned 1 only when NOTHING was read and never processed backslashes.
// The status is what makes `while read -d : f` terminate on a final field with
// no trailing delimiter.
// ─────────────────────────────────────────────────────────────────────
mod read_delim_eof_and_backslash {
    use super::*;

    /// EOF before the delimiter: the value IS assigned (c:7107) but the status
    /// is 1 (c:7116). zsh: `[abc] rc=1`.
    #[test]
    fn eof_before_delim_assigns_but_returns_one() {
        assert_parity(r#"read -d : x <<< 'abc'; print -r -- "[$x] rc=$?""#);
    }

    /// Delimiter found → status 0.
    #[test]
    fn delim_found_returns_zero() {
        assert_parity(r#"read -d : x <<< 'ab:c'; print -r -- "[$x] rc=$?""#);
    }

    /// The loop this status drives: the last field has no trailing delimiter.
    #[test]
    fn read_delim_loop_consumes_final_unterminated_field() {
        assert_parity(
            "printf 'a:b:c' | { while read -d : f; do print -r -- \"f=$f\"; done; \
             print -r -- \"last=$f\"; }",
        );
    }

    /// Without -r a backslash escapes the next byte (c:7055-7057).
    #[test]
    fn backslash_is_processed_without_dash_r() {
        assert_parity(r#"read -d ' ' x <<< 'esc\ttab'; print -r -- "[$x] rc=$?""#);
    }

    /// A backslash-escaped DELIMITER is a continuation: both bytes vanish and
    /// the record keeps going (c:7041-7043). zsh: `[esc] rc=0`.
    #[test]
    fn escaped_delimiter_is_a_continuation() {
        assert_parity(r#"read -d 't' x <<< 'esc\ttab'; print -r -- "[$x] rc=$?""#);
    }

    /// `-r` keeps the backslash literal.
    #[test]
    fn raw_mode_keeps_backslash() {
        assert_parity(r#"read -rd ' ' x <<< 'esc\ttab'; print -r -- "[$x] rc=$?""#);
    }

    /// The NUL-delimiter idiom (`find -print0 | while read -d ''`) still works.
    #[test]
    fn nul_delimiter_loop_unchanged() {
        assert_parity("printf 'p1\\0p2\\0' | { while read -d '' p; do print -r -- \"p=$p\"; done; }");
    }
}

// ─────────────────────────────────────────────────────────────────────
// I. An unquoted multibyte word was split apart as if it were whitespace
//
// The lexer walks `char`s, but the blank/digit/ident classifiers are BYTE
// tables (C's lexer walks bytes, where no UTF-8 byte can be 0x20 or 0x09).
// Casting a `char` to `u8` truncated the codepoint into those ranges, so
// `三` (U+4E09 → 0x09 = TAB) and `丠` (U+4E20 → 0x20 = SPACE) terminated the
// word: `print -r -- 三` printed NOTHING, and `v=三a` split into an
// assignment plus a stray command `a`. Only ASCII can be a blank/digit.
// ─────────────────────────────────────────────────────────────────────
mod multibyte_word_lexing {
    use super::*;

    /// U+4E09's low byte is 0x09 (TAB) — the original repro. zsh: `三`.
    #[test]
    fn cjk_word_with_tab_low_byte_survives() {
        assert_parity("print -r -- 三");
    }

    /// U+4E20's low byte is 0x20 (SPACE).
    #[test]
    fn cjk_word_with_space_low_byte_survives() {
        assert_parity("print -r -- 丠");
    }

    /// The word was truncated, not just dropped: `v=三a` must stay one word.
    #[test]
    fn multibyte_does_not_terminate_an_assignment_word() {
        assert_parity(r#"v=三a; print -r -- "[$v]""#);
    }

    /// A codepoint whose low byte is an ASCII digit must not read as an fd.
    #[test]
    fn multibyte_is_not_a_redirection_fd() {
        assert_parity(r#"v=丰; print -r -- "[$v]""#);
    }

    /// Several such words in one list.
    #[test]
    fn multibyte_words_split_on_real_blanks_only() {
        assert_parity("for x in 三 丠 二; do print -r -- \"<$x>\"; done");
    }
}

// ─────────────────────────────────────────────────────────────────────
// J. (V) rendered a CHARACTER through the BYTE renderer
//
// C's (V) calls nicedupstring → mb_niceformat (Src/utils.c:5366), which
// decodes the string and tests each WIDE char with iswprint, passing printable
// multibyte through untouched; only bytes that fail to decode fall back to the
// `&0xff` `\M-`/`^X` renderer. zshrs called nicechar per char, so the codepoint
// was masked to a byte: `一`(U+4E00) → `^@`, `二`(U+4E8C) → `\M-^L`,
// `é`(U+00E9) → `\M-i`.
// ─────────────────────────────────────────────────────────────────────
mod visible_flag_multibyte {
    use super::*;

    /// Wide CJK is printable — passes through verbatim.
    #[test]
    fn wide_chars_pass_through() {
        assert_parity(r#"w=一二三; print -r -- "${(V)w}""#);
    }

    /// Latin-1 range (0x80..=0xFF) was the arm that masked to a byte.
    #[test]
    fn latin1_chars_pass_through() {
        assert_parity(r#"v=éöü; print -r -- "${(V)v}""#);
    }

    /// A genuinely undecodable byte STILL renders as `\M-x`.
    #[test]
    fn undecodable_byte_still_renders_meta() {
        assert_parity(r#"v=$'\M-a'; print -r -- "${(V)v}""#);
    }

    /// Control characters keep their named / caret forms.
    #[test]
    fn controls_keep_named_forms() {
        assert_parity(r#"v=$'a\tb\x01'; print -r -- "${(V)v}""#);
    }

    /// The (q)-flag's internal Snull marker must not leak into (V) output.
    #[test]
    fn quote_flag_marker_does_not_leak() {
        assert_parity(r#"print -r -- "${(Vq)empty}""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// K. quotestring: $'…' must escape what it cannot re-parse
//
// C walks the string with MB_METACHARLENCONV; anything that does not decode
// (WEOF) or is not printable goes byte-by-byte through addunprintable()
// (Src/utils.c:6082): a named escape or a 3-digit OCTAL one. zshrs walked
// `.chars()` and re-emitted the raw byte, producing a $'…' string that could
// not be read back. Two further C details: addunprintable has NO `\e` case (ESC
// is `\033`), and QT_DOLLARS backslashes the history char under BANGHIST.
// ─────────────────────────────────────────────────────────────────────
mod dollar_quote_escaping {
    use super::*;

    /// The byte 0xE1 is not valid UTF-8 — it must come out as `$'\341'`.
    #[test]
    fn undecodable_byte_becomes_octal() {
        assert_parity(r#"v=$'\M-a'; print -rn -- ${(qqqq)v} | od -An -tx1"#);
    }

    /// Same rule via the (q) backslash mode.
    #[test]
    fn undecodable_byte_becomes_octal_under_backslash_mode() {
        assert_parity(r#"v=$'\M-a'; print -rn -- ${(q)v} | od -An -tx1"#);
    }

    /// C has no `\e` escape — ESC is octal.
    #[test]
    fn esc_is_octal_not_backslash_e() {
        assert_parity(r#"v=$'\e'; print -r -- ${(qqqq)v}"#);
    }

    /// BANGHIST backslashes `!` inside $'…' even non-interactively.
    #[test]
    fn bang_is_escaped_in_dollar_quotes() {
        assert_parity(r#"v="a!b"; print -r -- ${(qqqq)v}"#);
    }

    /// Printable multibyte is NOT escaped.
    #[test]
    fn printable_multibyte_is_not_escaped() {
        assert_parity(r#"v=é; print -r -- ${(qqqq)v}"#);
    }

    /// Named escapes still win over octal for the ones C names.
    #[test]
    fn named_escapes_preserved() {
        assert_parity(r#"v=$'a\tb\x01'; print -r -- ${(qqqq)v}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// L. (q-) minimal quoting over-quoted `=` and `~`
//
// C only forces quoting for `=`/`~` when the char is at offset 0, or
// MAGIC_EQUAL_SUBST is set and the previous byte is `=`/`:`, or it is `~` under
// EXTENDED_GLOB (Src/utils.c:6301-6306). The QT_BACKSLASH arm already had that
// gate; QT_SINGLE_OPTIONAL did not, so a mid-word `=` opened a quote span and
// `a=b` came out as `'a=b'`.
// ─────────────────────────────────────────────────────────────────────
mod minimal_quoting_eq_tilde {
    use super::*;

    /// A mid-word `=` needs no quoting.
    #[test]
    fn mid_word_equals_is_bare() {
        assert_parity(r#"v="a=b"; print -r -- ${(q-)v}"#);
    }

    /// Nor does a mid-word `~`.
    #[test]
    fn mid_word_tilde_is_bare() {
        assert_parity(r#"v="a~b"; print -r -- ${(q-)v}"#);
    }

    /// A LEADING `=` still must be quoted (it would be a command substitution).
    #[test]
    fn leading_equals_is_quoted() {
        assert_parity(r#"v="=ab"; print -r -- ${(q-)v}"#);
    }

    /// A leading `~` still must be quoted.
    #[test]
    fn leading_tilde_is_quoted() {
        assert_parity(r#"v="~home"; print -r -- ${(q-)v}"#);
    }

    /// Genuine specials still force a quote span.
    #[test]
    fn spaces_still_quoted() {
        assert_parity(r#"v="a b"; print -r -- ${(q-)v}"#);
    }

    /// Round trip: (Q) undoes (q-).
    #[test]
    fn q_minus_round_trips() {
        assert_parity(r#"v="a=b"; print -r -- ${(Q)${(q-)v}}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// M. zstat must not clobber its target parameter when the stat FAILS
//
// C accumulates into a local buffer and publishes it to the parameter only
// after the loop, and only if every file stat'd cleanly (Src/Modules/stat.c:613
// — `if (ret) freearray(array); else setaparam(...)`). zshrs assigned
// unconditionally, so a failing `zstat -H h <dangling-link>` wiped the assoc a
// previous successful zstat had left in `h`. With -A/-f the first failure also
// BREAKS the loop (c:567) rather than continuing.
// ─────────────────────────────────────────────────────────────────────
mod zstat_failure_preserves_target {
    use super::*;

    /// A failing `-H` must leave the previous assoc contents intact.
    #[test]
    fn failed_hash_stat_does_not_clobber() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_zstat_XXXXXX) || exit 1
print -n x > $d/f
ln -s /nonexistent-target-xyz $d/dangling
zmodload zsh/stat
zstat -H h $d/f
zstat -s -H h $d/dangling 2>/dev/null
print -r -- "n=${#h}"
case $d in (/tmp/pf_zstat_*) command rm -rf -- "$d";; esac"#,
        );
    }

    /// A failing `-A` must leave the previous array intact AND stop the loop.
    #[test]
    fn failed_array_stat_does_not_clobber_and_breaks() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_zstat_XXXXXX) || exit 1
print -n x > $d/f
ln -s /nonexistent-target-xyz $d/dangling
zmodload zsh/stat
a=(PRE)
zstat -A a +mode $d/f $d/dangling $d/f 2>/dev/null
print -r -- "rc=$? a=(${(j:,:)a})"
case $d in (/tmp/pf_zstat_*) command rm -rf -- "$d";; esac"#,
        );
    }

    /// Without -A/-H, a failure still CONTINUES to the remaining files.
    ///
    /// Runs from inside the fixture so the multi-file form's filename prefix is
    /// relative — the mktemp path itself is nondeterministic and must never
    /// reach stdout, or the two shells would "diverge" on the temp name.
    #[test]
    fn plain_stat_continues_past_failure() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_zstat_XXXXXX) || exit 1
print -n x > $d/f
ln -s /nonexistent-target-xyz $d/dangling
builtin cd -q -- $d || exit 1
zmodload zsh/stat
zstat +mode dangling f 2>/dev/null
print -r -- "rc=$?"
builtin cd -q /
case $d in (/tmp/pf_zstat_*) command rm -rf -- "$d";; esac"#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// N. `autoload -U` must parse the loaded body with aliases DISABLED
//
// -U records PM_UNALIASED (Src/builtin.c:3354), and loadautofn copies that bit
// into the global `noaliases` for the duration of the file parse
// (Src/exec.c:5684-5704). zshrs recorded the bit but never consulted it, so a
// body calling `helper` picked up a caller-defined `alias helper=…` — exactly
// what -U exists to prevent, and why every `autoload -Uz` in a plugin framework
// depends on this.
// ─────────────────────────────────────────────────────────────────────
mod autoload_unaliased {
    use super::*;

    /// -U: the body's `helper` resolves to the FUNCTION, not the alias.
    #[test]
    fn dash_u_suppresses_alias_expansion() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_auto_XXXXXX) || exit 1
mkdir -p $d/fns
print -r -- 'helper arg' > $d/fns/af_alias
fpath=($d/fns)
helper() { print -r -- "FUNC $1" }
alias helper='print -r -- HIJACKED'
autoload -Uz af_alias
af_alias
case $d in (/tmp/pf_auto_*) command rm -rf -- "$d";; esac"#,
        );
    }

    /// WITHOUT -U the alias does apply — the flag must actually gate something.
    #[test]
    fn without_dash_u_alias_still_expands() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_auto_XXXXXX) || exit 1
mkdir -p $d/fns
print -r -- 'helper arg' > $d/fns/af_alias
fpath=($d/fns)
helper() { print -r -- "FUNC $1" }
alias helper='print -r -- HIJACKED'
autoload -z af_alias
af_alias
case $d in (/tmp/pf_auto_*) command rm -rf -- "$d";; esac"#,
        );
    }

    /// -U must not otherwise disturb loading: args, $#, repeat calls.
    #[test]
    fn dash_u_body_still_loads_normally() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_auto_XXXXXX) || exit 1
mkdir -p $d/fns
print -r -- 'print -r -- "args=$* n=$#"' > $d/fns/af_args
fpath=($d/fns)
autoload -Uz af_args
af_args a b c
af_args
case $d in (/tmp/pf_auto_*) command rm -rf -- "$d";; esac"#,
        );
    }

    /// Alias expansion must be RESTORED after the autoload parse.
    #[test]
    fn aliases_work_again_after_a_dash_u_autoload() {
        assert_parity(
            r#"d=$(mktemp -d /tmp/pf_auto_XXXXXX) || exit 1
mkdir -p $d/fns
print -r -- 'print -r -- loaded' > $d/fns/af_plain
fpath=($d/fns)
autoload -Uz af_plain
af_plain
alias g='print -r -- ALIAS_OK'
eval g
case $d in (/tmp/pf_auto_*) command rm -rf -- "$d";; esac"#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// O. `=~` used the wrong regex LANGUAGE
//
// C dispatches `[[ =~ ]]` to a module, and which one is an option
// (Src/cond.c:115): `zsh/pcre` under REMATCH_PCRE, else `zsh/regex`, which is
// regcomp(REG_EXTENDED) — POSIX ERE. ERE is a different language from the RE2
// syntax the Rust regex crate speaks: there is no `\d`/`\w`/`\s` (a backslash
// before an ordinary character yields that character, so `\d` matches a literal
// `d`), and `(?…)` is a repetition operator with no operand, i.e. a compile
// error, not a group modifier. zshrs accepted both and so MATCHED text zsh does
// not — and it ignored REMATCH_PCRE entirely, so `setopt rematchpcre` did
// nothing.
// ─────────────────────────────────────────────────────────────────────
mod regex_ere_vs_pcre {
    use super::*;

    /// `\d` is a literal `d` in ERE — this must NOT match.
    #[test]
    fn backslash_d_is_a_literal_d() {
        assert_parity(r#"[[ '2024-06' =~ '(\d{2,4})-(\d{2})' ]] && print -r -- "M=$MATCH" || print -r -- NOMATCH"#);
    }

    /// …and it DOES match an actual run of `d`s.
    #[test]
    fn backslash_d_matches_the_letter_d() {
        assert_parity(r#"[[ 'ddd-06' =~ '(\d+)' ]] && print -r -- "M=$MATCH" || print -r -- NOMATCH"#);
    }

    /// `\w` likewise.
    #[test]
    fn backslash_w_is_a_literal_w() {
        assert_parity(r#"[[ 'user@site.com' =~ '^(\w+)@(\w+)\.com$' ]] && print -r -- "M=$MATCH" || print -r -- NOMATCH"#);
    }

    /// `(?…)` is a compile error in ERE, not a group modifier.
    #[test]
    fn paren_question_is_a_compile_error() {
        assert_parity(r#"[[ 'abc' =~ '(?<w>[a-z]+)' ]] && print -r -- "M=$MATCH" || print -r -- NOMATCH"#);
    }

    /// A real ERE escape keeps its meaning.
    #[test]
    fn escaped_dot_is_still_a_literal_dot() {
        assert_parity(r#"[[ 'a.b' =~ 'a\.b' ]] && print -r -- YES || print -r -- NO"#);
    }

    /// `.` matches newline (regcomp without REG_NEWLINE).
    #[test]
    fn dot_matches_newline() {
        assert_parity("[[ $'a\\nb' =~ 'a.b' ]] && print -r -- YES || print -r -- NO");
    }

    /// REMATCH_PCRE re-points `=~` at PCRE, where `\d` IS a digit class.
    #[test]
    fn rematchpcre_switches_engines() {
        assert_parity(r#"setopt rematchpcre; [[ '2024-06' =~ '(\d{2,4})-(\d{2})' ]] && print -r -- "M=$MATCH m=(${(j:,:)match})" || print -r -- NOMATCH"#);
    }

    /// …and named groups compile under PCRE.
    #[test]
    fn rematchpcre_allows_named_groups() {
        assert_parity(r#"setopt rematchpcre; [[ 'abc123' =~ '(?<word>[a-z]+)' ]] && print -r -- "M=[$MATCH]" || print -r -- NOMATCH"#);
    }

    /// The `[[ ]]` path must honour BASH_REMATCH (it did not, before the
    /// duplicate inline implementation was replaced by a module dispatch).
    #[test]
    fn bash_rematch_array_is_populated() {
        assert_parity(r#"setopt bashrematch; [[ 'ab' =~ '(a)(b)' ]]; print -r -- "[${BASH_REMATCH[1]}][${BASH_REMATCH[2]}]""#);
    }

    /// CASE_MATCH off maps to REG_ICASE.
    #[test]
    fn casematch_off_is_case_insensitive() {
        assert_parity(r#"unsetopt casematch; [[ 'AbC' =~ 'abc' ]] && print -r -- ICASE || print -r -- NO"#);
    }

    /// Captures, $match and $mbegin still come out right.
    #[test]
    fn captures_and_offsets_intact() {
        assert_parity(r#"[[ 'abc123' =~ '([a-z]+)([0-9]+)' ]] && print -r -- "M=$MATCH m=(${(j:,:)match}) b=(${(j:,:)mbegin}) e=(${(j:,:)mend})""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// P. pcre_match: -n is a START OFFSET, not a slice; and captures are the
//    PAIRS SET, not the pattern's group count
//
// C passes the offset to the matcher against the whole subject
// (Src/Modules/pcre.c:381 `pcre2_match(pat, subject, subject_len, offset_start,
// …)`), so `^` still anchors to the true start of the string. zshrs sliced the
// subject, which re-anchored `^` at the new start and reported matches zsh does
// not. Separately, PCRE reports the number of ovector PAIRS SET — the highest
// participating group plus one — so a TRAILING group that did not participate
// is not reported at all.
// ─────────────────────────────────────────────────────────────────────
mod pcre_offset_and_captures {
    use super::*;

    /// `^` still anchors to the string start, so matching from offset 1 fails.
    #[test]
    fn start_offset_does_not_reanchor_caret() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile '^(\w+)@(\w+)\.com$'; pcre_match -n 1 'user@site.com'; print -r -- "rc=$?""#);
    }

    /// A non-anchored pattern still matches from the offset.
    #[test]
    fn start_offset_still_searches() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile 'b'; pcre_match -n 2 'abab'; print -r -- "rc=$?""#);
    }

    /// -b offsets are absolute (relative to the whole subject), under -n too.
    #[test]
    fn offsets_are_absolute_under_start_offset() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile 'b'; pcre_match -b -n 2 'abab'; print -r -- "rc=$? op=[$ZPCRE_OP]""#);
    }

    /// A trailing group that did not participate is not reported.
    #[test]
    fn trailing_unset_group_is_truncated() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile 'x(y)?z'; pcre_match -a arr 'xz'; print -r -- "n=${#arr}""#);
    }

    /// An unset group BEFORE a participating one IS reported, as empty.
    #[test]
    fn leading_unset_group_is_kept_empty() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile '(a)?(b)'; pcre_match -a arr 'b'; print -r -- "n=${#arr} [${(j:,:)arr}]""#);
    }

    /// The ordinary all-groups-participate case is unchanged.
    #[test]
    fn participating_groups_all_reported() {
        assert_parity(r#"zmodload zsh/pcre; pcre_compile '([a-z]+)([0-9]+)'; pcre_match 'abc123'; print -r -- "M=$MATCH m=(${(j:,:)match})""#);
    }
}
