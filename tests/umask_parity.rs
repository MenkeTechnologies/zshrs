//! umask builtin parity tests.

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

mod set_octal {
    use super::*;

    /// Set and read back octal umask.
    #[test]
    fn set_022_and_read_back() {
        assert_parity(r#"umask 022; umask"#);
    }

    #[test]
    fn set_077_and_read_back() {
        assert_parity(r#"umask 077; umask"#);
    }

    #[test]
    fn set_000_and_read_back() {
        assert_parity(r#"umask 000; umask"#);
    }
}

mod symbolic {
    use super::*;

    /// `umask -S` prints symbolic form.
    #[test]
    fn dash_S_symbolic_form() {
        assert_parity(r#"umask 022; umask -S"#);
    }

    #[test]
    fn dash_S_077() {
        assert_parity(r#"umask 077; umask -S"#);
    }

    #[test]
    fn dash_S_with_zero_mask() {
        assert_parity(r#"umask 000; umask -S"#);
    }
}

mod symbolic_input {
    use super::*;

    /// Symbolic umask input: `u=rwx,g=rx,o=rx` → 022.
    #[test]
    fn symbolic_set_u_rwx_g_rx_o_rx() {
        assert_parity(r#"umask u=rwx,g=rx,o=rx; umask"#);
    }

    /// `u-w` removes write bit from user.
    #[test]
    fn symbolic_remove_user_write() {
        assert_parity(r#"umask 022; umask u-w; umask"#);
    }
}

mod inheritance {
    use super::*;

    /// Setting umask affects subshell-created files.
    /// Test by creating file in subshell, checking perms.
    #[test]
    fn umask_022_creates_644_file() {
        assert_parity(
            r#"
umask 022
touch /tmp/zshrs_umask_test_$$
stat -f '%Lp' /tmp/zshrs_umask_test_$$ 2>/dev/null || stat -c '%a' /tmp/zshrs_umask_test_$$
rm /tmp/zshrs_umask_test_$$
"#,
        );
    }

    #[test]
    fn umask_077_creates_600_file() {
        assert_parity(
            r#"
umask 077
touch /tmp/zshrs_umask_test2_$$
stat -f '%Lp' /tmp/zshrs_umask_test2_$$ 2>/dev/null || stat -c '%a' /tmp/zshrs_umask_test2_$$
rm /tmp/zshrs_umask_test2_$$
"#,
        );
    }
}

mod invalid {
    use super::*;

    /// Invalid octal → error.
    #[test]
    fn invalid_octal_999_errors() {
        assert_parity(r#"umask 999 2>/dev/null; echo exit=$?"#);
    }

    /// Non-numeric, non-symbolic → error.
    #[test]
    fn invalid_string_errors() {
        assert_parity(r#"umask xyz 2>/dev/null; echo exit=$?"#);
    }
}

mod read_only {
    use super::*;

    /// `umask` with no arg → current mask in octal.
    #[test]
    fn no_arg_prints_octal() {
        assert_parity(r#"umask 022; umask"#);
    }
}
