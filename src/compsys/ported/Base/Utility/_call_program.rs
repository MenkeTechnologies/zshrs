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
//! Runs the requested command via std::process::Command,
//! capturing stdout. Returns 1 on spawn failure. The `_comp_locale`
//! C-locale dance is implicit in the spawn env (we set LANG=C and
//! preserve LC_CTYPE per the `_comp_locale` port).

use crate::compsys::ported::_comp_locale::_comp_locale;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::getsparam;
use std::env;
use std::process::Command;

/// `_call_program` — run a helper command and capture stdout.
/// First arg is the style key suffix; flags `-p` (privileged) and
/// `-l` (skip locale reset) come first.
///
/// Returns 0 on successful spawn, 1 otherwise. Stdout is returned
/// via the shell-side param `REPLY` so callers can read it (mirrors
/// the shell pattern where `_call_program` runs in `$(...)` and
/// stdout is captured into a var).
pub fn _call_program(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_call_program");
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
        return 1;
    }

    // sh:26  zstyle -s … command tmp — when set, replace argv[1..]
    //   with the styled command line.
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:{}", curcontext, argv[0]);
    let styled = lookupstyle(&style_ctx, "command")
        .first()
        .cloned()
        .unwrap_or_default();
    let cmdline: Vec<String> = if !styled.is_empty() {
        if let Some(rest) = styled.strip_prefix('-') {
            // sh:28  eval … "$tmp[2,-1]" "$argv[2,-1]"
            let mut v: Vec<String> = vec![rest.to_string()];
            if argv.len() > 1 {
                v.extend(argv[1..].iter().cloned());
            }
            v
        } else {
            // sh:30  eval … $prefix "$tmp"
            vec![styled]
        }
    } else {
        // sh:33  eval … $prefix "$argv[2,-1]"
        if argv.len() > 1 {
            argv[1..].to_vec()
        } else {
            return 1;
        }
    };

    // Apply C-locale reset (subshell-equivalent) unless `-l`.
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(cmdline.join(" "));
    cmd.env("COLUMNS", "999");
    if use_locale {
        let saved_lang = env::var("LANG").ok();
        let saved_ctype = env::var("LC_CTYPE").ok();
        let _ = _comp_locale();
        // _comp_locale set LANG=C; propagate to subprocess env via
        //   inheritance (Command default uses current env).
        cmd.env("LANG", env::var("LANG").unwrap_or_else(|_| "C".to_string()));
        if let Some(ct) = env::var("LC_CTYPE").ok() {
            cmd.env("LC_CTYPE", ct);
        }
        // Restore parent env after spawn args are set.
        if let Some(v) = saved_lang {
            env::set_var("LANG", v);
        }
        if let Some(v) = saved_ctype {
            env::set_var("LC_CTYPE", v);
        }
    }

    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return 1,
    };

    // Publish stdout for caller via REPLY (a zshrs convenience the native
    // callers — `_pick_variant` — read directly).
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = crate::ported::params::setsparam("REPLY", &stdout);
    // C's `_call_program` is `eval $command` — it prints the helper's stdout,
    // which a shell caller captures with `local help="$(_call_program …)"`
    // (e.g. `_netcat`). The port piped stdout into `$REPLY` only, so those
    // captures came back empty. Re-emit stdout to fd 1 — but ONLY when fd 1 is
    // CAPTURED (a pipe/file), not a terminal. Inside a `$(…)` fd 1 is the
    // capture pipe → `!isatty` → the write is captured, not shown. In the DIRECT
    // `_call_program` call native `_pick_variant` makes during completion, fd 1
    // is the live display (a tty) → `isatty` → skip, so we don't leak the
    // helper's output onto the screen (which regressed ls-/chmod/find-/grep-/
    // df-/tr- when emitted unconditionally). isatty is more robust than the
    // `cmdsubst_outer_stdout()` stack, which is empty during the completion-
    // context cmdsub (that capture path doesn't push it).
    if std::env::var_os("ZSHRS_CSDBG").is_some() {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/cs.log")
            .map(|mut f| {
                use std::io::Write as _;
                let _ = writeln!(
                    f,
                    "CALLPROG cmdline={:?} out_len={} isatty1={}",
                    cmdline.join(" "),
                    output.stdout.len(),
                    unsafe { libc::isatty(1) }
                );
            });
    }
    if !output.stdout.is_empty() && unsafe { libc::isatty(1) } == 0 {
        use std::io::Write as _;
        let mut so = std::io::stdout();
        let _ = so.write_all(&output.stdout);
        let _ = so.flush();
    }
    // sh — upstream `_call_program` does `exec {err_fd}>&2` when fd 2 is a
    // redirect (caller's `2>&1` capture) and `>/dev/null` when fd 2 is the tty.
    // Mirror the stdout gate: re-emit stderr only when fd 2 is NOT a tty, so a
    // helper that writes its result to stderr and is captured via `2>&1` (e.g.
    // docker/tar subcommand listers) is seen, while live-display completion
    // (fd 2 = tty) still discards it (no screen leak).
    if !output.stderr.is_empty() && unsafe { libc::isatty(2) } == 0 {
        use std::io::Write as _;
        let mut se = std::io::stderr();
        let _ = se.write_all(&output.stderr);
        let _ = se.flush();
    }
    if output.status.success() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_call_program(&[]), 1);
    }

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
    fn captures_stdout_into_reply() {
        let _g = crate::test_util::global_state_lock();
        let _ = _call_program(&[
            "my-style-key".to_string(),
            "printf".to_string(),
            "hello".to_string(),
        ]);
        assert_eq!(getsparam("REPLY").as_deref(), Some("hello"));
    }
}
