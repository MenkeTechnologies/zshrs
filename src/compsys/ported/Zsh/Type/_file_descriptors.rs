//! Port of `_file_descriptors` from
//! `Completion/Zsh/Type/_file_descriptors`.
//!
//! Full upstream body (59 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  fds=( /dev/fd/<3->(N:t) )
//! sh: 7  fds=( ${(n)fds} )
//! sh: 9  if zstyle -T … verbose; then
//! sh:11    zstyle -s … list-separator sep || sep=--
//! sh:13    if [[ $OSTYPE = freebsd* ]]; then …
//! sh:17    elif … /proc/$$/(fd|path) walk …
//! sh:18      zmodload -F zsh/stat / readlink fallbacks
//! sh:42    if (( list[…] )); then
//! sh:43      list=( "${…}:-0 $sep standard input" … $list )
//! sh:53      disp=( -d list )
//! sh:54    fi
//! sh:55  fi
//! sh:56  fds=( 0 1 2 $fds )
//! sh:58  _description -V file-descriptors expl 'file descriptor'
//! sh:59  compadd $disp -o nosort "$@" "$expl[@]" -a fds
//! ```
//!
//! `/dev/fd/<N>` scanning + readlink-based target resolution for
//! the verbose list. Always emits `0 1 2` in the output regardless
//! of platform.

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Reach `_file_descriptors` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_file_descriptors` (Completion/Zsh/Context/_condition sh:10) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_file_descriptors_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _file_descriptors(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_file_descriptors", args, || {
        _file_descriptors_impl(args)
    })
}

/// `_file_descriptors` — complete numeric file-descriptor names
/// (always 0/1/2 + any fd ≥ 3 currently open for this process).
pub fn _file_descriptors_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_file_descriptors");
    // sh:6-7 — scan /dev/fd for fds ≥ 3
    let mut extra: Vec<i64> = Vec::new();
    if let Ok(entries) = fs::read_dir("/dev/fd") {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if let Ok(n) = name.parse::<i64>() {
                if n >= 3 {
                    extra.push(n);
                }
            }
        }
    }
    extra.sort();

    // sh:9-55  verbose mode: build display lines via readlink(2)
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let verbose_vals = lookupstyle(
        &format!(":completion:{}:file-descriptors", curcontext),
        "verbose",
    );
    let verbose = verbose_vals.is_empty()
        || verbose_vals
            .first()
            .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
            .unwrap_or(true);

    let mut disp: Vec<String> = Vec::new();
    if verbose {
        let sep = lookupstyle(
            &format!(":completion:{}:file-descriptors", curcontext),
            "list-separator",
        )
        .first()
        .cloned()
        .unwrap_or_else(|| "--".to_string());

        let pid = std::process::id();
        let mut list: Vec<String> = Vec::new();
        let pad_width = extra.last().map(|n| n.to_string().len()).unwrap_or(1);
        // Standard streams
        list.push(format!("{:>w$} {} standard input", 0, sep, w = pad_width));
        list.push(format!("{:>w$} {} standard output", 1, sep, w = pad_width));
        list.push(format!("{:>w$} {} standard error", 2, sep, w = pad_width));
        for n in &extra {
            let proc_path = format!("/proc/{}/fd/{}", pid, n);
            let target = fs::read_link(Path::new(&proc_path))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| String::from("(unknown)"));
            list.push(format!("{:>w$} {} {}", n, sep, target, w = pad_width));
        }
        setaparam("list", list);
        disp = vec!["-d".to_string(), "list".to_string()];
    }

    // sh:56  fds = 0 1 2 + extra
    let mut fds: Vec<String> = vec!["0".to_string(), "1".to_string(), "2".to_string()];
    for n in extra {
        fds.push(n.to_string());
    }
    setaparam("fds", fds);

    // sh:58
    let _ = _description(&[
        "-V".to_string(),
        "file-descriptors".to_string(),
        "expl".to_string(),
        "file descriptor".to_string(),
    ]);

    // sh:59
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv: Vec<String> = disp;
    compadd_argv.push("-o".to_string());
    compadd_argv.push("nosort".to_string());
    compadd_argv.extend(args.iter().cloned());
    compadd_argv.extend(expl);
    compadd_argv.push("-a".to_string());
    compadd_argv.push("fds".to_string());
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn fds_array_always_includes_standard_streams() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _file_descriptors_impl(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let fds = getaparam("fds").unwrap_or_default();
        assert!(fds.contains(&"0".to_string()));
        assert!(fds.contains(&"1".to_string()));
        assert!(fds.contains(&"2".to_string()));
    }
}
