//! Parity for `local`/`typeset` shadowing a SPECIAL parameter inside a function.
//!
//! The interesting leg is a SCALAR value assigned to a name whose global is a
//! tied array special (`path`/`PATH`, `fignore`/`FIGNORE`, `cdpath`/`CDPATH`,
//! `manpath`, `fpath`, `psvar`, `mailpath`, ...). C's `typeset_single`
//! (Src/builtin.c) clears `usepm` at c:2078-2090 when the existing pm lives at
//! an outer `locallevel` and `PM_LOCAL` is requested, so the
//! "inconsistent type for assignment" test at c:2233-2237 — which lives inside
//! `if (usepm)` — is never reached. The newspecial block (c:2381-2425) then
//! preserves the special's `PM_TYPE`, and the assignment tail at c:2564-2578
//! leniently turns the scalar into a one-element array:
//!
//!     /*
//!      * Attempt to assign a scalar value to an array.
//!      * This can happen if the array is special.
//!      * We'll be lenient and guess what the user meant.
//!      * This is how normal assignment works.
//!      */
//!
//! zshrs evaluated the c:2236 test unconditionally, so every
//! `local path=<scalar>` in a function was rejected. docs/BUGS.md #1110.
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
    stderr: String,
}

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path()).args(["-fc", s]).output().expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// stdout must match byte-for-byte.
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
}

/// stdout AND stderr must match byte-for-byte. Used for the negative controls,
/// where the *diagnostic* is the observable behaviour under test.
///
/// The shell's exit status is deliberately not compared here: `local status=0`
/// exits 0 under zsh 5.9 and 1 under zshrs today (both abort the function and
/// print the identical `read-only variable: status` diagnostic). That gap is
/// unrelated to the tied-special shadowing this file covers, and pinning it
/// would make these tests fail for the wrong reason.
fn assert_parity_err(s: &str) {
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
        z.stderr, r.stderr,
        "stderr divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stderr, r.stderr
    );
    assert!(
        !z.stderr.is_empty(),
        "negative control produced no zsh diagnostic; the case no longer errors:\n{s}"
    );
}

/// The positive leg: a scalar value on a tied-array special being localized.
mod scalar_shadowing_tied_special {
    use super::*;

    /// The headline repro. Bug #1110.
    #[test]
    fn local_path_scalar_value_is_accepted() {
        assert_parity(r#"f(){ local path=/a/b; print -r -- "got:$path"; }; f"#);
    }

    /// c:2381-2425 keeps `PM_TYPE`, so the local is still the tied array; the
    /// scalar becomes a ONE-element array (c:2570-2572 `mkarray(ztrdup(...))`).
    #[test]
    fn local_path_scalar_stays_a_tied_array() {
        assert_parity(r#"f(){ local path=/a/b; print -r -- "${(t)path} $#path"; }; f"#);
    }

    /// c:2574-2576 — an EMPTY scalar is `mkarray(NULL)`, i.e. a zero-element
    /// array, not a one-element array holding "".
    #[test]
    fn local_path_empty_scalar_is_an_empty_array() {
        assert_parity(r#"f(){ local path=""; print -r -- "${(t)path} $#path"; }; f"#);
    }

    /// The tie has to stay live: writing the lowercase array must be visible
    /// through the uppercase scalar partner for the duration of the scope.
    #[test]
    fn local_path_scalar_propagates_to_PATH() {
        assert_parity(r#"f(){ local path=/a/b; print -r -- "$PATH"; }; f"#);
    }

    /// The shadow must unwind at scope exit — a clobbered `$PATH` would break
    /// every later command lookup in the shell.
    #[test]
    fn PATH_is_restored_after_the_scope_ends() {
        assert_parity(
            r#"save=$PATH; f(){ local path=/a/b; }; f; [[ $PATH == $save ]] && print restored || print CLOBBERED"#,
        );
    }

    /// A callee inherits the localized value (dynamic scoping through the tie).
    #[test]
    fn nested_function_sees_the_localized_PATH() {
        assert_parity(r#"f(){ local path=/a/b; g(){ print -r -- "in-g=$PATH"; }; g; }; f"#);
    }

    /// c:2306 — an array/hashed param gets no scalar env entry of its own; the
    /// tie reaches the environment through `PATH`. A stray lowercase `path=`
    /// in `environ` would be inherited by every child process.
    #[test]
    fn localized_path_does_not_leak_a_lowercase_env_entry() {
        assert_parity(
            r#"f(){ local path=/a/b; local -a e; e=(${(f)"$(/usr/bin/env)"}); print -r -- "leaks=${#${(M)e:#path=*}}"; }; f"#,
        );
    }

    /// The other tied pairs travel the same code path — the fix must be about
    /// PM_TIED/PM_SPECIAL in general, never about the name `path`.
    #[test]
    fn other_tied_specials_take_the_same_path() {
        assert_parity(r#"f(){ local fignore=x; print -r -- "${(t)fignore} $fignore"; }; f"#);
        assert_parity(r#"f(){ local cdpath=/a; print -r -- "${(t)cdpath} $cdpath"; }; f"#);
        assert_parity(r#"f(){ local manpath=/m; print -r -- "${(t)manpath} $MANPATH"; }; f"#);
        assert_parity(r#"f(){ local psvar=abc; print -r -- "${(t)psvar} $psvar"; }; f"#);
        assert_parity(r#"f(){ local mailpath=/m; print -r -- "${(t)mailpath} $mailpath"; }; f"#);
    }

    /// `typeset` is the same builtin with a different `func`; `local` is not a
    /// special case of the rule.
    #[test]
    fn typeset_form_behaves_like_local() {
        assert_parity(r#"f(){ typeset path=/a/b; print -r -- "${(t)path} $path"; }; f"#);
    }

    /// An anonymous function is also a `locallevel` bump.
    #[test]
    fn anonymous_function_scope_also_shadows() {
        assert_parity(r#"f(){ () { local path=/a/b; print -r -- $path; }; }; f"#);
    }

    /// An outer *scalar-shaped* write to the tied array must be restored.
    #[test]
    fn outer_value_is_restored_after_the_shadow() {
        assert_parity(r#"path=/top; f(){ local path=/in; print -r -- $path; }; f; print -r -- $path"#);
    }
}

/// Legs that already worked and must not regress: the fix must not turn every
/// type mismatch into a silent coercion.
mod unaffected_legs {
    use super::*;

    #[test]
    fn array_value_form_still_works() {
        assert_parity(r#"f(){ local path=(/a /b); print -r -- "${(t)path} $path"; }; f"#);
    }

    /// The uppercase tied partner is a genuine scalar; it must stay one.
    #[test]
    fn tied_scalar_partner_stays_scalar() {
        assert_parity(r#"f(){ local PATH=/a/b; print -r -- "${(t)PATH} $PATH"; }; f"#);
    }

    #[test]
    fn declare_then_assign_array_still_works() {
        assert_parity(r#"f(){ local -a path; path=(/x); print -r -- "${(t)path} $path"; }; f"#);
    }

    /// The already-fixed assoc leg (memory note `local_shadow_special_assoc_wipe`).
    #[test]
    fn local_assoc_shadow_of_a_hashed_special() {
        assert_parity(r#"f(){ local -A commands; print -r -- ${#commands}; }; f"#);
    }

    /// `-h` opts OUT of the newspecial preserve (c:2083-2085 `!(on & PM_HIDE)`),
    /// so the local is an ordinary scalar and the scalar value lands as-is.
    #[test]
    fn hide_flag_makes_it_a_plain_local_scalar() {
        assert_parity(r#"f(){ local -h path=/a/b; print -r -- "${(t)path} $path"; }; f"#);
    }

    /// A NON-special outer array is NOT preserved: `newspecial` stays NS_NONE,
    /// so `createparam` makes a plain local scalar and the outer array survives.
    #[test]
    fn non_special_outer_array_yields_a_plain_local_scalar() {
        assert_parity(
            r#"v=(1 2); f(){ local v=x; print -r -- "${(t)v} $v"; }; f; print -r -- "out=$v""#,
        );
    }

    #[test]
    fn unique_flag_still_dedups_the_localized_array() {
        assert_parity(r#"f(){ local -U path=(/a /a /b); print -r -- $path; }; f"#);
    }

    #[test]
    fn export_flag_rides_along_on_the_preserved_special() {
        assert_parity(r#"f(){ local -x path=/a/b; print -r -- "${(t)path}"; }; f"#);
    }
}

/// Negative controls. Every one of these MUST keep erroring; a fix that
/// silences any of them has over-reached.
mod still_rejected {
    use super::*;

    /// `local status=0` — `status` is the readonly alias of `?`. Different
    /// diagnostic (`read-only variable`), and it must survive untouched.
    #[test]
    fn local_status_is_still_read_only() {
        assert_parity_err(r#"f(){ local status=0; print -r -- reached; }; f"#);
    }

    /// c:2340-2345 — an EXPLICITLY requested array (`-a`) with a scalar value
    /// is inconsistent no matter what the outer parameter is. This is the half
    /// of the test that lives OUTSIDE `if (usepm)`, so it still fires.
    #[test]
    fn local_dash_a_with_scalar_value_still_errors() {
        assert_parity_err(r#"f(){ local -a path=/a/b; print -r -- reached; }; f"#);
    }

    /// At top level `locallevel == pm->level`, so `usepm` survives c:2078 and
    /// the c:2236 test still applies.
    #[test]
    fn top_level_scalar_assign_to_tied_special_still_errors() {
        assert_parity_err(r#"typeset path=/a/b; print -r -- reached"#);
        assert_parity_err(r#"local path=/a/b; print -r -- reached"#);
    }

    /// `-g` clears `PM_LOCAL`, so the c:2078 clause cannot fire and the reuse
    /// branch (with its c:2236 test) is taken even inside a function.
    #[test]
    fn typeset_g_inside_a_function_still_errors() {
        assert_parity_err(r#"f(){ typeset -g path=/a/b; print -r -- reached; }; f"#);
    }

    /// Re-declaring at the SAME `locallevel`: the second `local` sees a pm at
    /// its own level, `usepm` stays set, c:2236 fires.
    #[test]
    fn redeclaring_at_the_same_level_still_errors() {
        assert_parity_err(r#"f(){ local path; local path=/a/b; print -r -- reached; }; f"#);
    }

    /// Same-scope local array, then a scalar assign to it.
    #[test]
    fn scalar_assign_to_a_same_scope_local_array_still_errors() {
        assert_parity_err(r#"f(){ local -a v=(1 2); local v=x; print -r -- reached; }; f"#);
    }

    /// The plain (non-special, non-local) reuse case from c:2236.
    #[test]
    fn scalar_assign_to_an_existing_assoc_still_errors() {
        assert_parity_err(r#"typeset -A h=(k v); typeset h=scalar; print -r -- reached"#);
    }

    #[test]
    fn scalar_assign_to_an_existing_array_still_errors() {
        assert_parity_err(r#"typeset -a a=(1 2); typeset a=x; print -r -- reached"#);
    }
}
