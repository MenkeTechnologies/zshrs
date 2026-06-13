//! Pipeline parity tests — |, |&, $?, $pipestatus, && / ||, &.

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

mod single_pipe {
    use super::*;

    #[test]
    fn basic_pipe_data_flow() {
        assert_parity("echo hello | cat");
    }

    #[test]
    fn pipe_through_grep() {
        assert_parity("echo -e 'a\\nb\\nc' | grep b");
    }

    #[test]
    fn pipe_through_wc_l() {
        assert_parity("echo -e 'a\\nb\\nc' | wc -l");
    }

    #[test]
    fn pipe_with_no_output() {
        assert_parity("true | true; echo done");
    }
}

mod multi_stage {
    use super::*;

    #[test]
    fn three_stage_pipeline() {
        assert_parity("echo -e 'c\\na\\nb' | sort | head -1");
    }

    #[test]
    fn four_stage_pipeline() {
        assert_parity("echo -e 'd\\na\\nc\\nb' | sort | tail -1 | tr a-z A-Z");
    }

    #[test]
    fn five_stage_pipeline() {
        assert_parity("echo hello | cat | cat | cat | wc -c");
    }
}

mod exit_status {
    use super::*;

    /// `$?` after pipeline = exit status of LAST command.
    #[test]
    fn dollar_q_is_last_stage() {
        assert_parity("false | true; echo $?");
    }

    #[test]
    fn dollar_q_is_last_stage_when_last_fails() {
        assert_parity("true | false; echo $?");
    }

    #[test]
    fn three_stage_last_fails() {
        assert_parity("true | true | false; echo $?");
    }

    #[test]
    fn three_stage_middle_fails_doesnt_affect_dollar_q() {
        assert_parity("true | false | true; echo $?");
    }
}

mod pipestatus_array {
    use super::*;

    /// `$pipestatus[N]` (zsh) gives Nth stage's exit status.
    #[test]
    fn pipestatus_two_stage() {
        assert_parity("false | true; echo \"${pipestatus[1]} ${pipestatus[2]}\"");
    }

    #[test]
    fn pipestatus_three_stage() {
        assert_parity("true | false | true; echo \"${pipestatus[@]}\"");
    }

    #[test]
    fn pipestatus_all_zero() {
        assert_parity("true | true | true; echo \"${pipestatus[@]}\"");
    }
}

mod pipe_amp {
    use super::*;

    /// `|&` merges stderr from prev stage into the pipe.
    #[test]
    fn pipe_amp_merges_stderr() {
        assert_parity(r#"sh -c 'echo OUT; echo ERR >&2' |& cat"#);
    }

    #[test]
    fn pipe_amp_three_stage() {
        assert_parity(r#"sh -c 'echo a; echo b >&2' |& tr a-z A-Z |& cat"#);
    }
}

mod logical_chains {
    use super::*;

    #[test]
    fn and_both_run() {
        assert_parity("true && echo yes");
    }

    #[test]
    fn and_short_circuit() {
        assert_parity("false && echo never; echo done");
    }

    #[test]
    fn or_short_circuit() {
        assert_parity("true || echo never; echo done");
    }

    #[test]
    fn or_fallback_runs() {
        assert_parity("false || echo fallback");
    }

    /// `cmd1 && cmd2 || cmd3` precedence: left-assoc.
    #[test]
    fn and_or_chain_left_assoc_first_succeeds() {
        assert_parity("true && echo got || echo failed");
    }

    #[test]
    fn and_or_chain_first_fails_fallback() {
        assert_parity("false && echo never || echo fallback");
    }

    #[test]
    fn and_or_chain_first_succeeds_second_fails() {
        assert_parity("true && false || echo fallback");
    }
}

mod combined_pipe_and {
    use super::*;

    /// `cmd | cmd2 && cmd3` — pipeline then AND.
    #[test]
    fn pipeline_then_and() {
        assert_parity("echo hi | grep hi && echo found");
    }

    #[test]
    fn pipeline_then_or() {
        assert_parity("echo hi | grep nope || echo not-found");
    }
}

mod background {
    use super::*;

    /// `cmd &` runs in background. wait, then check $?.
    /// Don't compare $! (PIDs differ); pin: command DID run, wait succeeds.
    #[test]
    fn background_then_wait() {
        assert_parity("sleep 0.05 &; wait; echo done");
    }

    #[test]
    fn background_returns_zero() {
        assert_parity("true &; wait; echo $?");
    }
}

mod redirect_in_pipe {
    use super::*;

    /// Redirect on a single pipe stage.
    #[test]
    fn redirect_stage_stdout_to_dev_null() {
        assert_parity("echo hi 2>/dev/null | cat");
    }

    #[test]
    fn redirect_input_to_first_stage() {
        assert_parity("cat <<< hello | tr h H");
    }
}
