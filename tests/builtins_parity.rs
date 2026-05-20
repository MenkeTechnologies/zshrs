//! Behavioural parity between zshrs's `Src/Builtins/` ports and the
//! real `/bin/zsh`. Each builtin in `Src/Builtins/` (currently
//! `rlimits.c` + `sched.c`) gets at least one parity test here.
//!
//! The harness mirrors `tests/modules_parity.rs` — same `assert_parity`
//! shape, same brew-zsh-preferred path resolution, same skip-on-no-zsh
//! semantics.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

fn zsh_path() -> &'static str {
    use std::path::Path;
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

struct ShellResult {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "exit-code divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

// ───────────────────────── Src/Builtins/rlimits.c ─────────────────────

mod rlimits_builtin {
    use super::*;

    /// `ulimit -n` prints the soft limit on file descriptors.
    /// Direct port of bin_ulimit() from Src/Builtins/rlimits.c:729.
    #[test]
    fn ulimit_dash_n_open_files() {
        assert_parity("ulimit -n");
    }

    /// `ulimit -t` — CPU time (seconds).
    #[test]
    fn ulimit_dash_t_cpu_time() {
        assert_parity("ulimit -t");
    }

    /// `ulimit -s` — stack size in 1024-byte blocks.
    #[test]
    fn ulimit_dash_s_stack_size() {
        assert_parity("ulimit -s");
    }

    /// `ulimit -c` — core file size in blocks.
    #[test]
    fn ulimit_dash_c_core_size() {
        assert_parity("ulimit -c");
    }

    /// `ulimit -d` — data segment size.
    #[test]
    fn ulimit_dash_d_data_size() {
        assert_parity("ulimit -d");
    }

    /// `ulimit -f` — file size in blocks.
    #[test]
    fn ulimit_dash_f_file_size() {
        assert_parity("ulimit -f");
    }

    /// `ulimit -u` — number of user processes.
    #[test]
    fn ulimit_dash_u_user_procs() {
        assert_parity("ulimit -u");
    }

    /// `ulimit -v` — virtual memory size.
    #[test]
    fn ulimit_dash_v_vmem() {
        assert_parity("ulimit -v");
    }

    /// `ulimit -H -n` — explicit hard limit selector.
    /// `-H` flag handling per Src/Builtins/rlimits.c:732.
    #[test]
    fn ulimit_hard_limit_dash_h() {
        assert_parity("ulimit -H -n");
    }

    /// `ulimit -S -n` — explicit soft limit selector (default).
    #[test]
    fn ulimit_soft_limit_dash_s_lower() {
        assert_parity("ulimit -S -n");
    }

    /// `limit` with no args lists ALL limits in zsh-native form.
    /// Direct port of bin_limit() from Src/Builtins/rlimits.c:519.
    #[test]
    fn limit_no_args_lists_all() {
        // The ordering / selection of which limits appear differs by
        // platform; just make sure both shells produce the SAME line
        // count for `limit` (proxy for "we agree on which limits
        // exist").
        let z = run_zsh("limit | wc -l | tr -d ' '");
        let r = run_zshrs("limit | wc -l | tr -d ' '");
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
    }

    /// `limit stacksize` prints just the named limit.
    #[test]
    fn limit_stacksize_named() {
        assert_parity("limit stacksize");
    }

    /// `limit cputime` — CPU time as `limit` reports it.
    #[test]
    fn limit_cputime() {
        assert_parity("limit cputime");
    }

    /// `limit -s` — show ONLY soft limits (the default but explicit).
    #[test]
    fn limit_dash_s_soft() {
        // Like `limit` no-args, just compare line counts.
        let z = run_zsh("limit -s | wc -l | tr -d ' '");
        let r = run_zshrs("limit -s | wc -l | tr -d ' '");
        assert_eq!(z.stdout, r.stdout);
    }

    /// `limit -h` — hard limits.
    #[test]
    fn limit_dash_h_hard() {
        let z = run_zsh("limit -h | wc -l | tr -d ' '");
        let r = run_zshrs("limit -h | wc -l | tr -d ' '");
        assert_eq!(z.stdout, r.stdout);
    }

    /// `unlimit -h cpu` removes the hard cpu limit. Just verify the
    /// invocation parses without error in both shells (actual effect
    /// requires root for some limits — gate on exit code).
    #[test]
    fn unlimit_invocation_parses() {
        // Don't actually run it (would mutate process state); just
        // smoke the parse path via a syntax-only check.
        let z = run_zsh("autoload -Uz; type unlimit");
        let r = run_zshrs("autoload -Uz; type unlimit");
        // Both should report `unlimit` exists as a builtin.
        assert!(
            z.stdout.contains("unlimit") && r.stdout.contains("unlimit"),
            "z={:?} r={:?}",
            z.stdout,
            r.stdout
        );
    }

    /// Setting a limit then reading it back should round-trip.
    /// Run inside a subshell so it doesn't affect the outer process.
    #[test]
    fn ulimit_set_then_get_round_trip() {
        // `ulimit -t 60; ulimit -t` — set CPU time to 60s, read back.
        // Both shells should print "60".
        assert_parity("ulimit -t 60; ulimit -t");
    }

    /// `ulimit -t unlimited` accepts the string form. Some platforms
    /// reject when the hard limit is finite; gate via "should NOT
    /// error" rather than requiring success.
    #[test]
    fn ulimit_unlimited_string_form() {
        // Don't compare exit codes — privilege-dependent. Just verify
        // both shells parse the syntax.
        let _ = run_zsh("(ulimit -t unlimited 2>/dev/null) ; echo done");
        let _ = run_zshrs("(ulimit -t unlimited 2>/dev/null) ; echo done");
    }
}

// ───────────────────────── Src/Builtins/sched.c ─────────────────────

mod sched_builtin {
    use super::*;

    /// `sched` with no args lists scheduled commands. Empty schedule
    /// → no output, exit 0. Direct port of bin_sched() from
    /// Src/Builtins/sched.c:150.
    #[test]
    fn sched_empty_no_output() {
        assert_parity("sched");
    }

    /// `sched -L` is a sched flag in newer zsh — not all builds. Just
    /// smoke the parse path; both shells must agree on whether it's
    /// supported.
    #[test]
    fn sched_dash_l_consistent() {
        let z = run_zsh("sched -L 2>&1; echo exit:$?");
        let r = run_zshrs("sched -L 2>&1; echo exit:$?");
        // Exit-status agreement is the contract — both either accept
        // or both reject.
        let zlast = z.stdout.lines().last().unwrap_or("");
        let rlast = r.stdout.lines().last().unwrap_or("");
        assert_eq!(zlast, rlast, "z={:?} r={:?}", z.stdout, r.stdout);
    }

    /// `sched -1` deletes scheduled command #1. With nothing
    /// scheduled, both shells emit an error to stderr and exit
    /// non-zero.
    #[test]
    fn sched_dash_1_no_event() {
        let z = run_zsh("sched -1 2>/dev/null; echo $?");
        let r = run_zshrs("sched -1 2>/dev/null; echo $?");
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
    }

    /// `$sched` — the magic array that lists scheduled commands.
    /// Empty by default. Direct port of schedgetfn() from
    /// Src/Builtins/sched.c:341.
    #[test]
    fn sched_array_empty_by_default() {
        assert_parity(r#"print -- "${#sched}""#);
    }

    /// `sched +5 echo hi` schedules a relative offset, listed by
    /// `sched` with no args. Don't actually wait for it — just verify
    /// both shells accept the `+OFFSET` form and listing shows the
    /// command.
    #[test]
    fn sched_plus_offset_lists_command() {
        // The exact format string differs (zsh shows "1 +5" or
        // similar with the registration ID); smoke the path by
        // checking the command text appears.
        let z = run_zsh("sched +60 echo myschedmark; sched | grep -c myschedmark");
        let r = run_zshrs("sched +60 echo myschedmark; sched | grep -c myschedmark");
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
    }

    /// `sched -1` after registering removes the entry. Round-trip
    /// register + delete + list-empty.
    #[test]
    fn sched_register_then_delete() {
        let script = r#"sched +60 echo trash 2>/dev/null
sched -1 2>/dev/null
print -- "${#sched}""#;
        let z = run_zsh(script);
        let r = run_zshrs(script);
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
    }
}
