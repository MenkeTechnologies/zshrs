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

/// c:Src/exec.c:5807-5816 — loading an autoloaded function installs its
/// definition and runs nothing else; none of the shell-exit hooks
/// (`trap … EXIT`, `TRAPEXIT`, `zshexit`) fire on that path. zshrs ran
/// the definition through the end-of-script pipeline, so the first call of
/// ANY autoloaded function fired them (and consumed the string EXIT trap).
mod autoload_does_not_fire_exit_hooks {
    use super::*;

    #[test]
    fn string_exit_trap_survives_first_autoload_call() {
        assert_parity(
            r#"trap "echo T" EXIT; autoload -Uz is-at-least; is-at-least 1.0 && echo ok; echo y"#,
        );
    }

    #[test]
    fn trapexit_and_zshexit_not_fired_by_autoload() {
        assert_parity(
            r#"TRAPEXIT() { echo T }; autoload -Uz is-at-least; is-at-least 1.0 && echo ok; echo y"#,
        );
        assert_parity(
            r#"zshexit() { echo Z }; autoload -Uz is-at-least; is-at-least 1.0 && echo ok; echo y"#,
        );
    }

    /// C03traps "autoloaded TRAPEXIT": an autoloaded TRAPEXIT runs once
    /// at `exit`, not once for the load and once more for the exit.
    #[test]
    fn autoloaded_trapexit_runs_once() {
        let dir = std::env::temp_dir().join(format!("zshrs-trapexit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("TRAPEXIT"), "print Running exit trap\n").unwrap();
        let d = dir.display();
        assert_parity(&format!(
            "fpath=({d} $fpath); autoload TRAPEXIT; print A; exit; print What"
        ));
        assert_parity(&format!(
            "fpath=({d} $fpath); autoload TRAPEXIT; fn() {{ print F }}; fn; print B; exit"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// c:Src/jobs.c:3091-3099 — getsigidx matches signal names with `strcmp`,
/// so a lower-case word is NOT a signal. `trap int INT` installs the
/// command `int` (C03traps "Outputting traps correctly"); zshrs upper-cased
/// the name, took `int` for SIGINT and ran the unset form instead.
mod signal_names_are_case_sensitive {
    use super::*;

    #[test]
    fn lowercase_command_word_is_the_trap_body() {
        assert_parity("trap -; trap int INT; trap sigterm SIGTERM; trap quit 3; trap");
    }

    #[test]
    fn lowercase_signal_name_is_undefined() {
        assert_parity("trap x int 2>&1; echo $?");
        assert_parity("trap x sigint 2>&1; echo $?");
        assert_parity("trap - int 2>&1; echo $?");
    }
}

/// c:Src/exec.c:1654-1657 — `eflag = errflag; errflag = 0; dotrap(SIGZERR);
/// errflag = eflag;`: an error raised inside the ZERR trap body ends the
/// trap, not the list that triggered it (C03traps "DDD").
mod zerr_trap_error_does_not_abort_the_list {
    use super::*;

    #[test]
    fn nounset_error_in_string_trap() {
        assert_parity(r#"set -u; trap 'echo t1; : $UNSET; echo t2' ZERR; false; echo after"#);
        assert_parity(
            r#"set -u; f() { trap 'echo t1; : $UNSET; echo t2' ZERR; false; echo after; }; f; echo out"#,
        );
    }

    #[test]
    fn nounset_error_in_function_called_by_trap() {
        assert_parity(
            r#"set -u; KO() { { : $KO } 2>&1 }; trap 'echo t1; KO; echo t2' ZERR; (false; echo in); echo "$?""#,
        );
    }

    /// c:Src/subst.c:3355-3366 — `${v?}` in a non-interactive shell still
    /// exits (C leaves through zexit before the restore).
    #[test]
    fn unset_question_in_trap_still_exits() {
        assert_parity(r#"trap 'echo t1; : ${UNSET?x}; echo t2' ZERR; false; echo after"#);
    }
}

/// A forked compound command runs its list with `exiting` set (c:Src/exec.c:3063
/// `last1 = forked = 1` → c:4098-4099 `do_exec = 1`), so an EXIT trap it sets
/// fires at the end of that list (c:1700-1706). Only `{ … }`, the taken `if`
/// branch, the last `for NAME in` iteration and a `case` arm pass `exiting`
/// on (c:494, Src/loop.c:175/588/684); a simple command, `while`, `repeat`,
/// the C-style `for` and `always` leave without firing it.
mod forked_compound_exit_trap {
    use super::*;

    #[test]
    fn brace_group_pipeline_stage() {
        assert_parity(r#"{ trap 'echo X' EXIT; echo A } | cat; echo after"#);
        assert_parity(r#"{ { trap 'echo X' EXIT; }; echo in; } | cat"#);
        assert_parity(r#"{ trap 'echo X $?' EXIT; (exit 4) } | cat; print $pipestatus"#);
        assert_parity(r#"{ TRAPEXIT() { echo T }; } | cat"#);
    }

    #[test]
    fn if_for_case_stages() {
        assert_parity(r#"if trap 'echo X' EXIT; then true; fi | cat"#);
        assert_parity(r#"if false; then :; else trap 'echo E' EXIT; fi | cat"#);
        assert_parity(r#"for i in 1 2; do trap "echo X$i" EXIT; done | cat"#);
        assert_parity(r#"case a in a) trap 'echo X' EXIT;; esac | cat"#);
    }

    #[test]
    fn shapes_that_do_not_fire() {
        assert_parity(r#"trap 'echo X' EXIT | cat"#);
        assert_parity(r#"while true; do trap 'echo X' EXIT; break; done | cat"#);
        assert_parity(r#"for ((i=0;i<1;i++)); do trap 'echo X' EXIT; done | cat"#);
        assert_parity(r#"{ trap 'echo X' EXIT } always { true } | cat"#);
    }

    #[test]
    fn async_brace_group() {
        assert_parity(r#"{ trap 'echo X' EXIT; } & wait"#);
        assert_parity(r#"{ trap 'echo X' EXIT; echo A } | cat & wait"#);
    }
}

// DEBUG and ZERR are raised by the commands themselves, so inside `$(…)`
// they fire in C's forked child, whose stdout is the capture pipe
// (c:Src/exec.c getoutput → child execode). Only a REAL signal trap belongs
// to the parent and prints to its stdout.
mod pseudo_signal_traps_in_cmdsubst {
    use super::*;

    #[test]
    fn debug_trap_output_is_captured() {
        assert_parity(r#"trap 'echo T' DEBUG; x=$(echo hi); echo "<$x>""#);
        assert_parity(r#"x=$(trap 'echo T' DEBUG; echo hi); echo "<$x>""#);
    }

    #[test]
    fn zerr_trap_output_is_captured() {
        assert_parity(r#"trap 'echo Z' ZERR; x=$(false; echo hi); echo "<$x>""#);
    }

    #[test]
    fn signal_trap_still_prints_in_the_parent() {
        assert_parity(r#"trap 'echo T' USR1; x=$(kill -USR1 $$; echo hi); echo "<$x>""#);
    }
}

// c:Src/exec.c:1484-1485 — `$ZSH_DEBUG_CMD` is `getpermtext(...)`, the
// permanent text with newlines and tab indents (c:Src/text.c:279,
// tnewlins=1), not the one-line job text `jobs` shows.
mod zsh_debug_cmd_permanent_text {
    use super::*;

    #[test]
    fn compound_commands_are_multi_line() {
        assert_parity(
            r#"trap 'print -r -- "[$ZSH_DEBUG_CMD]"' DEBUG; while false; do :; done; case a in a) echo A;; esac"#,
        );
        assert_parity(r#"trap 'print -r -- "[$ZSH_DEBUG_CMD]"' DEBUG; { echo a } always { echo b }"#);
        assert_parity(
            r#"trap 'print -r -- "[$ZSH_DEBUG_CMD]"' DEBUG; if false; then :; elif true; then echo e; else echo n; fi"#,
        );
    }

    /// The text is rendered from what was parsed: no second alias
    /// expansion, and an RCQUOTES `''` pair stays as written.
    #[test]
    fn simple_commands_keep_their_spelling() {
        assert_parity(r#"alias ll='echo LL'; trap 'print -r -- "[$ZSH_DEBUG_CMD]"' DEBUG; ll x; x=(1 2) y=3"#);
        assert_parity(r#"setopt rcquotes; trap 'print -r -- "[$ZSH_DEBUG_CMD]"' DEBUG; x='it''s'; echo "$x""#);
    }
}
