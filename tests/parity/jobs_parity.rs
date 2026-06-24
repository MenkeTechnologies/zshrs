//! Job-control parity: the canonical jobtab populated by `cmd &`
//! (BUILTIN_RUN_BG → initjob/addproc/spawnjob, c:Src/exec.c:1700-1758)
//! and every consumer of it — `jobs` listing format (printjob,
//! c:Src/jobs.c:1138), jobspec resolution (getjob, c:Src/jobs.c:2063),
//! `wait %N` (bin_fg BIN_WAIT, c:Src/jobs.c:2655), `kill %N`
//! (bin_kill, c:Src/jobs.c:2989), `disown` (c:Src/jobs.c:2729), the
//! subshell job-table isolation (clearjobtab, c:Src/jobs.c:1780), and
//! the zsh/parameter introspection assocs `$jobtexts` / `$jobstates` /
//! `$jobdirs` (Src/Modules/parameter.c:1255-1453).
//!
//! Bugs #79, #257, #259, #369, #462 in docs/BUGS.md.
//!
//! All scripts are non-interactive `-fc` (MONITOR off) and kill their
//! own background jobs — no orphan processes survive a test run. PIDs
//! are normalized out of compared output (provably-impossible
//! variance); everything else is byte-compared.

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

/// Replace any 3+-digit run with `PID` — process ids differ between
/// the two shells by construction; nothing else in these scripts
/// produces a 3-digit number.
fn normalize_pids(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 3 {
                out.push_str("PID");
            } else {
                out.push_str(&s[start..i]);
            }
        } else {
            // Advance one full UTF-8 char.
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

struct R {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: normalize_pids(&String::from_utf8_lossy(&o.stdout)),
        stderr: normalize_pids(&String::from_utf8_lossy(&o.stderr)),
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
        stdout: normalize_pids(&String::from_utf8_lossy(&o.stdout)),
        stderr: normalize_pids(&String::from_utf8_lossy(&o.stderr)),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Byte-parity on pid-normalized stdout + stderr + exit code.
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout mismatch for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        s, z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr mismatch for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        s, z.stderr, r.stderr
    );
    assert_eq!(
        z.exit, r.exit,
        "exit mismatch for {:?} (zsh {} vs zshrs {})",
        s, z.exit, r.exit
    );
}

// ── #79: job table populated by `cmd &` ────────────────────────────

#[test]
fn jobs_lists_running_bg_job() {
    // c:Src/jobs.c:1295 — `[1]  + running    sleep 5` exact spacing.
    assert_parity("sleep 5 & jobs; kill %1");
}

#[test]
fn jobs_plus_minus_markers_two_jobs() {
    // c:Src/jobs.c:1274-1275 — `+` = curjob (latest), `-` = prevjob.
    assert_parity("sleep 5 & sleep 4 & jobs; kill %1 %2");
}

#[test]
fn jobs_l_includes_pid() {
    // c:Src/jobs.c:1281 — `jobs -l` prints `pid ` (single space).
    assert_parity("sleep 5 & jobs -l; kill %1");
}

#[test]
fn jobs_p_includes_group_leader() {
    // c:Src/jobs.c:1283-1290 — `jobs -p` prints the gleader pid.
    assert_parity("sleep 5 & jobs -p; kill %1");
}

#[test]
fn jobs_r_and_s_filters() {
    // c:Src/jobs.c:2515-2519 — -r lists running only, -s stopped only.
    assert_parity("sleep 5 & jobs -r; jobs -s; kill %1");
}

#[test]
fn done_job_leaves_table() {
    // c:Src/jobs.c:1350-1356 — printjob deletes finished entries;
    // `jobs` after the bg job exits prints nothing.
    assert_parity("sleep 0.1 & sleep 0.5; jobs; echo rc=$?");
}

#[test]
fn kill_by_jobspec() {
    // c:Src/jobs.c:2989-2993 — `kill %1` resolves via getjob + killjb.
    assert_parity("sleep 5 & kill %1; echo rc=$?");
}

#[test]
fn kill_by_job_name_prefix() {
    // c:Src/jobs.c:2135-2140 — `%sle` matches command-text prefix.
    assert_parity("sleep 5 & kill %sle; echo rc=$?");
}

#[test]
fn kill_by_job_name_miss() {
    // getjob miss → "job not found: lee" + kill exit 1.
    assert_parity("sleep 5 & kill %lee; echo rc=$?; kill %1");
}

#[test]
fn bare_numeric_jobspec_is_a_name() {
    // c:Src/jobs.c:2070-2072 — args without `%` are job NAMES
    // (findjobnam), not indices: `jobs 1` → "job not found: 1" rc 127.
    assert_parity("sleep 5 & jobs 1; echo rc=$?; kill %1");
}

#[test]
fn jobspec_percent_percent_and_plus() {
    // c:Src/jobs.c:2074-2083 — `%%` and `%+` are the current job.
    assert_parity("sleep 5 & jobs %%; jobs %+; kill %1");
}

// ── #369: wait %N ───────────────────────────────────────────────────

#[test]
fn wait_jobspec_returns_zero() {
    assert_parity("sleep 0.1 & wait %1; echo rc=$?");
}

#[test]
fn wait_jobspec_propagates_exit_status() {
    // c:Src/jobs.c:2655-2657 — retval = zwaitjob → lastval2.
    assert_parity("{ sleep 0.2; exit 3 } & wait %1; echo rc=$?");
}

#[test]
fn wait_nonexistent_jobspec_errors_127() {
    // c:Src/jobs.c:2111-2113 — "%2: no such job", wait exits 127.
    assert_parity("wait %2; echo rc=$?");
}

#[test]
fn second_wait_same_spec_errors() {
    // The finished entry left the table (printjob delete tail), so a
    // second `wait %1` reports "no such job" rc 127.
    assert_parity("sleep 0.1 & wait %1; echo rc=$?; wait %1; echo rc2=$?");
}

#[test]
fn wait_by_job_name() {
    // c:Src/jobs.c:2135-2140 — `wait %sleep` resolves by name.
    assert_parity("sleep 0.2 & wait %sleep; echo rc=$?");
}

#[test]
fn wait_by_search_string() {
    // c:Src/jobs.c:2116-2127 — `%?lee` searches inside the text.
    assert_parity("sleep 0.2 & wait %?lee; echo rc=$?");
}

#[test]
fn wait_by_pid_still_works() {
    assert_parity("sleep 0.1 & wait $!; echo rc=$?");
}

#[test]
fn wait_no_args_drains_all_and_clears_table() {
    assert_parity("sleep 0.1 & sleep 0.15 & wait; echo rc=$?; jobs");
}

// ── #79: disown ─────────────────────────────────────────────────────

#[test]
fn disown_no_args_removes_current_job() {
    // c:Src/jobs.c:2498 + 2729 — no-arg disown deletes curjob.
    assert_parity("sleep 5 & disown; jobs; echo rc=$?; kill $!");
}

#[test]
fn disown_jobspec_removes_job() {
    assert_parity("sleep 5 & disown %1; echo rc=$?; jobs; kill $!");
}

#[test]
fn disown_numeric_without_percent_is_a_name() {
    assert_parity("sleep 5 & disown 1; echo rc=$?; kill %1");
}

// ── #462: subshell job-table isolation ─────────────────────────────

#[test]
fn subshell_disown_silently_eats_control_job() {
    // c:Src/jobs.c:1780-1828 — the subshell's cleared table holds only
    // the procless control job; inherited curjob=1 points at it, so
    // `(disown)` deletes IT silently. Parent's table is untouched.
    assert_parity("sleep 0.3 & (disown); jobs; kill %1");
}

#[test]
fn subshell_bg_then_disown_no_current_job() {
    // c:Src/jobs.c:1900 — spawnjob in a subshell skips the curjob
    // promotion, so `(cmd & disown)` errors "no current job" rc 1.
    assert_parity("(sleep 0.2 & disown); echo rc=$?");
}

#[test]
fn subshell_jobs_sees_empty_table() {
    // c:Src/exec.c:2862 ESUB_PGRP → clearjobtab: `(jobs)` prints
    // nothing even while the parent has a running job.
    assert_parity("sleep 5 & (jobs); kill %1");
}

#[test]
fn subshell_kill_jobspec_hits_control_job() {
    // `(kill %1)` resolves to the procless control job — vacuous
    // success rc 0; the parent's job 1 survives.
    assert_parity("sleep 5 & (kill %1); echo rc=$?; kill %1");
}

#[test]
fn parent_table_intact_after_subshell() {
    assert_parity("sleep 5 & (jobs); jobs; kill %1");
}

// ── #257/#259: zsh/parameter job introspection assocs ───────────────

#[test]
fn jobtexts_running_bg_job() {
    // Src/Modules/parameter.c:1255-1273 pmjobtext.
    assert_parity("sleep 5 & print -r -- \"[$jobtexts[1]]\"; kill %1");
}

#[test]
fn jobtexts_keys_iteration() {
    // Src/Modules/parameter.c:1308-1335 scanpmjobtexts.
    assert_parity(
        "sleep 5 & sleep 4 & for n in ${(ko)jobtexts}; do print -r -- \"[$n: $jobtexts[$n]]\"; done; kill %1 %2",
    );
}

#[test]
fn jobstates_and_jobdirs_running_bg_job() {
    // Src/Modules/parameter.c:1340-1379 pmjobstate ("running:+:PID=running")
    // + 1447-1453 pmjobdir (job pwd, falling back to shell pwd).
    assert_parity("cd /tmp; sleep 5 & print -r -- \"[$jobstates[1]][$jobdirs[1]]\"; kill %1");
}

#[test]
fn jobstates_markers_two_jobs() {
    // `:+` on curjob, `:-` on prevjob (parameter.c:1346-1351).
    assert_parity("sleep 5 & sleep 4 & print -r -- \"$jobstates[1]|$jobstates[2]\"; kill %1 %2");
}

#[test]
fn jobtexts_empty_in_subshell() {
    // Subshell table is cleared; the control job has no procs, so
    // pmjobtext's `jtab[job].procs` gate (parameter.c:1295) yields "".
    assert_parity("sleep 5 & (print -r -- \"[$jobtexts[1]]\"); kill %1");
}

// ── misc ────────────────────────────────────────────────────────────

#[test]
fn pipeline_bg_job_text() {
    // getjobtext renders the full pipeline (Src/text.c:235).
    assert_parity("sleep 5 | cat & print -r -- \"[$jobtexts[1]]\"; kill %1");
}

#[test]
fn fg_bg_error_without_job_control() {
    // c:Src/jobs.c:2461-2464 — "no job control in this shell."
    assert_parity("sleep 5 & fg; echo rc=$?; kill %1");
}

#[test]
fn dollar_bang_set_by_bg() {
    assert_parity("sleep 5 & [[ -n $! ]] && echo set; kill $!");
}

// ── wait $pid bgstatus retention (c:Src/jobs.c:684-699 update_bg_job →
//    addbgstatus; bin_wait getbgstatus fallback c:2566-2570) ──────────

/// `wait $pid` after the bg child has finished must return its stored
/// exit status (the reap records it in the bgstatus ring), not error
/// "pid N is not a child of this shell".
#[test]
fn wait_pid_after_finish_returns_status() {
    assert_parity("(exit 5) & p=$!; sleep 0.1; wait $p; echo $?");
}

/// Waiting on a second bg job must not forget the first — each finished
/// job's status stays retrievable.
#[test]
fn wait_two_pids_each_status_retained() {
    assert_parity(
        r#"(exit 1) & p=$!; (exit 2) & q=$!; wait $q; echo "q=$?"; wait $p; echo "p=$?""#,
    );
}
