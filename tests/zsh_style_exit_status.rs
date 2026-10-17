//! A zsh-STYLE leg (`zshrs --ksh --zsh -c SCRIPT`) stands in for
//! `zsh -c 'emulate ksh -c "$SCRIPT"'`. `emulate` is a builtin, so a script that
//! leaves an error pending exits 1 there even when its own last status is 0.

use std::process::{Command, Stdio};

fn status(mut cmd: Command) -> Option<i32> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?.code()
}

fn zshrs_leg(mode: &str, script: &str) -> Option<i32> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zshrs"));
    cmd.args([mode, "--zsh", "-c", script]);
    status(cmd)
}

fn oracle(mode: &str, script: &str) -> Option<i32> {
    let mut cmd = Command::new("zsh");
    cmd.args(["-f", "-c", "__s=$1; set --; emulate \"$2\" -c \"$__s\"", "zsh", script, mode.trim_start_matches("--")]);
    status(cmd)
}

#[test]
fn pending_error_makes_the_leg_exit_one() {
    for mode in ["--ksh", "--sh"] {
        for script in [
            "setopt NO_UNSET; [[ -z ${arr[99]} ]]; print -r -- $?",
            "setopt NO_UNSET; case ${arr[1]} in (x[0-9]) print X;; (*) print O;; esac",
        ] {
            assert_eq!(zshrs_leg(mode, script), Some(1), "{mode}: {script}");
            if let Some(expected) = oracle(mode, script) {
                assert_eq!(expected, 1, "zsh oracle disagrees with the pinned status: {script}");
            }
        }
    }
}

#[test]
fn clean_scripts_keep_their_own_status() {
    for (script, want) in [("true", 0), ("false", 1), ("(exit 3)", 3)] {
        assert_eq!(zshrs_leg("--ksh", script), Some(want), "{script}");
    }
}
