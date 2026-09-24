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

/// Every script here backgrounds a `sleep` and is expected to finish in
/// well under a second. Run it under a hard deadline so a job that never
/// gets reaped fails the test instead of wedging the suite.
const PROBE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

fn run_deadlined(mut cmd: Command, who: &str) -> R {
    use std::io::Read;
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {who}: {e}"));
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    // Drain on threads: the probes print little, but a child blocked on a
    // full pipe would otherwise outlive the deadline for the wrong reason.
    let ot = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let et = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let deadline = std::time::Instant::now() + PROBE_DEADLINE;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(st) => break st,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{who} exceeded the {PROBE_DEADLINE:?} probe deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };
    let ob = ot.join().unwrap_or_default();
    let eb = et.join().unwrap_or_default();
    R {
        stdout: normalize_pids(&String::from_utf8_lossy(&ob)),
        stderr: normalize_pids(&String::from_utf8_lossy(&eb)),
        exit: status.code().unwrap_or(-1),
    }
}

fn run_zsh(s: &str) -> R {
    let mut c = Command::new(zsh_path());
    c.args(["-fc", s]);
    run_deadlined(c, "zsh")
}

fn run_zshrs(s: &str) -> R {
    let mut c = Command::new(zshrs_bin());
    c.args(["--zsh", "-f", "-c", s]).env_remove("ZSHRS_CACHE");
    run_deadlined(c, "zshrs")
}

fn run_zsh_cols(cols: u32, s: &str) -> R {
    let mut c = Command::new(zsh_path());
    c.args(["-fc", s]).env("COLUMNS", cols.to_string());
    run_deadlined(c, "zsh")
}

fn run_zshrs_cols(cols: u32, s: &str) -> R {
    let mut c = Command::new(zshrs_bin());
    c.args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .env("COLUMNS", cols.to_string());
    run_deadlined(c, "zshrs")
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

/// Same byte-parity check, with the terminal width PINNED on both shells.
///
/// `printjob` breaks a job's process list across lines using
/// `lineleng = zterm_columns` (c:Src/jobs.c:1151), and a non-interactive zsh
/// adopts `$COLUMNS` from the environment (c:Src/init.c:1294-1301), so pinning
/// it is what makes the wrap point a property of the script rather than of
/// whatever terminal the suite happens to run under.
fn assert_parity_cols(cols: u32, s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_cols(cols, s);
    let r = run_zshrs_cols(cols, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout mismatch at COLUMNS={} for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        cols, s, z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr mismatch at COLUMNS={} for {:?}\n zsh:   {:?}\n zshrs: {:?}",
        cols, s, z.stderr, r.stderr
    );
    assert_eq!(z.exit, r.exit, "exit mismatch at COLUMNS={} for {:?}", cols, s);
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

/// c:Src/jobs.c:1344-1353 — `jobs -d` prints `(pwd : DIR)` under each job;
/// DIR is the job's `pwd`, stamped by `cd` via setjobpwd
/// (c:Src/builtin.c:1241), so it stays the start directory after a `cd`.
#[test]
fn jobs_d_prints_the_start_directory_across_a_cd() {
    assert_parity(
        "cd /tmp; sleep 5 & jobs -d; cd /; jobs -d; print -r -- \"[$jobdirs[1]]\"; jobs -ld; kill %1",
    );
}

/// c:Src/exec.c:2916 → c:1127-1140 — an async child resets string traps
/// and, without job control, ignores SIGINT/SIGQUIT (`settrap(SIG, NULL)`),
/// which `trap` then lists. zshrs's background child kept the parent's
/// traps and set neither ignore.
#[test]
fn async_child_resets_traps_and_ignores_int_quit() {
    assert_parity(
        "trap 'print t' USR1; { trap } & wait; { trap } | cat & wait; trap 'print e' ERR; { trap } & wait",
    );
}

/// Src/signames2.awk builds sig_msg[] with USE_SUSPENDED, which config.h
/// always defines: a stopped job reads "suspended (signal)", not "stopped
/// (signal)". zshrs kept a second, divergent table in jobs.rs.
#[test]
fn stopped_job_reads_suspended() {
    assert_parity("sleep 5 & kill -STOP $!; sleep 0.5; jobs; kill -9 %1");
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

// ── #1150: compound-command job text (getjobtext → gettext2) ────────
//
// C never stores a job's source text; `getjobtext()` (Src/text.c:315)
// DEPARSES the stored `Eprog` through `gettext2()` (Src/text.c:415),
// which has a rendering arm per node type. zshrs's fusevm compiler used
// to substitute the placeholders `"for ..."`, `"while ..."`, `"if ..."`,
// `"case ..."`, `"repeat ..."` and `"until ..."`, so every compound
// background job listed as an indistinguishable stub.
//
// Every expectation below is whatever `/opt/homebrew/bin/zsh` itself
// prints, including the spacing that looks wrong but is not: `do;` after
// the loop opener, `then;` after the condition, `{;` opening an `always`
// block, and the space before `;;` in a `case` arm.

#[test]
fn jobtext_for_loop() {
    // c:Src/text.c:635-663 WC_FOR — `for X in W; do; BODY; done`.
    assert_parity(r#"for jt_x in 1 2; do sleep 5; done & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_for_positional() {
    // c:647-651 — WC_FOR_PPARAM has no ` in ` clause.
    assert_parity(
        r#"set -- 1 2; for jt_x; do sleep 5; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_for_arithmetic() {
    // c:638-645 — WC_FOR_COND renders `for ((i; c; s)) do`, with no `;`
    // between `))` and `do` unlike every other loop opener.
    assert_parity(
        r#"for ((jt_i=0; jt_i<2; jt_i++)) do sleep 5; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_for_nested() {
    assert_parity(
        r#"for jt_x in 1; do for jt_y in 2; do sleep 5; done; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_select_with_list() {
    // c:665-683 WC_SELECT. Redirected so the numbered menu the forked
    // job prints cannot interleave with the parent's own output.
    assert_parity(
        r#"select jt_s in a b; do sleep 5; done >/dev/null 2>&1 & print -r -- "[$jobtexts[1]]""#,
    );
}

#[test]
fn jobtext_select_without_list() {
    // c:669-672 — no WC_SELECT_LIST, so no ` in ` clause.
    assert_parity(
        r#"set -- a b; select jt_s; do sleep 5; done >/dev/null 2>&1 & print -r -- "[$jobtexts[1]]""#,
    );
}

#[test]
fn jobtext_while_loop() {
    // c:685-704 WC_WHILE.
    assert_parity(
        r#"jt_q=1; while [[ -n $jt_q ]]; do sleep 5; jt_q=; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_until_loop() {
    // c:687-688 — WC_WHILE_UNTIL prints `until `.
    assert_parity(
        r#"jt_q=1; until [[ -z $jt_q ]]; do sleep 5; jt_q=; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_repeat_loop() {
    // c:705-720 WC_REPEAT.
    assert_parity(r#"repeat 2; do sleep 5; done & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_if_elif_else() {
    // c:820-859 WC_IF — every branch keyword is preceded by `; `.
    assert_parity(
        r#"if true; then sleep 5; elif false; then echo a; else echo b; fi & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_case_all_three_terminators() {
    // c:721-818 WC_CASE — ` ;;`, ` ;&` and ` ;|` each keep their leading
    // space, and the arms are separated by a bare `' '` (c:779-780),
    // never by `"; "`.
    assert_parity(
        r#"jt_v=a; case $jt_v in (a) sleep 5 ;& (b|c) echo x ;| (*) echo y;; esac & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_case_no_arms() {
    // c:730-736 — an empty `case` closes with `' '` + `esac`.
    assert_parity(r#"jt_v=x; case $jt_v in esac & print -r -- "[$jobtexts[1]]""#);
}

#[test]
fn jobtext_subshell() {
    // c:525-542 WC_SUBSH — `(` SP body `; )`.
    assert_parity(r#"( sleep 5; echo hi ) & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_current_shell_group() {
    // c:543-559 WC_CURSH.
    assert_parity(r#"{ sleep 5; echo hi } & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_always_block() {
    // c:982-1004 WC_TRY — opens with `taddnl(0)`, not WC_CURSH's
    // `taddnl(1)`, so both braces are followed by a semicolon.
    assert_parity(r#"{ sleep 5 } always { echo x } & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_funcdef_body_elided() {
    // c:586-590 — with `tjob` set the body collapses to `{ ... }`.
    assert_parity(r#"jt_fn () { sleep 5 } & print -r -- "[$jobtexts[1]]""#);
}

#[test]
fn jobtext_funcdef_multiple_names() {
    assert_parity(r#"function jt_a jt_b { sleep 5 } & print -r -- "[$jobtexts[1]]""#);
}

#[test]
fn jobtext_funcdef_anonymous() {
    // c:584 — `if (nargs) taddstr(" ")`; an unnamed function emits
    // neither a name nor the space, so the text starts at `()`.
    assert_parity(r#"() { sleep 5 } & print -r -- "[$jobtexts[1]]""#);
}

#[test]
fn jobtext_cond_parenthesises_opposite_operator() {
    // c:905-923 — an `&&` operand under `||` is wrapped in `( … )`,
    // so `[[ -n x && ! -z y || a = b ]]` comes back parenthesised.
    assert_parity(
        r#"while [[ -n x && ! -z y || a = b ]]; do sleep 5; break; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_cond_keeps_explicit_grouping() {
    assert_parity(
        r#"while [[ ( a = a || c = d ) && -n e ]]; do sleep 5; break; done & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_arithmetic_command() {
    // c:972-976 WC_ARITH — the stored expression keeps its inner
    // spacing, so `(( 1 + 2 ))` round-trips byte for byte.
    assert_parity(r#"if (( 1 + 2 )); then sleep 5; fi & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_simple_command_redirections() {
    // c:503-511 WC_REDIR → getredirs (c:1019). Redirections were
    // dropped entirely, so this listed as a bare `sleep 5`.
    assert_parity(
        r#"sleep 5 >>/dev/null 2>&1 <<<"str" 3</dev/null & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_compound_redirections() {
    assert_parity(r#"{ sleep 5 } >/dev/null & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_heredoc_becomes_herestring() {
    // c:1056-1096 — a here-document is rewritten to a here-STRING for
    // display, so the job line carries the body, not the delimiter. An
    // unquoted body is tokenized, so `has_token` picks the double-quoted
    // form and the `$` survives unescaped.
    assert_parity(
        "sleep 5 <<EOT >/dev/null & print -r -- \"[$jobtexts[1]]\"; kill %1\nline $jt_v here\nEOT\n",
    );
}

#[test]
fn jobtext_quoted_heredoc_becomes_single_quoted_herestring() {
    // c:1085-1088 — a `<<'EOT'` body has no tokens, so it takes the
    // single-quoted branch and `$jt_v` stays literal.
    assert_parity(
        "sleep 5 <<'EOT' >/dev/null & print -r -- \"[$jobtexts[1]]\"; kill %1\nraw $jt_v body\nEOT\n",
    );
}

#[test]
fn jobtext_preserves_quoting_and_expansions() {
    // c:304/:342 — `untokenize()` over the finished buffer maps `Qstring`
    // back to `$`. Skipping it dropped the `$` of an expansion inside
    // double quotes: `"a $jt_v b"` listed as `"a jt_v b"`.
    assert_parity(
        r#"sleep 5 "a $jt_v b" 'c$d' \$e & print -r -- "[$jobtexts[1]]"; kill %1"#,
    );
}

#[test]
fn jobtext_assignment_prefixes() {
    // c:183-207 taddassign — every assignment is followed by a space,
    // and an array value is `name=(v1 v2)`.
    assert_parity(r#"jt_p=1 jt_q=(a b) sleep 5 & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobtext_time_keyword() {
    // c:561-573 WC_TIMED.
    assert_parity(r#"time sleep 5 & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn jobs_listing_shows_compound_text() {
    // The `jobs` listing is the user-visible consumer: c:Src/jobs.c:1295
    // prints `[1]  + running    ` followed by the deparsed text.
    assert_parity(r#"for jt_x in 1 2; do sleep 5; done & jobs; kill %1"#);
}

// ── multi-stage pipeline job shape (execpline2 → one addproc per stage) ──
//
// c:Src/exec.c:1795 — the Z_ASYNC arm of `execpline()` runs `execpline2()` in
// the CURRENT shell rather than wrapping the pipeline in one extra process.
// execpline2 recurses once per stage (c:2092) and every stage's `execcmd`
// forks and calls `addproc(pid, text, …)` (c:2907), so a backgrounded
// pipeline holds one proc PER STAGE — each with its own pid and its own
// deparsed text.
//
// zshrs used to fork a single wrapper child for the whole pipeline and
// register ONE proc carrying the whole pipeline's text, so `jobs` printed one
// line where zsh prints one per process, `jobs -l` showed a single pid for a
// multi-process job, and `$jobtexts` was built from one string instead of
// joined per-process texts.
//
// `printjob` (c:Src/jobs.c:1264-1336) then decides the line breaks: it packs
// consecutive same-status procs onto a line while they fit in
// `lineleng = zterm_columns` (c:1151), and `jobs -l` / `jobs -p` (`lng & 3`,
// c:1266-1267) force one proc per line regardless. Every proc that is not the
// last of the JOB prints a trailing `" | "` (c:1331-1332), which is why a
// wrapped pipeline leaves the separator dangling at end of line.

/// Two-stage pipeline, width too narrow to pack: one line per process, and
/// the first line keeps the dangling `" | "` separator (c:1331-1332).
#[test]
fn bg_pipeline_two_stages_one_line_each() {
    assert_parity_cols(20, "sleep 5 | cat & jobs; kill %1");
}

/// Three stages: the middle process also ends its line with `" | "` because a
/// further process follows it in the job; only the last one does not.
#[test]
fn bg_pipeline_three_stages_one_line_each() {
    assert_parity_cols(20, "sleep 5 | cat | cat & jobs; kill %1");
}

/// The same job at a width that fits packs back onto ONE line — the grouping
/// loop at c:1266-1276 is a width test, not an unconditional split. Guards the
/// fix against over-splitting.
#[test]
fn bg_pipeline_packs_onto_one_line_when_it_fits() {
    assert_parity_cols(80, "sleep 5 | cat | cat & jobs; kill %1");
}

/// c:1265 + c:1272-1274 — the fit test is
/// `strlen(qn->text) + len2 + (qn->next ? 3 : 0) > lineleng`, with `len2`
/// seeded at `10 + len` (19 here) and the FIRST text on a line never charged
/// to it. For `sleep 5 | cat` that puts the break at exactly 21/22 columns.
/// Pinning both sides catches an accumulator that is off by even one column.
#[test]
fn bg_pipeline_break_point_just_too_narrow() {
    assert_parity_cols(21, "sleep 5 | cat & jobs; kill %1");
}

#[test]
fn bg_pipeline_break_point_just_wide_enough() {
    assert_parity_cols(22, "sleep 5 | cat & jobs; kill %1");
}

/// Three stages exercise the `+ 3` term: at 24 columns only the FIRST process
/// fits alone, at 25 the first two pack together and the last wraps, and at 27
/// all three share a line. The middle row is the one that fails if the
/// dangling-separator charge is dropped.
#[test]
fn bg_pipeline_three_stage_break_first_alone() {
    assert_parity_cols(24, "sleep 5 | cat | cat & jobs; kill %1");
}

#[test]
fn bg_pipeline_three_stage_break_after_second() {
    assert_parity_cols(25, "sleep 5 | cat | cat & jobs; kill %1");
}

#[test]
fn bg_pipeline_three_stage_all_on_one_line() {
    assert_parity_cols(27, "sleep 5 | cat | cat & jobs; kill %1");
}

/// c:1266-1267 — `jobs -l` sets `lng & 1`, which both forces one process per
/// line and prints each process's OWN pid. A single-proc job table cannot
/// produce a distinct pid per line.
#[test]
fn bg_pipeline_jobs_l_pid_per_process() {
    assert_parity_cols(80, "sleep 5 | cat & jobs -l; kill %1");
}

#[test]
fn bg_pipeline_jobs_l_three_stages() {
    assert_parity_cols(80, "sleep 5 | cat | cat & jobs -l; kill %1");
}

/// c:1291-1301 — `jobs -p` prints the job's group LEADER once, then clears the
/// flag and indents every following line by the width of that pid plus one
/// (the `skip` counter). The continuation lines are blank where the pid was.
#[test]
fn bg_pipeline_jobs_p_indents_continuation_lines() {
    assert_parity_cols(80, "sleep 5 | cat & jobs -p; kill %1");
}

/// `jobs -r` (running only) still walks the same proc list.
#[test]
fn bg_pipeline_jobs_r_lists_each_process() {
    assert_parity_cols(20, "sleep 5 | cat & jobs -r; kill %1");
}

/// c:Src/parse.c:919-928 — `|&` is desugared by the PARSER: a
/// `REDIR_MERGEOUT 2>&1` node is appended to the first command's redirection
/// list, so the deparser renders `a 2>&1 | b` and the first process's job text
/// carries the redirect. Src/text.c has no `|&` arm at all.
#[test]
fn bg_pipeline_errpipe_text_is_desugared_redirect() {
    assert_parity_cols(20, "sleep 5 |& cat & jobs; kill %1");
}

#[test]
fn bg_pipeline_errpipe_jobtexts_is_desugared_redirect() {
    assert_parity_cols(80, r#"sleep 5 |& cat & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

/// c:Src/Modules/parameter.c:1257-1273 — `$jobtexts` is built by walking the
/// job's procs and joining their texts with `" | "`, so it is downstream of
/// the same per-process registration.
#[test]
fn bg_pipeline_jobtexts_joins_process_texts() {
    assert_parity_cols(80, r#"sleep 5 | cat & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

/// Compound stages keep their own deparsed text per process rather than one
/// merged string — `{ … }` on one line, the `while` loop on the next.
#[test]
fn bg_pipeline_compound_stages_keep_own_text() {
    assert_parity_cols(
        20,
        "{ sleep 5 } | while read jt_x; do :; done & jobs; kill %1",
    );
}

/// c:Src/jobs.c:2063 getjob — `%?string` searches the job's text for a
/// substring. It has to match against a stage that is not the first, which
/// only works if that stage's text is actually registered.
#[test]
fn bg_pipeline_job_match_by_later_stage() {
    assert_parity_cols(20, "sleep 5 | cat & jobs %?cat; kill %1");
}

#[test]
fn bg_pipeline_job_match_by_first_stage() {
    assert_parity_cols(20, "sleep 5 | cat & jobs %?sleep; kill %1");
}

/// Two backgrounded pipelines: the `+` / `-` markers still track curjob and
/// prevjob while each job now spans several lines.
#[test]
fn bg_pipeline_two_jobs_markers_with_multiline_jobs() {
    assert_parity_cols(20, "sleep 5 | cat & sleep 5 | cat & jobs; kill %1 %2");
}

/// `( … )` is ONE fork in both shells, so a pipeline inside a subshell stays a
/// single process — the fix must not split it.
#[test]
fn bg_subshell_wrapping_a_pipeline_is_one_process() {
    assert_parity_cols(20, "( sleep 5 | cat ) & jobs; kill %1");
}

/// A single-command background job is unchanged by the per-stage path.
#[test]
fn bg_single_command_still_one_process() {
    assert_parity_cols(20, "sleep 5 & jobs -l; kill %1");
}

// ── async `&&` / `||` / `!` chains: only the final pipeline is a job ───
//
// c:Src/parse.c:660-667 puts Z_ASYNC on the LIST code. execlist runs each
// pipeline ahead of `&&` / `||` with `execpline(state, code, Z_SYNC, 0)`
// (c:Src/exec.c:1557, c:1590) and hands `ltype` only to the final
// WC_SUBLIST_END pipeline (c:1545). Its job text is `getjobtext()` taken at
// that PIPE (c:2059-2064), and execpline's async arm returns `lastval = 0`
// (c:1818) before any `!` inversion. zshrs used to fork the whole chain as
// one job titled with the whole chain.

#[test]
fn async_and_chain_job_text_is_final_pipeline() {
    assert_parity(r#"true && sleep 5 & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn async_or_chain_jobs_listing_is_final_pipeline() {
    assert_parity_cols(80, "false || sleep 5 & jobs; kill %1");
}

#[test]
fn async_negated_job_text_drops_bang() {
    assert_parity(r#"! sleep 5 & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

#[test]
fn async_and_chain_final_brace_group_text() {
    assert_parity(r#"true && { sleep 5; } & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

/// The head runs in THIS shell, so its assignment is visible to the parent.
#[test]
fn async_and_chain_head_runs_in_foreground() {
    assert_parity("x=0; { x=1 } && sleep 5 & print x=$x; kill %1");
}

#[test]
fn async_or_chain_middle_runs_in_foreground() {
    assert_parity(r#"x=0; false || { x=2 } || sleep 5 & print -r -- "x=$x n=${#jobtexts}""#);
}

/// `sleep || true &` — the sync head succeeds, `||` skips the tail, and no
/// job is ever created.
#[test]
fn async_or_chain_short_circuit_makes_no_job() {
    assert_parity(r#"sleep 0.2 || true & print -r -- "st=$? bang=$! n=${#jobtexts}"; kill %1"#);
}

#[test]
fn async_and_chain_skipped_tail_keeps_head_status() {
    assert_parity(r#"false && sleep 5 & print -r -- "st=$? bang=$! n=${#jobtexts}""#);
}

/// The child runs the bare pipeline; `!` is never applied to its status.
#[test]
fn async_negated_child_status_not_inverted() {
    assert_parity("! false & wait $!; print w=$?");
}

#[test]
fn async_coproc_job_text_drops_keyword() {
    assert_parity(r#"coproc sleep 5 & print -r -- "[$jobtexts[1]]"; kill %1"#);
}

/// c:Src/jobs.c:1659-1664 (waitforpid) and c:1711-1717 (zwaitjob) — the
/// `wait` builtin sleeps with signal queueing off (`dont_queue_signals`,
/// c:1641 / c:1689), so a trapped signal runs its trap during the wait, and
/// the wait then returns `128 + last_signal`. A second `wait` for the same
/// child gets its real exit status (Test/C03traps.ztst "waiting for trapped
/// signal"). The wait held bin_fg's signal queue, so the trap ran only once
/// the child had exited, and the first wait returned the child's status.
#[test]
fn wait_interrupted_by_a_trapped_signal() {
    assert_parity(
        r#"child() { sleep 0.3; print sending; kill -15 $parentpid; sleep 0.6; print exiting; exit 33 }
parentpid=$$
child &
cpid=$!
trap 'print trapped' 15
wait $cpid
print "first=$?"
wait $cpid
print "second=$?""#,
    );
    assert_parity(
        r#"child() { sleep 0.3; kill -15 $parentpid; sleep 0.6; exit 7 }
parentpid=$$
child &
trap 'print trapped' 15
wait %1
print "first=$?"
wait %1
print "second=$?""#,
    );
}

/// A background job started inside `$( … )` exits while the substitution is
/// unwinding. Its SIGCHLD handler (c:Src/signals.c:429-431 wait_for_processes
/// → update_bg_job) reads options; the in-process substitution was restoring
/// the option table under a write lock on the same thread, so the handler
/// blocked forever and the substitution never returned (about one run in
/// two). The job-table restore after the body had the same race on the
/// JOBTAB mutex. c:Src/signals.c:410-424 queues the signal while they are
/// held. 200 iterations make a hang near-certain on the unfixed shell.
#[test]
fn sigchld_during_cmdsubst_option_restore_does_not_hang() {
    assert_parity("repeat 200 echo $(echo a &) w");
}

/// c:Src/jobs.c:2578-2580 — `wait PID` for a pid that is not a child warns
/// "pid N is not a child of this shell" only when POSIX_BUILTINS is unset;
/// the status is 127 either way.
#[test]
fn wait_unknown_pid_warning_is_suppressed_under_posix_builtins() {
    assert_parity("wait 1 2>&1; print $?");
    assert_parity("(setopt POSIX_BUILTINS; wait 1 2>&1; print $?)");
}
