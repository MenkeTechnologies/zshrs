//! Port of `_call_program` from
//! `Completion/Base/Utility/_call_program`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #autoload +X
//! sh: 3  local -xi COLUMNS=999
//! sh: 4  local curcontext="${curcontext}" tmp err_fd=-1 clocale='_comp_locale;'
//! sh: 5  local -a prefix
//! sh: 7  if [[ "$1" = -p ]]; then
//! sh: 8    shift
//! sh: 9    if (( $#_comp_priv_prefix )); then
//! sh:10      curcontext="${curcontext%:*}/${${(@M)_comp_priv_prefix:#^*[^\\]=*}[1]}:"
//! sh:11      zstyle -t ":completion:${curcontext}:${1}" gain-privileges &&
//! sh:12        prefix=( $_comp_priv_prefix )
//! sh:13    fi
//! sh:14  elif [[ "$1" = -l ]]; then
//! sh:15    shift
//! sh:16    clocale=''
//! sh:17  fi
//! sh:26  if zstyle -s ":completion:${curcontext}:${1}" command tmp; then
//! sh:27    if [[ "$tmp" = -* ]]; then
//! sh:28      eval $clocale "$tmp[2,-1]" "$argv[2,-1]"
//! sh:29    else
//! sh:30      eval $clocale $prefix "$tmp"
//! sh:31    fi
//! sh:32  else
//! sh:33    eval $clocale $prefix "$argv[2,-1]"
//! sh:34  fi 2>&$err_fd
//! ```
//!
//! Runs the helper through the SHELL'S OWN `eval` (sh:28/30/33), so the
//! helper sees this shell's functions, aliases, options and parameters, and
//! a missing command is reported by zsh with zsh's status:
//! `(eval):1: command not found: foo`, exit 127. The port previously spawned
//! `sh -c`, which reported `sh: foo: command not found`, collapsed every
//! failure to 1, and ran the helper outside the shell entirely.
//!
//! The three function-scoped side effects upstream gets for free from the
//! `$( … )` fork its callers write — `local -xi COLUMNS=999` (sh:3), the
//! `_comp_locale` reset (sh:4) and `2>&$err_fd` (sh:19-22 + sh:34) — are
//! save/restored by hand here, because zshrs runs command substitution
//! in-process. See `HelperEnv` and `StderrDiscard`.
//!
//! [`_call_program`] is the function; [`call_program_capture`] is the
//! `$( _call_program … )` its callers write, for Rust callers that cannot.

use crate::compsys::ported::_comp_locale::_comp_locale;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::getsparam;
use std::env;
use std::io::Read;
use std::process::{Command, Output, Stdio};

/// `_call_program` — run a helper command. First arg is the style key
/// suffix; flags `-p` (privileged) and `-l` (skip locale reset) come first.
///
/// Returns the helper's exit status. The helper's stdout is published on
/// `$REPLY` (a zshrs convenience with no upstream counterpart — see
/// [`publish`]) and written to fd 1 when fd 1 is not a terminal, so a shell
/// `$( _call_program … )` sees it. A Rust caller that wants the output calls
/// [`call_program_capture`], which IS that `$( … )`.
pub fn _call_program(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_call_program");
    match command_line(args) {
        Some((cmdline, use_locale, line)) => run_helper(&cmdline, use_locale, line, false).1,
        None => 1,
    }
}

/// Upstream's `output="$(_call_program … )"` — the shape every shell caller
/// writes (`_pick_variant` sh:36, `_arguments` sh:98, `_path_commands`).
///
/// Returns `(stdout, status)` and publishes the stdout on `$REPLY` as well,
/// which is what native `_pick_variant` reads.
///
/// This exists because a Rust port cannot write `$( … )` around a Rust call.
/// Without it the only way to give a native caller the output was to capture
/// unconditionally inside `_call_program` and then guess, from `isatty(1)`,
/// whether to echo the bytes back out — which leaked the helper's stdout into
/// every context where fd 1 was a file rather than a terminal (the
/// stock-utility sweep saw `hello` printed above `_pick_variant`'s report).
pub fn call_program_capture(args: &[String]) -> (String, i32) {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_call_program");
    match command_line(args) {
        Some((cmdline, use_locale, line)) => run_helper(&cmdline, use_locale, line, true),
        None => (String::new(), 1),
    }
}

/// sh:7-33 — flag parse plus the `command` style, yielding the word list the
/// `eval` at sh:28 / sh:30 / sh:33 is handed, whether the `_comp_locale`
/// reset applies (sh:4/sh:16), and WHICH of those three lines it is.
fn command_line(args: &[String]) -> Option<(Vec<String>, bool, u64)> {
    let mut argv: Vec<String> = args.to_vec();
    let mut use_locale = true;

    // sh:7-17  flag parse
    if let Some(first) = argv.first() {
        if first == "-p" {
            argv.remove(0);
            // sh:9-13  privileged prefix — we don't model
            //   _comp_priv_prefix processing fully; just drop the
            //   flag and proceed with the rest.
        } else if first == "-l" {
            argv.remove(0);
            use_locale = false;
        }
    }

    if argv.is_empty() {
        return None;
    }

    // sh:26  zstyle -s … command tmp — when set, replace argv[1..]
    //   with the styled command line.
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:{}", curcontext, argv[0]);
    let styled = lookupstyle(&style_ctx, "command")
        .first()
        .cloned()
        .unwrap_or_default();
    if !styled.is_empty() {
        if let Some(rest) = styled.strip_prefix('-') {
            // sh:28  eval $clocale "$tmp[2,-1]" "$argv[2,-1]"
            let mut v: Vec<String> = vec![rest.to_string()];
            if argv.len() > 1 {
                v.extend(argv[1..].iter().cloned());
            }
            return Some((v, use_locale, 28));
        }
        // sh:30  eval $clocale $prefix "$tmp"
        return Some((vec![styled], use_locale, 30));
    }
    // sh:33  eval $clocale $prefix "$argv[2,-1]"
    if argv.len() > 1 {
        Some((argv[1..].to_vec(), use_locale, 33))
    } else {
        None
    }
}

/// sh:34 `… 2>&$err_fd`, with `err_fd` chosen at sh:19-22:
///
/// ```text
/// sh:19  if (( ${debug_fd:--1} > 2 )) || [[ ! -t 2 ]]
/// sh:20  then exec {err_fd}>&2	# debug_fd is saved stderr, 2 is trace or redirect
/// sh:21  else exec {err_fd}>/dev/null
/// sh:22  fi
/// ```
///
/// i.e. a CAPTURED fd 2 (the caller's `2>&1`, a redirect, a trace fd) passes
/// through and a TERMINAL fd 2 is discarded, so a helper's usage spew never
/// lands on the screen mid-completion. The subprocess path expressed this by
/// choosing whether to re-emit the captured stderr; the in-process `eval`
/// writes to fd 2 directly, so the redirect has to be real.
///
/// RAII: `dup`s the old fd 2 aside, points fd 2 at `/dev/null`, and restores
/// on drop.
struct StderrDiscard {
    saved: libc::c_int,
}

impl StderrDiscard {
    /// sh:19-22 — returns `Some` only in the `else` arm (fd 2 is a terminal
    /// and `$debug_fd` is not a saved stderr above it), which is the only arm
    /// that redirects.
    fn maybe_enter() -> Option<Self> {
        let debug_fd: i32 = getsparam("debug_fd")
            .and_then(|s| s.parse().ok())
            .unwrap_or(-1);
        if debug_fd > 2 || unsafe { libc::isatty(2) } == 0 {
            return None; // sh:20 — `exec {err_fd}>&2`
        }
        // sh:21 — `exec {err_fd}>/dev/null`
        let devnull = unsafe {
            libc::open(
                b"/dev/null\0".as_ptr() as *const libc::c_char,
                libc::O_WRONLY,
            )
        };
        if devnull < 0 {
            return None;
        }
        let saved = unsafe { libc::dup(2) };
        if saved < 0 {
            unsafe { libc::close(devnull) };
            return None;
        }
        unsafe {
            libc::dup2(devnull, 2);
            libc::close(devnull);
        }
        Some(StderrDiscard { saved })
    }
}

impl Drop for StderrDiscard {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.saved, 2);
            libc::close(self.saved);
        }
    }
}

/// sh:3 `local -xi COLUMNS=999` and sh:4 `clocale='_comp_locale;'` —
/// both function-scoped upstream, so both have to be undone on return.
///
/// Upstream gets that for free: every caller wraps `_call_program` in
/// `$( … )`, which in zsh FORKS, so the exported `COLUMNS` and the locale
/// `_comp_locale` rewrites die with the subshell. zshrs runs command
/// substitution IN-PROCESS (`exec.rs:8672`, `run_command_substitution` —
/// `SubshStateGuard`, no fork), so nothing else would undo them: the shell
/// would come back from a `git <TAB>` with `LANG=C` and every `LC_*` gone.
/// Save and restore by hand instead.
///
/// `COLUMNS` is deliberately set in the PROCESS ENVIRONMENT and not through
/// `setsparam`: the shell parameter is special and its setter runs
/// `adjustwinsize(3)` (`params.rs:10731`), an ioctl on the terminal. That is
/// harmless in a forked subshell and is not harmless in the live shell in the
/// middle of a ZLE redisplay.
struct HelperEnv {
    saved: Vec<(String, Option<String>)>,
}

impl HelperEnv {
    fn enter(use_locale: bool) -> Self {
        let mut saved: Vec<(String, Option<String>)> = Vec::new();
        // sh:3 — `local -xi COLUMNS=999`
        saved.push(("COLUMNS".to_string(), env::var("COLUMNS").ok()));
        env::set_var("COLUMNS", "999");
        if use_locale {
            // sh:4 / sh:28,30,33 — the `_comp_locale;` prefix on the eval.
            // Every name it can touch has to be on the restore list before it
            // runs: LANG, and every `LC_*` currently exported (it does
            // `unset -m LC_\*` at sh:13/sh:17).
            saved.push(("LANG".to_string(), env::var("LANG").ok()));
            for (k, v) in env::vars() {
                if k.starts_with("LC_") {
                    saved.push((k, Some(v)));
                }
            }
            if env::var("LC_CTYPE").is_err() {
                saved.push(("LC_CTYPE".to_string(), None));
            }
            let _ = _comp_locale();
        }
        HelperEnv { saved }
    }
}

impl Drop for HelperEnv {
    fn drop(&mut self) {
        for (k, v) in self.saved.iter().rev() {
            match v {
                Some(v) => env::set_var(k, v),
                None => env::remove_var(k),
            }
        }
    }
}

/// POSIX single-quoting, for wrapping the helper's command text as ONE word
/// of an `eval`. Every byte is literal inside `'…'`; an embedded `'` closes,
/// escapes and reopens.
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Is there a shell to run the `eval` in?
///
/// `exec::execute_script` / `exec::getoutput` need either an active VM
/// context or the installed session executor; with neither they return
/// "nothing ran, status 0" (`exec.rs:8639`). That is the state a
/// `#[cfg(test)]` binary and `--doctor` are in, and answering a helper
/// invocation with silence there is worse than running a subprocess — so
/// [`run_helper`] falls back rather than no-op.
fn have_shell_executor() -> bool {
    if crate::fusevm_bridge::try_with_executor(|_| ()).is_some() {
        return true;
    }
    // `with_session_context` enters the session executor's context when one
    // is installed and is a plain call-through when it is not, so the inner
    // probe answers "either kind of executor exists".
    crate::fusevm_bridge::with_session_context(|| {
        crate::fusevm_bridge::try_with_executor(|_| ()).is_some()
    })
}

/// sh:26-34 — run the helper.
///
/// `capture` selects between the two upstream call shapes: `false` is
/// `_call_program …` written as a command, `true` is the
/// `$( _call_program … )` every upstream caller actually writes.
///
/// Returns `(stdout, status)`.
fn run_helper(cmdline: &[String], use_locale: bool, line: u64, capture: bool) -> (String, i32) {
    // In-editor dispatch (LSP): a completion helper must never outlive the
    // request budget, and with exec disabled it must not run at all. This is
    // one of the two paths that still goes through a `sh -c` subprocess, and
    // it does so deliberately — an in-process `eval` cannot be killed at a
    // deadline, and a hung helper would wedge the completion thread with
    // nobody to interrupt it. Outside an in-editor dispatch `exec_policy()`
    // is `None` and the shell's own `eval` below runs.
    if let Some(policy) = crate::compsys::in_editor::exec_policy() {
        return run_helper_subprocess(cmdline, use_locale, Some(policy), capture);
    }
    // The other one: no shell to eval in at all.
    if !have_shell_executor() {
        return run_helper_subprocess(cmdline, use_locale, None, capture);
    }

    let _env = HelperEnv::enter(use_locale); // sh:3-4
    let _err = StderrDiscard::maybe_enter(); // sh:19-22 + sh:34 `2>&$err_fd`

    // sh:28 / sh:30 / sh:33 — `eval $clocale $prefix "$argv[2,-1]"`, run by
    // THIS SHELL, not by a `sh -c` subprocess:
    //
    //   * the helper sees this shell's functions, aliases, options and
    //     parameters, which is the whole point of a `command` style that
    //     names a shell function;
    //   * a missing command is reported by zsh, in zsh's words and with
    //     zsh's status — `(eval):1: command not found: foo`, exit 127. The
    //     `sh -c` spawn said `sh: foo: command not found` and this function
    //     then collapsed every failure to 1, so no caller could tell a
    //     missing helper from one that merely exited non-zero.
    //
    // The `eval` word is what produces the `(eval):1:` prefix: the builtin
    // pushes an `FS_EVAL` funcstack frame named `(eval)`
    // (`Src/builtin.c:6164-6199`) and parses its argument as a fresh program
    // starting at line 1. Wrapping the whole command text in one
    // single-quoted word is how upstream's own `eval $clocale $prefix "$…"`
    // reaches the same place.
    //
    let text = cmdline.join(" ");
    if !capture {
        // `_call_program …` written as a plain command: nothing captures,
        // the helper's stdout flows to fd 1 exactly as it produced it —
        // trailing newline included. `eval_comp` is the shared port of
        // `static int eval(char **argv)` (`Src/builtin.c:6151`); the
        // `FS_EVAL` frame it pushes (c:6164-6199) is named `(eval)`, which
        // is where zsh's `(eval):1: command not found: foo` comes from.
        return (
            String::new(),
            crate::compsys::ported::shared::eval_comp(&text, line),
        );
    }

    // `$( _call_program … )`. `run_command_substitution` is the in-process
    // body of C's `getoutput` (`Src/exec.c:4753-4790`, `exec.rs:8672`) — the
    // same executor call `$( … )` makes, with `SubshStateGuard` standing in
    // for `entersubsh(ESUB_PGRP|ESUB_NOMONITOR)`; it publishes the body's
    // status on `cmdoutval` (c:4759/4775) and trims trailing newlines the way
    // the qt post-walk does (c:4855-4871). The `eval` word around the command
    // text is upstream's own sh:28/30/33, kept inside the substitution so a
    // diagnostic still reads `(eval):1:`.
    crate::compsys::ported::shared::set_sh_lineno(line);
    let out =
        crate::ported::exec::run_command_substitution(&format!("eval {}", single_quote(&text)));
    let status = crate::ported::exec::cmdoutval.load(std::sync::atomic::Ordering::Relaxed);
    let _ = crate::ported::params::setsparam("REPLY", &out);
    (out, status)
}

/// Deliver the SUBPROCESS arm's stdout the way the call shape requires: the
/// raw bytes to fd 1 for a plain `_call_program …`, and the
/// trailing-newline-trimmed string (`$( … )` semantics, c:4855-4871) as the
/// value plus `$REPLY` for a capture.
///
/// `$REPLY` is a zshrs convenience with no upstream counterpart, needed
/// because a Rust port cannot write `$( … )` around a Rust call. Every native
/// `_NAME` port whose shell source says `$( _call_program … )` calls
/// [`call_program_capture`] and reads `$REPLY` from there.
fn publish(raw: &str, capture: bool) -> String {
    if !capture {
        if !raw.is_empty() {
            use std::io::Write as _;
            let mut so = std::io::stdout();
            let _ = so.write_all(raw.as_bytes());
            let _ = so.flush();
        }
        return String::new();
    }
    // c:Src/exec.c:4855-4871 — the `$( … )` post-walk trims TRAILING newlines.
    let trimmed = raw.trim_end_matches('\n').to_string();
    let _ = crate::ported::params::setsparam("REPLY", &trimmed);
    trimmed
}

/// The subprocess arm of [`run_helper`], for the two situations where the
/// shell's own `eval` is not usable:
///
///   * `policy = Some(…)` — in-editor (LSP) dispatch, where the helper has
///     to be KILLABLE at a deadline; see the comment at the call site.
///   * `policy = None` — no VM executor at all (a `#[cfg(test)]` binary,
///     `--doctor`), where an `eval` would silently run nothing.
///
/// Both are documented divergences from sh:28/30/33: the helper runs outside
/// this shell and so sees none of its state.
fn run_helper_subprocess(
    cmdline: &[String],
    use_locale: bool,
    policy: Option<(bool, std::time::Instant)>,
    capture: bool,
) -> (String, i32) {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(cmdline.join(" "));
    cmd.env("COLUMNS", "999"); // sh:3
    if use_locale {
        // sh:4 — apply `_comp_locale` to the CHILD's env only, restoring the
        // parent's immediately: this arm never runs the helper in-process,
        // so the parent must not keep the C locale.
        let saved_lang = env::var("LANG").ok();
        let saved_ctype = env::var("LC_CTYPE").ok();
        let _ = _comp_locale();
        cmd.env("LANG", env::var("LANG").unwrap_or_else(|_| "C".to_string()));
        if let Ok(ct) = env::var("LC_CTYPE") {
            cmd.env("LC_CTYPE", ct);
        }
        if let Some(v) = saved_lang {
            env::set_var("LANG", v);
        }
        if let Some(v) = saved_ctype {
            env::set_var("LC_CTYPE", v);
        }
    }
    let output = match policy {
        // Exec-free mode: no subprocess. Callers see an empty `$REPLY` +
        // non-zero status and fall back to their static specs, same as a
        // helper that produced nothing.
        Some((false, _)) => {
            let _ = crate::ported::params::setsparam("REPLY", "");
            return (String::new(), 1);
        }
        Some((true, deadline)) => match run_with_deadline(cmd, deadline) {
            Some(o) => o,
            None => {
                let _ = crate::ported::params::setsparam("REPLY", "");
                return (String::new(), 1);
            }
        },
        None => match cmd.output() {
            Ok(o) => o,
            Err(_) => {
                let _ = crate::ported::params::setsparam("REPLY", "");
                return (String::new(), 1);
            }
        },
    };
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let stdout = publish(&raw, capture);
    // sh:34 `2>&$err_fd` — pass the helper's stderr through only when fd 2
    // is captured, discard it when fd 2 is the display (sh:19-22).
    if !output.stderr.is_empty() && unsafe { libc::isatty(2) } == 0 {
        use std::io::Write as _;
        let mut se = std::io::stderr();
        let _ = se.write_all(&output.stderr);
        let _ = se.flush();
    }
    (stdout, output.status.code().unwrap_or(1))
}

/// Run `cmd` but kill it at `deadline`, returning its output if it
/// finished in time.
///
/// Used only by the in-editor (LSP) dispatch. `Command::output()`
/// waits forever, which is correct at an interactive prompt — the
/// user can hit ^C — and wrong in an editor, where a slow or hung
/// helper (`git ls-remote`, an unreachable `kubectl` context) would
/// wedge the completion thread with nobody to interrupt it.
///
/// stdout is drained on a reader thread so a helper that fills the
/// pipe buffer can still be killed: with an unread pipe the child
/// blocks in `write()` and never exits, so `try_wait` would spin to
/// the deadline even for fast commands.
fn run_with_deadline(mut cmd: Command, deadline: std::time::Instant) -> Option<Output> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    // NEVER inherit stdin here. In the LSP the parent's fd 0 is the
    // JSON-RPC stream from the editor: a helper that reads stdin (any
    // `git` subcommand that thinks it can prompt, `sh -c` reading a
    // heredoc it never got) consumes the protocol bytes, the server
    // then sees EOF and exits mid-session. Observed as "stdin EOF,
    // shutting down" one dispatch after the first `git <tab>`.
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut stdout = stdout;
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            tracing::debug!(
                target: "zshrs::compsys::in_editor",
                "_call_program: helper killed at completion deadline",
            );
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let stdout = reader.join().unwrap_or_default();
    Some(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_call_program(&[]), 1);
    }

    // A `#[cfg(test)]` binary has no VM executor, so the three tests below
    // that actually RUN something take `run_helper`'s no-shell fallback —
    // the `sh -c` subprocess — not sh:28/30/33's `eval`. They still pin what
    // they always pinned (status plumbing and `$REPLY`); what they cannot
    // reach is the in-shell eval, which the stock-utility sweep covers
    // end to end against a live shell (`_call_program/echo`, `/failing`,
    // `/stderr`, `/nonexistent` — that last cell is the one that caught the
    // `sh:` prefix and the 1-vs-127 status).
    //
    // sh:7-33 — the flag parse and the `command`-style branch that decide
    // WHICH eval line runs, and with what words — needs no shell at all;
    // those are the `command_line` tests further down.

    #[test]
    fn invokes_true_command_successfully() {
        let _g = crate::test_util::global_state_lock();
        let r = _call_program(&["my-style-key".to_string(), "true".to_string()]);
        assert_eq!(r, 0);
    }

    #[test]
    fn invokes_false_command_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = _call_program(&["my-style-key".to_string(), "false".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn plain_call_does_not_publish_reply() {
        // `_call_program …` written as a plain command captures NOTHING —
        // sh:28/30/33 is a bare `eval`, its stdout goes to fd 1, and there is
        // no upstream `$REPLY`. The port used to publish one from every call;
        // that is now [`call_program_capture`]'s job (the `$( … )` shape), so
        // a stale value must not be mistaken for this call's output.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("REPLY", "STALE");
        let _ = _call_program(&[
            "my-style-key".to_string(),
            "printf".to_string(),
            "hello".to_string(),
        ]);
        assert_eq!(getsparam("REPLY").as_deref(), Some("STALE"));
    }

    /// [`call_program_capture`] is the `$( _call_program … )` shape: it
    /// returns the bytes to the Rust caller AND leaves `$REPLY` set, but
    /// never writes them to fd 1 — that write is what leaked the probe's
    /// output into `_pick_variant`'s report.
    #[test]
    fn capture_entry_point_returns_the_output() {
        let _g = crate::test_util::global_state_lock();
        let (out, rc) = call_program_capture(&[
            "my-style-key".to_string(),
            "printf".to_string(),
            "hello".to_string(),
        ]);
        assert_eq!(out, "hello");
        assert_eq!(rc, 0);
        assert_eq!(getsparam("REPLY").as_deref(), Some("hello"));
    }

    #[test]
    fn command_line_uses_sh33_for_a_plain_call() {
        let _g = crate::test_util::global_state_lock();
        let (words, locale, line) =
            command_line(&["a-style-key".to_string(), "printf hi".to_string()]).unwrap();
        assert_eq!(words, vec!["printf hi".to_string()]); // sh:33 `"$argv[2,-1]"`
        assert!(locale, "sh:4 — `clocale` defaults to `_comp_locale;`");
        assert_eq!(line, 33);
    }

    #[test]
    fn command_line_dash_l_clears_the_locale_reset() {
        // sh:14-16 — `elif [[ "$1" = -l ]]; then shift; clocale=''`
        let _g = crate::test_util::global_state_lock();
        let (words, locale, line) = command_line(&[
            "-l".to_string(),
            "a-style-key".to_string(),
            "true".to_string(),
        ])
        .unwrap();
        assert_eq!(words, vec!["true".to_string()]);
        assert!(!locale);
        assert_eq!(line, 33);
    }

    #[test]
    fn command_line_dash_p_is_consumed() {
        // sh:7-13 — `-p` is a flag, never part of the command line.
        let _g = crate::test_util::global_state_lock();
        let (words, _locale, _line) = command_line(&[
            "-p".to_string(),
            "a-style-key".to_string(),
            "true".to_string(),
        ])
        .unwrap();
        assert_eq!(words, vec!["true".to_string()]);
    }

    #[test]
    fn command_line_is_none_when_there_is_nothing_to_run() {
        // sh:33 with `$argv[2,-1]` empty: the style key alone is not a
        // command.
        let _g = crate::test_util::global_state_lock();
        assert!(command_line(&["a-style-key".to_string()]).is_none());
    }
}
