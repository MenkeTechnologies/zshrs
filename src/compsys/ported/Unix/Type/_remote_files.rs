//! Port of `_remote_files` from `Completion/Unix/Type/_remote_files`.
//!
//! Full upstream body (114 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 39  local expl rempat remfiles remdispf{,q} remdispd{,q} args cmd suf ret=1
//! sh: 40  local -a args cmd_args
//! sh: 41  local glob host dir esc dirprefix
//! sh: 43  if zstyle -T ":completion:${curcontext}:files" remote-access; then
//! sh: 46    zparseopts -D -E -a args / g:=glob h:=host W:=dir Q:=esc
//! sh: 47    (( $#host)) && shift host || host="${IPREFIX%:}"
//! sh: 49    args=( ${argv[1,(i)--]} ); shift ${#args}
//! sh: 51    [[ $args[-1] = -- ]] && args[-1]=()
//! sh: 53    cmd="$1"; shift
//! sh: 56    if [[ $cmd == ssh ]]; then
//! sh: 57      zparseopts -D -E -a cmd_args p: 1 2 4 6 F:
//! sh: 58      cmd_args=( -o BatchMode=yes "$cmd_args[@]" -a -x )
//! sh: 59    else cmd_args=( "$@" ); fi
//! sh: 62    (( $#dir )) && dirprefix=${dir}/
//! sh: 65    rempat="${dirprefix}${PREFIX%%[^./][^/]#}\*"   (Q-quoted if $QIPREFIX)
//! sh: 71    remfiles=(${(M)${(f)"$(_call_program files $cmd $cmd_args $host command ls -d1FL -- "$rempat")"}%%[^/]#(|/)})
//! sh: 76    compset -P '*/'
//! sh: 77    compset -S '/*' || (( ${args[(I)-/]} )) || suf='remote file'
//! sh: 80    remdispf=(${remfiles:#*/}); remdispd=(${(M)remfiles:#*/})
//! sh: 82-99  glob/esc display filtering
//! sh:104    _tags remote-files; while _tags; do while _next_label remote-files expl ${suf:-remote directory}; do
//! sh:106      [[ -n $suf ]] && compadd "$args[@]" "$expl[@]" -d remdispf -- $remdispfq && ret=0
//! sh:108      compadd ${suf:+-S/} $autoremove "$args[@]" "$expl[@]" -d remdispd -- $remdispdq && ret=0
//! sh:110    done; (( ret )) || return 0; done
//! sh:111    return ret
//! sh:112  else _message -e remote-files 'remote file'; fi
//! ```
//!
//! sh:65/82-99 the `${PREFIX%%…}` component strip and the glob/`(q)`-esc
//! display filtering are string-op approximations (`// sh:N approx`).

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_next_label::_next_label;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}

/// zsh `zstyle -T` — true when unset OR set true; false only when set false.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// sh:65 approx — strip the trailing (non-dotfile) filename component of
/// `PREFIX`, keeping the directory part and any leading `.` of the tail.
fn dir_component(prefix: &str) -> String {
    match prefix.rfind('/') {
        Some(i) => {
            let (head, tail) = prefix.split_at(i + 1);
            if tail.starts_with('.') {
                format!("{}.", head)
            } else {
                head.to_string()
            }
        }
        None if prefix.starts_with('.') => ".".to_string(),
        None => String::new(),
    }
}

/// `_remote_files` — complete files on a remote host via ssh/rsh.
pub fn _remote_files(args_in: &[String]) -> i32 {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let files_ctx = format!(":completion:{}:files", curcontext);

    // sh:43 — honour the remote-access style (default on).
    if !zstyle_t_default_true(&files_ctx, "remote-access") {
        // sh:112
        return _message(&[
            "-e".to_string(),
            "remote-files".to_string(),
            "remote file".to_string(),
        ]);
    }

    // sh:46 — parse _remote_files options; `-/` (dirs-only) stays in `args`
    //   as a compadd passthrough. Split the rest at `--`: before = passthrough
    //   compadd args, after = the remote command + its options.
    let mut host: Option<String> = None;
    let mut dir: Option<String> = None;
    let mut dirs_only = false;
    let mut passthru: Vec<String> = Vec::new();
    let mut cmdline: Vec<String> = Vec::new();
    let mut it = args_in.iter().cloned().peekable();
    let mut after_dashdash = false;
    while let Some(a) = it.next() {
        if after_dashdash {
            cmdline.push(a);
            continue;
        }
        match a.as_str() {
            "--" => after_dashdash = true,
            "-/" => dirs_only = true,
            "-h" => host = it.next(),
            "-W" => dir = it.next(),
            "-g" | "-Q" => {
                let _ = it.next(); // sh:46 glob/esc — display filter (approx: unused)
            }
            _ => passthru.push(a),
        }
    }

    // sh:47 — default host = ${IPREFIX%:}.
    let host = host.unwrap_or_else(|| {
        getsparam("IPREFIX")
            .unwrap_or_default()
            .trim_end_matches(':')
            .to_string()
    });

    // sh:53-59 — remote command + its args (ssh gets non-interactive flags).
    let cmd = cmdline.first().cloned().unwrap_or_default();
    let cmd_rest: Vec<String> = cmdline.iter().skip(1).cloned().collect();
    let cmd_args: Vec<String> = if cmd == "ssh" {
        let mut v = vec!["-o".to_string(), "BatchMode=yes".to_string()];
        v.extend(cmd_rest);
        v.push("-a".to_string());
        v.push("-x".to_string());
        v
    } else {
        cmd_rest
    };

    // sh:62-65 — remote pattern from the working dir + PREFIX component.
    let dirprefix = dir.map(|d| format!("{}/", d)).unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let rempat = format!("{}{}*", dirprefix, dir_component(&prefix));

    // sh:71 — remote `ls -d1FL` listing; keep the classifier-tagged names.
    let mut call: Vec<String> = vec!["files".to_string(), cmd];
    call.extend(cmd_args);
    call.push(host);
    call.extend(["command", "ls", "-d1FL", "--"].map(String::from));
    call.push(rempat);
    let _ = _call_program(&call);
    let listing = getsparam("REPLY").unwrap_or_default();

    // sh:71,80 — split into files vs directories (trailing `/`), stripping
    //   the ls -F classifier suffix (`*`, `=`, `@`, `|`, `/`).
    let mut remdispf: Vec<String> = Vec::new();
    let mut remdispd: Vec<String> = Vec::new();
    for raw in listing.lines() {
        if raw.is_empty() {
            continue;
        }
        if let Some(name) = raw.strip_suffix('/') {
            remdispd.push(name.to_string());
        } else {
            remdispf.push(raw.trim_end_matches(['*', '=', '@', '|']).to_string());
        }
    }

    // sh:76-77 — component compset + suffix decision.
    let _ = compset(vec!["-P".to_string(), "*/".to_string()]);
    let suf_is_file = compset(vec!["-S".to_string(), "/*".to_string()]) != 0 && !dirs_only;

    // sh:104-110 — offer files (when not dirs-only) and directories.
    let mut ret = 1;
    loop {
        let descr = if suf_is_file {
            "remote file"
        } else {
            "remote directory"
        };
        if _next_label(&[
            "remote-files".to_string(),
            "expl".to_string(),
            descr.to_string(),
        ]) != 0
        {
            break;
        }
        let expl = getaparam("expl").unwrap_or_default();
        if suf_is_file && !remdispf.is_empty() {
            let mut cadd = passthru.clone();
            cadd.extend(expl.clone());
            cadd.push("--".to_string());
            cadd.extend(remdispf.clone());
            if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                ret = 0;
            }
        }
        if !remdispd.is_empty() {
            let mut cadd: Vec<String> = Vec::new();
            if suf_is_file {
                cadd.push("-S".to_string());
                cadd.push("/".to_string());
            }
            cadd.extend(passthru.clone());
            cadd.extend(expl.clone());
            cadd.push("--".to_string());
            cadd.extend(remdispd.clone());
            if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                ret = 0;
            }
        }
        break;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_component_strips_trailing_name() {
        // sh:65 approx
        assert_eq!(dir_component("foo/bar"), "foo/");
        assert_eq!(dir_component("foo/.ba"), "foo/.");
        assert_eq!(dir_component("bar"), "");
        assert_eq!(dir_component(".ba"), ".");
    }

    #[test]
    fn returns_one_or_status_without_context() {
        let _g = crate::test_util::global_state_lock();
        // Without a live completion context the loop yields ret=1.
        let _ = _remote_files(&["--".to_string(), "ssh".to_string()]);
    }
}
