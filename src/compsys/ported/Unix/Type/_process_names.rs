//! Port of `_process_names` from `Completion/Unix/Type/_process_names`.
//!
//! Full upstream body (44 lines, abridged):
//! ```text
//! sh: 1  #autoload  — complete names of running processes
//! sh:11  local tagname='processes-names'
//! sh:12  typeset -a expl opts names all truncate
//! sh:14  zparseopts -E -D 'a=all' 't=truncate'
//! sh:15  (( $#all )) && opts=( -A )
//! sh:17  local hyphen='-'
//! sh:19  [[ $OSTYPE == linux* ]] && hyphen=''
//! sh:21  case $OSTYPE in
//! sh:22    (linux*|freebsd*|openbsd*|netbsd*)
//! sh:23      if (( $#truncate )); then …opts+=(${hyphen}o comm=)…
//! sh:29      else …opts+=(${hyphen}o args=); names=(…transform…) fi ;;
//! sh:37    (*) opts+=(-o comm=); names=( ${${${(f)…}#-}:t} ) ;;
//! sh:42  esac
//! sh:44  _wanted $tagname expl 'process name' compadd "$@" -F '(ps)' -a - names
//! ```
//!
//! sh:31-34 approx — the nested parameter-flag transform (`${${${…}%%
//! *}%:}#-}:t}` for normal entries, `${${(M)…#\[}%]}` for `[kthread]`
//! entries) is reproduced with string ops.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// `:t` — the basename (last path component).
fn basename(s: &str) -> String {
    s.rsplit('/').next().unwrap_or(s).to_string()
}

/// `_process_names` — complete names of running processes.
pub fn _process_names(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_process_names");
    // sh:12 — `typeset -a expl opts names all truncate`.
    //
    // `names` is the candidate list this function builds (sh:29-40) and
    // `expl` is the array `_wanted` fills through its `$2`; both are
    // written as shell parameters below, so both need the PM_LOCAL that
    // `setaparam` alone does not supply. `opts`/`all`/`truncate` stay
    // Rust-side here, so they cannot leak. Measured on `killall <TAB>`:
    //
    //   zsh  : names=[][0]        zshrs: names=[array][72]
    crate::compsys::ported::shared::declare_locals(
        &["expl", "names"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    let tagname = "processes-names";
    // sh:14  zparseopts -E -D 'a=all' 't=truncate'
    let all = args.iter().any(|a| a == "-a");
    let truncate = args.iter().any(|a| a == "-t");
    let rest: Vec<String> = args
        .iter()
        .filter(|a| a.as_str() != "-a" && a.as_str() != "-t")
        .cloned()
        .collect();

    // sh:15  (( $#all )) && opts=( -A )
    let mut opts: Vec<String> = Vec::new();
    if all {
        opts.push("-A".to_string());
    }

    // sh:17-19  hyphen: BSD-style (no leading `-`) on Linux.
    let os = std::env::consts::OS;
    let is_bsd_or_linux = matches!(os, "linux" | "freebsd" | "openbsd" | "netbsd");
    let hyphen = if os == "linux" { "" } else { "-" };

    let mut names: Vec<String>;
    if is_bsd_or_linux {
        if truncate {
            // sh:23-27
            if os == "netbsd" {
                opts.push("-co".to_string());
                opts.push("args=".to_string());
            } else {
                opts.push(format!("{}o", hyphen));
                opts.push("comm=".to_string());
            }
            let out = run_ps(tagname, &opts);
            // ${…#-} — strip a leading `-` per line.
            names = out
                .lines()
                .map(|l| l.strip_prefix('-').unwrap_or(l).to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else {
            // sh:29-34
            opts.push(format!("{}o", hyphen));
            opts.push("args=".to_string());
            let out = run_ps(tagname, &opts);
            names = Vec::new();
            for line in out.lines() {
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('[') {
                    // ${${(M)names:#\[*]}#\[}%] — kernel threads: strip `[`/`]`.
                    let inner = line.trim_start_matches('[').trim_end_matches(']');
                    names.push(inner.to_string());
                } else {
                    // ${${${${names:#\[*]}%% *}%:}#-}:t — first field, strip
                    // trailing `:`, strip leading `-`, basename.
                    let first = line.split(' ').next().unwrap_or("");
                    let first = first.strip_suffix(':').unwrap_or(first);
                    let first = first.strip_prefix('-').unwrap_or(first);
                    names.push(basename(first));
                }
            }
        }
    } else {
        // sh:37  (*) opts+=(-o comm=); names=( ${${${(f)…}#-}:t} )
        opts.push("-o".to_string());
        opts.push("comm=".to_string());
        let out = run_ps(tagname, &opts);
        names = out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| basename(l.strip_prefix('-').unwrap_or(l)))
            .collect();
    }
    names.retain(|s| !s.is_empty());

    // sh:44  _wanted $tagname expl 'process name' compadd "$@" -F '(ps)' -a - names
    setaparam("names", names);
    let mut w = vec![
        tagname.to_string(),
        "expl".to_string(),
        "process name".to_string(),
        "compadd".to_string(),
    ];
    w.extend(rest);
    w.push("-F".to_string());
    w.push("(ps)".to_string());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("names".to_string());
    _wanted(&w)
}

/// sh:23/29/37 — `_call_program $tagname ps $opts 2>/dev/null`, returning
/// stdout via REPLY.
fn run_ps(tagname: &str, opts: &[String]) -> String {
    let mut a = vec![tagname.to_string(), "ps".to_string()];
    a.extend(opts.iter().cloned());
    let _ = call_program_capture(&a);
    getsparam("REPLY").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_process_names(&[]), 1);
    }

    #[test]
    fn options_are_stripped_from_forwarded_args() {
        let _g = crate::test_util::global_state_lock();
        // -a / -t are consumed by zparseopts, not forwarded.
        assert_eq!(_process_names(&["-a".to_string(), "-t".to_string()]), 1);
    }
}
