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

// ─────────────────────────────────────────────────────────────────────
// Q. The shell parked its OWN descriptors in the script's fd range
//
// zsh routes every internal descriptor through movefd() (Src/utils.c:1990 —
// `fcntl(fd, F_DUPFD, 10)`), because fds 0-9 belong to the SCRIPT: `exec 3>out`,
// `read -u 4` and `print -u 3` address them by number. zshrs held its log on fd
// 3, the history sqlite on fd 4 and the compsys sqlite on 5-7, and parked
// redirection SAVE descriptors (a plain dup(), which returns the lowest free fd)
// on fd 3 as well. Consequences: `print -u 3 -r -- x` appended to the shell's own
// log and reported SUCCESS where zsh says `bad file number: 3`, and `exec 3>f`
// would dup2 over a live internal handle.
// ─────────────────────────────────────────────────────────────────────
mod internal_fds_stay_out_of_the_script_range {
    use super::*;

    /// fds 3-9 must be closed at startup, exactly as in zsh.
    #[test]
    fn low_fds_are_free_at_startup() {
        assert_parity(
            r#"for f in 3 4 5 6 7 8 9; do print -u $f -rn -- "" 2>/dev/null && print -r -- "fd $f OPEN"; done; print -r -- done"#,
        );
    }

    /// Writing to an unopened fd fails; it must not land in an internal file.
    #[test]
    fn print_to_unopened_fd_fails() {
        assert_parity(r#"print -u 3 -r -- X 2>/dev/null; print -r -- "rc=$?""#);
    }

    /// …including fd 4, which used to be the history database.
    #[test]
    fn print_to_fd_four_fails() {
        assert_parity(r#"print -u 4 -r -- X 2>/dev/null; print -r -- "rc=$?""#);
    }

    /// A redirection on the SAME command must not expose its saved fd as fd 3.
    #[test]
    fn redirection_save_fd_is_not_visible() {
        assert_parity(r#"print -u 3 -r -- X 2>/dev/null; print -r -- "rc=$?""#);
    }

    /// The script's own use of fd 3 still works end to end.
    #[test]
    fn script_can_still_use_fd_three() {
        assert_parity(
            r#"f=$(mktemp /tmp/pf_fd_XXXXXX) || exit 1
exec 3> $f
print -u 3 -r -- hello
exec 3>&-
cat $f
case $f in (/tmp/pf_fd_*) command rm -f -- "$f";; esac"#,
        );
    }

    /// …and reading through an explicitly opened fd.
    #[test]
    fn script_can_read_through_fd_three() {
        assert_parity(
            r#"f=$(mktemp /tmp/pf_fd_XXXXXX) || exit 1
print -l a b > $f
exec 3< $f
read -u 3 x
exec 3<&-
print -r -- "[$x]"
case $f in (/tmp/pf_fd_*) command rm -f -- "$f";; esac"#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// R. $LINENO inside a function is measured from the DEFINITION line
//
// C stamps `shf->lineno` with the line the function was defined on
// (Src/exec.c:5384) and the body's line numbers are relative to it, so a
// one-line `f() { print $LINENO }` reads 0 wherever it sits. zshrs subtracted
// `first_body_line - 1`, which equals the definition line only when the body
// starts on the line AFTER `f() {` — so every function with an INLINE body
// defined below line 1 reported $LINENO one too high.
// ─────────────────────────────────────────────────────────────────────
mod lineno_in_functions {
    use super::*;

    /// The original repro: inline body, function defined on line 2.
    #[test]
    fn inline_body_below_line_one() {
        assert_parity("print -r -- top\nf() { print -r -- \"in=$LINENO\" }; f");
    }

    /// Inline body on line 1 (this one already worked — pin it).
    #[test]
    fn inline_body_on_line_one() {
        assert_parity(r#"f() { print -r -- "in=$LINENO" }; f"#);
    }

    /// A multi-line body counts from the definition line.
    #[test]
    fn multi_line_body_counts_from_def() {
        assert_parity("f() {\n  print -r -- \"a=$LINENO\"\n  print -r -- \"b=$LINENO\"\n}\nf");
    }

    /// A nested definition is relative to its own definition line.
    #[test]
    fn nested_function_definition() {
        assert_parity("outer() {\n  inner() { print -r -- \"i=$LINENO\" }\n  inner\n}\nouter");
    }

    /// Top-level LINENO is unaffected.
    #[test]
    fn top_level_lineno_still_absolute() {
        assert_parity("print -r -- $LINENO\nprint -r -- $LINENO\nprint -r -- $LINENO");
    }
}

// ─────────────────────────────────────────────────────────────────────
// S. Tying an already-exported scalar must PRESERVE the export
//
// C: "Variable already exists in the current scope but is not tied. We're
// preserving its value and export attribute but no other attributes upon
// converting to 'tied'." — Src/builtin.c:2953,
//     on |= (pm->node.flags & ~roff) & PM_EXPORTED;
// zshrs built the tie's attributes from the command-line flags alone, so
// `export E=…; typeset -T E e` silently dropped E out of the environment of
// every later child.
// ─────────────────────────────────────────────────────────────────────
mod tie_preserves_export {
    use super::*;

    /// The export attribute survives the tie.
    #[test]
    fn exported_scalar_stays_exported() {
        assert_parity(r#"export E=x; typeset -T E e; print -r -- "${(t)E}""#);
    }

    /// …and the variable is really still in the environment.
    #[test]
    fn exported_scalar_still_reaches_children() {
        assert_parity(r#"export E=a:b; typeset -T E e; print -r -- "${(t)E} n=${#e}"; printenv E"#);
    }

    /// A fresh (unexported) scalar is NOT exported by the tie.
    #[test]
    fn fresh_scalar_is_not_exported() {
        assert_parity(r#"typeset -T TS ts; TS=a:b; print -r -- "${(t)TS} ${(t)ts}""#);
    }

    /// An explicit +x still wins over the inherited attribute.
    #[test]
    fn explicit_plus_x_removes_export() {
        assert_parity(r#"export E=a:b; typeset -T E e; typeset +x E; print -r -- "${(t)E}""#);
    }

    /// `typeset -xT` exports the scalar only (not the array).
    #[test]
    fn explicit_xT_exports_the_scalar() {
        assert_parity(r#"typeset -xT X x; X=a:b; print -r -- "${(t)X}"; printenv X"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// T. `case` pattern GLOB_SUBST was inverted
//
// In a pattern, a substituted parameter's glob metachars are LITERAL unless
// `$~`/`${~}` or the GLOB_SUBST option forces them active (c:Src/subst.c — the
// singsub split between substituted bytes and source-level metas). `[[ = ]]`
// already got this right; `case` had it backwards — plain `$p` globbed (should
// be literal) and `$~p` was literal (should glob). The two paths now share one
// helper, so they cannot drift again.
// ─────────────────────────────────────────────────────────────────────
mod case_glob_subst {
    use super::*;

    /// Plain `$p` in a case pattern is LITERAL — `a*` matches only "a*".
    #[test]
    fn plain_param_is_literal() {
        assert_parity(r#"p='a*'; case abc in $p) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// …and it matches the literal string.
    #[test]
    fn plain_param_matches_literal() {
        assert_parity(r#"p='a*'; case 'a*' in $p) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// `$~p` forces the value to glob.
    #[test]
    fn tilde_forces_glob() {
        assert_parity(r#"p='a*'; case abc in $~p) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// `${~p}` (braced) forces the glob too.
    #[test]
    fn braced_tilde_forces_glob() {
        assert_parity(r#"p='a*'; case abc in ${~p}) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// GLOB_SUBST makes plain `$p` glob.
    #[test]
    fn globsubst_option_makes_plain_glob() {
        assert_parity(r#"setopt globsubst; p='a*'; case abc in $p) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// A SOURCE-level meta adjacent to a substitution still globs.
    #[test]
    fn source_meta_beside_subst_still_globs() {
        assert_parity(r#"H=foo; case foobar in $H*) print -r -- Y;; *) print -r -- N;; esac"#);
    }

    /// The `[[ = ]]` path stays correct (shared helper regression).
    #[test]
    fn cond_path_still_correct() {
        assert_parity(r#"p='a*'; [[ abc = $p ]] && print -r -- Y || print -r -- N; [[ abc = $~p ]] && print -r -- Y || print -r -- N"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// U. Word modifiers missing from the UNBRACED `$var:mod` form
//
// zsh applies the whole modifier set to a bare parameter, not just a braced
// one. Three modifiers were absent from the unbraced scanner and so were left
// as literal text (`$p:c` stayed `…:c`) while `${p:c}` worked: `:c` (PATH
// search, c:Src/hist.c:863), `:fs` (the `f` "repeat until stable" prefix on
// `:s`, c:hist.c), and `:&` (repeat the last `:s`, c:hist.c:903).
// ─────────────────────────────────────────────────────────────────────
mod unbraced_modifiers {
    use super::*;

    /// `:c` resolves a command through $PATH, unbraced.
    #[test]
    fn c_path_search_unbraced() {
        assert_parity(r#"v=ls; print -r -- $v:c"#);
    }

    /// A non-command value is left unchanged by `:c`.
    #[test]
    fn c_leaves_non_command_alone() {
        assert_parity(r#"v=a:b; print -r -- $v:c"#);
    }

    /// `:c` in an assignment RHS.
    #[test]
    fn c_in_assignment_rhs() {
        assert_parity(r#"v=ls; w=$v:c; print -r -- "[$w]""#);
    }

    /// `:fs` — the `f` repeat prefix, unbraced.
    #[test]
    fn fs_repeat_prefix_unbraced() {
        assert_parity(r#"f=/a/a/c; print -r -- $f:fs/a/Z/"#);
    }

    /// `:fs` vs `:s` — one pass vs repeat-until-stable.
    #[test]
    fn fs_differs_from_single_s() {
        assert_parity(r#"f=/a/a/a; print -r -- "one=$f:s/a/b/ all=$f:fs/a/b/""#);
    }

    /// `:&` — repeat the last substitution, unbraced (quoted so `&` is literal).
    #[test]
    fn ampersand_repeat_unbraced() {
        assert_parity(r#"f=/x/x/z; print -r -- "$f:s/x/Y/:&""#);
    }

    /// The braced spellings still work (regression).
    #[test]
    fn braced_forms_still_work() {
        assert_parity(r#"v=ls; f=/a/a/c; print -r -- "${v:c} ${f:fs/a/Z/}""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// V. SH_WORD_SPLIT must not split (or trim) an assignment RHS
//
// A scalar assignment's value is expanded with PREFORK_SINGLE (c:Src/exec.c:2554
// addvars), so it is NOT word-split even under SH_WORD_SPLIT. zshrs ran the RHS
// through the splitting GET_VAR, which — because `sepsplit(' a:b ')` yields the
// single field `a:b` after IFS-whitespace elision — TRIMMED the surrounding
// spaces (`w=$v` gave `[a:b]` not `[ a:b ]`), and for `export E=$v` with a
// multi-word value it dropped everything after the first word.
// ─────────────────────────────────────────────────────────────────────
mod shwordsplit_assignment {
    use super::*;

    /// Leading/trailing IFS whitespace on the RHS survives.
    #[test]
    fn assignment_rhs_keeps_surrounding_whitespace() {
        assert_parity(r#"setopt shwordsplit; v=' a:b '; w=$v; print -r -- "[$w]""#);
    }

    /// Interior whitespace survives too (it was never the split boundary here).
    #[test]
    fn assignment_rhs_keeps_interior_whitespace() {
        assert_parity(r#"setopt shwordsplit; v='x  y'; w=$v; print -r -- "[$w]""#);
    }

    /// `export NAME=$v` keeps the whole multi-word value.
    #[test]
    fn export_assignment_keeps_all_words() {
        assert_parity(r#"setopt shwordsplit; v='a b'; export E=$v; print -r -- "[$E]""#);
    }

    /// `typeset` / `local` too.
    #[test]
    fn typeset_assignment_keeps_all_words() {
        assert_parity(r#"setopt shwordsplit; v=' a '; typeset E=$v; print -r -- "[$E]""#);
    }

    /// A REGULAR command argument still splits (the fix is scoped to assigns).
    #[test]
    fn command_argument_still_splits() {
        assert_parity(r#"setopt shwordsplit; v='a b'; print $v | wc -w"#);
    }

    /// An ARRAY assignment still splits.
    #[test]
    fn array_assignment_still_splits() {
        assert_parity(r#"setopt shwordsplit; v='a b c'; arr=($v); print -r -- "n=${#arr}""#);
    }

    /// A `for` list still splits.
    #[test]
    fn for_list_still_splits() {
        assert_parity(r#"setopt shwordsplit; v='a b'; for x in $v; do print -r -- "<$x>"; done"#);
    }

    /// With the option OFF the RHS is unchanged (regression pin).
    #[test]
    fn no_split_option_off() {
        assert_parity(r#"v=' a:b '; w=$v; print -r -- "[$w]""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// W. `print -v` dropped the trailing separator when `-l` was set
//
// C captures the trailing separator into the `-v` value too — it is written to
// the same (memory) stream and read back into the buffer — and suppresses it
// only when `-n`, a `\c` escape, OR `-v` WITHOUT `-l` apply (c:Src/builtin.c:544-546).
// So `print -v x -l a b c` stores `a\nb\nc\n` where `print -v x a b c` stores
// `a b c`. zshrs suppressed the terminator for EVERY `-v`, losing the `-l`
// trailing newline.
// ─────────────────────────────────────────────────────────────────────
mod print_v_trailing_sep {
    use super::*;

    /// `-v` with `-l` keeps the trailing newline.
    #[test]
    fn dash_v_dash_l_keeps_trailing_newline() {
        assert_parity(r#"print -v x -l a b c; print -r -- "[$x]""#);
    }

    /// …reflected in the length.
    #[test]
    fn dash_v_dash_l_length() {
        assert_parity(r#"print -v x -l a b c; print -r -- "n=${#x}""#);
    }

    /// `-v` WITHOUT `-l` has no trailing separator.
    #[test]
    fn dash_v_without_l_no_trailing() {
        assert_parity(r#"print -v x a b c; print -r -- "[$x]""#);
    }

    /// `-n` suppresses the trailing newline even with `-l`.
    #[test]
    fn dash_n_suppresses_even_with_l() {
        assert_parity(r#"print -nv x -l a b c; print -r -- "[$x]""#);
    }

    /// A single-arg `-v -l` still gets its terminator.
    #[test]
    fn single_arg_dash_l_terminated() {
        assert_parity(r#"print -v x -l a; print -r -- "[$x]""#);
    }

    /// `printf -v` (no -l) is unaffected.
    #[test]
    fn printf_v_unaffected() {
        assert_parity(r#"printf -v y "%s" hi; print -r -- "[$y]""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// X. Deferred baseline gaps, fixed with C code (round: "all deferred")
//
// typeset attribute merge / type conversion (c:Src/builtin.c:2289,2355-2378,
// 2232-2238); (V) meta-byte render + (Q) bang unescape (c:utils.c:6230);
// whence -v autoload source (c:exec.c:5657); ERR_EXIT + always (c:exec.c:1618);
// =() real temp file (c:exec.c:4906).
// ─────────────────────────────────────────────────────────────────────
mod deferred_fixes {
    use super::*;

    // --- typeset attribute merge / type conversion ---
    #[test]
    fn typeset_merge_keeps_right_zeros_on_attr_add() {
        assert_parity(r#"typeset -Z 3 x=7; typeset -r x; print -r -- "${(t)x}""#);
    }
    #[test]
    fn typeset_type_change_clears_right_zeros() {
        assert_parity(r#"typeset -Z 3 x=7; typeset -i x=1; print -r -- "${(t)x}""#);
    }
    #[test]
    fn typeset_scalar_to_array_clears_padding() {
        assert_parity(r#"typeset -Z 3 x=7; typeset -a x=(1 2); print -r -- "${(t)x}""#);
    }
    #[test]
    fn typeset_assoc_to_float_migrates() {
        assert_parity(r#"typeset -A x=(k v); typeset -F x=1.5; print -r -- "${(t)x}""#);
    }
    #[test]
    fn typeset_scalar_to_existing_arraylike_errors() {
        assert_parity(r#"typeset -A x=(k v); typeset x=s 2>&1; print -r -- "rc=$?""#);
    }
    #[test]
    fn typeset_iZ_single_command_keeps_both() {
        assert_parity(r#"typeset -iZ 5 x=7; print -r -- "${(t)x}""#);
    }

    // --- (V) meta-byte render ---
    #[test]
    fn visible_meta_control_byte() {
        assert_parity(r#"v=$'\M-\C-a'; print -r -- "[${(V)v}]""#);
    }
    #[test]
    fn visible_two_meta_bytes() {
        assert_parity(r#"v=$'\x81\x82'; print -r -- "[${(V)v}]""#);
    }

    // --- (Q) bang unescape roundtrip ---
    #[test]
    fn quote_q_bang_roundtrips() {
        assert_parity(r#"v=a!b; r=${(Q)${(qqqq)v}}; [[ $r == $v ]] && print -r -- ok || print -r -- "BAD[$r]""#);
    }
    #[test]
    fn quote_q_unknown_escape_kept() {
        assert_parity("w=$'a\\qb'; print -r -- \"[${(Q)w}]\"");
    }

    // --- ERR_EXIT + always ---
    #[test]
    fn errexit_try_fail_skips_always() {
        assert_parity(r#"setopt errexit; { false } always { print -r -- A }; print -r -- after"#);
    }
    #[test]
    fn errexit_try_success_runs_always() {
        assert_parity(r#"setopt errexit; { true } always { print -r -- A }; print -r -- after"#);
    }
    #[test]
    fn errexit_in_function_skips_always() {
        assert_parity(r#"setopt errexit; f(){ { false } always { print -r -- A } }; f; print -r -- after"#);
    }
    #[test]
    fn errexit_exit_trap_still_fires() {
        assert_parity(r#"setopt errexit; trap "print -r -- EXITTRAP" EXIT; { false } always { print -r -- A }"#);
    }
    #[test]
    fn errexit_or_guard_runs_always() {
        assert_parity(r#"setopt errexit; { false } always { print -r -- A } || print -r -- OR; print -r -- after"#);
    }

    // --- =() real temp file ---
    #[test]
    fn equals_subst_cat() {
        assert_parity(r#"cat =(print -l a b c)"#);
    }
    #[test]
    fn equals_subst_two_args() {
        assert_parity(r#"cat =(print A) =(print B)"#);
    }
    #[test]
    fn equals_subst_seekable() {
        assert_parity(r#"wc -l < =(print -l a b c d)"#);
    }
    #[test]
    fn process_sub_pipe_unchanged() {
        assert_parity(r#"cat <(print pipe)"#);
    }

    // --- (@) on a SCALAR is a no-op: ${#${(@)scalar}} counts CHARS,
    // not elements (c:subst.c:2915 isarr from scanflags, c:3881 LF_ARRAY
    // tracks isarr). A real array/assoc keeps element-count. ---
    #[test]
    fn at_flag_scalar_len_counts_chars() {
        assert_parity(r#"s=hi; print -r -- "${#${(@)s}}""#);
    }
    #[test]
    fn at_flag_empty_scalar_len_zero() {
        assert_parity(r#"s=; print -r -- "${#${(@)s}}""#);
    }
    #[test]
    fn at_flag_scalar_len_multichar() {
        assert_parity(r#"s=abcde; print -r -- "${#${(@)s}}""#);
    }
    #[test]
    fn at_flag_cmdsubst_scalar_len_counts_chars() {
        assert_parity(r#"print -r -- "${#${(@)$(print hi)}}""#);
    }
    #[test]
    fn at_flag_array_len_counts_elements() {
        assert_parity(r#"a=(x y z); print -r -- "${#${(@)a}}""#);
    }
    #[test]
    fn at_flag_one_elem_array_len_counts_one() {
        assert_parity(r#"a=(only); print -r -- "${#${(@)a}}""#);
    }
    #[test]
    fn at_flag_assoc_len_counts_pairs() {
        assert_parity(r#"typeset -A h; h=(k1 v1 k2 v2); print -r -- "${#${(@)h}}""#);
    }

    // --- (@) with a SINGLE-index subscript picks the element, does not
    // re-splat the whole array (c:params.c:2926 getarrvalue applies the
    // index before nojoin affects joining). ---
    #[test]
    fn at_flag_single_index_subscript() {
        assert_parity(r#"a=(a b c); print -r -- "${(@)a[2]}""#);
    }
    #[test]
    fn at_flag_first_index_subscript() {
        assert_parity(r#"a=(a b c); print -r -- "${(@)a[1]}""#);
    }
    #[test]
    fn at_flag_negative_index_subscript() {
        assert_parity(r#"a=(a b c); print -r -- "${(@)a[-1]}""#);
    }
    #[test]
    fn at_flag_index_subscript_with_surround() {
        assert_parity(r#"a=(a b c); print -r -- "x${(@)a[2]}y""#);
    }
    #[test]
    fn at_flag_search_subscript_single_value() {
        assert_parity(r#"a=(foo bar baz); print -r -- "${(@)a[(r)bar]}""#);
    }
    #[test]
    fn at_flag_range_subscript_still_splats() {
        assert_parity(r#"a=(a b c d); print -rl -- "${(@)a[2,4]}""#);
    }
    #[test]
    fn at_flag_star_subscript_still_splats() {
        assert_parity(r#"a=(a b c); print -rl -- "${(@)a[@]}""#);
    }

    // --- (@) + (o) sort with a single index: the index picks one element
    // BEFORE the sort/splat blocks (isarr=0), so sort is a no-op and the
    // whole array is not re-fetched (c:params.c:2915 + c:4245). ---
    #[test]
    fn at_sort_single_index_picks_element() {
        assert_parity(r#"a=(a b c d e); print -rl -- ${(@o)a[1]}"#);
    }
    #[test]
    fn at_sort_single_index_surrounded() {
        assert_parity(r#"a=(c a b); print -rl -- x${(@o)a[2]}y"#);
    }
    #[test]
    fn at_sort_full_array_still_sorts() {
        assert_parity(r#"a=(c a b); print -rl -- ${(@o)a}"#);
    }

    // --- (@) on a SCALAR char slice `[lo,hi]` is the substring, not an
    // array range on a (nonexistent) indexed array. ---
    #[test]
    fn at_scalar_char_slice() {
        assert_parity(r#"s=hello; print -r -- "${(@)s[1,3]}""#);
    }
    #[test]
    fn at_scalar_char_slice_nested_len() {
        assert_parity(r#"s=hello; print -r -- "${#${(@)s[2,4]}}""#);
    }

    // --- (A) flag (arrasg) forces a scalar into an array (c:subst.c:4235):
    // a non-empty value → 1-element array, an empty value → empty array. ---
    #[test]
    fn a_flag_scalar_becomes_one_elem() {
        assert_parity(r#"s=hi; print -r -- "${#${(A@)s}}""#);
    }
    #[test]
    fn a_flag_empty_scalar_becomes_empty_array() {
        assert_parity(r#"s=; print -r -- "${#${(A@)s}}""#);
    }
    #[test]
    fn a_flag_array_index_one_elem() {
        assert_parity(r#"a=(one two three); print -r -- "${#${(A@)a[2]}}""#);
    }
    #[test]
    fn a_flag_out_of_range_index_empty() {
        assert_parity(r#"s=x; print -r -- "${#${(A@)s[2]}}""#);
    }

    // --- (@) on an assoc with an `[@]` splat subscript splats the VALUES
    // (same as bare `${(@)h}`); the `[@]` is the splat the `(@)` requests. ---
    #[test]
    fn at_assoc_splat_subscript_values() {
        assert_parity(r#"typeset -A h; h=(k1 v1 k2 v2); print -rl -- ${(@)h[@]}"#);
    }
    #[test]
    fn at_assoc_splat_subscript_len() {
        assert_parity(r#"typeset -A h; h=(k1 v1 k2 v2); print -r -- "${#${(@)h[@]}}""#);
    }
    #[test]
    fn at_assoc_splat_subscript_joined() {
        assert_parity(r#"typeset -A h; h=(k1 v1 k2 v2); print -r -- "${(j:,:)${(@)h[@]}}""#);
    }
    #[test]
    fn at_assoc_single_key_unaffected() {
        assert_parity(r#"typeset -A h; h=(k1 v1 k2 v2); print -r -- "${(@)h[k1]}""#);
    }

    // --- `${${(P)assoc}[key]}` — the (P) named-ref makes the outer
    // subscript an assoc KEY lookup on the referenced hash, not an index
    // into the flattened-values temp (c:subst.c (P) aspar). Used by
    // zinit/p10k (`${${(P)mapname}[key]}`). ---
    #[test]
    fn p_indirect_assoc_string_key() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -r - ${${(P)n}[b]}"#);
    }
    #[test]
    fn p_indirect_assoc_var_key() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; k=a; print -r - ${${(P)n}[$k]}"#);
    }
    #[test]
    fn p_indirect_assoc_positional_ref() {
        assert_parity(r#"typeset -A opts=(a 1 b 2); set -- opts; print -r - ${${(P)1}[b]}"#);
    }
    #[test]
    fn p_indirect_assoc_missing_key() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - ${${(P)n}[missing]}"#);
    }
    #[test]
    fn p_indirect_assoc_search_subscript() {
        assert_parity(r#"typeset -A h=(x 10 y 20); n=h; print -r - ${${(P)n}[(R)10]}"#);
    }
    #[test]
    fn p_indirect_assoc_splat() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - "${${(P)n}[@]}""#);
    }
    #[test]
    fn p_indirect_array_index_unaffected() {
        assert_parity(r#"arr=(x y z); n=arr; print -r - ${${(P)n}[2]}"#);
    }

    // --- `${#${subexp}[N]}` — the outer subscript applies BEFORE the
    // length op; the length counts the SUBSCRIPTED value, not the whole
    // inner result (matches the non-nested `${#name[N]}` path). ---
    #[test]
    fn len_subexp_scalar_index() {
        assert_parity(r#"s=hello; print -r -- "${#${s}[2]}""#);
    }
    #[test]
    fn len_subexp_scalar_slice() {
        assert_parity(r#"s=hello; print -r -- "${#${s}[2,4]}""#);
    }
    #[test]
    fn len_subexp_p_scalar_index() {
        assert_parity(r#"s=hello; n=s; print -r -- "${#${(P)n}[2]}""#);
    }
    #[test]
    fn len_subexp_p_scalar_neg_index() {
        assert_parity(r#"s=hello; n=s; print -r -- "${#${(P)n}[-1]}""#);
    }
    #[test]
    fn len_subexp_bare_array_char_index() {
        assert_parity(r#"arr=(one two three); print -r -- "${#${arr}[2]}""#);
    }
    #[test]
    fn subexp_scalar_index_no_len_unaffected() {
        assert_parity(r#"s=hello; print -r -- "${${s}[2]}""#);
    }

    // --- Nested BARE assoc splats its VALUES as an array (c:subst.c:3947 —
    // a PM_HASHED param's aval is its values), so the shape survives the
    // nesting (`${${h}}` prints one line per value, not the joined scalar).
    // The non-nested `${h}` already splatted; only the nested form was
    // collapsing to a joined scalar. ---
    #[test]
    fn nested_bare_assoc_splats_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); print -rl -- ${${h}}"#);
    }
    #[test]
    fn nested_bare_assoc_at_flag_splats() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); print -rl -- ${(@)${h}}"#);
    }
    #[test]
    fn nested_bare_assoc_len_counts_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); print -r -- ${#${h}}"#);
    }
    #[test]
    fn nested_bare_assoc_values_with_spaces() {
        assert_parity(r#"typeset -A h=(a "x y" b "z w"); x=(${${h}}); print ${#x}"#);
    }
    #[test]
    fn nested_p_assoc_bare_splats_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -rl -- ${${(P)n}}"#);
    }
    #[test]
    fn nested_bare_assoc_join() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); print -r -- ${(j:,:)${h}}"#);
    }

    // --- `(P)`-assoc `[@]`/`[*]` splat via a name-ref redirect emits the
    // assoc VALUES (getvaluearr), not the IFS-joined scalar. ---
    #[test]
    fn p_assoc_at_subscript_splats_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -rl -- ${${(P)n}[@]}"#);
    }
    #[test]
    fn p_assoc_star_subscript_splats_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -rl -- ${${(P)n}[*]}"#);
    }
    #[test]
    fn p_assoc_at_subscript_values_with_spaces() {
        assert_parity(r#"typeset -A h=(a "x y" b "z w"); n=h; print -rl -- ${${(P)n}[@]}"#);
    }
    #[test]
    fn direct_assoc_at_subscript_unaffected() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); print -rl -- ${h[@]}"#);
    }

    // --- getarrvalue `nular` padding (c:params.c:2570-2585): a slice whose
    // start is at/beyond the array end returns a single empty element (capped
    // at nular's one element), visible via `${#}`. ---
    #[test]
    fn empty_array_slice_len_is_one() {
        assert_parity(r#"arr=(); print -r -- ${#arr[1,2]}"#);
    }
    #[test]
    fn empty_array_slice_degenerate_len_zero() {
        assert_parity(r#"arr=(); print -r -- ${#arr[1,1]}"#);
    }
    #[test]
    fn empty_array_slice_neg_start_len_one() {
        assert_parity(r#"arr=(); print -r -- ${#arr[-1,2]}"#);
    }
    #[test]
    fn nonempty_array_out_of_range_slice_len_one() {
        assert_parity(r#"arr=(x); print -r -- ${#arr[2,3]}"#);
    }
    #[test]
    fn empty_array_slice_value_empty() {
        assert_parity(r#"arr=(); print -r -- "[${arr[1,2]}]""#);
    }
    #[test]
    fn empty_array_slice_unquoted_capture_zero() {
        assert_parity(r#"arr=(); x=(${arr[1,2]}); print ${#x}"#);
    }
    #[test]
    fn empty_array_slice_quoted_capture_one() {
        assert_parity(r#"arr=(); x=("${arr[1,2]}"); print ${#x}"#);
    }

    // --- Nested empty-array stays array-shaped through the nesting, so the
    // outer slice pads via getarrvalue (`${#${arr}[1,2]}` is 1 for arr=()). ---
    #[test]
    fn nested_empty_array_slice_len_one() {
        assert_parity(r#"arr=(); print -rl -- ${#${arr}[1,2]}"#);
    }
    #[test]
    fn nested_p_empty_array_slice_len_one() {
        assert_parity(r#"arr=(); n=arr; print -rl -- ${#${(P)n}[1,2]}"#);
    }
    #[test]
    fn nested_empty_array_bare_unaffected() {
        assert_parity(r#"arr=(); print -rl -- ${${arr}}"#);
    }

    // --- (A) array-force collapses a LONE empty-string result to an empty
    // array (getarrvalue nular phantom OR a single real ""), but keeps
    // multi-element results (c:subst.c hmkarray on the scalarized value). ---
    #[test]
    fn a_flag_empty_array_slice_collapses() {
        assert_parity(r#"a=(); print -r -- ${#${(A@)a[1,2]}}"#);
    }
    #[test]
    fn a_flag_single_empty_elem_collapses() {
        assert_parity(r#"a=(""); print -r -- ${#${(A@)a[1,1]}}"#);
    }
    #[test]
    fn a_flag_bare_single_empty_collapses() {
        assert_parity(r#"a=(""); print -r -- ${#${(A@)a}}"#);
    }
    #[test]
    fn a_flag_two_empty_elems_kept() {
        assert_parity(r#"a=("" ""); print -r -- ${#${(A@)a[1,2]}}"#);
    }
    #[test]
    fn a_flag_mixed_empty_kept() {
        assert_parity(r#"a=(x ""); print -r -- ${#${(A@)a[1,2]}}"#);
    }

    // --- `(#e)` end-anchor ZERO-WIDTH match in replacement: the scan probes
    // the end position (c:glob.c:3029-3082) without over-firing for `pat#`
    // interior empties. ---
    #[test]
    fn replace_end_anchor_global() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s//(#e)/X}"#);
    }
    #[test]
    fn replace_end_anchor_single() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s/(#e)/X}"#);
    }
    #[test]
    fn replace_end_anchor_empty_subject() {
        assert_parity(r#"setopt extendedglob; s=; print -r -- ${s//(#e)/X}"#);
    }
    #[test]
    fn replace_zero_or_more_no_extra_end() {
        assert_parity(r#"setopt extendedglob; s=bbb; print -r -- ${s//a#/X}"#);
    }
    #[test]
    fn replace_end_anchored_char() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s//c(#e)/X}"#);
    }

    // --- Top-level alternation FIRST-BRANCH: `(a|ab)` matches the first
    // branch `a`, not the longer `ab` (c:Src/pattern.c leftmost-alternation),
    // across single/global and anchored replace paths. ---
    #[test]
    fn replace_alternation_first_branch_single() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s/(a|ab)/X}"#);
    }
    #[test]
    fn replace_alternation_first_branch_amp() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s/(a|ab)/&}"#);
    }
    #[test]
    fn replace_alternation_anchor_single() {
        assert_parity(r#"setopt extendedglob; s=abc; print -r -- ${s/#(a|ab)/X}"#);
    }
    #[test]
    fn replace_alternation_anchor_global() {
        assert_parity(r#"setopt extendedglob; s=abcabc; print -r -- ${s//#(a|ab)/-}"#);
    }
    #[test]
    fn replace_alternation_backref_still_captures() {
        assert_parity(r#"setopt extendedglob; s=camelCase; print -r -- ${s/(#b)([A-Z])/_${match[1]}}"#);
    }
    #[test]
    fn replace_greedy_star_unaffected() {
        assert_parity(r#"setopt extendedglob; s=abcabc; print -r -- ${s/a*c/-}"#);
    }

    // --- Nested `:^` / `:^^` zip in DQ scalar context collapses the LEFT
    // operand to a sepjoined scalar before zipping (c:subst.c:3032, via
    // SUBEXP_SCALAR_CTX), matching a directly-quoted expansion. The `(@)`
    // flag (nojoin=2) suppresses the collapse → per-element zip. ---
    #[test]
    fn nested_zip_short_collapses_left() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -r -- "${${a:^b}}""#);
    }
    #[test]
    fn nested_zip_long_collapses_left() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -r -- "${${a:^^b}}""#);
    }
    #[test]
    fn nested_zip_join_flag() {
        assert_parity(r#"a=(1 2); b=(x y z); print -r -- "${(j:,:)${a:^^b}}""#);
    }
    #[test]
    fn direct_quoted_zip_still_collapses() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -r -- "${a:^b}""#);
    }
    #[test]
    fn at_flag_zip_stays_per_element() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -rl -- "${(@)a:^b}""#);
    }
    #[test]
    fn at_flag_zip_long_stays_per_element() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -rl -- "${(@)a:^^b}""#);
    }
    #[test]
    fn unquoted_zip_stays_per_element() {
        assert_parity(r#"a=(1 2 3); b=(a b); print -r -- ${${a:^b}}"#);
    }
    #[test]
    fn at_subscript_zip_per_element_unaffected() {
        assert_parity(r#"a=(1 2 3); b=(x y); print -r -- "${a[@]:^b}""#);
    }

    // --- `(m)` display-cell padding TRUNCATION: when the value exceeds the
    // width, wide (2-cell) chars must be counted in CELLS, not chars, on both
    // the left (keep rightmost fitting chars) and right (keep leftmost, copy
    // the crossing char) — c:Src/subst.c:912-925 / :1072-1080. ---
    #[test]
    fn m_pad_left_truncate_wide() {
        assert_parity(r#"j=日本語テキスト; print -r -- "[${(ml:8::.:)j}]""#);
    }
    #[test]
    fn m_pad_right_truncate_wide() {
        assert_parity(r#"j=日本語テキスト; print -r -- "[${(mr:8::.:)j}]""#);
    }
    #[test]
    fn m_pad_left_truncate_short_width() {
        assert_parity(r#"j=日本語; print -r -- "[${(ml:3::.:)j}]""#);
    }
    #[test]
    fn m_pad_right_truncate_crossing_char() {
        assert_parity(r#"j=日本語; print -r -- "[${(mr:3::.:)j}]""#);
    }
    #[test]
    fn m_pad_left_partial_wide() {
        assert_parity(r#"j=一二三; print -r -- "[${(ml:4::.:)j}]""#);
    }
    #[test]
    fn non_m_pad_truncate_unaffected() {
        assert_parity(r#"s=abcdef; print -r -- "[${(r:3:)s}][${(l:3:)s}]""#);
    }
    #[test]
    fn m_pad_no_truncate_still_pads() {
        assert_parity(r#"j=日本語; print -r -- "[${(ml:8::.:)j}]""#);
    }
    #[test]
    fn m_pad_both_sides_wide() {
        assert_parity(r#"j=日本語テキスト; print -r -- "[${(ml:4::.:mr:4::+:)j}]""#);
    }

    // --- Assignment inside a parameter expansion (`${(A)n::=v}` etc.): the RHS
    // splits into array elements ONLY under `spsep`/`spbreak` (the `(s:X:)` or
    // `=` flag), across all three operators (c:Src/subst.c:3272). ---
    #[test]
    fn assign_array_eq_flag_word_splits() {
        assert_parity(r#"unset o; v="1 2 3"; : ${(A)=o::=$v}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_no_flag_one_element() {
        assert_parity(r#"unset o; v="1 2 3"; : ${(A)o::=$v}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_colon_eq_no_split() {
        assert_parity(r#"unset o; v="1 2 3"; : ${(A)o:=$v}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_eq_op_no_split() {
        assert_parity(r#"unset o; v="1 2 3"; : ${(A)o=$v}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_s_separator_splits() {
        assert_parity(r#"unset o; v="a:b:c"; : ${(As.:.)o=$v}; print -r -- "${#o}""#);
    }
    // --- Empty/scalar element shapes (c:3282-3293). ---
    #[test]
    fn assign_array_empty_one_element() {
        assert_parity(r#"unset o; : ${(A)o::=}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_eq_flag_empty_one_element() {
        assert_parity(r#"unset o; : ${(A)=o::=}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_assoc_empty_zero_elements() {
        assert_parity(r#"unset o; : ${(AA)o::=}; print -r -- "${#o}""#);
    }
    #[test]
    fn assign_array_s_separator_empty_zero() {
        assert_parity(r#"unset o; : ${(As.:.)o::=}; print -r -- "${#o}""#);
    }
    // --- (AA) odd key/value count errors. ---
    #[test]
    fn assign_assoc_odd_count_errors() {
        assert_parity(r#"unset o; : ${(AA)o::=k1 v1 k2}; print -r -- done"#);
    }
    #[test]
    fn assign_assoc_eq_flag_even_ok() {
        assert_parity(r#"unset o; v="k1 v1 k2 v2"; : ${(AA)=o::=$v}; print -r -- "${o[k2]}""#);
    }

    // --- ksh-style autoload of a file whose body has a TOP-LEVEL `return`
    // (e.g. add-zle-hook-widget's `zmodload -e zsh/zle || return 1`) must
    // WARN "not defined by file" and CONTINUE, not abort the caller's shell.
    // The load runs at the autoload invocation's source level, so the
    // top-level `return` is contained (c:Src/exec.c:5739, bin_return c:5840),
    // exactly like zsh. Real-world trigger: a plugin's `emulate sh` leaks
    // ksh_autoload on, then z-sy-h's `autoload -U add-zle-hook-widget` loads
    // it ksh-style; before this fix zshrs aborted precmd and broke the prompt.
    #[test]
    fn ksh_autoload_body_toplevel_return_continues() {
        assert_parity(
            r#"d=$(mktemp -d); print "print BODY; return 1" > $d/kfn; setopt ksh_autoload; fpath=($d $fpath); autoload kfn; print BEFORE; kfn 2>/dev/null; print "AFTER rc=$?"; rm -rf $d"#,
        );
    }
    #[test]
    fn ksh_autoload_body_no_def_continues() {
        assert_parity(
            r#"d=$(mktemp -d); print "print RAN" > $d/kfn2; setopt ksh_autoload; fpath=($d $fpath); autoload kfn2; print BEFORE; kfn2 2>/dev/null; print "AFTER rc=$?"; rm -rf $d"#,
        );
    }
    #[test]
    fn ksh_autoload_body_defines_fn_ok() {
        assert_parity(
            r#"d=$(mktemp -d); print "kfn3() { print DEF:\$1 }" > $d/kfn3; setopt ksh_autoload; fpath=($d $fpath); autoload kfn3; print BEFORE; kfn3 arg; print "AFTER rc=$?"; rm -rf $d"#,
        );
    }
    #[test]
    fn emulate_sh_leaks_ksh_autoload() {
        assert_parity(r#"emulate sh; print "kshauto=${options[kshautoload]}""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Q. RC_EXPAND_PARAM keeps interior empty fields from a forced (s::)
//    split in QUOTED context.
//
// `sepsplit` (utils.c:3962 → wordcount(s,sep,1)) never drops empties;
// the interior-empty collapse is downstream (prefork's empty-node
// removal). Under `setopt rcexpandparam` a forced (s:X:) split keeps
// its interior empties in a quoted expansion — `a=("${(s.:.)v}")` for
// v="a:b::c" is a 4-element array (`a b "" c`), not 3. zshrs's
// split-time collapse (Bug #542) dropped the empty even under plan9;
// the fix gates that collapse (and the !qt empty filter) on !plan9.
// ─────────────────────────────────────────────────────────────────────
mod rcexpandparam_split_empty {
    use super::*;

    /// zsh: 4 (interior empty kept). Was 3 before the plan9 gate.
    #[test]
    fn quoted_forced_split_keeps_interior_empty() {
        assert_parity(
            r#"v="a:b::c"; setopt rcexpandparam; a=("${(s.:.)v}"); print $#a"#,
        );
    }

    /// (@) forced split under rcexpandparam — also 4.
    #[test]
    fn quoted_at_forced_split_keeps_interior_empty() {
        assert_parity(
            r#"v="a:b::c"; setopt rcexpandparam; a=("${(@s.:.)v}"); print $#a"#,
        );
    }

    /// Unquoted forced split under rcexpandparam still drops empties (3).
    #[test]
    fn unquoted_forced_split_drops_interior_empty() {
        assert_parity(
            r#"v="a:b::c"; setopt rcexpandparam; a=(${(s.:.)v}); print $#a"#,
        );
    }

    /// Non-plan9 quoted forced split still collapses interior empties
    /// (Bug #542 path unchanged) → 3.
    #[test]
    fn non_plan9_quoted_forced_split_collapses() {
        assert_parity(r#"v="a:b::c"; a=("${(s.:.)v}"); print $#a"#);
    }

    /// Bug #578 must not regress under rcexpandparam: a leading empty
    /// from ${a%x*} on the first element stays dropped unquoted.
    #[test]
    fn rcexpand_leading_empty_still_dropped() {
        assert_parity(r#"setopt rcexpandparam; a=(x y z); echo ${a%x*}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// R. Nested `${#${(flag)v}}` counts elements under KSH_ARRAYS.
//
// getlen (subst.c:3849) counts array ELEMENTS whenever `isarr` is set;
// KSHARRAYS never touches that block. A nested flag-split — `${(z)v}`,
// `${(s:X:)v}`, `${(f)v}` — sets isarr from the split, so the outer `#`
// must count its elements even under ksharrays. zshrs materialized the
// inner result into a synthetic __subexp_arr_N temp and then wrongly
// applied the KSHARRAYS bare-array→element-1 scalarization to it,
// collapsing `${#${(z)v}}` for v="a b c" to element-0's char length (1)
// instead of the element count (3). The fix excludes subexp temps from
// that scalarization.
// ─────────────────────────────────────────────────────────────────────
mod ksharrays_nested_split_count {
    use super::*;

    /// zsh: 3 (element count of the (z) split). Was 1 before the fix.
    #[test]
    fn ksharrays_nested_z_split_counts_elements() {
        assert_parity(r#"v="a b c"; setopt ksharrays; print -r -- ${#${(z)v}}"#);
    }

    /// (s:X:) forced split nested count under ksharrays → 3.
    #[test]
    fn ksharrays_nested_s_split_counts_elements() {
        assert_parity(r#"v="a:b:c"; setopt ksharrays; print -r -- ${#${(s.:.)v}}"#);
    }

    /// (f) line split nested count under ksharrays → 3.
    #[test]
    fn ksharrays_nested_f_split_counts_elements() {
        assert_parity(
            "v=$'x\\ny\\nz'; setopt ksharrays; print -r -- ${#${(f)v}}",
        );
    }

    /// A REAL bare array `${#a}` under ksharrays still yields element-0's
    /// char length (KSHARRAYS scalarization must NOT regress).
    #[test]
    fn ksharrays_bare_array_length_unchanged() {
        assert_parity(r#"a=(xx yy zz); setopt ksharrays; print -r -- ${#a}"#);
    }

    /// `${#a[@]}` keeps the element count under ksharrays.
    #[test]
    fn ksharrays_at_subscript_length_unchanged() {
        assert_parity(r#"a=(xx yy zz); setopt ksharrays; print -r -- ${#a[@]}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// S. An unquoted expansion flanked by SEPARATE double-quoted spans stays
//    unquoted: `"x"$a"y"`.
//
// The fusevm word compiler classified a word as whole-word double-quoted
// whenever its tokenized form merely STARTED and ENDED with a Dnull
// marker. For `"x"$a"y"` the leading/trailing Dnulls belong to DIFFERENT
// quote runs and the middle `$a` is UNQUOTED, so zsh word-splits the
// array (`a=(one two three)` → `xone two threey`, three words) — but the
// mis-classification bumped dq_context_depth and joined it to one scalar.
// The fix counts Dnulls only at brace/bracket/paren depth 0: a genuine
// single wrap has exactly 2, sibling spans have 4+, and Dnulls nested
// inside `${…}` sit at depth>0 and are ignored.
// ─────────────────────────────────────────────────────────────────────
mod quoted_flanked_unquoted_expansion {
    use super::*;

    /// zsh: 3 words (`xone`, `two`, `threey`). Was 1 (joined) before fix.
    #[test]
    fn both_quoted_flanks_array_splits() {
        assert_parity(r#"a=(one two three); print -rl -- "x"${a}"y""#);
    }

    /// No-brace form `"x"$a"y"` behaves identically.
    #[test]
    fn both_quoted_flanks_array_no_brace() {
        assert_parity(r#"a=(one two three); print -rl -- "x"$a"y""#);
    }

    /// A set-op expansion flanked by quotes still applies the set-op AND
    /// splices: `"x"${a:|b}"y"` → `xone three fivey`.
    #[test]
    fn both_quoted_flanks_setop_splice() {
        assert_parity(
            r#"a=(one two three four five); b=(two four six eight); print -r -- "x"${a:|b}"y""#,
        );
    }

    /// Genuine whole-word DQ wrap must NOT split (regression guard).
    #[test]
    fn genuine_dq_wrap_no_split() {
        assert_parity(r#"a=(one two three); print -rl -- "pre ${a} post""#);
    }

    /// A double-quote NESTED inside `${…}` keeps the outer wrap intact.
    #[test]
    fn nested_dq_inside_braces_keeps_wrap() {
        assert_parity(r#"x=(o t); print -rl -- "a${x:-"n"}b""#);
    }

    /// One-sided quote (quoted prefix, literal suffix) already worked —
    /// pin it so the depth-count change can't regress it.
    #[test]
    fn one_sided_quote_splits() {
        assert_parity(r#"a=(one two three); print -rl -- "x"${a}suf"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// T. The ZERR trap fires when a failing command sets errflag (readonly
//    reassignment, bad redirect) — not only on a plain non-zero exit.
//
// C's sublist_done (exec.c:1598-1603) runs dotrap(SIGZERR) BEFORE the
// enclosing list loop breaks on errflag. zshrs's BUILTIN_ERREXIT_CHECK
// fired ZERR in the retflag escape and the plain fall-through, but the
// errflag branch returned early WITHOUT firing — so `TRAPZERR() { … };
// typeset -r ro=1; ro=2` aborted (correct) yet never ran the trap. The
// fix fires ZERR in the errflag branch, clearing errflag across the
// dispatch (signals.c:1101 dotrapargs returns early on errflag; 1174/
// 1216 save+restore) so the trap body runs, then restoring it so the
// script still aborts.
// ─────────────────────────────────────────────────────────────────────
mod zerr_trap_on_errflag {
    use super::*;

    /// zsh fires TRAPZERR on the readonly reassignment. Was silent before.
    #[test]
    fn trapzerr_fires_on_readonly_reassign() {
        assert_parity(
            r#"TRAPZERR() { print -r -- zerr }; typeset -r ro=1; ro=2; echo end"#,
        );
    }

    /// `trap … ERR` string form also fires.
    #[test]
    fn err_trap_fires_on_readonly_reassign() {
        assert_parity(
            r#"trap 'print -r -- ERRFIRED' ERR; typeset -r ro=1; ro=2; echo end"#,
        );
    }

    /// Inside a { } always { } block the trap fires for the failed body
    /// and again at the sublist boundary — two `zerr` lines like zsh.
    #[test]
    fn zerr_fires_in_always_block() {
        assert_parity(
            r#"TRAPZERR() { print -r -- zerr }; { typeset -r ro=1; ro=2 } always { print -r -- always }; print -r -- end"#,
        );
    }

    /// No-trap readonly reassignment must STILL abort with exit 1
    /// (errflag preserved across the — absent — dispatch).
    #[test]
    fn no_trap_readonly_still_aborts() {
        assert_parity(r#"typeset -r ro=1; ro=2; echo SHOULD_NOT_PRINT"#);
    }

    /// A function-scope readonly reassignment fires ZERR and unwinds.
    #[test]
    fn zerr_fires_in_function_scope() {
        assert_parity(
            r#"TRAPZERR() { print -r -- z }; f() { local -r x=5; x=10; print -r -- inner }; f; print -r -- after"#,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// U. A string-form `trap 'body' SIG` replaces a pre-existing
//    FUNCTION-form `TRAPSIG() { … }` trap.
//
// C's settrap (signals.c:707) calls unsettrap → removetrap
// (c:836-843 removeshfuncnode) which drops any existing function-form
// trap before installing the new body. zshrs's removetrap never touched
// shfunctab, so a leftover `TRAPZERR() { … }` shadowed a later
// `trap 'body' ERR` — the dispatch prefers the function form, so the OLD
// function fired instead of the new string body. The fix removes the
// `TRAP<canonical>` shfunctab node when a string trap is installed, for
// virtual signals (ERR/EXIT/DEBUG, beyond SIGCOUNT) as well as real ones.
// ─────────────────────────────────────────────────────────────────────
mod string_trap_replaces_function {
    use super::*;

    /// zsh fires the NEW string body, not the replaced TRAPZERR function.
    #[test]
    fn string_err_replaces_trapzerr_function() {
        assert_parity(
            r#"TRAPZERR() { print -r -- zerr }; trap 'print -r -- strerr' ERR; false; print -r -- mid"#,
        );
    }

    /// EXIT (virtual signal) — string form replaces TRAPEXIT function.
    #[test]
    fn string_exit_replaces_trapexit_function() {
        assert_parity(
            r#"TRAPEXIT() { print -r -- fe }; trap 'print -r -- se' EXIT; print -r -- body"#,
        );
    }

    /// Real signal INT — string form replaces TRAPINT function.
    #[test]
    fn string_int_replaces_trapint_function() {
        assert_parity(
            r#"TRAPINT() { print -r -- fi }; trap 'print -r -- si' INT; trap"#,
        );
    }

    /// Function-only trap still fires (no over-removal).
    #[test]
    fn function_only_still_fires() {
        assert_parity(r#"TRAPZERR() { print -r -- z }; false; print -r -- mid"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// V. `(q+)` / quotedzputs `$'…'`-quote a high-bit (meta) byte.
//
// is_mb_niceformat (utils.c:5474) decides whether quotedzputs upgrades to
// the `$'…'` form. zshrs unmetafied its input with the CHAR-level `unmeta`,
// which decodes an eight-bit meta byte (Meta 0x83 + 0xC1 for `$'\M-a'`)
// into the valid Rust char U+00E1 (`á`) — read as a printable Latin-1
// letter, so is_mb_niceformat returned 0 and `(q+)` left the raw byte
// unquoted (`\341`) where zsh emits `$'\M-a'`. Using BYTE-level
// unmetafy_str (C's unmetafy) preserves the raw 0xE1 so the MB_INVALID arm
// flags it nice. Control chars already worked; printable UTF-8 stays
// unquoted.
// ─────────────────────────────────────────────────────────────────────
mod qplus_meta_byte_quoting {
    use super::*;

    /// zsh: `$'\M-a'`. Was the raw byte `\341` before the fix.
    #[test]
    fn qplus_quotes_meta_byte() {
        assert_parity(r#"v=$'\M-a'; print -rn -- "${(q+)v}" | od -An -tx1"#);
    }

    /// Meta byte embedded in printable text.
    #[test]
    fn qplus_quotes_embedded_meta_byte() {
        assert_parity(r#"v=$'a\M-ab'; print -rn -- "${(q+)v}" | od -An -tx1"#);
    }

    /// Control char still upgrades to `$'\C-A'` (no regression).
    #[test]
    fn qplus_control_char_unchanged() {
        assert_parity(r#"v=$'\x01'; print -rn -- "${(q+)v}" | od -An -tx1"#);
    }

    /// Printable multibyte (café) must NOT be over-quoted.
    #[test]
    fn qplus_printable_utf8_not_quoted() {
        assert_parity(r#"v=café; print -rn -- "${(q+)v}" | od -An -tx1"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// W. A flag expansion on an array/assoc must not leak array-ness into a
//    following unrelated `${#${#scalar}}`.
//
// PARAMSUBST_LF_ARRAY is the thread-local that hands a paramsubst call's
// array-ness back to its caller (stringsubst / the nested-subexp reader).
// C resets it to the non-array default (subst.c:3932) up front. zshrs only
// SET it on the array-producing paths, so the getlen early-return
// (`${#name}`) left whatever the PREVIOUS call stored. `${(U)arr}` sets it
// true; a following `${#${#s}}` then read the stale true, materialised the
// scalar length into a __subexp_arr_N temp, and counted 1 ELEMENT instead
// of 2 CHARS. Initialising it false at the top of every paramsubst fixes
// the cross-expansion state leak. This was a real correctness bug, not
// just a fuzz artifact.
// ─────────────────────────────────────────────────────────────────────
mod flag_expansion_no_array_state_leak {
    use super::*;

    /// zsh: `[2]`. Was `[1]` after `${(U)arr}` leaked LF_ARRAY.
    #[test]
    fn uppercase_array_then_nested_length() {
        assert_parity(
            r#"s=Hello_World; a=(x y); : ${(U)a}; print -r -- "[${#${#s}}]""#,
        );
    }

    /// Same via an associative array and the (U) listing form.
    #[test]
    fn uppercase_assoc_then_nested_length() {
        assert_parity(
            r#"s=Hello_World; typeset -A m; m=(k1 v1 k2 v2); print -rl -- ${(U)m}; print -r -- "[${#${#s}}]""#,
        );
    }

    /// (o) sort flag on an array is the same class of leak.
    #[test]
    fn sort_array_then_nested_length() {
        assert_parity(r#"s=Hello_World; a=(3 1 2); : ${(o)a}; print -r -- "[${#${#s}}]""#);
    }

    /// The nested-array count itself must STILL work (no over-reset).
    #[test]
    fn nested_split_count_unaffected() {
        assert_parity(r#"v="a b c"; print -r -- ${#${(z)v}}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// X. A nested SPLIT sub-expression drops its split-derived empty fields.
//
// C's nested SUBEXP multsub pass runs prefork, which deletes the empty
// nodes a forced split (`(s:X:)`/`(f)`) produced (subst.c:100 `else if
// (!keep) uremnode`): `${${(s.:.):a:}}` is `a`, `${${(s.:.):}}` is empty
// — NOT `" a "` / `" "`. zshrs kept them and sepjoined to spaces. The
// inner paramsubst now flags SUBEXP_NONAT_SPLIT when it is a NON-`@`
// forced split; the nested reader drops empties on that signal. `@`
// splits and real-array elements (`(v)`/`(o)`/`(u)`/`(P)`/bare) leave the
// flag false, so their empties survive — matching zsh.
// ─────────────────────────────────────────────────────────────────────
mod nested_split_drops_empties {
    use super::*;

    /// zsh: `[]`. Was `[ ]` (single space) before the fix.
    #[test]
    fn all_empty_split_nested() {
        assert_parity(r#"v=":"; print -r -- "[${${(s.:.)v}}]""#);
    }

    /// Leading/trailing empties dropped → `a`.
    #[test]
    fn lead_trail_empty_split_nested() {
        assert_parity(r#"v=":a:"; print -r -- "[${${(s.:.)v}}]""#);
    }

    /// Interior empty dropped → `a b`.
    #[test]
    fn interior_empty_split_nested() {
        assert_parity(r#"v="a::b"; print -r -- "[${${(s.:.)v}}]""#);
    }

    /// `(j:,:)` join of a nested split drops the empties first → `a,b`.
    #[test]
    fn join_of_nested_split() {
        assert_parity(r#"v="a::b"; print -r -- "[${(j:,:)${(s.:.)v}}]""#);
    }

    /// `@`-flagged split KEEPS its empties (regression guard).
    #[test]
    fn at_split_keeps_empties() {
        assert_parity(r#"v=":a:"; print -r -- "[${${(@s.:.)v}}]""#);
    }

    /// A REAL array's empties survive nesting (assoc values).
    #[test]
    fn assoc_values_keep_empties() {
        assert_parity(r#"typeset -A h=(x "" y 2); print -r -- "[${${(v)h}}]""#);
    }

    /// `(o)` sort over a real array with an empty keeps it.
    #[test]
    fn sort_real_array_keeps_empty() {
        assert_parity(r#"arr=(a "" b); print -r -- "[${${(o)arr}}]""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Y. A bare `$assoc` under KSH EMULATION is `${assoc[0]}` (key-"0"),
//    not the first bucket value.
//
// c:Src/params.c:2351-2358 — `case PM_HASHED: if (!v->scanflags &&
// EMULATION(EMULATE_KSH))` builds `s="[0]"`, does a KEY-"0" getindex, and
// returns that element — EMPTY unless the hash has a key "0". This is
// EMULATION-gated, NOT the KSHARRAYS option: `emulate -L ksh; typeset -A
// h=(a 1 b 2); print $h` is empty, whereas `setopt ksharrays; …; print $h`
// is the first bucket value `1`. zshrs collapsed both to the first value.
// Fixed in the bare-assoc reader (get_var_impl / ksharrays_bare_words).
// ─────────────────────────────────────────────────────────────────────
mod ksh_emulation_bare_assoc {
    use super::*;

    /// zsh: empty (no key "0"). Was `1` (first value) before the fix.
    #[test]
    fn emulate_ksh_bare_assoc_empty() {
        assert_parity(r#"emulate -L ksh; typeset -A h=(a 1 b 2); print -r -- "[$h]""#);
    }

    /// `$h[k]` under ksh emulation: bare $h (empty) then literal `[k]`.
    #[test]
    fn emulate_ksh_assoc_bracket_literal() {
        assert_parity(r#"emulate -L ksh; typeset -A h=(k v); print -r -- $h[k]"#);
    }

    /// WITH a real key "0", the bare form yields that value.
    #[test]
    fn emulate_ksh_bare_assoc_with_key_zero() {
        assert_parity(r#"emulate -L ksh; typeset -A h=(0 zero a 1); print -r -- "[$h]""#);
    }

    /// `setopt ksharrays` (no ksh emulation) stays first bucket value.
    #[test]
    fn setopt_ksharrays_bare_assoc_first_value() {
        assert_parity(r#"setopt ksharrays; typeset -A h=(a 1 b 2); print -r -- "[$h]""#);
    }

    /// `emulate sh` bare assoc is also the first value (not key "0").
    #[test]
    fn emulate_sh_bare_assoc_first_value() {
        assert_parity(r#"emulate -L sh; typeset -A h=(a 1 b 2); print -r -- "[$h]""#);
    }

    /// A regular array under ksh emulation still yields element 0.
    #[test]
    fn emulate_ksh_bare_array_first_element() {
        assert_parity(r#"emulate -L ksh; a=(x y z); print -r -- "[$a]""#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Z. A process substitution in a `[[ … ]]` cond operand errors.
//
// c:Src/exec.c:4918/5040/5069 — getoutputfile/getproc error "process
// substitution %s cannot be used here" when `thisjob == -1`, which holds
// during cond evaluation. zshrs's THISJOB never distinguishes that
// context at runtime, so the compiler (where `in_cond_operand` is known)
// emits BUILTIN_PROCSUB_COND_ERROR in place of ProcessSubIn/Out: it
// zerrs, sets errflag (aborting the statement), and returns empty — so
// `[[ -f =(cmd) ]] && …` produces empty stdout and exit 1, matching zsh
// instead of the file-exists `true`. Command-argument uses (`cat =(…)`,
// `diff <(…)`, `wc < =(…)`) are unaffected (in_cond_operand is false).
// ─────────────────────────────────────────────────────────────────────
mod procsub_in_cond_errors {
    use super::*;

    /// zsh: empty stdout, exit 1 (error). Was `regular` (exit 0) before.
    #[test]
    fn eq_procsub_in_dbracket_f() {
        assert_parity(
            r#"[[ -f =(print x) ]] && print -r -- regular || print -r -- notregular"#,
        );
    }

    /// `<(…)` in `[[ -f ]]` also errors.
    #[test]
    fn in_procsub_in_dbracket_f() {
        assert_parity(r#"[[ -f <(print x) ]] && print -r -- y || print -r -- n"#);
    }

    /// The error exit status propagates.
    #[test]
    fn eq_procsub_in_dbracket_e_status() {
        assert_parity(r#"[[ -e =(echo hi) ]]; print -r -- "rc=$?""#);
    }

    /// Command-argument `=(…)` still works (regression guard).
    #[test]
    fn eq_procsub_as_command_arg_works() {
        assert_parity(r#"cat =(print hello)"#);
    }

    /// `<(…)` command-argument still works.
    #[test]
    fn in_procsub_as_command_arg_works() {
        assert_parity(r#"diff <(print a) <(print a) && print -r -- same"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// AA. A `=(cmd)` temp file is deleted at the end of the command that
//     created it — including an assignment-only command.
//
// c:Src/jobs.c deletefilelist — the temp is bound to the JOB of its
// creating command and unlinked when that command completes. `f==(cmd)`
// has no consuming builtin/exec, so zshrs's PsubFdGuard (which cleans
// consuming commands) never fired and the temp leaked: a later
// `[[ -f $f ]]` / `$(<$f)` still saw it. zsh deletes it at the end of the
// assignment (even `f==(x) && cat $f` fails). Fixed by cleaning pending
// procsub temps in BUILTIN_ASSIGN_ONLY_STATUS. Command-argument `=(…)`
// uses are unaffected (their consuming command still cleans them AFTER
// the read).
// ─────────────────────────────────────────────────────────────────────
mod eq_procsub_temp_lifetime {
    use super::*;

    /// zsh: `exists=n` (temp gone after the assignment). Was `y` before.
    #[test]
    fn assign_procsub_temp_gone_next_statement() {
        assert_parity(
            r#"f==(print -l a b); print -r -- "exists=$([[ -f $f ]] && print y || print n)""#,
        );
    }

    /// Reading the deleted temp yields empty.
    #[test]
    fn assign_procsub_read_after_is_empty() {
        assert_parity(r#"f==(print hi); print -r -- "[$(<$f 2>/dev/null)]""#);
    }

    /// The temp is gone even within the same `&&` list.
    #[test]
    fn assign_procsub_gone_in_same_and_list() {
        assert_parity(r#"f==(print hi) && cat $f 2>/dev/null | grep -c hi"#);
    }

    /// Command-argument `=(…)` still works (temp survives until read).
    #[test]
    fn command_arg_procsub_still_works() {
        assert_parity(r#"cat =(print hello)"#);
    }

    /// `x=$(cat =(…))` — nested use is cleaned by cat, not the outer assign.
    #[test]
    fn cmdsub_arg_procsub_still_works() {
        assert_parity(r#"x=$(cat =(print abc)); print -r -- $x"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// AB. `${(z)v}` keeps a literal `$` / backtick UNESCAPED.
//
// The (z) shell-tokenize result must keep a `$foo` word unexpanded AND
// unescaped: `v='a$b'; ${(z)v}` is `a$b`, not `a\$b`. zshrs protected the
// `$`/backtick from stringsubst re-expansion (Bug #363) by prefixing it
// with Bnull — but Bnull UNTOKENIZES to a literal backslash (C ztokens
// maps Bnull → `\`), so every `(z)` result with a `$` grew a spurious
// backslash. Switched to Snull single-quote markers, which stringsubst
// strips as a literal region (subst.rs:654) BEFORE any untokenize, so the
// char survives bare — the re-expansion guard is kept, the backslash gone.
// ─────────────────────────────────────────────────────────────────────
mod z_flag_literal_dollar {
    use super::*;

    /// zsh: `a$b` (b defined, still literal). Was `a\$b` before.
    #[test]
    fn z_keeps_dollar_literal_unescaped() {
        assert_parity(r#"b=X; v='a$b'; print -rl -- ${(z)v}"#);
    }

    /// `$HOME` in a tokenized word stays literal and unescaped.
    #[test]
    fn z_keeps_dollar_home_literal() {
        assert_parity(r#"v='echo $HOME'; print -rl -- ${(z)v}"#);
    }

    /// Multi-word: the `$b` word is one literal token.
    #[test]
    fn z_multiword_dollar_literal() {
        assert_parity(r#"v='a $b c'; print -rl -- ${(z)v}"#);
    }

    /// The (q)-then-(z) round trip from the fuzz.
    #[test]
    fn z_of_q_quoted_backslash_dollar() {
        assert_parity(
            r#"v="${(q)$(print -rn -- 'a\$b')}"; print -rl -- ${(z)v}; print -r -- END"#,
        );
    }

    /// Backtick stays literal too.
    #[test]
    fn z_keeps_backtick_literal() {
        assert_parity(r#"v='foo `bar`'; print -rl -- ${(z)v}"#);
    }

    /// Separators/operators still tokenize as their own words.
    #[test]
    fn z_separators_still_tokenize() {
        assert_parity(r#"v='a; b && c'; print -rl -- ${(z)v}"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// AC. remnulargs is Meta-aware: a metafied eight-bit byte is data, not a
//     sentinel — even when it aliases one.
//
// C strings are byte arrays, so an eight-bit char (`$'\M-\C-a'` = 0x81)
// is stored RAW and its byte never aliases an inull sentinel. A Rust
// String is UTF-8, so zshrs METAFIES it — Meta (0x83) + (byte ^ 0x20) —
// and 0x81 ^ 0x20 = 0xA1 = the Nularg sentinel (0xBD/0xBE/0xBF likewise
// alias Snull/Dnull/Bnull). remnulargs (glob.c:3649) is a byte walk that
// strips inull chars; on the metafied string it dropped the 0xA1 and left
// the lone Meta, so `${(qq)v}` for v=$'\M-\C-a' emitted `'\xc2\x83'` where
// zsh emits `'\x81'`. Treat Meta+next as an opaque data pair on both the
// scan and copy walks — the C byte walk needs no such guard.
// ─────────────────────────────────────────────────────────────────────
mod remnulargs_meta_aware {
    use super::*;

    /// zsh: `'<0x81>'`. Was `'<Meta>'` (0xc2 0x83) before the fix.
    #[test]
    fn qq_quote_meta_ctrl_a_nularg_alias() {
        assert_parity(r#"v=$'\M-\C-a'; print -rn -- "[${(qq)v}]" | od -An -tx1"#);
    }

    /// 0xBD aliases Snull.
    #[test]
    fn qq_quote_meta_eq_snull_alias() {
        assert_parity(r#"v=$'\M-='; print -rn -- "[${(qq)v}]" | od -An -tx1"#);
    }

    /// 0xBE aliases Dnull.
    #[test]
    fn qq_quote_meta_gt_dnull_alias() {
        assert_parity(r#"v=$'\M->'; print -rn -- "[${(qq)v}]" | od -An -tx1"#);
    }

    /// 0xBF aliases Bnull.
    #[test]
    fn qq_quote_meta_q_bnull_alias() {
        assert_parity(r#"v=$'\M-?'; print -rn -- "[${(qq)v}]" | od -An -tx1"#);
    }

    /// A non-aliasing meta byte (0xE1) still works (regression guard).
    #[test]
    fn qq_quote_meta_a_no_alias() {
        assert_parity(r#"v=$'\M-a'; print -rn -- "[${(qq)v}]" | od -An -tx1"#);
    }

    /// A real empty array still drops empty words (remnulargs Nularg path).
    #[test]
    fn empty_array_word_drop_unaffected() {
        assert_parity(r#"a=(x '' y); print -rl -- $a | wc -l"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// AD. An expansion flanked by SIBLING empty double-quoted spans stays
//     unquoted: `""${a}""`.
//
// Many word-compile fast paths decided DQ-context with a naive
// `starts_with(Dnull) && ends_with(Dnull)` test. That misfires on SIBLING
// spans — `""${a}""` (two empty `""`) starts and ends with a Dnull but its
// middle `${a}` is UNQUOTED, so zsh word-splits the array (a=(1 2 3) → 3
// words) where the naive test joined it to one. The concat path already
// used a depth-aware count (my earlier "x"${a}"y" fix); this extracts it
// into word_is_single_dq_span and applies it to all 12 DQ-context
// fast paths (${NAME}, ${(flags)…}, ${a[@]}, set-op/zip, …). A real single
// wrap `"${a}"` (2 depth-0 Dnulls) still joins; sibling spans (4+) split.
// ─────────────────────────────────────────────────────────────────────
mod sibling_empty_dq_spans {
    use super::*;

    /// zsh: 3 words. Was 1 (joined) before the fix.
    #[test]
    fn empty_quotes_flank_array_splits() {
        assert_parity(r#"a=(1 2 3); print -rl -- ""${a}""#);
    }

    /// Nested flag + set-op in empty quotes (the fuzz seed).
    #[test]
    fn empty_quotes_nested_join_setop() {
        assert_parity(r#"a=(1 2 3); b=(2); print -r -- ""${(j:,:)${a:|b}}""#);
    }

    /// `(j)` join inside empty quotes still applies.
    #[test]
    fn empty_quotes_join_flag() {
        assert_parity(r#"a=(1 2 3); print -r -- ""${(j:-:)a}""#);
    }

    /// Genuine DQ wrap `"${a}"` must STILL join (regression guard).
    #[test]
    fn real_dq_wrap_still_joins() {
        assert_parity(r#"a=(1 2 3); print -rl -- "${a}" | wc -l"#);
    }

    /// DQ nested set-op stays scalar.
    #[test]
    fn real_dq_nested_setop_joins() {
        assert_parity(r#"a=(1 2); b=(2); print -r -- "${(j:,:)${a:|b}}""#);
    }

    /// `"${a[@]}"` still splats to N words.
    #[test]
    fn real_dq_splat_unaffected() {
        assert_parity(r#"a=(1 2 3); print -rl -- "${a[@]}" | wc -l"#);
    }
}

// ─────────────────────────────────────────────────────────────────────
// ztst-mined gaps (2026-07, from ~/forkedRepos/zsh/Test/*.ztst diffing).
// The differential fuzzer (3000 cases) and 130+ hand-picked common
// expressions found ZERO divergences — these are the adversarial edge
// cases the zsh test suite deliberately exercises.
// ─────────────────────────────────────────────────────────────────────
mod ztst_mined {
    use super::*;

    /// D08cmdsubst — an UNBRACED `$*`/`$@` in a word that ALSO contains a
    /// literal `"` from a `\"` escape is left unexpanded, so its `*`
    /// globs and fails "no matches found". `${*}` (braced) works, `$1`
    /// works. Root cause: a word containing any `\"`-escape takes a
    /// compile path (compile_zsh.rs) that doesn't detect the unbraced
    /// `$*`/`$@`. zsh: `"hi"`; zshrs: nomatch error. Fix is hot-path
    /// word-compiler and needs dedicated investigation.
    #[test]
    #[ignore = "zshrs gap: unbraced $*/$@ not expanded in a word containing a \\\" escape (globs instead)"]
    fn escaped_quote_with_dollar_star() {
        assert_parity(r#"set -- hi; print \"$*\""#);
    }

    #[test]
    #[ignore = "zshrs gap: unbraced $@ in a \\\"-escaped word drops its trailing quote"]
    fn escaped_quote_with_dollar_at() {
        assert_parity(r#"set -- hi; print \"$@\""#);
    }

    /// The braced/positional forms already work — pin so a fix to the
    /// above can't regress them.
    #[test]
    fn braced_star_in_escaped_word_ok() {
        assert_parity(r#"set -- hi; print \"${*}\""#);
    }
    #[test]
    fn positional_digit_in_escaped_word_ok() {
        assert_parity(r#"set -- hi; print \"$1\""#);
    }

    /// A06assign — `typeset a` under TYPESET_TO_UNSET declares an
    /// unset scalar; `a+=(1 2 3)` converts it to an array and zsh
    /// prepends the (empty) scalar value as element 0 (`'' 1 2 3`).
    /// zshrs drops the empty element (`1 2 3`). Without the option the
    /// two agree (declared-empty scalar → element 0 IS kept).
    #[test]
    #[ignore = "zshrs gap: TYPESET_TO_UNSET + scalar+=(array) drops the empty element-0"]
    fn typeset_to_unset_append_array_keeps_empty_elem() {
        assert_parity(
            r#"setopt typeset_to_unset; typeset a; a+=(1 2 3); print "${(q@)a}""#,
        );
    }

    /// D06subscript — a `(r)PAT,(R)PAT2` range subscript where PAT
    /// expands (via `$x`) to a string containing the `,` range
    /// separator. zsh finds the LITERAL separator comma in the
    /// unexpanded subscript text; zshrs errors "bad substitution".
    #[test]
    #[ignore = "zshrs gap: (r)$x,(R)$x range subscript with a comma in the expanded pattern → bad substitution"]
    fn range_subscript_comma_in_pattern() {
        assert_parity(
            r#"s='Twinkle, twinkle, little *, [how] I [wonder] what?'; x=','; print ${s[(r)$x,(R)$x]}"#,
        );
    }

    /// D06subscript — search-flag subscripts `(r)`/`(R)`/`(i)` whose
    /// pattern is a literal `[` or `]` (escaped). zshrs returns the
    /// whole string unchanged instead of the matched range/index.
    #[test]
    #[ignore = "zshrs gap: (r)/(R) search subscript with literal bracket pattern returns whole string"]
    fn range_subscript_bracket_pattern() {
        assert_parity(
            r#"s='Twinkle, [how] I [wonder]'; print $s[(r)\],(R)\[]"#,
        );
    }

    /// D09brace — a brace range `{X..Y}` over non-ASCII single bytes
    /// (metafied high bytes) doesn't expand in zshrs. Niche.
    #[test]
    #[ignore = "zshrs gap: brace range over high/multibyte single-byte endpoints not expanded"]
    fn brace_range_high_bytes() {
        assert_parity(r#"print -r -- {$'\M-\C-@'..$'\M-\C-A'}"#);
    }
}
