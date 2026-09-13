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

/// c:Src/exec.c:2916-2918 + c:4417 — a background SIMPLE command's child
/// leaves execcmd_exec through `_realexit()` and never reaches execlist's
/// `sublist_done`, so the ZERR trap (which survives the fork: SIGZERR is
/// SIGCOUNT+1, past entersubsh's c:1127-1131 loop) does not fire for it. A
/// compound or function job runs a list in the child, where it does fire.
mod zerr_in_background_child {
    use super::*;

    #[test]
    fn simple_command_job_fires_no_zerr() {
        assert_parity(r#"trap "print Z" ZERR; false & wait; print end"#);
        assert_parity(r#"trap "print Z" ZERR; /usr/bin/false & wait; print end"#);
        assert_parity(r#"trap "print Z" ZERR; false | false & wait; print end"#);
    }

    #[test]
    fn compound_and_function_jobs_still_fire() {
        assert_parity(r#"trap "print Z" ZERR; { false } & wait; print end"#);
        assert_parity(r#"trap "print Z" ZERR; (false) & wait; print end"#);
        assert_parity(r#"trap "print Z" ZERR; f(){ false; print in }; f & wait; print end"#);
    }
}

/// c:Src/jobs.c:651-652 — update_job ends with `if (sigtrapped[SIGCHLD] &&
/// job != thisjob) dotrap(SIGCHLD);`, so the CHLD trap runs whenever a
/// background child changes state: during a foreground command, during
/// `wait PID`, and once per child (Test/A05execution.ztst "Background job exit
/// does not affect reaping foreground job"). A foreground command's own exit
/// is `thisjob` and runs no trap.
mod chld_trap_for_background_children {
    use super::*;

    #[test]
    fn fires_while_a_foreground_command_runs() {
        assert_parity(r#"callfromchld() { true && { print CHLD } }; TRAPCHLD() { callfromchld }; sleep 0.2 & sleep 0.7; print OK"#);
        assert_parity(r#"trap 'print C' CHLD; /usr/bin/true & sleep 0.4; print OK"#);
        assert_parity(r#"TRAPCHLD() { print C }; sleep 0.1 & sleep 0.2 & sleep 0.6; print OK"#);
    }

    #[test]
    fn fires_once_per_child_under_wait() {
        assert_parity(r#"trap 'print C' CHLD; sleep 0.1 & wait $!; print OK"#);
        assert_parity(r#"trap 'print C' CHLD; sleep 0.1 & wait; print OK"#);
    }

    #[test]
    fn a_foreground_command_runs_no_trap() {
        assert_parity(r#"trap 'print C' CHLD; /usr/bin/true; print fg"#);
    }
}

/// c:Src/exec.c:1228-1245 — `eval` runs its string through execstring/execode,
/// which runs no EXIT trap. The script's EXIT trap belongs to zexit
/// (c:Src/builtin.c:6037-6043) and a function's to endtrapscope
/// (c:Src/signals.c:880). Firing the end-of-script hooks after every `eval`
/// ran the trap in the middle of the script.
mod exit_trap_not_fired_by_eval {
    use super::*;

    #[test]
    fn script_exit_trap_waits_for_the_end() {
        assert_parity(r#"trap "print T" EXIT; eval "print e"; print after"#);
        assert_parity(r#"emulate sh -c 'trap "print T" EXIT'; eval :; print after"#);
    }

    #[test]
    fn function_exit_trap_waits_for_the_function_end() {
        assert_parity(r#"f(){ trap "print T" EXIT; eval "print e"; print in; }; f; print after"#);
    }
}

/// c:Src/exec.c:1651-1659 — ZERR runs for a failing sublist unless
/// `donetrap` is already set. A subshell is a fork: the ZERR its body runs
/// sets `donetrap` in the child only, so the parent runs ZERR again for the
/// subshell's own non-zero status. An async `( … )` child leaves through
/// `_realexit()` (c:4417) with no outer check, so `(false) &` stays at one.
mod zerr_after_a_failing_subshell {
    use super::*;

    #[test]
    fn parent_runs_zerr_for_the_subshell_status() {
        assert_parity("TRAPZERR() { print ZERR; }; (false); echo rc=$?");
        assert_parity("TRAPZERR() { print ZERR; }; f() { (false) }; f; echo rc=$?");
        assert_parity(
            "TRAPZERR() { print ZERR trapped; }; testfn() { setopt localoptions $2; print $1 before; false; print $1 after; }; (testfn on errexit); testfn off",
        );
    }

    #[test]
    fn a_succeeding_subshell_runs_no_zerr() {
        assert_parity("TRAPZERR() { print ZERR; }; (true); echo rc=$?; false; echo rc2=$?");
    }

    #[test]
    fn async_subshells_keep_one_zerr() {
        assert_parity(r#"trap "print Z" ZERR; (false) & wait; print end"#);
        assert_parity(r#"trap "print Z" ZERR; ( (false) ) & wait; print end"#);
    }
}
