//! export / unset / readonly parity tests.

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
    crate::oracle::zsh_path()
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

mod export_basic {
    use super::*;

    /// Exported var visible in subprocess.
    #[test]
    fn exported_var_visible_in_child() {
        assert_parity(r#"export X=visible; sh -c 'echo $X'"#);
    }

    /// Non-exported var invisible to child.
    #[test]
    fn non_exported_var_invisible_in_child() {
        assert_parity(r#"X=hidden; sh -c 'echo "[$X]"'"#);
    }

    /// `export NAME` (no value) exports existing var.
    #[test]
    fn export_existing_var_no_value() {
        assert_parity(r#"X=value; export X; sh -c 'echo $X'"#);
    }

    /// `export` then reassign — child still sees new value.
    #[test]
    fn export_then_reassign() {
        assert_parity(r#"export X=first; X=second; sh -c 'echo $X'"#);
    }

    /// Multiple exports in one line.
    #[test]
    fn export_multiple_at_once() {
        assert_parity(r#"export A=1 B=2 C=3; sh -c 'echo $A $B $C'"#);
    }
}

mod export_listing {
    use super::*;

    /// `export` (no args) lists exported vars. Skip strict compare;
    /// pin exit code.
    #[test]
    fn export_no_args_exits_zero() {
        assert_parity("export >/dev/null; echo $?");
    }
}

mod export_unexport {
    use super::*;

    /// `export -n NAME` removes export attribute (keeps var).
    /// After `-n`, child no longer sees the var.
    #[test]
    fn export_dash_n_unexports() {
        assert_parity(r#"export X=visible; export -n X; sh -c 'echo "[$X]"'; echo "self:$X""#);
    }
}

mod unset_basic {
    use super::*;

    #[test]
    fn unset_removes_var() {
        assert_parity(r#"X=value; unset X; echo "[${X:-empty}]""#);
    }

    #[test]
    fn unset_unset_var_noop() {
        assert_parity(r#"unset NEVER_SET; echo $?"#);
    }

    /// `unset` of multiple at once.
    #[test]
    fn unset_multiple_at_once() {
        assert_parity(r#"A=1; B=2; C=3; unset A B C; echo "[$A][$B][$C]""#);
    }

    /// `unset -f` removes function (separate namespace).
    #[test]
    fn unset_f_removes_function_separately() {
        assert_parity(r#"f() { echo hi; }; unset -f f; f 2>/dev/null; echo done"#);
    }

    /// `unset -v NAME` is the var-only form.
    #[test]
    fn unset_v_var_only() {
        assert_parity(r#"X=val; unset -v X; echo "[${X:-cleared}]""#);
    }
}

mod readonly_basic {
    use super::*;

    #[test]
    fn readonly_set_succeeds() {
        assert_parity(r#"readonly X=value; echo $X"#);
    }

    /// Reassigning a readonly var errors (and may exit on some shells).
    #[test]
    fn readonly_reassign_errors() {
        assert_parity(r#"readonly X=value; X=new 2>/dev/null; echo "[$X]"; echo "exit=$?""#);
    }

    /// `unset` of readonly var errors.
    #[test]
    fn unset_readonly_errors() {
        assert_parity(r#"readonly X=value; unset X 2>/dev/null; echo "[$X]"; echo "exit=$?""#);
    }
}

mod typeset_x_equivalence {
    use super::*;

    /// `typeset -x` is alias for `export`.
    #[test]
    fn typeset_x_exports() {
        assert_parity(r#"typeset -x X=value; sh -c 'echo $X'"#);
    }

    /// `declare -x` (bash-compat) — same.
    #[test]
    fn declare_x_exports() {
        assert_parity(r#"declare -x X=value; sh -c 'echo $X'"#);
    }
}

mod export_with_unset {
    use super::*;

    /// Unsetting exported var removes from env too.
    #[test]
    fn unset_exported_var_clears_env() {
        assert_parity(r#"export X=value; unset X; sh -c 'echo "[$X]"'"#);
    }
}

mod array_export_zsh {
    use super::*;

    /// zsh refuses to export non-PATH-style array vars: `typeset -gx arr`
    /// is accepted but the array doesn't appear in the env passed to `sh`.
    /// Pin: both shells emit `[]` because `sh` sees an unset `arr`.
    /// Previously marked divergent; regression-pinned now that zshrs
    /// agrees.
    #[test]
    fn export_array_joins_with_colon() {
        assert_parity(r#"arr=(a b c); typeset -gx arr; sh -c 'echo "[$arr]"'"#);
    }
}

mod export_in_pipeline {
    use super::*;

    /// `export X=val | cmd` — `cmd` doesn't see X (each pipeline stage
    /// is a subshell in zsh by default).
    #[test]
    fn export_in_pipeline_doesnt_leak_to_next_stage() {
        // The export happens in left stage; cat on right sees stdin only.
        assert_parity(r#"export X=val | cat; echo "outer:$X""#);
    }
}

/// Exporting a PM_SPECIAL scalar whose value legitimately contains a NUL.
///
/// C's `export_param` (`Src/params.c:2670`) ends in
/// `zputenv` → `setenv(name, value, 1)`, and `setenv` reads the value as a
/// NUL-TERMINATED C string: an embedded NUL simply ends it. `env::set_var`
/// PANICS on one instead, and the panic was reachable from a bare
/// `export IFS`, because zsh's stock `$IFS` is `" \t\n\0"` — the NUL is
/// the DEFAULT value, not an exotic one.
///
/// It stayed hidden while `export_param` read every scalar through
/// `strgetfn`: a PM_SPECIAL keeps its value in a process global reached
/// through its GSU, so `u.str` was empty and the export wrote nothing.
/// Routing specials through their own getfn (which fixed `$TERM`
/// disappearing from child environments) made the real value — NUL and
/// all — reach the environment writer for the first time.
mod export_special_with_nul {
    use super::*;

    /// The crash case. `export IFS` must not abort the shell, and the
    /// child must see the value truncated at the NUL exactly as C's
    /// `setenv` truncates it.
    #[test]
    fn exporting_ifs_truncates_at_the_nul_instead_of_panicking() {
        assert_parity(r#"export IFS; print "child=[$(/usr/bin/printenv IFS)]"; print rc=$?"#);
    }

    /// The same truncation for an ordinary parameter, so the fix is the
    /// C rule rather than a special-case for `IFS`.
    #[test]
    fn an_embedded_nul_ends_the_exported_value() {
        assert_parity(
            r#"zqnul=$'a\0b'; export zqnul; print "child=[$(/usr/bin/printenv zqnul)]""#,
        );
    }

    /// The siblings that reach their value through a GSU global rather
    /// than `u.str`. These are what regressed when `$TERM` stopped
    /// reaching children; pin them together so a future change to
    /// `export_param`'s getfn dispatch cannot quietly empty one.
    #[test]
    fn special_scalars_export_their_real_value() {
        assert_parity(
            r#"export TERM HOME WORDCHARS; for v in TERM HOME WORDCHARS; do print "$v=[$(/usr/bin/printenv $v)]"; done"#,
        );
    }
}

/// POSIX_BUILTINS readonly rules (c:Src/builtin.c:2194-2206, c:2283-2286):
/// a valueless `readonly NAME` leaves NAME unset but listed, re-declaring it
/// readonly is idempotent, and `typeset +r` on it is refused. B02typeset.ztst
/// "readonly with POSIX_BUILTINS".
mod posix_builtins_readonly {
    use super::*;

    #[test]
    fn valueless_readonly_stays_unset_and_is_listed() {
        assert_parity("setopt posixbuiltins; readonly pbro; print ${+pbro}; readonly -p | grep -x 'readonly pbro'; typeset -gr pbro; print ${+pbro}");
    }

    #[test]
    fn plus_r_on_posix_readonly_is_refused() {
        assert_parity("(setopt posixbuiltins; readonly pbro; typeset -g +r pbro 2>/dev/null); echo $?");
    }

    #[test]
    fn valueless_export_stays_unset() {
        assert_parity("setopt posixbuiltins; export eu; print ${+eu}; export -p | grep -x 'export eu'");
    }

    /// The POSIX `readonly -p` form prints no tied peer (c:Src/params.c:6204,
    /// c:6257 — the peer is printed only for PRINT_TYPE / PRINT_TYPESET).
    #[test]
    fn posix_readonly_listing_omits_tied_peer() {
        assert_parity("function { emulate -L sh; MANPATH=/bin; export MANPATH; readonly MANPATH; readonly -p; }");
    }
}

/// c:Src/exec.c:758-768 — for an external command zsh takes ARGV0 from the
/// command's environment as argv[0] and then `unsetenv("ARGV0")`, so
/// `/usr/bin/env` never lists it. zshrs's in-process `env` stand-in printed
/// and passed on the shell's ARGV0. An ARGV0 given to env as its own operand
/// is not the shell's and still passes through.
mod env_does_not_see_argv0 {
    use super::*;

    #[test]
    fn shell_argv0_is_removed_from_env() {
        assert_parity(r#"ARGV0=foo env | grep -c '^ARGV0='; FOO=bar env | grep '^FOO='"#);
        assert_parity(r#"export ARGV0=bar; env | grep -c '^ARGV0='; env /bin/sh -c 'echo ${ARGV0-unset}'"#);
    }

    #[test]
    fn argv0_operand_of_env_still_passes_through() {
        assert_parity(r#"env ARGV0=x /usr/bin/env | grep '^ARGV0='; ARGV0=foo env ARGV0=y /bin/sh -c 'echo $ARGV0'"#);
    }
}

/// c:Src/params.c:3911-3913 — unsetting a tied special keeps both nodes,
/// PM_UNSET, with the tie intact: the array revived by `manpath=(…)` is
/// still the special, so unsetting `MANPATH` takes `manpath` with it.
mod unset_tied_special_keeps_the_tie {
    use super::*;

    #[test]
    fn revived_pair_unsets_together() {
        assert_parity(
            "unset manpath; print $+MANPATH; manpath=(/here /there); print $MANPATH; \
             unset MANPATH; print $+manpath; MANPATH=/a:/b; print $manpath",
        );
        assert_parity("unset fpath; FPATH=/a:/b; print ${(t)fpath}; unset FPATH; print $+fpath");
    }
}

/// c:Src/params.c:893-977 — the startup exports. After the environ import,
/// createparamtable exports HOME (c:960-965), LOGNAME (c:968-970) and the
/// incremented SHLVL (c:971-974), and set_pwd_env's addenv (c:977,
/// c:Src/builtin.c:821-826) exports PWD and OLDPWD last. A child therefore
/// sees the inherited variables in their own order, then those five in that
/// order. zshrs published PWD/OLDPWD at the top of ShellExecutor::new and
/// SHLVL before LOGNAME, so every child's environment was ordered
/// differently. MANPATH/INFOPATH/FPATH are left out: exporting the bundled
/// doc and function paths is a separate, open decision.
mod startup_environment_order {
    use super::*;

    fn child_env(shell: &Path, zshrs: bool, vars: &[(&str, &str)]) -> Vec<String> {
        let mut c = Command::new(shell);
        if zshrs {
            c.arg("--zsh");
        }
        c.args(["-f", "-c", "/usr/bin/env; :"]).env_clear().current_dir("/");
        for (k, v) in vars {
            c.env(k, v);
        }
        let o = c.output().expect("run shell");
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| {
                !["MANPATH=", "INFOPATH=", "FPATH=", "_="]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .map(String::from)
            .collect()
    }

    fn assert_env_parity(vars: &[(&str, &str)]) {
        if !zsh_available() {
            return;
        }
        let z = child_env(Path::new(zsh_path()), false, vars);
        let r = child_env(&zshrs_bin(), true, vars);
        assert_eq!(z, r, "environment divergence for {vars:?}");
    }

    #[test]
    fn home_logname_shlvl_then_pwd_oldpwd() {
        assert_env_parity(&[("HOME", "/tmp"), ("PATH", "/bin:/usr/bin")]);
        assert_env_parity(&[("SHLVL", "3"), ("HOME", "/tmp"), ("PATH", "/bin"), ("LOGNAME", "x")]);
    }

    #[test]
    fn inherited_pwd_keeps_its_place() {
        assert_env_parity(&[("HOME", "/tmp"), ("PWD", "/"), ("FOO", "1"), ("PATH", "/bin")]);
    }

    /// c:Src/init.c:1125-1130 — with no PATH in the environment `$path` is
    /// the compiled-in default, not one empty element.
    #[test]
    fn missing_path_is_the_compiled_in_default() {
        if !zsh_available() {
            return;
        }
        let script = r#"print -r -- "$PATH"; typeset -p path"#;
        let run = |shell: &Path, zshrs: bool| {
            let mut c = Command::new(shell);
            if zshrs {
                c.arg("--zsh");
            }
            let o = c
                .args(["-f", "-c", script])
                .env_clear()
                .env("HOME", "/tmp")
                .output()
                .expect("run shell");
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        assert_eq!(run(Path::new(zsh_path()), false), run(&zshrs_bin(), true));
    }
}

/// Parameters the shell sets for itself must not reach a child's environ.
///
/// zsh-5.9.1 Src/init.c:1182-1186 creates ZSH_EXECUTION_STRING / ZSH_SCRIPT /
/// ZSH_NAME with `setsparam` (unexported), and Src/jobs.c:2123 keeps the
/// `jobs -Z` span in a file-static. The port wrote all of them, plus the
/// dev-only ZSH_EXEPATH, with `std::env::set_var`, so every external command
/// run from a `zsh_main` shell (`-s`, `-i`, a terminal) saw them.
mod shell_internal_env {
    use super::*;
    use std::io::Write;
    use std::process::Stdio;

    fn run_stdin(shell: &Path, zshrs: bool) -> String {
        let script = "/usr/bin/env | /usr/bin/grep -E '^(ZSH_|__zshrs)'\n\
                      print -r -- exepath=${+ZSH_EXEPATH}\n";
        let mut c = Command::new(shell);
        if zshrs {
            c.arg("--zsh");
        }
        let mut child = c
            .args(["-f", "-s"])
            .env_clear()
            .env("HOME", "/tmp")
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn shell");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let o = child.wait_with_output().expect("wait shell");
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn stdin_shell_exports_no_internal_params() {
        if !zsh_available() {
            return;
        }
        let z = run_stdin(Path::new(zsh_path()), false);
        assert_eq!(z, "exepath=0\n", "reference zsh output changed");
        assert_eq!(run_stdin(&zshrs_bin(), true), z);
        assert_eq!(run_stdin(&zshrs_bin(), false), z);
    }
}
