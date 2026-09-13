//! Child reaping and `$pipestatus` parity across every dispatch path.
//!
//! C installs the SIGCHLD reaper from `init_signals`
//! (c:Src/init.c:1455 — `install_handler(SIGCHLD);`), which `main`
//! reaches on EVERY invocation, and `wait_for_processes`
//! (c:Src/signals.c:285 — `waitpid(-1, &status, WAITFLAGS)`) collects
//! every child, job-table entry or not. zshrs dispatches `-c` and a
//! script FILE without going through `ported::init::zsh_main`, so
//! neither armed the reaper and every background child of those two
//! paths stayed a zombie for the life of the shell.
//!
//! Arming it puts the reaper in a race with the pipeline's own targeted
//! `waitpid` for each forked stage, which C never has: C's foreground
//! waits do not call `waitpid` at all (`waitforpid` polls `kill(pid, 0)`
//! and sleeps in `signal_suspend`, c:Src/jobs.c:1652-1666), so the
//! reaper is C's only collector and `storepipestats` reads every stage's
//! status straight out of `pn->status` (c:Src/jobs.c:423-435). The
//! reaped-status ring (`extensions/reaped_status.rs`) is what gives
//! zshrs's second collector the status back, so the `$pipestatus` cases
//! here are the pins on that.
//!
//! All scripts are non-interactive or `-i -c` with a non-tty stdin, and
//! none leaves a process behind.

use std::io::Write;
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

/// Which of the four ways a shell can be handed its input. The bug was
/// dispatch-path-specific in both directions — the zombie leak hit `-c`
/// and a script file, the `$pipestatus` loss hit only paths that had the
/// reaper — so every case runs on all four.
#[derive(Clone, Copy, Debug)]
enum Mode {
    DashC,
    DashIC,
    ScriptFile,
    Stdin,
}

struct R {
    out: String,
    exit: i32,
}

fn write_script(body: &str) -> PathBuf {
    // Tests in this file run in parallel and each writes its own
    // script, so the name has to be unique per CALL, not per test — a
    // wall-clock stamp is not (two threads inside the same microsecond
    // shared a file and one test read the other's script).
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "zshrs_reap_parity_{}_{}.zsh",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let mut f = std::fs::File::create(&p).expect("script temp file");
    f.write_all(body.as_bytes()).expect("write script");
    p
}

fn run(bin: &Path, extra: &[&str], mode: Mode, body: &str) -> R {
    let mut cmd = Command::new(bin);
    cmd.args(extra);
    let mut script: Option<PathBuf> = None;
    match mode {
        Mode::DashC => {
            cmd.args(["-f", "-c", body]);
        }
        Mode::DashIC => {
            cmd.args(["-f", "-i", "-c", body]);
        }
        Mode::ScriptFile => {
            let p = write_script(body);
            cmd.args(["-f"]).arg(&p);
            script = Some(p);
        }
        Mode::Stdin => {
            let p = write_script(body);
            cmd.arg("-f")
                .stdin(std::fs::File::open(&p).expect("reopen script"));
            script = Some(p);
        }
    }
    let o = cmd.output().expect("shell run");
    if let Some(p) = script {
        let _ = std::fs::remove_file(p);
    }
    let mut out = String::from_utf8_lossy(&o.stdout).into_owned();
    out.push_str(&String::from_utf8_lossy(&o.stderr));
    R {
        out,
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Byte-parity of stdout+stderr and exit code on all four dispatch
/// paths at once.
fn assert_parity_all_modes(body: &str) {
    assert_parity_modes(body, &[Mode::DashC, Mode::DashIC, Mode::ScriptFile, Mode::Stdin]);
}

fn assert_parity_modes(body: &str, modes: &[Mode]) {
    if !zsh_available() {
        return;
    }
    for &mode in modes {
        let z = run(Path::new(zsh_path()), &[], mode, body);
        let r = run(&zshrs_bin(), &["--zsh"], mode, body);
        assert_eq!(
            z.out, r.out,
            "output mismatch on {:?} for {:?}\n zsh:   {:?}\n zshrs: {:?}",
            mode, body, z.out, r.out
        );
        assert_eq!(
            z.exit, r.exit,
            "exit mismatch on {:?} for {:?} (zsh {} vs zshrs {})",
            mode, body, z.exit, r.exit
        );
    }
}

// ── the reaper is armed on every dispatch path ─────────────────────

/// c:Src/init.c:1455 + c:Src/signals.c:285 — a background child that
/// nothing waits for is still collected, so `ps` cannot find it at all.
/// Without the handler `ps -o stat=` printed `Z` and exited 0; zsh
/// prints nothing and `ps` exits 1 because the pid is gone.
#[test]
fn a_background_child_is_reaped_without_a_wait() {
    assert_parity_all_modes("sleep 0.05 & ; sleep 0.4; ps -o stat= -p $!; print rc=$?");
}

/// Same, for a child the job table never owns a status for because it
/// exits before anything asks: two of them, so a single-shot reap would
/// still leave one behind.
#[test]
fn several_background_children_are_all_reaped() {
    assert_parity_all_modes(
        "sleep 0.05 & ; p=$!; sleep 0.05 & ; q=$!; sleep 0.4; \
         ps -o stat= -p $p $q; print rc=$?",
    );
}

/// c:Src/exec.c:1748 — execpline holds SIGCHLD blocked (`child_block()`)
/// while it forks the stages. Without that span, the reaper armed here
/// runs inside `fork()` when an earlier stage exits, re-enters malloc on
/// libmalloc's fork lock, and the shell dies with SIGKILL ("Trying to
/// recursively lock an os_unfair_lock"). One pipeline hits the window
/// only sometimes; three hundred in one shell hit it on every run, and
/// nothing was printed.
#[test]
fn many_pipelines_in_one_shell_survive_the_reaper() {
    assert_parity_all_modes("repeat 300 { true | false | true }; print ok");
}

/// The async spawn path forks too: c:Src/exec.c:1748 through c:1815.
#[test]
fn many_background_jobs_in_one_shell_survive_the_reaper() {
    assert_parity_all_modes("repeat 200 { (exit 1) & }; wait; print ok");
}

// ── $pipestatus survives the reaper ────────────────────────────────

/// c:Src/jobs.c:423-435 (`storepipestats`) — every stage contributes its
/// own status. Each early stage here exits immediately while the last
/// stage runs for 0.2s IN THE PARENT, so the reaper collects all three
/// long before the parent's targeted `waitpid` runs and every one of
/// them came back as a clean exit 0: `0 0 0 3` instead of `5 6 7 3`.
#[test]
fn pipestatus_keeps_stages_the_reaper_collected_first() {
    assert_parity_all_modes(
        "(exit 5) | (exit 6) | (exit 7) | (sleep 0.2; exit 3); print -r -- \"$pipestatus\"",
    );
}

/// The two-stage shape of the same loss, plus `$?`.
#[test]
fn pipestatus_two_stage_early_exit() {
    assert_parity_all_modes("(exit 5) | (sleep 0.2; exit 3); print -r -- \"$pipestatus | $?\"");
}

/// c:Src/jobs.c:430-432 — a stage KILLED by a signal stores
/// `0200 | WTERMSIG`, not an exit status. `sh` signals its own parent,
/// which is the forked stage itself, so the stage is genuinely
/// `WIFSIGNALED`; the reaper collects it at once while the last stage
/// sleeps. Losing it reported 0 rather than 143.
#[test]
fn pipestatus_keeps_a_signalled_stage() {
    assert_parity_modes(
        SIGNALLED_STAGE,
        &[Mode::DashC, Mode::ScriptFile, Mode::Stdin],
    );
}

const SIGNALLED_STAGE: &str = "( sh -c 'kill -TERM $PPID'; sleep 2 ) | ( sleep 0.3; exit 3 ); \
                               print -r -- \"$pipestatus | $?\"";

/// SEPARATE, PRE-EXISTING GAP — not the reaper. Under `-i -c` the same
/// pipeline reports the signalled stage as 0 where zsh says 143, and it
/// did so identically on a build with NO SIGCHLD handler on that path at
/// all (measured against the pinned pre-fix binary: `-c` gave `143 3`,
/// `-i -c` gave `0 3`, both before and after the reaper was armed). So
/// an interactive `-c` loses a signalled stage's status somewhere other
/// than the reap, and un-ignoring this is that investigation, not this
/// one.
#[test]
#[ignore]
fn pipestatus_keeps_a_signalled_stage_interactive() {
    assert_parity_modes(SIGNALLED_STAGE, &[Mode::DashIC]);
}

/// c:Src/jobs.c:434-435 + c:451-454 — `pipefail` promotes the last
/// non-zero stage status, so a lost stage status silently turns a failed
/// pipeline into a successful one.
#[test]
fn pipefail_reads_a_stage_the_reaper_collected() {
    assert_parity_all_modes(
        "setopt pipefail; (exit 5) | (sleep 0.2; true); print -r -- \"$pipestatus | $?\"",
    );
}

/// A background job exiting mid-pipeline is a second reaper wake-up with
/// nothing to do with the pipeline — it must not disturb the stages.
#[test]
fn pipestatus_survives_an_unrelated_background_exit() {
    assert_parity_all_modes(
        "{ sleep 0.05; exit 9 } & ; (exit 5) | (sleep 0.3; exit 3); \
         print -r -- \"$pipestatus | $?\"",
    );
}

// ── the wait builtin still sees what the reaper took ───────────────

/// c:Src/jobs.c:2571 — `wait PID` is `waitforpid(pid, 1)`, a LOOP round
/// `signal_suspend`. The port's single `waitpid` is interrupted by the
/// shell's own SIGCHLD (`install_handler` sets `sa_flags = 0`, no
/// SA_RESTART — c:Src/signals.c:104) and reported the EINTR as status 1.
#[test]
fn wait_on_a_pid_is_not_ended_by_the_reapers_own_signal() {
    assert_parity_all_modes("true & ; wait $!; print rc=$?");
}

/// The same wait, for a child that exits with a status worth losing.
#[test]
fn wait_on_a_pid_reports_a_nonzero_status() {
    assert_parity_all_modes("(exit 7) & ; wait $!; print rc=$?");
}

/// c:Src/jobs.c:684-699 — the status of a child that finished while the
/// shell was busy has to survive until `wait` asks for it.
#[test]
fn wait_after_the_child_already_finished() {
    assert_parity_all_modes("{ sleep 0.05; exit 7 } & ; p=$!; sleep 0.4; wait $p; print rc=$?");
}

/// `wait` with no arguments drains every background job; the reaper
/// racing it must not change what it reports.
#[test]
fn bare_wait_drains_every_background_job() {
    assert_parity_all_modes(
        "sleep 0.05 & ; sleep 0.1 & ; wait; print rc=$?; jobs; print jobsrc=$?",
    );
}

// ── job control is unchanged by the reaper ─────────────────────────

/// `$!`, `jobs` and `kill %1` all read the same job table the reaper now
/// writes into on these paths.
#[test]
fn jobspec_kill_and_jobs_listing() {
    assert_parity_all_modes("sleep 5 & ; jobs; kill %1; sleep 0.2; wait; print rc=$?; jobs");
}
