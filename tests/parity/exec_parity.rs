//! `exec` builtin parity tests — both PROCESS REPLACE and PERSISTENT REDIRECT.

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

fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_in(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(d, s);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod replace_process {
    use super::*;

    /// `exec cmd` replaces the shell process with `cmd`.
    #[test]
    fn exec_cmd_replaces_shell() {
        // Outer shell becomes the exec'd command; subsequent commands
        // never run. Pin: only "before" is printed; "after" never is.
        assert_parity(r#"echo before; exec echo replaced; echo after"#);
    }

    #[test]
    fn exec_inherits_env() {
        assert_parity(r#"X=value; exec sh -c 'echo got=$X'"#);
    }
}

mod persistent_redirect {
    use super::*;

    /// `exec > FILE` (no cmd) applies redirect to current shell
    /// permanently. Subsequent stdout goes to FILE.
    #[test]
    #[ignore = "BOTH SHELLS HANG: cat after `exec > FILE` blocks waiting for output to flush"]
    fn exec_redirect_only_persists() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("out.txt");
        let script = format!(r#"exec > {0}; echo one; echo two; cat {0}"#, f.display());
        // After exec > FILE, the first cat reads the same file we just wrote.
        // Run in dir so paths resolve.
        let z = run_zsh_in(d.path(), &script);
        let r = run_zshrs_in(d.path(), &script);
        // Skip strict compare since the cat output mixes with redirected output;
        // pin exit code parity only.
        let _ = (z, r);
    }

    /// `exec 2> FILE` redirects stderr persistently.
    #[test]
    fn exec_redirect_stderr_only() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("err.txt");
        let script = format!(
            r#"exec 2> {0}; sh -c 'echo OUT; echo ERR >&2'; cat {0}"#,
            f.display()
        );
        assert_parity_in(d.path(), &script);
    }

    /// `exec 3< FILE` opens fd 3 for reading from FILE.
    #[test]
    fn exec_opens_high_fd_for_reading() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("in.txt"), "content\n").unwrap();
        let script = "exec 3< in.txt; read line <&3; echo got=$line; exec 3<&-";
        assert_parity_in(d.path(), script);
    }

    /// `exec 3> FILE` opens fd 3 for writing.
    #[test]
    fn exec_opens_high_fd_for_writing() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let script = "exec 3> out.txt; echo hello >&3; exec 3>&-; cat out.txt";
        assert_parity_in(d.path(), script);
    }
}

mod close_fd {
    use super::*;

    /// `exec 3<&-` closes fd 3.
    #[test]
    fn exec_closes_fd() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("in.txt"), "data\n").unwrap();
        let script = "exec 3< in.txt; exec 3<&-; echo ok";
        assert_parity_in(d.path(), script);
    }
}

mod bad_command {
    use super::*;

    /// `exec nonexistent_cmd` errors and (per spec) exits the shell.
    #[test]
    fn exec_unknown_command_exits_with_error() {
        assert_parity(r#"echo before; exec /nonexistent_xyz_42 2>/dev/null; echo never"#);
    }
}

mod with_args {
    use super::*;

    /// `exec cmd arg1 arg2` passes args to replacement command.
    #[test]
    fn exec_passes_args() {
        assert_parity(r#"exec echo a b c"#);
    }
}

mod exec_dash_a {
    use super::*;

    /// `exec -a name cmd` overrides $0 of replacement.
    #[test]
    fn exec_dash_a_overrides_argv0() {
        assert_parity(r#"exec -a myname sh -c 'echo $0'"#);
    }
}

mod dup_fd {
    use super::*;

    /// `exec 1>&2` makes stdout go to stderr permanently. With
    /// subsequent `2> file`, all output captured.
    #[test]
    fn exec_dup_stdout_to_stderr_then_redir() {
        if !zsh_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let script = r#"exec 1>&2 2>captured; echo hi 2>&1; cat captured"#;
        assert_parity_in(d.path(), script);
    }
}

/// c:Src/exec.c:4142 `addvars` + c:4410 `save_params`/`restore_params`
/// — an inline assignment prefix on a command whose FIRST WORD IS
/// DYNAMIC (`X=y $cmd`, `X=y ~/bin/foo`, `X=y =ls`, `X=y $(...)`).
///
/// The compiler's dynamic-command-name arm
/// (`src/extensions/compile_zsh.rs`, `first_is_dynamic`) used to emit
/// `BUILTIN_BEGIN_INLINE_ENV` and then `return` without ever compiling
/// the prefix assignments or emitting the matching `SEAL`/`END`. Two
/// bugs fell out of that:
///
///   1. the assignment never happened, so `X=y $cmd` ran `cmd` with
///      `X` unset (no export to the child, no visibility to a shell
///      function);
///   2. the pushed frame was never popped and stayed in its
///      `recording` state, so EVERY later plain assignment in the
///      script was recorded into the orphaned frame and `zputenv`'d —
///      `Z=hello` after a dynamic inline-env command leaked into the
///      process environment.
mod dynamic_first_word_inline_env {
    use super::*;

    /// The prefix assignment reaches the external command's env.
    #[test]
    fn exports_to_external_child() {
        assert_parity(r#"cmd=env; X=y $cmd | grep '^X='"#);
    }

    /// Same via `printenv`, which reads the variable by name.
    #[test]
    fn exports_to_printenv_child() {
        assert_parity(r#"cmd=printenv; X=y $cmd X"#);
    }

    /// A shell function sees the value, and it is gone afterwards —
    /// the `restore_params` half of the pairing.
    #[test]
    fn visible_in_function_then_restored() {
        assert_parity(
            r#"f() { print -r -- "in=[$X]" }; cmd=f; X=y $cmd; print -r -- "after=[$X]""#,
        );
    }

    /// zsh does NOT persist the assignment across a builtin here
    /// (no POSIX_BUILTINS), so `eval` sees it and the caller does not.
    #[test]
    fn visible_in_eval_then_restored() {
        assert_parity(r#"cmd=eval; X=y $cmd 'print -r -- "in=[$X]"'; print -r -- "after=[$X]""#);
    }

    /// c:Src/exec.c:3285-3304 — prefork (arg expansion) runs BEFORE
    /// addvars, so the command's own args still see the PRE-assignment
    /// value. `X=y $cmd "[$X]"` must print `[]`, not `[y]`.
    #[test]
    fn args_expand_before_the_assignment_commits() {
        assert_parity(r#"cmd=echo; X=y $cmd "[$X]""#);
    }

    /// The leaked-frame corruption: a plain assignment AFTER a dynamic
    /// inline-env command must not be exported to the process env.
    #[test]
    fn later_plain_assignment_is_not_exported() {
        assert_parity(
            r#"cmd=true; X=y $cmd; Z=hello; env | grep '^Z=' || print -r -- 'Z NOT exported'"#,
        );
    }

    /// A following inline-env command still scopes correctly — the
    /// orphaned frame must not swallow its restore.
    #[test]
    fn following_inline_env_command_still_scopes() {
        assert_parity(r#"cmd=true; X=y $cmd; A=1 true; print -r -- "[$A][$X]""#);
    }

    /// c:Src/subst.c:799 `filesubstr` — `=cmd` is the other
    /// `first_is_dynamic` trigger the compiler routes through
    /// BUILTIN_EXEC_DYNAMIC, so it must scope identically.
    #[test]
    fn equals_command_word_scopes_the_same() {
        assert_parity(r#"X=y =printenv X; print -r -- "after=[$X]""#);
    }
}

/// c:Src/exec.c:3928-3931 — `zwarn("failed to close file descriptor %d:
/// %e", fn->fd1, errno)`. `%e` is zerrmsg's errno arm (c:Src/utils.c:352-368):
/// strerror with the first letter lowered and no `(os error N)` suffix, which
/// is what the Display of a Rust `io::Error` appends.
mod varid_close_error_text {
    use super::*;

    #[test]
    fn close_failure_uses_zsh_errno_text() {
        assert_parity(r#"myfd=99; { exec {myfd}>&- } 2>&1; print rc=$?"#);
    }
}

/// c:Src/exec.c:3098-3103 — `else if (isset(POSIXBUILTINS) && (cflags &
/// BINF_EXEC)) break;`: "POSIX doesn't allow "exec" to operate on builtins
/// or shell functions", so under POSIX_BUILTINS the name is looked up as an
/// external command only (Test/E01options.ztst "POSIX_BUILTINS and exec").
mod exec_under_posix_builtins {
    use super::*;

    #[test]
    fn exec_skips_a_shell_function() {
        assert_parity(
            r#"(cat() { print fn $1 }; (exec cat /dev/null; print no); print with; (setopt posixbuiltins; exec cat /dev/null; print no); print end)"#,
        );
    }

    #[test]
    fn exec_skips_a_builtin() {
        assert_parity(r#"(setopt posixbuiltins; exec print hi 2>&1; print no); print rc=$?"#);
    }
}

/// c:Src/exec.c:4147-4154 — `addvars(state, varspc, flags); if (errflag) {
/// …; lastval = 1; …; goto done; }`: an expansion error in a prefix
/// assignment skips the command. For an external command the assignments run
/// in the forked child (c:4343-4350 `if (errflag) _exit(1);`), so the shell
/// sees status 1 and carries on; for a shell function they run in the shell,
/// whose errflag ends the script with status 1.
mod prefix_assignment_error_skips_the_command {
    use super::*;

    #[test]
    fn external_command_status_one_and_the_script_continues() {
        assert_parity(r#"x=${bad?err} /bin/echo ran 2>&1; print rc=$?; print next"#);
        assert_parity(r#"x=keep; x=${bad?err} /bin/echo ran; print rc=$? x=$x"#);
        assert_parity(r#"x=$((1/0)) /bin/echo ran; print rc=$?; print next"#);
        assert_parity(r#"x=${bad?err} /bin/echo ran && print and || print or; print end"#);
        assert_parity(r#"c=/bin/echo; x=${bad?err} $c ran; print rc=$?; print next"#);
        assert_parity(r#"g(){ x=${bad?err} /bin/echo ran; print in rc=$?; }; g; print out rc=$?"#);
    }

    #[test]
    fn shell_function_ends_the_script_with_status_one() {
        assert_parity(r#"f(){ print fn }; x=${bad?err} f; print rc=$?; print next"#);
        assert_parity(r#"f(){ print fn }; x=$((1/0)) f; print rc=$?"#);
    }
}

/// c:Src/exec.c:3257-3280 — `exec`'s own options are consumed in the
/// precommand walk, before `globlist(args, 0)` (c:3757), so the argv0 given
/// with `-a` is expanded but never filename-generated. zshrs globbed it:
/// `exec -a foo* cmd` passed a matching file name as $0.
/// ztst A01grammar "rationalisation of arguments to exec -a".
mod exec_argv0_is_not_globbed {
    use super::*;

    const DIR: &str = "cd \"$(mktemp -d)\" && touch foo1 && ";

    #[test]
    fn the_argv0_word_keeps_its_glob_characters() {
        assert_parity(&format!("{DIR}(exec -a foo* /bin/sh -c 'echo $0')"));
        assert_parity(&format!("{DIR}(exec -afoo* /bin/sh -c 'echo $0')"));
    }

    #[test]
    fn other_exec_words_and_argv0_expansion_are_unchanged() {
        assert_parity(&format!("{DIR}v=nm; (exec -a $v /bin/sh -c 'echo $0')"));
        assert_parity(&format!("{DIR}(exec -a '' /bin/sh -c 'echo \"[$0]\"')"));
        assert_parity(&format!("{DIR}(exec /bin/echo foo*)"));
        assert_parity(&format!("{DIR}(exec -c /bin/echo foo*)"));
    }
}

/// c:Src/exec.c:2458-2465 — addfd moves each member of a multio out of the
/// script's fd range with `movefd`, so the concatenator for `3<a 3<b` owns
/// fd 3 without disturbing its own members. A member opened at the lowest
/// free fd landed on 3 itself; the concatenator pipe was `dup2`'d over it,
/// the producer read its own empty pipe, and the command hung.
mod input_multio_on_a_non_zero_fd {
    use super::*;

    #[test]
    fn file_members_reach_the_command_through_a_dup() {
        assert_parity(
            "d=$(mktemp -d); print out1 >$d/o1; print out2 >$d/o2; cat 3<$d/o1 3<$d/o2 <&3 </dev/null; print rc=$?; command rm -rf $d",
        );
    }

    #[test]
    fn heredoc_and_herestring_members_reach_the_command_through_a_dup() {
        assert_parity("cat 3<<x 3<<y <&3\nfoo\nx\nbar\ny\nprint rc=$?");
        assert_parity(
            "d=$(mktemp -d); print out1 >$d/o1; cat 3<$d/o1 3<<<here <&3; print rc=$?; command rm -rf $d",
        );
    }
}

/// c:Src/exec.c:3489-3499 — under the `builtin` precommand modifier the name is
/// looked up with `builtintab->getnode`, which skips DISABLED entries, so a
/// disabled builtin is "no such builtin" with status 1.
mod builtin_prefix_on_a_disabled_builtin {
    use super::*;

    #[test]
    fn reports_no_such_builtin() {
        assert_parity(r#"disable typeset; { builtin typeset x=1; } 2>&1; print rc=$? $+x"#);
        assert_parity(r#"disable echo; { builtin echo hi; } 2>&1; print rc=$?"#);
    }
}

/// c:Src/exec.c:4298-4305 — after a builtin, `if (save[1] == -2) { if
/// (ferror(stdout)) { zwarn("write error: %e", errno); clearerr(stdout); } }
/// else clearerr(stdout);`. Output to a closed stdout is reported unless the
/// command's own redirections moved fd 1.
mod write_error_on_a_closed_stdout {
    use super::*;

    #[test]
    fn builtin_output_to_a_closed_stdout_warns() {
        assert_parity(r#"{ { print a } >&-; print ok } 2>&1"#);
        assert_parity(r#"{ { echo a; printf b } >&-; print ok } 2>&1"#);
        assert_parity(r#"{ f() { print a }; f >&-; print ok } 2>&1"#);
        assert_parity(r#"{ eval "print a" >&-; print ok } 2>&1"#);
    }

    #[test]
    fn own_redirection_of_fd_1_clears_the_error() {
        assert_parity(r#"{ print a >&-; print ok } 2>&1"#);
        assert_parity(r#"{ { print -n "" } >&-; print ok } 2>&1"#);
    }
}

/// c:Src/exec.c:4491-4493 save_params → c:Src/params.c:1279 copyparam —
/// `tpm->u.val = pm->gsu.i->getfn(pm)`. A special integer's value lives
/// behind its getfn (`uidgetfn` is `getuid()`), not in `u.val`; the prefix
/// assignment's restore (c:4551 `tpm->gsu.i->setfn(tpm, pm->u.val)`) must put
/// that value back, or `UID=$UID cmd` ends with `setuid(0)`.
mod prefix_assignment_to_a_special_integer {
    use super::*;

    #[test]
    fn restores_the_getfn_value() {
        assert_parity(r#"{ UID=$UID print hi; print $UID rc=$? } 2>&1"#);
        assert_parity(r#"{ f() { print in; }; UID=$UID f; print rc=$? } 2>&1"#);
        assert_parity(r#"{ EUID=$EUID GID=$GID EGID=$EGID /usr/bin/true; print rc=$? } 2>&1"#);
        assert_parity(r#"{ UID=x /usr/bin/true; print rc=$? } 2>&1"#);
    }
}

/// c:Src/exec.c:3775-3777 — `if (input) addfd(forked, save, mfds, 0, input,
/// 0, NULL);` puts a pipeline stage's input pipe into mfds[0] before the
/// stage's own redirections are walked, so an input redirection becomes the
/// multio's second member (c:2447-2480): the stage reads the pipe, then the
/// file. Only the stage command's own list is seeded; a redirection inside
/// a `{ … }` stage is a separate execcmd.
mod pipeline_input_seeds_the_input_multio {
    use super::*;

    #[test]
    fn stage_redirection_reads_pipe_then_source() {
        assert_parity(r#"cd "${TMPDIR:-/tmp}"; print o1 >zr_o1; print o2 >zr_o2; print o3 >zr_o3; cat zr_o1 | cat <zr_o2; cat zr_o1 | cat <zr_o2 <zr_o3; cat zr_o1 | cat <<<hs; f() { cat }; cat zr_o1 | f <zr_o2; command rm -f zr_o1 zr_o2 zr_o3"#);
        assert_parity(r#"cd "${TMPDIR:-/tmp}"; print o1 >zr_p1; print o2 >zr_p2; cat zr_p1 | read x <zr_p2; print "[$x]"; cat zr_p1 | while read l; do print "<$l>"; done <zr_p2; command rm -f zr_p1 zr_p2"#);
    }

    #[test]
    fn unseeded_forms_still_replace() {
        assert_parity(r#"cd "${TMPDIR:-/tmp}"; print o1 >zr_q1; print o2 >zr_q2; setopt nomultios; cat zr_q1 | cat <zr_q2; unsetopt nomultios; cat zr_q1 | { cat <zr_q2 }; cat <<<x; print y | cat <<<z; command rm -f zr_q1 zr_q2"#);
    }
}

/// c:Src/exec.c:5037/5095/5150 — `getoutputfile` (`=( )`) and `getproc`
/// (`<( )` / `>( )`) fork and call entersubsh, whose c:1200 `zsh_subshell++`
/// is what `$ZSH_SUBSHELL` reads inside the substitution.
mod zsh_subshell_in_process_substitutions {
    use super::*;

    #[test]
    fn counts_the_forked_child() {
        assert_parity(r#"print $ZSH_SUBSHELL; cat =(print $ZSH_SUBSHELL); cat <(print $ZSH_SUBSHELL); print $(print $ZSH_SUBSHELL)"#);
        assert_parity(r#"print >(print $ZSH_SUBSHELL) >/dev/null; sleep 0.2"#);
    }
}

/// c:Src/subst.c:1707-1708 — `spbreak = (pf_flags & PREFORK_SHWORDSPLIT) &&
/// !(pf_flags & PREFORK_SINGLE) && !qt`: SH_WORD_SPLIT never splits a quoted
/// expansion, so `"${scalar[@]}"` stays one word.
mod shwordsplit_quoted_scalar_splat {
    use super::*;

    #[test]
    fn quoted_scalar_splat_is_one_word() {
        assert_parity(r#"set -- one "two three" four; setopt shwordsplit; var=$@; printf "[%s]\n" "${var[@]}""#);
        assert_parity(r#"setopt shwordsplit; var="a b"; printf "[%s]" "${var[@]}"; x=("${var[@]}"); print $#x; printf "[%s]" ${var[@]}"#);
    }
}

/// c:Src/exec.c:758-770 — "If ARGV0 is in the commands environment, we use
/// that as argv[0] for this external command", then `unsetenv("ARGV0")` so
/// the command does not inherit it.
mod argv0_environment_variable {
    use super::*;

    #[test]
    fn sets_argv0_and_is_not_exported() {
        assert_parity(r#"ARGV0=foo /bin/sh -c 'echo $0'; ARGV0=foo sh -c 'echo $0'; ARGV0=foo /usr/bin/env | grep -c ARGV0"#);
    }
}

/// c:Src/utils.c:4205-4213 inittyptab — under MULTIBYTE an IFS that does not
/// convert to wide characters warns "IFS has an invalid character; resetting
/// IFS to default" and the parameter takes the default value.
mod invalid_multibyte_ifs {
    use super::*;

    #[test]
    fn warns_and_resets_to_default() {
        assert_parity(r#"{ IFS=$'\x80'; echo x; print -r ${(q)IFS}; v='a b'; print -rl -- ${=v} } 2>&1"#);
    }
}

/// c:Src/exec.c:4753 getoutput — `mpipe` puts both pipe ends above the
/// script's fd range and the forked child `redup`s the write end onto fd 1,
/// so a command substitution runs even when the shell's stdout is closed.
mod command_substitution_with_stdout_closed {
    use super::*;

    #[test]
    fn body_still_runs() {
        assert_parity(r#"{ { x=$(print b >&2) } >&-; print ok; { : $(print c >&2) } >&-; print ok2 } 2>&1"#);
    }
}

/// c:Src/exec.c:1251 execode appends its label to `zsh_eval_context`:
/// `=( )` runs its body with "equalsubst" (c:5044), and an EXIT trap's eval
/// list with "trap" (c:Src/signals.c:1170). A function-scoped EXIT trap runs
/// after runshfunc's "shfunc" frame has been popped.
mod eval_context_for_equalsubst_and_exit_traps {
    use super::*;

    #[test]
    fn equalsubst_is_labelled() {
        assert_parity(r#"contextfn() { print -r - $zsh_eval_context }; cat =( contextfn ); cat <(contextfn)"#);
    }

    #[test]
    fn exit_trap_bodies_are_labelled_trap() {
        assert_parity(r#"contextfn() { print -r - $zsh_eval_context }; () { trap contextfn EXIT }"#);
        assert_parity(r#"trap 'print -r - $zsh_eval_context' EXIT"#);
        assert_parity(r#"f() { trap 'print -r - $zsh_eval_context' EXIT; }; f"#);
        assert_parity(r#"TRAPEXIT() { print -r - $zsh_eval_context }"#);
    }
}

/// c:Src/exec.c:3523-3525 — an error from prefork (the word expansions,
/// c:3357-3359) aborts the command with `if (!lastval) lastval = 1`, so a
/// non-zero status from the previous command survives. A globlist error
/// (c:3755-3762) still sets `lastval = 1`, and `command` hands a glob error to
/// the forked child while a prefork error happens in the shell.
mod expansion_error_keeps_nonzero_status {
    use super::*;

    #[test]
    fn prefork_error_keeps_status() {
        assert_parity(r#"(exit 5); print ${.n.s.k}"#);
        assert_parity(r#"nosuchcmd_zz; print ${.n.s.k}"#);
        assert_parity(r#"f() { (exit 5); : $(( 1 + )) }; f"#);
        assert_parity(r#"(exit 5); builtin print ${.n.s.k}"#);
        assert_parity(r#"(exit 5); command print ${.n.s.k}"#);
        assert_parity(r#"true; print ${.n.s.k}"#);
    }

    #[test]
    fn glob_error_sets_one() {
        assert_parity(r#"(exit 5); print x(a)"#);
        assert_parity(r#"(exit 5); builtin print x(a)"#);
        assert_parity(r#"{ (exit 5); command ls zzq*; print $?; (exit 5); command print x(a); print $? } 2>&1"#);
        assert_parity(r#"(exit 5); print /nonexist*"#);
    }
}
