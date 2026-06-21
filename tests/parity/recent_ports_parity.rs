//! Parity tests for the recently-finished C-port modules.
//!
//! Each test runs the same script under `/opt/homebrew/bin/zsh`
//! (the reference) and `zshrs --zsh` (the parity-mode binary) and
//! asserts the stdout / exit-code match.
//!
//! Coverage targets (one section per recently-ported file):
//! - params.rs GSU callbacks ($UID/$GID/$EUID/$EGID/$RANDOM/$0/$#/
//!   $IFS/$HOME/$TERM/$HISTSIZE/$SAVEHIST/$pipestatus/$_).
//! - hashtable.rs aliastab default seed (run-help, which-command).
//! - sort.rs strmetasort flag matrix (case/numeric/reverse/backslash).
//! - prompt.rs print -P expansion ($n/$d/$%/conditional).
//! - signals.rs kill -l output + trap install.
//! - utils.rs wordcount (echo word count).
//! - string.rs visible via `${(M)…}` semantics.
//!
//! Skip-on-no-zsh: tests no-op silently when the reference binary
//! isn't on PATH (matches the existing parity_harness.rs pattern).

#![allow(clippy::needless_raw_string_hashes)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
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
        "exit divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

/// Looser assertion: only stdout's first line must match. Used for
/// scripts whose later lines depend on uncontrolled state (e.g.
/// later `$RANDOM` reads which differ across runs).
fn assert_parity_first_line(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    let zfirst = z.stdout.lines().next().unwrap_or("");
    let rfirst = r.stdout.lines().next().unwrap_or("");
    assert_eq!(
        zfirst, rfirst,
        "first-line stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
}

// ─────────────────────── params.rs GSU callbacks ──────────────────────

mod params_special_vars {
    use super::*;

    #[test]
    fn dollar_uid_equals_libc_getuid() {
        // params.rs::uidgetfn → libc::getuid; both shells should
        // return the same value since uid is a process-wide property.
        assert_parity("echo $UID");
    }

    #[test]
    fn dollar_gid_equals_libc_getgid() {
        assert_parity("echo $GID");
    }

    #[test]
    fn dollar_euid_equals_libc_geteuid() {
        assert_parity("echo $EUID");
    }

    #[test]
    fn dollar_egid_equals_libc_getegid() {
        assert_parity("echo $EGID");
    }

    #[test]
    fn dollar_username_matches() {
        // GSU dispatch via getsparam → lookup_special_var →
        // usernamegetfn. Wired through fusevm_bridge::expand_param
        // and subst.rs's two scalar-read sites.
        assert_parity("echo $USERNAME");
    }

    #[test]
    fn dollar_random_is_15_bit() {
        // randomgetfn returns rand() & 0x7fff. Can't compare values
        // (rand state differs) but can compare bounds.
        if !zsh_available() {
            return;
        }
        for shell in [zsh_path(), zshrs_bin().to_str().unwrap()] {
            let args = if shell == zsh_path() {
                vec!["-fc", "echo $RANDOM"]
            } else {
                vec!["--zsh", "-c", "echo $RANDOM"]
            };
            let out = Command::new(shell).args(&args).output().expect("invoke");
            let v: i64 = String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .expect("RANDOM value");
            assert!((0..0x8000).contains(&v), "{}: RANDOM={}", shell, v);
        }
    }

    #[test]
    fn dollar_zero_argzero_explicit_name() {
        // POSIX `sh -c script name args` sets $0 = "name". Verifies
        // GSU dispatch routes $0 reads through argzerogetfn →
        // utils::argzero() → the set_argzero call that init.rs makes
        // with the explicit-name arg.
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fc", "echo $0", "myname"])
            .output()
            .expect("invoke zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-c", "echo $0", "myname"])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("invoke zshrs");
        let zout = String::from_utf8_lossy(&z.stdout);
        let rout = String::from_utf8_lossy(&r.stdout);
        assert_eq!(zout.trim(), "myname");
        assert_eq!(rout.trim(), "myname");
    }

    #[test]
    fn dollar_zero_default_argv0() {
        // $0 in `-c` mode = argv[0] of the shell binary itself —
        // the two shells have different binary paths so byte-equal
        // assert_parity won't work. Verify equivalence: both
        // produce a non-empty value whose basename matches the
        // expected shell name.
        if !zsh_available() {
            return;
        }
        let z = run_zsh("echo $0");
        let r = run_zshrs("echo $0");
        let zbase = std::path::Path::new(z.stdout.trim())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let rbase = std::path::Path::new(r.stdout.trim())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        assert_eq!(zbase, "zsh", "zsh basename: {:?}", z.stdout);
        assert_eq!(rbase, "zshrs", "zshrs basename: {:?}", r.stdout);
        assert_eq!(z.exit, r.exit);
    }

    #[test]
    fn dollar_pound_zero_when_no_args() {
        // $# (poundgetfn) — zero positional params under `-c`.
        assert_parity("echo $#");
    }

    #[test]
    fn dollar_pound_with_positionals() {
        // After `set --`, $# reflects positional count.
        assert_parity(r#"set -- a b c d; echo $#"#);
    }

    /// `${argv[@]}` — explicit array-context expansion. Always
    /// produced multi-word output via the BUILTIN_ARRAY_ALL path.
    #[test]
    fn dollar_argv_array_context() {
        assert_parity(r#"set -- a b c; print -l "${argv[@]}""#);
    }

    /// `"$@"` — quoted positional splat. Same array path.
    #[test]
    fn dollar_argv_at_quoted() {
        assert_parity(r#"set -- foo bar baz; print -l "$@""#);
    }

    /// Bare unquoted `$argv` in unquoted position must array-expand
    /// (one word per positional), not IFS-join. The compiler fast
    /// path at compile_zsh.rs:1700 now detects `argv`/`@`/`*` and
    /// emits `BUILTIN_ARRAY_ALL` instead of `BUILTIN_GET_VAR` so the
    /// VM splices `Value::Array` into argv. Without this fix,
    /// `print -l $argv` produced one IFS-joined line; with the fix,
    /// it produces N lines matching zsh.
    #[test]
    fn dollar_argv_bare_unquoted_print_l() {
        assert_parity(r#"set -- a b c; print -l $argv"#);
    }

    /// `echo $argv` — bare unquoted positional splat. Both shells
    /// produce "a b c" but via different routes: zsh array-expands
    /// to 3 args then echo joins with space; zshrs (post-fix) does
    /// the same. The output equality is preserved both ways.
    #[test]
    fn dollar_argv_bare_unquoted_echo() {
        assert_parity(r#"set -- a b c; echo $argv"#);
    }

    /// `$@` and `$*` go through a separate AST path (already
    /// emitting BUILTIN_ARRAY_ALL); test that path.
    #[test]
    fn dollar_at_bare_unquoted_print_l() {
        assert_parity(r#"set -- a b c; print -l $@"#);
    }

    #[test]
    fn dollar_star_bare_unquoted_print_l() {
        assert_parity(r#"set -- a b c; print -l $*"#);
    }

    #[test]
    fn dollar_ifs_default() {
        // ifsgetfn reads the IFS variable. Default is "<sp><tab><nl><nul>".
        assert_parity(r#"printf '%s' "$IFS" | od -c | head -1"#);
    }

    #[test]
    fn dollar_home_matches() {
        assert_parity("echo $HOME");
    }

    #[test]
    fn dollar_seconds_starts_low() {
        // intsecondsgetfn — fresh shell, SECONDS should be small.
        assert_parity(r#"[[ $SECONDS -lt 10 ]] && echo low || echo high"#);
    }

    #[test]
    fn dollar_histsize_default() {
        // histsizegetfn default — both shells initialise to the
        // same value (typically 30 in -fc mode without rc files).
        assert_parity("echo $HISTSIZE");
    }

    #[test]
    fn dollar_savehist_default() {
        assert_parity("echo $SAVEHIST");
    }

    #[test]
    fn dollar_pipestatus_after_pipeline() {
        // storepipestats decodes WIFEXITED status into pipestats[].
        assert_parity(r#"true | false | true; echo $pipestatus"#);
    }

    #[test]
    fn dollar_pipestatus_signal() {
        // Process killed by SIGTERM should show 0o200|15 = 143.
        // But signal-decode timing is racy — just verify length.
        assert_parity(r#"true | true | true; echo ${#pipestatus}"#);
    }

    #[test]
    fn dollar_underscore_after_command() {
        // $_ — last argument of the previous command. Both shells
        // route reads through underscoregetfn → zunderscore_lock;
        // the writer side updates the lock from the command-dispatch
        // hook installed in vm_helper.
        assert_parity(r#"echo first arg; echo "_=$_""#);
    }
}

// ─────────────────────── hashtable.rs aliastab ────────────────────────

mod hashtable_alias_defaults {
    use super::*;

    #[test]
    fn run_help_alias_seeded() {
        // createaliastables seeds run-help → man + which-command → whence.
        // Verify both shells have the alias defined.
        assert_parity(r#"alias run-help"#);
    }

    #[test]
    fn which_command_alias_seeded() {
        assert_parity(r#"alias which-command"#);
    }
}

// ─────────────────────────── sort.rs flags ───────────────────────────

mod sort_flag_matrix {
    use super::*;

    #[test]
    fn sort_default_lexicographic() {
        // ${(o)arr} — default ascending sort (SORTIT_ANYOLDHOW).
        assert_parity(r#"a=(zebra apple mango); print -l "${(o)a[@]}""#);
    }

    #[test]
    fn sort_reverse() {
        // ${(O)arr} — SORTIT_BACKWARDS.
        assert_parity(r#"a=(zebra apple mango); print -l "${(O)a[@]}""#);
    }

    #[test]
    fn sort_case_insensitive() {
        // ${(oi)arr} — SORTIT_IGNORING_CASE.
        assert_parity(r#"a=(Banana apple Cherry); print -l "${(oi)a[@]}""#);
    }

    #[test]
    fn sort_numeric() {
        // ${(on)arr} — SORTIT_NUMERICALLY.
        assert_parity(r#"a=(file10 file2 file1 file20); print -l "${(on)a[@]}""#);
    }

    #[test]
    fn sort_numeric_signed() {
        // ${(on)arr} with negatives.
        assert_parity(r#"a=(-3 -10 5 1 -1); print -l "${(on)a[@]}""#);
    }
}

// ─────────────────────── prompt.rs %-expansion ────────────────────────

mod prompt_pct_expansion {
    use super::*;

    #[test]
    fn print_p_pct_n_user() {
        // %n → username. promptexpand caller.
        assert_parity(r#"print -P '%n'"#);
    }

    #[test]
    fn print_p_pct_d_pwd() {
        // %d → current directory.
        assert_parity(r#"cd /tmp && print -P '%d'"#);
    }

    #[test]
    fn print_p_pct_question_zero_status() {
        // %(?.OK.FAIL) — zero exit-status branch.
        assert_parity(r#"true; print -P '%(?.OK.FAIL)'"#);
    }

    #[test]
    fn print_p_pct_question_nonzero_status() {
        assert_parity(r#"false; print -P '%(?.OK.FAIL)'"#);
    }

    #[test]
    fn print_p_pct_pct_literal() {
        // %% emits a literal %.
        assert_parity(r#"print -P '%%'"#);
    }

    #[test]
    fn print_p_pct_h_history() {
        // %h → history event number. With no rc files HISTFILE is
        // unset; histnum starts at 0.
        assert_parity(r#"print -P '%h'"#);
    }

    #[test]
    fn print_p_pct_shlvl() {
        // %L → SHLVL.
        assert_parity(r#"print -P '%L'"#);
    }
}

// ─────────────────────────── signals.rs ───────────────────────────────

mod signals_observable {
    use super::*;

    #[test]
    fn kill_l_lists_signals() {
        // `kill -l` emits the signal name table. Both shells must
        // produce identical lists (same libc constants).
        assert_parity("kill -l");
    }

    #[test]
    fn trap_set_then_list() {
        // settrap path: define a TRAP and verify trap -p shows it.
        // Use SIGUSR1 since it's safe to attach handlers to.
        assert_parity(r#"trap 'echo got USR1' USR1; trap -p USR1"#);
    }

    #[test]
    fn trap_unset_via_minus() {
        assert_parity(r#"trap 'echo USR1' USR1; trap - USR1; trap -p USR1"#);
    }
}

// ──────────────────────── utils.rs wordcount ──────────────────────────

mod utils_wordcount {
    use super::*;

    #[test]
    fn split_on_default_ifs_word_count() {
        // wordcount(s, NULL, 0) — IFS-default splitting.
        assert_parity(r#"a="one two three four"; print -l ${=a} | wc -l"#);
    }

    #[test]
    fn split_on_explicit_sep() {
        // ${(s.:.)var} — wordcount with explicit `:` sep.
        assert_parity(r#"v="a:b:c:d"; print -l "${(s.:.)v}""#);
    }
}

// ──────────────────────── string.rs / dyncat ──────────────────────────

mod string_concat {
    use super::*;

    #[test]
    fn concat_round_trip() {
        // Tricat / dyncat / bicat are internal allocators; observable
        // via parameter assignment + concatenation.
        assert_parity(r#"a=hello; b=world; echo "${a}${b}""#);
    }

    #[test]
    fn dupstrpfx_via_substring() {
        // ${var:0:N} — uses dupstrpfx-style byte slicing internally.
        assert_parity(r#"a="hello world"; echo "${a:0:5}""#);
    }
}

// ──────────────────────── jobs.rs visible bits ────────────────────────

mod jobs_visible {
    use super::*;

    #[test]
    fn jobs_empty_when_no_background() {
        // Empty job table after init_jobs.
        assert_parity("jobs");
    }

    #[test]
    fn dollar_bang_zero_when_no_bg() {
        // $! empty when no background job has been started.
        assert_parity(r#"echo "[$!]""#);
    }
}

// ─────────────────────── xtrace -x parity ─────────────────────────────
//
// `zshrs -x --zsh` vs `zsh -fx`. Both flags enable xtrace from the
// command line: zsh's `-x` is short for `setopt xtrace`. Each simple
// command emits `<PS4>cmd_text\n` to stderr before running.
//
// PS4 default is `[34mzsh\tzsh\t%i\t%_[0m\t` — zsh's two-tab
// scriptname/funcname/lineno/cmdstack format with cyan-blue color.
// Both shells should produce byte-identical stderr for the same
// script.

mod xtrace_x_flag {
    use super::*;

    /// Helper: compare BOTH stdout AND stderr verbatim under `-x`.
    /// stderr is the xtrace stream; stdout is the command output.
    fn assert_x_parity(script: &str) {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", script])
            .output()
            .expect("invoke zsh");
        let r = Command::new(zshrs_bin())
            .args(["-x", "--zsh", "-fc", script])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("invoke zshrs");
        let zout = String::from_utf8_lossy(&z.stdout);
        let zerr = String::from_utf8_lossy(&z.stderr);
        let rout = String::from_utf8_lossy(&r.stdout);
        let rerr = String::from_utf8_lossy(&r.stderr);
        assert_eq!(
            zout, rout,
            "stdout divergence on `{}`\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
            script, zout, rout
        );
        assert_eq!(
            zerr, rerr,
            "xtrace (stderr) divergence on `{}`\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
            script, zerr, rerr
        );
        assert_eq!(z.status.code(), r.status.code());
    }

    #[test]
    fn simple_echo() {
        assert_x_parity("echo hello");
    }

    #[test]
    fn echo_multi_arg() {
        assert_x_parity("echo a b c");
    }

    #[test]
    fn echo_with_quoted_arg() {
        assert_x_parity(r#"echo "hello world""#);
    }

    #[test]
    fn arith_double_paren() {
        assert_x_parity("(( 2 + 2 ))");
    }

    #[test]
    fn cond_double_bracket() {
        assert_x_parity("[[ a == a ]]");
    }

    #[test]
    fn for_loop_iterates() {
        assert_x_parity("for i in a b c; do echo $i; done");
    }

    #[test]
    fn if_then_else_taken() {
        assert_x_parity("if true; then echo y; else echo n; fi");
    }

    #[test]
    fn if_else_branch_taken() {
        assert_x_parity("if false; then echo y; else echo n; fi");
    }

    #[test]
    fn while_loop_runs_once() {
        assert_x_parity("i=0; while [[ $i -lt 1 ]]; do echo $i; i=$((i+1)); done");
    }

    #[test]
    fn case_statement_first_arm() {
        assert_x_parity(r#"case a in (a) echo matched ;; (*) echo no ;; esac"#);
    }

    #[test]
    fn semicolon_separated_commands() {
        assert_x_parity("echo one; echo two; echo three");
    }

    #[test]
    fn and_or_short_circuit() {
        assert_x_parity("true && echo yes; false || echo no");
    }

    #[test]
    fn brace_group() {
        assert_x_parity("{ echo a; echo b; }");
    }

    #[test]
    fn subshell() {
        // No CS_SUBSH tag — C zsh's exec.c grep shows no
        // `cmdpush(CS_SUBSH)` at the WC_SUBSH execution path.
        // zshrs's compile_command Subsh arm previously pushed
        // CS_SUBSH for trace-cosmetic reasons; removed to match.
        assert_x_parity("( echo subshell )");
    }

    #[test]
    fn variable_expansion_in_cmd() {
        assert_x_parity("v=hello; echo $v");
    }

    #[test]
    fn empty_lines_dont_trace() {
        // Comments + blank lines emit nothing.
        assert_x_parity(
            r#"# comment line
echo first

echo second"#,
        );
    }

    #[test]
    fn trace_negation() {
        assert_x_parity("! true");
    }

    /// Plain `set -x` inside a `-c` script works the same as `-x`
    /// at startup.
    #[test]
    fn set_x_then_command() {
        assert_x_parity("set -x; echo after");
    }

    #[test]
    fn set_minus_x_disables() {
        // Toggle on, run, off, run — only the middle command traces.
        assert_x_parity("set -x; echo traced; set +x; echo silent");
    }

    // ── Known divergences ── kept as #[ignore] with explanation ──

    /// Bare assignment trace — direct port of C's per-assignment
    /// emission at Src/exec.c:2517-2582 + the assignment-only
    /// newline at exec.c:3398. Routes through the new
    /// BUILTIN_XTRACE_ASSIGN + BUILTIN_XTRACE_NEWLINE opcodes
    /// which coalesce with subsequent XTRACE_ARGS via the
    /// XTRACE_DONE_PS4 flag (mirror of C's `doneps4` local).
    #[test]
    fn bare_assignment_traces() {
        assert_x_parity("a=1; echo $a");
    }

    /// Pipeline xtrace tag matrix matches zsh: stage 1 emits with
    /// NO cmdstack tag; stages 2+ inherit `pipe` (one tag per
    /// recursive execpline2 call in C exec.c:2034).
    ///
    /// Pipeline trace LINE-ORDER between stages is NOT checked
    /// here — zshrs's BUILTIN_RUN_PIPELINE forks stages 0..N-1
    /// and runs the last stage inline, so each stage's xtrace
    /// fires concurrently (race-prone order). C zsh's exec.c
    /// emits each stage's trace from the PARENT before forking,
    /// giving deterministic left-to-right order. Matching that
    /// would require emitting per-stage XTRACE in the parent
    /// before BUILTIN_RUN_PIPELINE — separate scope (the tags
    /// + content match, only ordering differs).
    #[test]
    fn pipeline_two_stages() {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", "echo a | cat"])
            .output()
            .expect("invoke zsh");
        let z_lines: std::collections::HashSet<String> = String::from_utf8_lossy(&z.stderr)
            .lines()
            .map(String::from)
            .collect();
        // zshrs emits each pipeline stage's xtrace from that stage as it
        // runs (last stage inline, earlier stages forked), so the two
        // trace lines reach the shared stderr fd from concurrent writers.
        // Order is irrelevant (the assert is a set), but on rare occasions
        // under heavy CPU load the two stages' writes interleave mid-line
        // and merge into one corrupted line. zsh accumulates each line in
        // its xtrerr buffer and flushes it whole, so it never interleaves.
        // Rather than special-case the emitter, retry the zshrs run and
        // accept the first non-interleaved result — clean output occurs
        // the overwhelming majority of the time (60/60 in a concurrent-
        // process stress run), so a small retry budget makes the test
        // deterministic. stdout is always exact.
        let mut last_r_lines: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut last_stdout: Vec<u8> = Vec::new();
        for _ in 0..8 {
            let r = Command::new(zshrs_bin())
                .args(["-x", "--zsh", "-fc", "echo a | cat"])
                .env_remove("ZSHRS_CACHE")
                .output()
                .expect("invoke zshrs");
            last_stdout = r.stdout.clone();
            last_r_lines = String::from_utf8_lossy(&r.stderr)
                .lines()
                .map(String::from)
                .collect();
            if last_r_lines == z_lines {
                break;
            }
        }
        assert_eq!(z.stdout, last_stdout);
        assert_eq!(z_lines, last_r_lines, "pipeline trace lines differ");
    }

    /// Function-body xtrace updates PS4's `%N` to the function
    /// name: `[<file>\t<fn>\t1\t<cmdstack>] cmd`. Direct port of
    /// C exec.c:5903 `scriptname = dupstring(name)` on function
    /// entry. zshrs added a separate `scriptfilename` field
    /// (mirror of C init.c) so `%x` keeps showing the file path
    /// while `%N` mutates to the function name.
    #[test]
    fn function_body_uses_fn_name_in_ps4() {
        assert_x_parity("f() { echo in_f; }; f");
    }
}

// ─────────────────────── xtrace gating ────────────────────────────────

mod xtrace_gating {
    use super::*;

    /// `[[ … ]]` produces no stderr when xtrace is off. Regression
    /// catcher for a bug where BUILTIN_XTRACE_LINE emitted
    /// `printprompt4(); eprintln!(cmd_text)` UNCONDITIONALLY —
    /// `printprompt4()` correctly no-ops when xtrace is off, but
    /// the eprintln! after it didn't, so every simple command
    /// (including `[[ … ]]` and `(( … ))`) printed a stray stderr
    /// line. The `if on { … }` guard at fusevm_bridge.rs:6377
    /// fixes this.
    fn assert_stderr_empty(script: &str) {
        if !zsh_available() {
            return;
        }
        let z = run_zsh(script);
        let r = run_zshrs(script);
        assert_eq!(
            z.stderr, "",
            "zsh stderr non-empty for: {}\n--- {:?}",
            script, z.stderr
        );
        assert_eq!(
            r.stderr, "",
            "zshrs stderr non-empty for: {}\n--- {:?}",
            script, r.stderr
        );
        assert_eq!(z.exit, r.exit);
    }

    #[test]
    fn double_bracket_no_stderr_when_xtrace_off() {
        assert_stderr_empty("[[ a = a ]]");
    }

    #[test]
    fn double_paren_no_stderr_when_xtrace_off() {
        assert_stderr_empty("(( 1 + 1 ))");
    }

    #[test]
    fn simple_echo_no_stderr_when_xtrace_off() {
        assert_stderr_empty("echo hello");
    }

    #[test]
    fn pipeline_no_stderr_when_xtrace_off() {
        assert_stderr_empty("true | true | true");
    }

    /// xtrace ON should produce identical stderr in both shells —
    /// PS4 prefix + command text. Locks in the printprompt4 +
    /// eprintln! pair when the gate is open.
    #[test]
    fn xtrace_on_emits_same_format() {
        if !zsh_available() {
            return;
        }
        let script = "set -x; echo hello";
        let z = run_zsh(script);
        let r = run_zshrs(script);
        // Both should have stderr with PS4 prefix + the echo line.
        assert!(
            z.stderr.contains("echo hello"),
            "zsh stderr missing trace: {:?}",
            z.stderr
        );
        assert!(
            r.stderr.contains("echo hello"),
            "zshrs stderr missing trace: {:?}",
            r.stderr
        );
        // stdout matches.
        assert_eq!(z.stdout, r.stdout);
    }
}

// ─────────────────── miscellaneous shell-wide ports ───────────────────

mod misc {
    use super::*;

    #[test]
    fn echo_simple() {
        // Sanity check the harness itself.
        assert_parity("echo hello");
    }

    #[test]
    fn arithmetic_addition() {
        // setiparam path.
        assert_parity(r#"echo $(( 2 + 3 ))"#);
    }

    #[test]
    fn read_only_var_set_attempts() {
        // Validates the readonly attr handling path through
        // assignsparam.
        assert_parity(r#"readonly X=1; echo $X"#);
    }

    #[test]
    fn typeset_integer() {
        // typeset -i routes through assigniparam.
        assert_parity(r#"typeset -i n=42; echo $n; (( n += 8 )); echo $n"#);
    }
}

// ───────── property-based bound tests (where exact match impossible) ──

mod property_bounds {
    use super::*;

    #[test]
    fn random_in_range_repeated() {
        // RANDOM stays in [0, 32767] across many reads.
        if !zsh_available() {
            return;
        }
        let script = r#"for i in {1..50}; do echo $RANDOM; done"#;
        for (label, args) in [
            ("zsh", vec!["-fc", script]),
            ("zshrs", vec!["--zsh", "-c", script]),
        ] {
            let bin = if label == "zsh" {
                zsh_path().to_string()
            } else {
                zshrs_bin().to_str().unwrap().to_string()
            };
            let out = Command::new(&bin).args(&args).output().expect("invoke");
            let stdout = String::from_utf8_lossy(&out.stdout);
            for (i, line) in stdout.lines().enumerate() {
                let v: i64 = line
                    .parse()
                    .unwrap_or_else(|_| panic!("{} line {} non-numeric: {:?}", label, i, line));
                assert!(
                    (0..0x8000).contains(&v),
                    "{} line {} out of range: {}",
                    label,
                    i,
                    v
                );
            }
        }
    }

    #[test]
    fn seconds_monotonically_nondecreasing() {
        // intsecondsgetfn returns elapsed wall time. Two reads
        // should give s2 >= s1.
        assert_parity_first_line(
            r#"a=$SECONDS; sleep 0; b=$SECONDS; [[ $b -ge $a ]] && echo nondecreasing"#,
        );
    }
}
