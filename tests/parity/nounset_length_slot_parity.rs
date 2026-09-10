//! `${#param[SLOT]}` under `NO_UNSET` — the length path must abort on an
//! unset SLOT, not just on an unset NAME.
//!
//! C decides `vunset` from the fetchvalue at `c:Src/subst.c:2801-2807`, and
//! that call runs `getindex` first, so the subscript has already selected a
//! slot by the time set-ness is judged. The `${#…}` branch then reaches the
//! `colonsubscript` guard at `c:Src/subst.c:3614-3620` and errors out.
//!
//! The port's length block had its own inline guard that probed the bare NAME
//! in the vars/arrays/assocs tables, so every subscripted shape read as "set"
//! and `${#nosuch[2]}` / `${#a[9]}` / `${#h[nokey]}` / `${#nosuch[@]}` all
//! answered `0` where zsh aborts. The general nounset guard further down the
//! same function already used the correct slot predicate (`is_set`) — the
//! length path just returns before reaching it.
//!
//! Two things had to stay put while that guard was swapped over:
//!
//!   * a BARE magic-assoc name (`${#builtins}`) is not in the vars/arrays/
//!     assocs tables at all — it is PARTAB scanfn dispatch — so it must not
//!     start erroring. The same name WITH a subscript is a single-key getnode
//!     lookup, which does error on a miss.
//!   * the default/alternate/assign/error operators supply or assign a value
//!     before the length is taken (`${#nosuch=x}` is `1`, and `nosuch` is set
//!     afterwards), so they suppress the abort.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// stdout + stderr + exit-status parity. stderr is compared after normalizing
/// the leading shell name, which differs by construction (`zsh:` vs `zshrs:`).
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");

    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );

    let norm = |s: &str| s.replace("zshrs:", "SHELL:").replace("zsh:", "SHELL:");
    let z_err = norm(&String::from_utf8_lossy(&z.stderr));
    let r_err = norm(&String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        z_err, r_err,
        "stderr divergence on script:\n{script}\n--- zsh ---\n{z_err:?}\n--- zshrs ---\n{r_err:?}"
    );

    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The bug: an unset SLOT under `${#…}` + NO_UNSET. Each of these answered `0`
// with exit 0 instead of aborting. The assertion pins the message text too —
// C truncates at `idend`, so the name it reports includes the subscript.
// ═══════════════════════════════════════════════════════════════════════════

mod unset_slot_aborts {
    use super::*;

    #[test]
    fn missing_name_with_numeric_subscript() {
        assert_parity("setopt nounset; print ${#nosuch[2]}");
    }

    #[test]
    fn array_index_past_the_end() {
        assert_parity("setopt nounset; a=(x y); print ${#a[9]}");
    }

    #[test]
    fn empty_array_index_one() {
        assert_parity("setopt nounset; a=(); print ${#a[1]}");
    }

    #[test]
    fn assoc_key_miss() {
        assert_parity("setopt nounset; typeset -A h=(k v); print ${#h[nokey]}");
    }

    #[test]
    fn assoc_key_miss_through_a_variable() {
        assert_parity("setopt nounset; typeset -A h=(k v); k=zz; print ${#h[$k]}");
    }

    #[test]
    fn missing_name_with_at_splat() {
        assert_parity("setopt nounset; print ${#nosuch[@]}");
    }

    #[test]
    fn missing_name_with_star_splat() {
        assert_parity("setopt nounset; print ${#nosuch[*]}");
    }

    #[test]
    fn missing_name_with_a_range() {
        assert_parity("setopt nounset; print ${#nosuch[1,2]}");
    }

    /// `getarg` math-evaluates a bare-name subscript, so the out-of-range
    /// answer has to come from the RESOLVED index, not from a digit literal.
    #[test]
    fn bare_name_subscript_past_the_end() {
        assert_parity("setopt nounset; a=(x y); i=9; print ${#a[i]}");
    }

    /// KSHARRAYS shifts the index origin; the set-ness probe has to use the
    /// same mapping the value path does, or `[5]` on a 2-element array is
    /// judged in-range.
    #[test]
    fn ksharrays_index_past_the_end() {
        assert_parity("setopt nounset; setopt ksharrays; a=(x y); print ${#a[5]}");
    }

    /// A search subscript on a plain ARRAY yields index 0 on a miss, which is
    /// unset — unlike the same flags on an assoc, covered below.
    #[test]
    fn array_search_subscript_miss() {
        assert_parity("setopt nounset; a=(x y); print ${#a[(r)zz]}");
    }

    /// A magic assoc WITH a subscript is a single-key getnode dispatch
    /// (`Src/Modules/parameter.c`), so a miss is unset like any other key.
    #[test]
    fn magic_assoc_key_miss() {
        assert_parity("setopt nounset; print ${#builtins[nosuchbuiltin]}");
    }

    #[test]
    fn magic_assoc_option_key_miss() {
        assert_parity("setopt nounset; print ${#options[nosuchopt]}");
    }

    #[test]
    fn magic_assoc_function_key_miss() {
        assert_parity("setopt nounset; print ${#functions[nosuchfn]}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Blast radius. Swapping the guard to the slot predicate also moves every
// other shape that reaches this block; these pin the ones that must NOT
// change. Half of them are cases where NOT erroring is the right answer, and
// the naive "any subscript is unset" spelling fails them.
// ═══════════════════════════════════════════════════════════════════════════

mod set_slots_still_answer {
    use super::*;

    #[test]
    fn in_range_array_index() {
        assert_parity("setopt nounset; a=(x y); print ${#a[2]}");
    }

    #[test]
    fn assoc_key_hit() {
        assert_parity("setopt nounset; typeset -A h=(k value); print ${#h[k]}");
    }

    #[test]
    fn scalar_character_index_in_and_out_of_range() {
        // zsh answers `${+s[N]}` with 1 for ANY N once `s` exists; the
        // out-of-range read is an empty VALUE, not an unset slot.
        assert_parity("setopt nounset; s=hello; print ${#s[2]} ${#s[9]}");
    }

    #[test]
    fn array_at_splat_and_range() {
        assert_parity("setopt nounset; a=(x y); print ${#a[@]} ${#a[1,2]} ${#a[3,4]}");
    }

    #[test]
    fn empty_array_and_empty_assoc_count_zero() {
        assert_parity("setopt nounset; a=(); typeset -A h=(); print ${#a} ${#a[@]} ${#h}");
    }

    /// A pattern SEARCH on an assoc scans the existing hash, so it is set even
    /// on a miss — the opposite of the same flags on a plain array.
    #[test]
    fn assoc_search_subscript_is_set_even_on_a_miss() {
        assert_parity("setopt nounset; typeset -A h=(k v); print ${#h[(R)nomatch]}");
    }

    #[test]
    fn array_index_search_returns_zero_not_unset() {
        assert_parity("setopt nounset; a=(x y); print ${#a[(i)x]} ${#a[(I)zz]}");
    }

    /// Bare magic-assoc names are PARTAB dispatch, not table entries. These
    /// are the shapes that would start erroring if the guard only consulted
    /// the slot predicate.
    #[test]
    fn bare_magic_assoc_names_are_set() {
        assert_parity(
            "setopt nounset; print $(( ${#builtins} > 0 )) $(( ${#options} > 0 )) \
             $(( ${#reswords} > 0 ))",
        );
    }

    #[test]
    fn magic_assoc_key_hit() {
        assert_parity("setopt nounset; print ${#options[nounset]}");
    }

    #[test]
    fn positional_parameters() {
        assert_parity("setopt nounset; f(){ print ${#1} }; f arg");
    }

    #[test]
    fn arg_count_and_argv_shapes() {
        assert_parity("setopt nounset; f(){ print ${#} ${#@} ${#*} ${#argv} }; f a b c");
    }

    /// A subexp result has no parameter-table entry of its own; it is set by
    /// construction. This one was erroring BEFORE the fix, from the same
    /// bare-name probe.
    #[test]
    fn subexp_result_is_set() {
        assert_parity("setopt nounset; v='a b c'; print ${#${(z)v}} ${#${(s: :)v}}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Operator suppression. `-` `+` `=` `?` (and their `:` forms) deal with the
// unset case themselves before the length is taken.
// ═══════════════════════════════════════════════════════════════════════════

mod operators_suppress_the_abort {
    use super::*;

    #[test]
    fn default_operators_on_a_missing_name() {
        assert_parity("setopt nounset; print ${#nosuch-x} ${#nosuch:-xy} ${#nosuch+q} ${#nosuch:+q}");
    }

    #[test]
    fn default_operators_on_a_missing_slot() {
        assert_parity("setopt nounset; print ${#nosuch[2]-abc} ${#nosuch[2]:-ab}");
    }

    /// `=` assigns, so the name is SET by the time the length is measured.
    #[test]
    fn assign_operators_assign_then_measure() {
        assert_parity("setopt nounset; print ${#v=q}${v}; print ${#w:=qq}${w}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// With NO_UNSET off, none of the above may error: the unset slot is the empty
// string and its length is 0.
// ═══════════════════════════════════════════════════════════════════════════

mod unset_option_off_is_unaffected {
    use super::*;

    #[test]
    fn unset_slots_measure_zero() {
        assert_parity(
            "a=(x y); typeset -A h=(k v); \
             print ${#nosuch[2]} ${#a[9]} ${#h[nokey]} ${#nosuch[@]} ${#nosuch}",
        );
    }

    #[test]
    fn magic_assoc_key_miss_measures_zero() {
        assert_parity("print ${#builtins[nosuchbuiltin]} ${#functions[nosuchfn]}");
    }

    #[test]
    fn assign_operator_still_assigns() {
        assert_parity("print ${#v=q}${v}");
    }
}
