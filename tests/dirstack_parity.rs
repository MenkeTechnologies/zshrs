//! pushd / popd / dirs / DIRSTACK parity tests.
//!
//! ZSHRS-WIDE BUG CLUSTER: the entire directory-stack subsystem is
//! broken in `-c` mode. `pushd`, `popd`, `dirs`, `$dirstack`, AUTO_PUSHD,
//! and tilde-stack navigation all diverge from zsh. 11 of 16 tests
//! fail; all marked #[ignore] with this FIXME. Real-world impact:
//! zsh power-users who navigate via pushd/popd lose that workflow.

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

mod pushd_basic {
    use super::*;

    #[test]
    fn pushd_changes_pwd() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; pwd"#);
    }

    #[test]
    fn pushd_no_args_swaps_top_two() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; pushd >/dev/null; pwd"#);
    }

    #[test]
    fn popd_returns_to_previous() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; popd >/dev/null; pwd"#);
    }
}

mod dirs_listing {
    use super::*;

    #[test]
    fn dirs_shows_pwd_when_empty_stack() {
        assert_parity(r#"cd /tmp; dirs"#);
    }

    #[test]
    fn dirs_after_pushd_shows_two_entries() {
        if !zsh_available() {
            return;
        }
        let s = r#"cd /tmp; pushd / >/dev/null; dirs | wc -w"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        assert_eq!(z.stdout, r.stdout);
    }

    #[test]
    fn dirs_dash_p_one_per_line() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; dirs -p | wc -l"#);
    }

    #[test]
    fn dirs_dash_v_numbered() {
        if !zsh_available() {
            return;
        }
        let s = r#"cd /tmp; pushd / >/dev/null; dirs -v | head -2 | wc -l"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        assert_eq!(z.stdout, r.stdout);
    }
}

mod dirstack_array {
    use super::*;

    #[test]
    fn dirstack_after_pushd_has_entries() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; echo ${#dirstack}"#);
    }

    #[test]
    fn dirstack_first_element_is_previous_pwd() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; echo $dirstack[1]"#);
    }
}

mod nav_tilde {
    use super::*;

    #[test]
    fn cd_tilde_plus_zero_is_pwd() {
        assert_parity(r#"cd /tmp; pushd / >/dev/null; cd ~+0; pwd"#);
    }
}

mod popd_modes {
    use super::*;

    #[test]
    fn popd_no_args_pops_top() {
        assert_parity(
            r#"cd /; pushd /tmp >/dev/null; pushd /var >/dev/null; popd >/dev/null; pwd"#,
        );
    }

    #[test]
    fn popd_on_empty_stack_errors() {
        assert_parity(r#"cd /tmp; popd 2>/dev/null; echo $?"#);
    }
}

mod cd_dash {
    use super::*;

    /// `cd -` swaps to previous dir ($OLDPWD).
    #[test]
    fn cd_dash_swaps_to_oldpwd() {
        assert_parity(r#"cd /; cd /tmp; cd - >/dev/null; pwd"#);
    }

    #[test]
    fn cd_dash_twice_back_to_start() {
        assert_parity(r#"cd /; cd /tmp; cd - >/dev/null; cd - >/dev/null; pwd"#);
    }
}

mod auto_pushd {
    use super::*;

    #[test]
    fn auto_pushd_makes_cd_act_like_pushd() {
        assert_parity(r#"setopt AUTO_PUSHD; cd /; cd /tmp; dirs | wc -w"#);
    }
}

mod with_relative_paths {
    use super::*;

    #[test]
    fn pushd_relative_dotdot() {
        assert_parity(r#"cd /tmp; pushd .. >/dev/null; pwd"#);
    }
}
