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

    /// KSH_ARRAYS also changes assoc scalar access: `${(ko)as}` → the first
    /// value. zsh: `v1`.  zshrs: `k1 k2`.
    #[test]
    #[ignore = "zshrs gap: KSH_ARRAYS ${(ko)assoc} scalar access diverges"]
    fn assoc_scalar_access() {
        assert_parity("typeset -A as; as=(k1 v1 k2 v2); setopt KSH_ARRAYS; print -r -- ${(ko)as}");
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
// X. Arithmetic-error exit code inside a typeset assignment
//
// A math error in the RHS of a `typeset`/`integer`/`float`/`local` assignment
// PRESERVES the prior exit status in zsh (exec.c:4287-4294 skips execbuiltin
// when errflag is set, leaving lastval unchanged) — so a fresh shell exits 0,
// but `false; typeset -i x=$(( 1/0 ))` exits 1. zshrs forces exit 1. This is a
// core-exec exit-status subtlety (it varies by command type: `print $((1/0))`
// exits 1 on both, `/bin/echo $((1/0))` diverges the other way), not a local
// typeset fix — left pinned rather than reworking exit-status propagation.
// ─────────────────────────────────────────────────────────────────────
mod typeset_arith_error_exit {
    use super::*;

    /// zsh: exit 0 (prior status preserved).  zshrs: exit 1.
    #[test]
    #[ignore = "zshrs gap: typeset -i x=$((1/0)) exits 1; zsh preserves prior status (0)"]
    fn integer_assign_divzero_exit() {
        assert_parity("typeset -i x=$(( 1/0 )); print ok");
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
}
