//! `kill` builtin + signal handling parity:
//! kill -l, kill -SIG, $signals array, signal-by-name vs number.

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

mod kill_dash_l {
    use super::*;

    /// `kill -l` lists all signals.
    #[test]
    fn dash_l_lists_signals() {
        assert_parity(r#"kill -l 2>&1 | wc -w"#);
    }

    /// `kill -l 9` → name of signal 9 (KILL).
    #[test]
    fn dash_l_number_returns_name() {
        assert_parity(r#"kill -l 9"#);
    }

    /// `kill -l KILL` → 9.
    #[test]
    fn dash_l_name_returns_number() {
        assert_parity(r#"kill -l KILL"#);
    }

    /// `kill -l INT` → 2.
    #[test]
    fn dash_l_int_signal() {
        assert_parity(r#"kill -l INT"#);
    }

    /// `kill -l TERM` → 15.
    #[test]
    fn dash_l_term_signal() {
        assert_parity(r#"kill -l TERM"#);
    }
}

mod signal_names_no_sig_prefix {
    use super::*;

    /// `kill -INT` accepted (no SIG prefix).
    #[test]
    fn kill_dash_int_to_subshell() {
        assert_parity(
            r#"
(sleep 5) &
PID=$!
sleep 0.1
kill -INT $PID 2>/dev/null
wait $PID 2>/dev/null
echo done
"#,
        );
    }

    /// `kill -SIGINT` (with prefix).
    #[test]
    fn kill_dash_sigint_to_subshell() {
        assert_parity(
            r#"
(sleep 5) &
PID=$!
sleep 0.1
kill -SIGINT $PID 2>/dev/null
wait $PID 2>/dev/null
echo done
"#,
        );
    }

    /// `kill -9` numeric.
    #[test]
    fn kill_dash_9_to_subshell() {
        assert_parity(
            r#"
(sleep 5) &
PID=$!
sleep 0.1
kill -9 $PID 2>/dev/null
wait $PID 2>/dev/null
echo done
"#,
        );
    }
}

mod signals_array {
    use super::*;

    /// $signals[1] should be EXIT in zsh.
    #[test]
    fn signals_array_first_is_exit() {
        assert_parity(r#"echo "${signals[1]}""#);
    }

    /// $signals should contain HUP, INT, TERM.
    #[test]
    fn signals_array_contains_common() {
        assert_parity(
            r#"
for s in HUP INT TERM KILL; do
  if [[ -n "${signals[(r)$s]}" ]]; then echo "$s:yes"; else echo "$s:no"; fi
done
"#,
        );
    }

    /// ${#signals} > 0.
    #[test]
    fn signals_array_nonempty() {
        assert_parity(r#"(( ${#signals} > 0 )); echo $?"#);
    }
}

mod kill_to_self {
    use super::*;

    /// `kill -0 $$` checks if process exists (no signal sent).
    #[test]
    fn kill_dash_0_self_check() {
        assert_parity(r#"kill -0 $$; echo $?"#);
    }

    /// `kill -0` on nonexistent PID errors.
    #[test]
    fn kill_dash_0_nonexistent_pid() {
        assert_parity(r#"kill -0 999999 2>/dev/null; echo $?"#);
    }
}

mod kill_to_zero_pid {
    use super::*;

    /// `kill 0` sends to all in current process group.
    /// We can't easily test sending without affecting the test runner.
    #[test]
    #[ignore = "kill 0 affects all in PGID; can't safely test in test runner"]
    fn kill_zero_pgid_placeholder() {
        assert_parity(r#"echo placeholder"#);
    }
}

mod errors {
    use super::*;

    /// Unknown signal name → error.
    #[test]
    fn kill_unknown_signal_errors() {
        assert_parity(r#"kill -NOSUCHSIG 1 2>/dev/null; echo exit=$?"#);
    }

    /// No args.
    #[test]
    fn kill_no_args_errors() {
        assert_parity(r#"kill 2>/dev/null; echo exit=$?"#);
    }

    /// Invalid PID format.
    #[test]
    fn kill_invalid_pid_errors() {
        assert_parity(r#"kill abc 2>/dev/null; echo exit=$?"#);
    }
}

mod trap_signal {
    use super::*;

    /// trap handler catches signal sent by kill.
    #[test]
    fn trap_catches_kill_signal() {
        assert_parity(
            r#"
trap 'echo "caught INT"' INT
kill -INT $$
echo "after-trap"
"#,
        );
    }
}

mod multiple_signals_in_one_call {
    use super::*;

    /// kill -9 PID1 PID2 — multiple PIDs.
    #[test]
    fn kill_multiple_pids() {
        assert_parity(
            r#"
(sleep 5) &
P1=$!
(sleep 5) &
P2=$!
sleep 0.1
kill -9 $P1 $P2 2>/dev/null
wait 2>/dev/null
echo done
"#,
        );
    }
}

mod kill_dash_s_signal {
    use super::*;

    /// `kill -s SIGNAME PID` POSIX-style.
    #[test]
    fn kill_dash_s_with_name() {
        assert_parity(
            r#"
(sleep 5) &
PID=$!
sleep 0.1
kill -s TERM $PID 2>/dev/null
wait $PID 2>/dev/null
echo done
"#,
        );
    }
}
