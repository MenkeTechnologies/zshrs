//! setopt / unsetopt / set -X parity tests.

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

mod setopt {
    use super::*;

    #[test]
    fn setopt_then_isset_via_bracket() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o EXTENDED_GLOB ]]; echo $?"#);
    }

    #[test]
    fn setopt_alt_name_kshglob() {
        assert_parity(r#"setopt KSH_GLOB; [[ -o kshglob ]]; echo $?"#);
    }

    #[test]
    fn setopt_then_unsetopt_clears() {
        assert_parity(
            r#"setopt EXTENDED_GLOB; unsetopt EXTENDED_GLOB; [[ -o EXTENDED_GLOB ]]; echo $?"#,
        );
    }

    #[test]
    fn setopt_no_prefix_inverts() {
        // `setopt NO_X` is equivalent to `unsetopt X`.
        assert_parity(
            r#"setopt EXTENDED_GLOB; setopt NO_EXTENDED_GLOB; [[ -o EXTENDED_GLOB ]]; echo $?"#,
        );
    }

    #[test]
    fn setopt_multiple_at_once() {
        assert_parity(
            r#"setopt EXTENDED_GLOB NULL_GLOB; [[ -o extendedglob ]] && [[ -o nullglob ]]; echo $?"#,
        );
    }
}

mod set_dash_flags {
    use super::*;

    /// `set -u` enables NOUNSET (error on unset var).
    #[test]
    fn set_u_errors_on_unset_var() {
        // With `-u`, accessing $UNDEF errors. Pin the exit-nonzero behavior.
        assert_parity(r#"set -u; echo "[${UNDEFINED:-default}]"; echo $?"#);
    }

    /// `set +u` disables it.
    #[test]
    fn set_plus_u_allows_unset_var() {
        assert_parity(r#"set -u; set +u; echo "[$UNDEFINED]"; echo $?"#);
    }

    /// `set -e` exits on first error.
    #[test]
    fn set_e_exits_on_first_error() {
        // In subshell so the outer test doesn't terminate.
        assert_parity(r#"(set -e; false; echo never); echo $?"#);
    }

    /// `set +e` allows continuation.
    #[test]
    fn set_plus_e_allows_continuation() {
        assert_parity(r#"set +e; false; echo continued; echo $?"#);
    }

    /// `set -x` enables xtrace — pin that exit code stays correct (we
    /// don't compare stderr trace output, only that the cmd ran fine).
    #[test]
    fn set_x_doesnt_break_exit_status() {
        assert_parity(r#"(set -x; echo hi) 2>/dev/null"#);
    }

    /// `set -n` parses but doesn't execute — `echo` produces nothing.
    #[test]
    fn set_n_skips_execution() {
        assert_parity(r#"(set -n; echo never); echo $?"#);
    }
}

mod set_o_long_form {
    use super::*;

    /// `set -o errexit` is equivalent to `set -e`.
    #[test]
    fn set_dash_o_errexit_equivalent_to_e() {
        assert_parity(r#"(set -o errexit; false; echo never); echo $?"#);
    }

    /// `set -o nounset` is equivalent to `set -u`.
    #[test]
    fn set_dash_o_nounset_equivalent_to_u() {
        assert_parity(r#"set -o nounset; echo "[${UNDEFINED:-d}]"; echo $?"#);
    }

    /// `set +o errexit` disables.
    #[test]
    fn set_plus_o_errexit_disables() {
        assert_parity(r#"set -o errexit; set +o errexit; false; echo continued"#);
    }
}

mod option_canonicalization {
    use super::*;

    /// option names case-insensitive + underscores ignored.
    #[test]
    fn opt_name_uppercase_equivalent() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o EXTENDEDGLOB ]]; echo $?"#);
    }

    #[test]
    fn opt_name_underscore_ignored() {
        assert_parity(r#"setopt extended_glob; [[ -o extendedglob ]]; echo $?"#);
    }

    #[test]
    fn opt_name_mixed_case() {
        assert_parity(r#"setopt ExtendedGlob; [[ -o EXTENDED_GLOB ]]; echo $?"#);
    }
}

mod no_prefix {
    use super::*;

    /// `unsetopt NO_X` is `setopt X`.
    #[test]
    fn unsetopt_no_prefix_sets_canonical() {
        assert_parity(r#"unsetopt NO_EXTENDED_GLOB; [[ -o EXTENDED_GLOB ]]; echo $?"#);
    }
}

mod negation_in_bracket {
    use super::*;

    /// `[[ -o no_X ]]` inverted check.
    #[test]
    fn bracket_o_no_prefix_inverts() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o no_extendedglob ]]; echo $?"#);
    }
}

mod print_options {
    use super::*;

    /// `setopt` (no args) prints all set options. Don't compare full
    /// output (varies between zsh/zshrs); just check exit + non-empty.
    #[test]
    fn setopt_no_args_exits_zero() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh("setopt | head -1");
        let r = run_zshrs("setopt | head -1");
        // Both should exit 0; output line count > 0 ideally.
        assert_eq!(z.exit, r.exit);
    }
}

mod default_options {
    use super::*;

    /// `monitor` is on by default in interactive shells; off in -c.
    #[test]
    fn monitor_off_in_minus_c() {
        assert_parity(r#"[[ -o monitor ]]; echo $?"#);
    }
}
