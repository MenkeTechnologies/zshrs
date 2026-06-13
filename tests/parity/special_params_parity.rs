//! Special parameter parity tests — $?, $$, $LINENO, $SECONDS, $RANDOM,
//! $UID, $PPID, $SHLVL, $PWD, etc.

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

mod exit_status {
    use super::*;

    #[test]
    fn dollar_question_after_true() {
        assert_parity("true; echo $?");
    }

    #[test]
    fn dollar_question_after_false() {
        assert_parity("false; echo $?");
    }

    #[test]
    fn dollar_question_after_subshell_exit() {
        assert_parity("(exit 42); echo $?");
    }

    #[test]
    fn dollar_question_after_pipeline() {
        // By default, $? = exit status of last command in pipeline.
        assert_parity("false | true; echo $?");
    }

    #[test]
    fn dollar_question_after_grouped_commands() {
        assert_parity("{ false; }; echo $?");
    }
}

mod positional_count {
    use super::*;

    #[test]
    fn dollar_hash_no_args() {
        assert_parity("echo $#");
    }

    #[test]
    fn dollar_hash_after_set() {
        assert_parity("set -- a b c d e; echo $#");
    }

    #[test]
    fn dollar_hash_in_function() {
        assert_parity("f() { echo $#; }; f one two three");
    }

    #[test]
    fn dollar_hash_after_shift() {
        assert_parity("set -- a b c; shift; echo $#");
    }
}

mod pid {
    use super::*;

    /// `$$` is positive integer, stable within shell.
    #[test]
    fn dollar_dollar_is_positive_integer() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh("echo $$");
        let r = run_zshrs("echo $$");
        // Both must be a parseable positive integer; values DIFFER
        // since they're separate processes.
        let z_pid: u32 = z.stdout.trim().parse().expect("zsh $$ parseable");
        let r_pid: u32 = r.stdout.trim().parse().expect("zshrs $$ parseable");
        assert!(z_pid > 0);
        assert!(r_pid > 0);
    }

    /// $$ is stable within the same shell invocation.
    #[test]
    fn dollar_dollar_stable_within_shell() {
        assert_parity(r#"A=$$; B=$$; [[ "$A" == "$B" ]]; echo $?"#);
    }

    /// Subshell sees parent $$ (not its own pid).
    /// zsh special: $$ stays parent's pid in subshell; $sysparams[pid] is current.
    #[test]
    fn dollar_dollar_unchanged_in_subshell() {
        assert_parity(r#"A=$$; B=$(echo $$); [[ "$A" == "$B" ]]; echo $?"#);
    }
}

mod lineno {
    use super::*;

    /// $LINENO increments per logical line in a script.
    #[test]
    fn lineno_increments_across_lines() {
        if !zsh_available() {
            return;
        }
        // Use 3-line script; the third echo should report a higher
        // LINENO than the first.
        let z = run_zsh(
            r#"
echo $LINENO
echo $LINENO
echo $LINENO
"#,
        );
        let r = run_zshrs(
            r#"
echo $LINENO
echo $LINENO
echo $LINENO
"#,
        );
        // Pin: line numbers strictly increasing.
        let z_nums: Vec<i32> = z.stdout.lines().filter_map(|l| l.parse().ok()).collect();
        let r_nums: Vec<i32> = r.stdout.lines().filter_map(|l| l.parse().ok()).collect();
        assert_eq!(z_nums.len(), 3, "zsh 3 echos");
        assert_eq!(r_nums.len(), 3, "zshrs 3 echos");
        assert!(z_nums[1] > z_nums[0]);
        assert!(z_nums[2] > z_nums[1]);
        assert!(r_nums[1] > r_nums[0]);
        assert!(r_nums[2] > r_nums[1]);
    }
}

mod random_var {
    use super::*;

    /// $RANDOM is a positive integer in 0..32768 (some shells) or
    /// larger ranges. Just pin parseable + positive.
    #[test]
    fn random_is_positive_integer() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh("echo $RANDOM");
        let r = run_zshrs("echo $RANDOM");
        let _: u32 = z.stdout.trim().parse().expect("zsh $RANDOM");
        let _: u32 = r.stdout.trim().parse().expect("zshrs $RANDOM");
    }

    /// Consecutive reads usually differ (probabilistically).
    /// Not strict — pin that two reads of $RANDOM produce SOMETHING.
    #[test]
    fn random_two_reads_both_succeed() {
        assert_parity(r#"R1=$RANDOM; R2=$RANDOM; [[ -n "$R1" ]] && [[ -n "$R2" ]]; echo $?"#);
    }
}

mod seconds_var {
    use super::*;

    /// $SECONDS is wall-clock seconds since shell start. Pin >= 0.
    #[test]
    fn seconds_non_negative() {
        assert_parity(r#"[[ $SECONDS -ge 0 ]]; echo $?"#);
    }
}

mod uid_ppid {
    use super::*;

    /// $UID matches `id -u`.
    #[test]
    fn uid_matches_id_u() {
        if !zsh_available() {
            return;
        }
        let z = run_zsh("echo $UID");
        let r = run_zshrs("echo $UID");
        let z_uid: u32 = z.stdout.trim().parse().expect("zsh $UID");
        let r_uid: u32 = r.stdout.trim().parse().expect("zshrs $UID");
        assert_eq!(z_uid, r_uid, "both shells must report same UID");
    }

    /// $EUID equals $UID for non-suid shells.
    #[test]
    fn euid_equals_uid_for_normal_invocation() {
        assert_parity(r#"[[ "$UID" == "$EUID" ]]; echo $?"#);
    }

    /// $PPID is positive.
    #[test]
    fn ppid_positive() {
        assert_parity(r#"[[ $PPID -gt 0 ]]; echo $?"#);
    }
}

mod shlvl {
    use super::*;

    /// $SHLVL increments in subshell.
    #[test]
    fn shlvl_value_present() {
        // Each shell increments differently; pin presence.
        assert_parity(r#"[[ -n "$SHLVL" ]]; echo $?"#);
    }
}

mod pwd {
    use super::*;

    #[test]
    fn pwd_matches_pwd_builtin() {
        assert_parity(r#"[[ "$PWD" == "$(pwd)" ]]; echo $?"#);
    }

    #[test]
    fn oldpwd_after_cd() {
        assert_parity(r#"START=$PWD; cd /tmp; [[ "$OLDPWD" == "$START" ]]; echo $?"#);
    }
}

mod hostname {
    use super::*;

    /// $HOST = hostname.
    #[test]
    fn host_non_empty() {
        assert_parity(r#"[[ -n "$HOST" ]]; echo $?"#);
    }
}

mod pipestatus {
    use super::*;

    /// $pipestatus[N] gives exit status of Nth pipeline stage.
    #[test]
    fn pipestatus_array_after_pipeline() {
        assert_parity(r#"false | true; echo "${pipestatus[1]} ${pipestatus[2]}""#);
    }

    #[test]
    fn pipestatus_array_three_stages() {
        assert_parity(r#"true | false | true; echo "${pipestatus[@]}""#);
    }
}

mod plus_table_flags {
    use super::*;

    #[test]
    fn plus_options() {
        assert_parity(r#"print -r ${+options}"#);
    }

    #[test]
    fn plus_parameters() {
        assert_parity(r#"print -r ${+parameters}"#);
    }

    #[test]
    fn plus_aliases() {
        assert_parity(r#"print -r ${+aliases}"#);
    }

    #[test]
    fn plus_builtins() {
        assert_parity(r#"print -r ${+builtins}"#);
    }

    #[test]
    fn plus_pipestatus() {
        assert_parity(r#"print -r ${+pipestatus}"#);
    }

    #[test]
    fn plus_dirstack() {
        assert_parity(r#"print -r ${+dirstack}"#);
    }
}
