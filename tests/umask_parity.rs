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

// ── C-parity pins: additional symbolic/octal edge cases ─────────────

mod octal_3_digit_pins {
    use super::*;

    /// 4-digit octal with leading zero is a common form.
    #[test]
    fn set_0022_leading_zero_form() {
        assert_parity(r#"umask 0022; umask"#);
    }

    /// Smallest non-zero mask.
    #[test]
    fn set_001_minimal_mask() {
        assert_parity(r#"umask 001; umask"#);
    }

    /// Asymmetric mask: 027 = group has read+execute, other none.
    #[test]
    fn set_027_asymmetric_mask() {
        assert_parity(r#"umask 027; umask"#);
    }

    /// Re-setting same mask is idempotent — observable via two prints.
    #[test]
    fn set_022_twice_is_idempotent() {
        assert_parity(r#"umask 022; umask; umask 022; umask"#);
    }
}

mod symbolic_a_class {
    use super::*;

    /// `a=rx` sets all three classes (user/group/other) to rx — mask 333.
    #[test]
    fn symbolic_a_equals_rx() {
        assert_parity(r#"umask a=rx; umask"#);
    }

    /// `a-w` removes write from all three classes — adds 222 to mask.
    #[test]
    fn symbolic_a_minus_w() {
        assert_parity(r#"umask 000; umask a-w; umask"#);
    }

    /// `+x` (no class prefix) → `a+x` per POSIX.
    #[test]
    fn symbolic_bare_plus_x_means_all() {
        assert_parity(r#"umask 777; umask +x; umask"#);
    }
}

mod symbolic_S_round_trip {
    use super::*;

    /// `umask -S` after symbolic set: u=rwx,g=rx,o=rx (i.e. 022).
    #[test]
    fn dash_S_after_symbolic_022() {
        assert_parity(r#"umask u=rwx,g=rx,o=rx; umask -S"#);
    }

    /// `umask -S` after symbolic restrictive set: 077.
    #[test]
    fn dash_S_after_symbolic_077() {
        assert_parity(r#"umask u=rwx,g=,o=; umask -S"#);
    }
}

mod invalid_extra {
    use super::*;

    /// Negative-looking argument → error.
    #[test]
    fn negative_octal_errors() {
        assert_parity(r#"umask -1 2>/dev/null; echo exit=$?"#);
    }

    /// Trailing garbage after octal → error.
    #[test]
    fn octal_with_trailing_garbage_errors() {
        assert_parity(r#"umask 022xyz 2>/dev/null; echo exit=$?"#);
    }

    /// Symbolic with invalid class letter → error.
    #[test]
    fn symbolic_invalid_class_errors() {
        assert_parity(r#"umask q=r 2>/dev/null; echo exit=$?"#);
    }
}
