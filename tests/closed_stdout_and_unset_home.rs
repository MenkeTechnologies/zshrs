//! Two zsh behaviours the upstream ztst suite pins (A04redirect `'>&-' with
//! attempt to use closed fd`, B01cd `Implicit cd with unset HOME`).

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn zshrs(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zshrs"));
    cmd.args(args);
    cmd
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A builtin that writes to a closed fd 1 reports `write error: %e` and keeps its
/// own status (c:Src/exec.c:4298-4305). `print` and `echo` used to lose the error
/// indicator before the report because the drop-in status check consumed it.
#[test]
fn builtin_output_to_a_closed_stdout_reports_a_write_error() {
    for body in ["print foo", "echo foo", "printf foo"] {
        let o = zshrs(&["-fc", &format!("{{ {body}; }} >&-; echo rc=$? >&2")]).output().unwrap();
        let err = stderr(&o);
        assert!(err.contains("write error: bad file descriptor"), "{body}: {err}");
        assert!(err.contains("rc=0"), "{body} keeps status 0 in zsh mode: {err}");
    }
    let o = zshrs(&["-fc", "(exec >&-; print foo)"]).output().unwrap();
    assert!(stderr(&o).contains("write error: bad file descriptor"));
}

/// A shell started without HOME in an sh or ksh emulation has no home directory at
/// all, so a bare `cd` fails (c:Src/builtin.c:881); zsh itself takes the passwd
/// entry, and a later `unset HOME` leaves an empty one that `cd` ignores.
#[test]
fn bare_cd_without_home_fails_only_in_the_drop_in_emulations() {
    for name in ["sh", "ksh"] {
        let o = zshrs(&["-c", "cd"]).env_remove("HOME").arg0(name).output().unwrap();
        assert_eq!(o.status.code(), Some(1), "{name}");
        assert!(stderr(&o).contains("cd:1: HOME not set"), "{name}: {}", stderr(&o));
    }
    let o = zshrs(&["-fc", "cd"]).env_remove("HOME").output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let o = zshrs(&["-fc", "unset HOME; cd"]).env_remove("HOME").output().unwrap();
    assert_eq!(o.status.code(), Some(0));
}
