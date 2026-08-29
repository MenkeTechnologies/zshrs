//! Parity for the two kinds of state that used to leak out of an
//! in-process `( … )`: signal dispositions / the trap table, and
//! resource limits.
//!
//! zshrs runs `( … )` in-process where zsh forks
//! (`Src/exec.c:2922` `entersubsh(flags, &esret)`). The isolation the
//! fork provides has to be reproduced by hand at the subshell
//! boundary. Everything below is measured against `zsh -f`, so a
//! regression in either direction fails.
//!
//! The signal cases matter because `$$` names the PARENT shell even
//! inside the subshell, so `kill -USR1 $$` is answered by the parent's
//! disposition — the one `entersubsh` never touched — and runs at the
//! point `zwaitjob` drains its trap queue (`Src/jobs.c:1688`
//! `queue_traps`, `Src/jobs.c:1694` `unqueue_traps`), i.e. after the
//! body has finished.

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

/// Both stdout and exit status must match. Exit status is load-bearing
/// here: several of these cases differ ONLY in whether the shell
/// survived the signal.
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
    assert_eq!(
        z.exit, r.exit,
        "exit-status divergence on:\n{s}\n zsh={} zshrs={}",
        z.exit, r.exit
    );
}

/// The four kinds of subshell isolation that already worked before the
/// signal/limit fix. They are here as the negative control: a
/// save/restore that is too broad breaks one of these.
mod already_isolated_control {
    use super::*;

    #[test]
    fn variable_assignment_does_not_leak() {
        assert_parity("v=out; ( v=in ); echo $v");
    }

    #[test]
    fn cd_does_not_leak() {
        assert_parity("cd /; ( cd /tmp ); pwd");
    }

    #[test]
    fn exit_status_propagates() {
        assert_parity("( exit 3 ); echo rc=$?");
    }

    #[test]
    fn option_change_does_not_leak() {
        assert_parity("setopt noglob; ( unsetopt noglob ); [[ -o noglob ]] && echo set || echo unset");
    }
}

mod trap_scope {
    use super::*;

    /// The headline case. `$$` is the parent, so zsh's OUTER trap runs;
    /// the subshell's INNER trap is on the child's private table and
    /// never sees a delivery. zshrs used to run INNER.
    #[test]
    fn kill_dollar_dollar_runs_outer_trap_not_inner() {
        assert_parity("trap 'echo OUT' USR1; ( trap 'echo IN' USR1; kill -USR1 $$ ); sleep 0.2");
    }

    /// Same delivery with no inner trap at all. zshrs used to drop the
    /// signal entirely: `entersubsh`'s trap reset had cleared the table
    /// the handler consults, and nothing replayed it afterwards.
    #[test]
    fn kill_dollar_dollar_runs_outer_trap_with_no_inner_trap() {
        assert_parity("trap 'echo OUT' USR1; ( kill -USR1 $$ ); sleep 0.2");
    }

    /// The trap runs AFTER the subshell body, where C's parent reaches
    /// it: `unqueue_traps()` at the end of `zwaitjob`
    /// (`Src/jobs.c:1694`). Ordering, not just occurrence.
    #[test]
    fn outer_trap_runs_after_the_subshell_body_finishes() {
        assert_parity("trap 'echo OUT' USR1; ( kill -USR1 $$; echo insub ); echo end");
    }

    /// `trap '' USR1` in the parent means the delivery is dropped.
    #[test]
    fn outer_ignored_trap_swallows_the_signal() {
        assert_parity("trap '' USR1; ( kill -USR1 $$ ); sleep 0.2; echo end");
    }

    /// A subshell EXIT trap fires at the end of the subshell.
    #[test]
    fn inner_exit_trap_fires_at_subshell_end() {
        assert_parity("( trap 'echo IN' EXIT; echo body )");
    }

    /// …and the parent's EXIT trap must NOT fire there.
    #[test]
    fn outer_exit_trap_does_not_fire_at_subshell_end() {
        assert_parity("trap 'echo OUT' EXIT; ( echo body ); echo after");
    }

    /// `entersubsh` clears the parent's traps in the child
    /// (`Src/exec.c:1127-1131`), so the body lists nothing.
    #[test]
    fn subshell_does_not_inherit_the_trap_listing() {
        assert_parity("trap 'echo OUT' USR1; ( trap ); echo end");
    }

    /// A trap set inside the subshell must not leak out.
    #[test]
    fn inner_trap_does_not_leak_to_parent_listing() {
        assert_parity("trap - USR1; ( trap 'echo IN' USR1 ); trap; echo done");
    }

    /// …and neither must the `sigaction` disposition it installed.
    /// `settrap` → `install_handler` (`Src/signals.c:730`) runs in the
    /// CHILD in zsh, so the parent keeps SIG_DFL and the later
    /// `kill -USR1 $$` kills the shell. zshrs's in-process
    /// `install_handler` used to leak and swallow the signal.
    #[test]
    fn inner_trap_does_not_leak_its_signal_disposition() {
        assert_parity("( trap 'echo IN' USR1 ); kill -USR1 $$; sleep 0.2; echo end");
    }

    /// Same leak via the ignore path (`Src/signals.c:715-722`).
    #[test]
    fn inner_ignored_trap_does_not_leak_sig_ign() {
        assert_parity("( trap '' USR1 ); kill -USR1 $$; sleep 0.2; echo end");
    }

    /// A trap set and removed by the parent before the subshell stays
    /// removed, and the subshell's own handling does not resurrect it.
    #[test]
    fn outer_trap_removed_before_subshell_stays_removed() {
        assert_parity("trap 'echo OUT' USR1; trap - USR1; ( echo body ); trap; echo done");
    }
}

mod rlimit_scope {
    use super::*;

    /// The headline case: `ulimit` inside `( … )` applies to the child
    /// only. zsh's `limits[]` is fork-copied (`Src/exec.c:315`) and the
    /// child applies its copy via `setlimits(NULL)`
    /// (`Src/exec.c:381-383`).
    #[test]
    fn ulimit_in_subshell_does_not_leak() {
        assert_parity("( ulimit -n 256 ); ulimit -n");
    }

    /// The body still sees its own limit while it runs.
    #[test]
    fn subshell_sees_its_own_limit_then_parent_is_restored() {
        assert_parity("( ulimit -n 256; ulimit -n ); ulimit -n");
    }

    /// A nested subshell restores to the value the OUTER subshell set,
    /// not to the top-level one.
    #[test]
    fn nested_subshell_limits_unwind_one_level_at_a_time() {
        assert_parity("( ulimit -n 512; ( ulimit -n 256 ); ulimit -n ); ulimit -n");
    }

    /// The real process limit is restored, not just the shell's view of
    /// it: after the subshell the parent can still raise it back up to
    /// a value the subshell's soft limit would have forbidden.
    #[test]
    fn process_soft_limit_is_really_restored() {
        assert_parity("( ulimit -n 64 ); ulimit -n 900; ulimit -n");
    }

    /// `limit` (the zsh-native spelling) has the same fork semantics.
    #[test]
    fn limit_builtin_in_subshell_does_not_leak() {
        assert_parity("( limit -s descriptors 256 ); limit descriptors");
    }
}
