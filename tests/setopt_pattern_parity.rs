//! `setopt`/`unsetopt` deep parity:
//! -m pattern, listing, no_underscore alias, set / [[ -o opt ]], INTERACTIVE_COMMENTS, etc.

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
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
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
        exit: o.status.code().unwrap_or(-1),
    }
}
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
    assert_eq!(z.exit, r.exit);
}

mod basic {
    use super::*;

    /// `setopt extendedglob` then `[[ -o extendedglob ]]`.
    #[test]
    fn setopt_then_test_o() {
        assert_parity(r#"setopt extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `[[ -o opt ]]` on an invalid option name returns $?=3 (per
    /// zsh's `[[ -o invalid ]]` semantics — distinct from unset's $?=1).
    #[test]
    fn test_o_not_set_default() {
        assert_parity(r#"[[ -o nonexistent_option_xyz ]] 2>/dev/null; echo $?"#);
    }

    /// `unsetopt extendedglob` toggles back.
    #[test]
    fn unsetopt_clears() {
        assert_parity(
            r#"setopt extendedglob; unsetopt extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod underscore_case_alias {
    use super::*;

    /// `setopt EXTENDED_GLOB` (underscored uppercase) same as `extendedglob`.
    #[test]
    fn underscored_uppercase_works() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o extendedglob ]]; echo $?"#);
    }

    /// CamelCase `setopt ExtendedGlob` should also work.
    #[test]
    fn camel_case_works() {
        assert_parity(r#"setopt ExtendedGlob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `no_extended_glob` is the negation form.
    #[test]
    fn no_prefix_negates() {
        assert_parity(
            r#"setopt extendedglob; setopt no_extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod no_alias_options {
    use super::*;

    /// Some opts have built-in `NO_` alias for inverse meaning.
    /// `setopt nounset` = `setopt no_unset` etc.
    #[test]
    fn nounset_via_short_form() {
        assert_parity(r#"setopt nounset; [[ -o nounset ]]; echo $?"#);
    }
}

mod setopt_m_pattern {
    use super::*;

    /// `setopt -m 'extended*'` matches multiple options.
    #[test]
    fn setopt_m_pattern_enables_multi() {
        assert_parity(r#"setopt -m 'extended*'; [[ -o extendedglob ]]; echo $?"#);
    }
}

mod listing {
    use super::*;

    /// `setopt` (no arg) lists active options.
    #[test]
    fn setopt_no_arg_lists() {
        assert_parity(r#"setopt extendedglob; setopt 2>&1 | grep -c extendedglob"#);
    }
}

mod set_dash_o {
    use super::*;

    /// `set -o name` enables.
    #[test]
    fn set_dash_o_enables() {
        assert_parity(r#"set -o extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `set +o name` disables.
    #[test]
    fn set_plus_o_disables() {
        assert_parity(
            r#"set -o extendedglob; set +o extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod set_dash_short_flags {
    use super::*;

    /// `set -e` = errexit.
    #[test]
    fn set_dash_e_is_errexit() {
        assert_parity(r#"set -e; [[ -o errexit ]]; echo $?"#);
    }

    /// `set -u` = nounset.
    #[test]
    fn set_dash_u_is_nounset() {
        assert_parity(r#"set -u; [[ -o nounset ]]; echo $?"#);
    }

    /// `set -x` = xtrace.
    #[test]
    fn set_dash_x_is_xtrace() {
        assert_parity(r#"set -x 2>/dev/null; [[ -o xtrace ]]; echo $?"#);
    }
}

mod common_options_behavior {
    use super::*;

    /// `setopt err_exit` then false → script aborts (in subshell).
    #[test]
    fn errexit_aborts_on_false() {
        assert_parity(r#"(set -e; false; echo "should not print"); echo exit=$?"#);
    }

    /// `setopt no_unset` (nounset) → reading unset var errors.
    #[test]
    fn nounset_errors_on_unset_var() {
        assert_parity(r#"(setopt nounset; echo "[$NONEXISTENT_XYZ]") 2>/dev/null; echo exit=$?"#);
    }

    /// `setopt pipefail` propagates first nonzero pipe exit.
    #[test]
    fn pipefail_propagates_first_failure() {
        assert_parity(r#"setopt pipefail; false | true; echo $?"#);
    }
}

mod interactive_comments {
    use super::*;

    /// Default zsh non-interactive: comments work in scripts.
    #[test]
    fn comments_in_script() {
        assert_parity(
            r#"
# this is a comment
echo hi
# trailing
"#,
        );
    }

    /// `setopt interactive_comments` enables `#` in interactive shells —
    /// non-interactive: always allowed.
    #[test]
    fn setopt_interactive_comments_takes() {
        assert_parity(r#"setopt interactive_comments; echo "before" # inline; echo "after""#);
    }
}

mod glob_options {
    use super::*;

    /// `setopt null_glob` → unmatched globs become empty.
    #[test]
    fn null_glob_unmatched_empty() {
        assert_parity(
            r#"setopt null_glob; for f in /tmp/nonexistent_xyz_*.txt; do echo "[$f]"; done; echo done"#,
        );
    }

    /// `setopt no_match` (default) → unmatched globs error.
    #[test]
    fn nomatch_errors_on_unmatched_glob() {
        assert_parity(r#"echo /tmp/nonexistent_xyz_*.txt 2>/dev/null; echo exit=$?"#);
    }

    /// `unsetopt nomatch` allows unmatched globs (literal).
    #[test]
    fn unsetopt_nomatch_allows_literal() {
        assert_parity(r#"unsetopt nomatch; echo /tmp/nonexistent_xyz_*.txt"#);
    }
}

mod option_precedence {
    use super::*;

    /// Last setopt wins.
    #[test]
    fn last_setopt_wins() {
        assert_parity(
            r#"setopt extendedglob; unsetopt extendedglob; setopt extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod option_persist_in_subshell {
    use super::*;

    /// Options set in outer visible in subshell.
    #[test]
    fn options_inherited_by_subshell() {
        assert_parity(
            r#"setopt extendedglob; (echo "[[ -o extendedglob ]] is: $([[ -o extendedglob ]]; echo $?)")"#,
        );
    }

    /// Options set in subshell don't leak out.
    #[test]
    fn subshell_setopt_no_leak() {
        assert_parity(r#"(setopt extendedglob); [[ -o extendedglob ]]; echo $?"#);
    }
}
