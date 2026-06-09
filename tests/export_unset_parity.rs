//! export / unset / readonly parity tests.

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

mod export_basic {
    use super::*;

    /// Exported var visible in subprocess.
    #[test]
    fn exported_var_visible_in_child() {
        assert_parity(r#"export X=visible; sh -c 'echo $X'"#);
    }

    /// Non-exported var invisible to child.
    #[test]
    fn non_exported_var_invisible_in_child() {
        assert_parity(r#"X=hidden; sh -c 'echo "[$X]"'"#);
    }

    /// `export NAME` (no value) exports existing var.
    #[test]
    fn export_existing_var_no_value() {
        assert_parity(r#"X=value; export X; sh -c 'echo $X'"#);
    }

    /// `export` then reassign — child still sees new value.
    #[test]
    fn export_then_reassign() {
        assert_parity(r#"export X=first; X=second; sh -c 'echo $X'"#);
    }

    /// Multiple exports in one line.
    #[test]
    fn export_multiple_at_once() {
        assert_parity(r#"export A=1 B=2 C=3; sh -c 'echo $A $B $C'"#);
    }
}

mod export_listing {
    use super::*;

    /// `export` (no args) lists exported vars. Skip strict compare;
    /// pin exit code.
    #[test]
    fn export_no_args_exits_zero() {
        assert_parity("export >/dev/null; echo $?");
    }
}

mod export_unexport {
    use super::*;

    /// `export -n NAME` removes export attribute (keeps var).
    /// After `-n`, child no longer sees the var.
    #[test]
    fn export_dash_n_unexports() {
        assert_parity(r#"export X=visible; export -n X; sh -c 'echo "[$X]"'; echo "self:$X""#);
    }
}

mod unset_basic {
    use super::*;

    #[test]
    fn unset_removes_var() {
        assert_parity(r#"X=value; unset X; echo "[${X:-empty}]""#);
    }

    #[test]
    fn unset_unset_var_noop() {
        assert_parity(r#"unset NEVER_SET; echo $?"#);
    }

    /// `unset` of multiple at once.
    #[test]
    fn unset_multiple_at_once() {
        assert_parity(r#"A=1; B=2; C=3; unset A B C; echo "[$A][$B][$C]""#);
    }

    /// `unset -f` removes function (separate namespace).
    #[test]
    fn unset_f_removes_function_separately() {
        assert_parity(r#"f() { echo hi; }; unset -f f; f 2>/dev/null; echo done"#);
    }

    /// `unset -v NAME` is the var-only form.
    #[test]
    fn unset_v_var_only() {
        assert_parity(r#"X=val; unset -v X; echo "[${X:-cleared}]""#);
    }
}

mod readonly_basic {
    use super::*;

    #[test]
    fn readonly_set_succeeds() {
        assert_parity(r#"readonly X=value; echo $X"#);
    }

    /// Reassigning a readonly var errors (and may exit on some shells).
    #[test]
    fn readonly_reassign_errors() {
        assert_parity(r#"readonly X=value; X=new 2>/dev/null; echo "[$X]"; echo "exit=$?""#);
    }

    /// `unset` of readonly var errors.
    #[test]
    fn unset_readonly_errors() {
        assert_parity(r#"readonly X=value; unset X 2>/dev/null; echo "[$X]"; echo "exit=$?""#);
    }
}

mod typeset_x_equivalence {
    use super::*;

    /// `typeset -x` is alias for `export`.
    #[test]
    fn typeset_x_exports() {
        assert_parity(r#"typeset -x X=value; sh -c 'echo $X'"#);
    }

    /// `declare -x` (bash-compat) — same.
    #[test]
    fn declare_x_exports() {
        assert_parity(r#"declare -x X=value; sh -c 'echo $X'"#);
    }
}

mod export_with_unset {
    use super::*;

    /// Unsetting exported var removes from env too.
    #[test]
    fn unset_exported_var_clears_env() {
        assert_parity(r#"export X=value; unset X; sh -c 'echo "[$X]"'"#);
    }
}

mod array_export_zsh {
    use super::*;

    /// zsh refuses to export non-PATH-style array vars: `typeset -gx arr`
    /// is accepted but the array doesn't appear in the env passed to `sh`.
    /// Pin: both shells emit `[]` because `sh` sees an unset `arr`.
    /// Previously marked divergent; regression-pinned now that zshrs
    /// agrees.
    #[test]
    fn export_array_joins_with_colon() {
        assert_parity(r#"arr=(a b c); typeset -gx arr; sh -c 'echo "[$arr]"'"#);
    }
}

mod export_in_pipeline {
    use super::*;

    /// `export X=val | cmd` — `cmd` doesn't see X (each pipeline stage
    /// is a subshell in zsh by default).
    #[test]
    fn export_in_pipeline_doesnt_leak_to_next_stage() {
        // The export happens in left stage; cat on right sees stdin only.
        assert_parity(r#"export X=val | cat; echo "outer:$X""#);
    }
}
