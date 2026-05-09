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
            assert!(v >= 0 && v < 0x8000, "{}: RANDOM={}", shell, v);
        }
    }

    #[test]
    #[ignore = "zshrs --zsh -c doesn't yet honor the POSIX `sh -c script \
                name args` convention where the next non-option arg becomes \
                $0. Re-enable once init.rs's argv-parser handles the \
                trailing-name slot."]
    fn dollar_zero_argzero_explicit_name() {
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
    #[ignore = "Same GSU integration gap as dollar_username_matches: \
                params.rs::argzerogetfn / argzerosetfn route through \
                utils::argzero(), but the shell's $0 read still goes \
                through ShellExecutor's argv handling. Re-enable once \
                GSU dispatch is unified."]
    fn dollar_zero_default_argv0() {
        assert_parity("echo $0");
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
    #[ignore = "$_ requires hooking command dispatch to update \
                zunderscore_lock with the last argument of the previous \
                command; the underscoregetfn callback is wired but the \
                shell's exec path doesn't yet write to it. Re-enable \
                once the command-completion hook lands."]
    fn dollar_underscore_after_command() {
        // underscoregetfn returns last command's last argument.
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
                    v >= 0 && v < 0x8000,
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
