//! A subscripted reference to an unset SLOT under `NO_UNSET` — the plain
//! value read, not the `${#…}` length (see `nounset_length_slot_parity`).
//!
//! `c:Src/subst.c:2804-2807` sets `vunset` when fetchvalue/getindex find no
//! node for the slot: a missing hash key (`c:Src/params.c:1597-1606`), an
//! array index outside the elements (`c:Src/subst.c:2944-2954`), or no
//! parameter at all. `c:Src/subst.c:3608-3613` then reports
//! `zerr("%s: parameter not set", idbeg)` under NO_UNSET, `idbeg` being the
//! reference as written.
//!
//! Two zshrs fast paths answered before that check: the single-key hash read
//! in the bridge's `array_index_lookup` (every `${h[k]}` / `$h[k]`), and the
//! unbraced walk an expanded subscript takes (`$a[$k]`, `$h[$k]`, `x=$a[$k]`).
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

#![allow(non_snake_case)]

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

fn zsh_path() -> &'static str {
    crate::oracle::zsh_path()
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

mod missing_hash_key_aborts {
    use super::*;

    #[test]
    fn braced_literal_key() {
        assert_parity("setopt nounset; typeset -A h; h[a]=1; print -r -- ${h[b]}; print -r -- after");
    }

    #[test]
    fn unbraced_literal_key() {
        assert_parity("setopt nounset; typeset -A h; h[a]=1; print -r -- $h[b]; print -r -- after");
    }

    #[test]
    fn unbraced_expanded_key_names_the_reference_as_written() {
        assert_parity("setopt nounset; typeset -A h; h[a]=1; k=b; print -r -- $h[$k]; print -r -- after");
    }

    #[test]
    fn scalar_assignment_rhs() {
        assert_parity("setopt nounset; typeset -A h; h[a]=1; k=b; x=$h[$k]; print -r -- after");
    }

    #[test]
    fn exact_key_flag() {
        assert_parity("setopt nounset; typeset -A h; print -r -- ${h[(e)b]}; print -r -- after");
    }

    #[test]
    fn inside_function() {
        assert_parity("setopt nounset; typeset -A h; f() { print -r -- ${h[b]} }; f; print -r -- st=$?");
    }
}

mod missing_array_slot_aborts {
    use super::*;

    #[test]
    fn expanded_index_past_the_end() {
        assert_parity("setopt nounset; a=(x); k=3; print -r -- $a[$k]; print -r -- after");
    }

    #[test]
    fn expanded_index_zero() {
        assert_parity("setopt nounset; a=(x); k=0; print -r -- \"$a[$k]\"; print -r -- after");
    }

    #[test]
    fn expanded_negative_index_before_the_start() {
        assert_parity("setopt nounset; a=(x); k=-5; x=$a[$k]; print -r -- after");
    }

    #[test]
    fn missing_parameter() {
        assert_parity("setopt nounset; k=1; print -r -- $nosuch[$k]; print -r -- after");
    }
}

/// The slots that DO exist, and the reads C never reports.
mod set_slots_still_read {
    use super::*;

    #[test]
    fn present_key_and_element() {
        assert_parity("setopt nounset; typeset -A h; h[a]=1; a=(x y); k=a; i=2; print -r -- $h[$k] ${h[a]} $a[$i]");
    }

    #[test]
    fn present_key_with_empty_value() {
        assert_parity("setopt nounset; typeset -A h; h[a]=; k=a; print -r -- \"[$h[$k]]\" \"[${h[a]}]\"");
    }

    #[test]
    fn scalar_character_past_the_end() {
        assert_parity("setopt nounset; s=abc; k=9; print -r -- \"[$s[$k]]\" st=$?");
    }

    #[test]
    fn alias_view_miss() {
        assert_parity("setopt nounset; print -r -- \"[${aliases[nosuch]}]\" st=$?");
    }

    #[test]
    fn default_operator_supplies_a_value() {
        assert_parity("setopt nounset; typeset -A h; print -r -- ${h[b]-d} ${h[b]:-e} ${+h[b]}");
    }

    #[test]
    fn nounset_off() {
        assert_parity("typeset -A h; k=b; a=(x); i=4; print -r -- \"[$h[$k]]\" \"[${h[b]}]\" \"[$a[$i]]\"");
    }
}
