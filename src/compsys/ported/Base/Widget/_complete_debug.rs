//! Port of `_complete_debug` from
//! `Completion/Base/Widget/_complete_debug`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-x?
//! sh: 3  eval "$_comp_setup"
//! sh: 5  (( $+_debug_count )) || integer -g _debug_count
//! sh: 6  local tmp=${TMPPREFIX}${$}${words[1]:t}$[++_debug_count]
//! sh: 7  local pager w="${(qq)words}"
//! sh: 9  integer debug_fd=-1
//! sh:10  {
//! sh:11    if [[ -t 2 ]]; then
//! sh:12      zmodload -F zsh/files b:zf_ln 2>/dev/null &&
//! sh:13      zf_ln -fn =(<<<'') $tmp &&
//! sh:14      exec {debug_fd}>&2 2>| $tmp
//! sh:15    fi
//! sh:22    local PROMPT4 PS4="${(j::)debug_indent}+%N:%i> "
//! sh:23    setopt xtrace
//! sh:24    : $ZSH_NAME $ZSH_VERSION
//! sh:25    ${1:-_main_complete}
//! sh:26    integer ret=$?
//! sh:27    unsetopt xtrace
//! sh:29    if (( debug_fd != -1 )); then
//! sh:30      zstyle -s ':completion:complete-debug::::' pager pager
//! sh:31      print -sR "${pager:-${PAGER:-${VISUAL:-${EDITOR:-more}}}} ${(q)tmp} ;: $w"
//! sh:32      _message -r "Trace output left in $tmp (up-history to view)"
//! sh:33      if [[ $compstate[nmatches] -le 1 && $compstate[list] != *force* ]]; then
//! sh:34          compstate[list]='list force messages'
//! sh:35      fi
//! sh:36    fi
//! sh:37  } always {
//! sh:38    (( debug_fd != -1 )) && exec 2>&$debug_fd {debug_fd}>&-
//! sh:39  }
//! sh:40  return ret
//! ```
//!
//! Wraps a completion run, capturing stderr into a per-attempt temp
//! file. The capture only happens when stderr is a terminal
//! (`[[ -t 2 ]]`, sh:11): the temp file is created, fd 2 is saved into
//! `debug_fd` and redirected to the file, and the `always{}` block
//! restores fd 2 afterwards. The user-facing "Trace output left in …"
//! message (sh:32) and the history/list side effects (sh:31,33-35) are
//! gated on `debug_fd != -1`, i.e. they fire only when a file was
//! actually created — mirroring the source exactly.
//!
//! Honest substrate gap: the shell `setopt xtrace` (sh:23) that prints
//! the PS4-formatted command trace to fd 2 is a shell-executor feature.
//! The Rust dispatch here runs the inner completion function
//! in-process, so xtrace line-by-line tracing of that dispatch is NOT
//! reproduced. What IS reproduced: fd 2 is genuinely redirected to the
//! temp file across the dispatch, so any stderr the run emits lands in
//! the file, the file is real, and the message truthfully points at it.

use crate::compsys::ported::_message::_message;
use crate::ported::builtin::bin_print;
use crate::ported::exec::dispatch_function_call;
use crate::ported::hashtable_h::BIN_PRINT;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setiparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zsh_h::{options, MAX_OPS};
use std::os::unix::io::AsRawFd;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_complete_debug` — run a completion with diagnostic context.
/// Optional `$1` overrides the default `_main_complete` target.
pub fn _complete_debug(args: &[String]) -> i32 {
    // sh:5-6  bump $_debug_count + derive tmp filename
    let n = getiparam("_debug_count") + 1;
    setiparam("_debug_count", n);
    let tmpprefix = getsparam("TMPPREFIX").unwrap_or_else(|| "/tmp/zsh".to_string());
    let pid = std::process::id();
    let words = getaparam("words").unwrap_or_default();
    let cmd_name = words.first().cloned().unwrap_or_default();
    let tmp = format!("{}{}{}{}", tmpprefix, pid, basename(&cmd_name), n);

    // sh:7  local pager w="${(qq)words}"  (single-quote each word, join)
    let w = words
        .iter()
        .map(|s| single_quote(s))
        .collect::<Vec<_>>()
        .join(" ");

    // sh:9  integer debug_fd=-1
    let mut debug_fd: i32 = -1;
    // Keep the redirect target file alive until the always-block restore.
    let mut _capture: Option<std::fs::File> = None;

    // sh:11-15  if [[ -t 2 ]]; then create $tmp, save fd 2, redirect fd 2 to $tmp
    if unsafe { libc::isatty(2) } == 1 {
        // sh:13  zf_ln -fn =(<<<'') $tmp — materialise an empty file at $tmp.
        if let Ok(f) = std::fs::File::create(&tmp) {
            // sh:14  exec {debug_fd}>&2  — dup current fd 2 aside.
            let saved = unsafe { libc::dup(2) };
            if saved >= 0 {
                // sh:14  2>| $tmp — point fd 2 at the trace file.
                if unsafe { libc::dup2(f.as_raw_fd(), 2) } >= 0 {
                    debug_fd = saved;
                    _capture = Some(f);
                } else {
                    unsafe {
                        libc::close(saved);
                    }
                }
            }
        }
    }

    // sh:23-27  setopt xtrace ; ${1:-_main_complete} ; unsetopt xtrace
    //   (xtrace tracing itself is a shell-executor feature — see module
    //   doc; the dispatch below runs the inner fn in-process.)
    let target = args
        .first()
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "_main_complete".to_string());
    // sh:23/27 — `setopt xtrace` … `unsetopt xtrace`. The executor emits
    // PS4 trace lines to fd 2 (exec.rs:5409-5445 `if isset(XTRACE)`), and
    // fd 2 is redirected to `$tmp` above, so the trace of the in-process
    // completion dispatch lands in the capture file — Bug #657 gap #4.
    // Only when we actually created the capture (debug_fd != -1), and the
    // prior XTRACE state is restored (the port has no `localoptions`).
    let prev_xtrace = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE);
    if debug_fd != -1 {
        crate::ported::options::setoption("xtrace", 1);
    }
    let ret = dispatch_function_call(&target, &[]).unwrap_or(1);
    if debug_fd != -1 {
        crate::ported::options::setoption("xtrace", prev_xtrace as i32);
    }

    // sh:29  if (( debug_fd != -1 )); then — only when a file was created.
    if debug_fd != -1 {
        // sh:30  zstyle -s ':completion:complete-debug::::' pager pager
        let style_pager = lookupstyle(":completion:complete-debug::::", "pager")
            .into_iter()
            .next()
            .unwrap_or_default();
        // sh:31  ${pager:-${PAGER:-${VISUAL:-${EDITOR:-more}}}}
        let pager = first_nonempty(&[
            style_pager,
            getsparam("PAGER").unwrap_or_default(),
            getsparam("VISUAL").unwrap_or_default(),
            getsparam("EDITOR").unwrap_or_default(),
            "more".to_string(),
        ]);
        // sh:31  print -sR "$pager ${(q)tmp} ;: $w"  (push a view command to history)
        let cmd = format!("{} {} ;: {}", pager, single_quote(&tmp), w);
        let mut ops = make_ops();
        ops.ind[b's' as usize] = 1;
        ops.ind[b'R' as usize] = 1;
        let _ = bin_print("print", &[cmd], &ops, BIN_PRINT);

        // sh:32  gated user-facing trace pointer message
        let _ = _message(&[
            "-r".to_string(),
            format!("Trace output left in {} (up-history to view)", tmp),
        ]);

        // sh:33-35  if nmatches <= 1 && list !*force* -> list='list force messages'
        let nmatches: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let list = get_compstate_str("list").unwrap_or_default();
        if nmatches <= 1 && !list.contains("force") {
            set_compstate_str("list", "list force messages");
        }
    }

    // sh:37-39  always { (( debug_fd != -1 )) && exec 2>&$debug_fd {debug_fd}>&- }
    if debug_fd != -1 {
        unsafe {
            libc::dup2(debug_fd, 2);
            libc::close(debug_fd);
        }
    }
    drop(_capture);

    // sh:40  return ret
    ret
}

fn basename(s: &str) -> String {
    s.rsplit('/').next().unwrap_or("").to_string()
}

/// First non-empty string in `xs`, else empty (models `${a:-${b:-…}}`).
fn first_nonempty(xs: &[String]) -> String {
    xs.iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default()
}

/// `${(q)…}`/`${(qq)…}` single-quote quoting: wrap in `'…'`, escaping
/// embedded single quotes the POSIX way (`'\''`).
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_complete_debug(&[]), 1);
    }

    #[test]
    fn increments_debug_count() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_debug_count", 5);
        let _ = _complete_debug(&[]);
        assert_eq!(getiparam("_debug_count"), 6);
    }

    // Under `cargo test`, fd 2 is not a terminal, so the sh:11 `[[ -t 2 ]]`
    // guard is false: no temp file is created and the sh:32 message /
    // sh:31 history push are gated off. Pin that the trace file is NOT
    // materialised when stderr isn't a tty (the bug this port fixes was
    // emitting a "Trace output left in <tmp>" message for a file that
    // never existed).
    #[test]
    fn no_trace_file_when_stderr_not_tty() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_debug_count", 0);
        let tmpprefix = getsparam("TMPPREFIX").unwrap_or_else(|| "/tmp/zsh".to_string());
        let pid = std::process::id();
        // Mirror the sh:6 tmp formula for count == 1 (fresh count), words empty.
        let tmp = format!("{}{}{}{}", tmpprefix, pid, "", 1);
        let _ = std::fs::remove_file(&tmp);
        let _ = _complete_debug(&[]);
        if unsafe { libc::isatty(2) } == 0 {
            assert!(
                !std::path::Path::new(&tmp).exists(),
                "no trace file must be created when stderr is not a tty"
            );
        }
    }
}
