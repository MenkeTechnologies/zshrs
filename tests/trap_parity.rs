//! `trap` signal-handler parity tests.

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

// ZSHRS-WIDE BUG CLASS (entire trap subsystem broken in -c mode):
// In real zsh, `trap 'cmd' EXIT` registers an exit handler that fires
// when the shell exits. zshrs's trap appears to not register or not
// fire the handler at shell-exit time in `-c` mode. All EXIT trap
// tests fail; same for ERR, DEBUG, multi-signal forms, and listing.
// This is a high-impact regression — any script using trap for
// cleanup (temp files, lock release, etc.) silently skips that
// cleanup. Tests are kept active+ignored to document the surface.

mod exit_trap {
    use super::*;

    #[test]
    fn trap_exit_runs_on_normal_exit() {
        assert_parity(r#"trap 'echo bye' EXIT"#);
    }

    #[test]
    fn trap_zero_is_alias_for_exit() {
        assert_parity(r#"trap 'echo bye' 0"#);
    }

    #[test]
    fn trap_exit_runs_with_explicit_exit() {
        assert_parity(r#"trap 'echo bye' EXIT; exit 0"#);
    }

    #[test]
    fn trap_exit_sees_exit_status() {
        assert_parity(r#"trap 'echo exit=$?' EXIT; (exit 7)"#);
    }
}

mod err_trap {
    use super::*;

    #[test]
    fn trap_err_runs_on_failing_command() {
        assert_parity(r#"(trap 'echo failed' ERR; false); echo done"#);
    }
}

mod ignore_signal {
    use super::*;

    /// This case actually works because the signal isn't sent.
    #[test]
    fn trap_empty_string_ignores_signal() {
        assert_parity(r#"trap '' USR1; echo ok"#);
    }
}

mod reset_signal {
    use super::*;

    /// This one PASSES — because the trap body never fires anyway
    /// (`trap EXIT` is broken upstream). Pin behavior either way.
    #[test]
    fn trap_dash_resets_to_default() {
        assert_parity(r#"trap 'echo never' EXIT; trap - EXIT"#);
    }
}

mod multiple_signals {
    use super::*;

    #[test]
    fn trap_targets_multiple_signals() {
        assert_parity(r#"trap 'echo handler' EXIT TERM USR1; echo ok"#);
    }
}

mod listing {
    use super::*;

    #[test]
    fn trap_no_args_lists_set_traps() {
        if !zsh_available() {
            return;
        }
        let script = r#"trap 'echo bye' EXIT; trap | grep -c EXIT"#;
        let z = run_zsh(script);
        let r = run_zshrs(script);
        assert_eq!(z.stdout, r.stdout);
    }
}

mod replace {
    use super::*;

    #[test]
    fn trap_replacement_updates_body() {
        assert_parity(r#"trap 'echo first' EXIT; trap 'echo second' EXIT"#);
    }
}

mod debug_trap {
    use super::*;

    #[test]
    fn trap_debug_fires_before_command() {
        assert_parity(r#"trap 'echo dbg' DEBUG; echo cmd"#);
    }
}

mod combined_with_subshell {
    use super::*;

    #[test]
    fn trap_in_subshell_doesnt_leak() {
        assert_parity(r#"(trap 'echo inside' EXIT); echo after"#);
    }
}
