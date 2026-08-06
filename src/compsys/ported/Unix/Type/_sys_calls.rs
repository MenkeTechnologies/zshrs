//! Port of `_sys_calls` from `Completion/Unix/Type/_sys_calls`.
//!
//! Full upstream body (20 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # -a  add "all";  -n  add "none"
//! sh: 8  local expl all none
//! sh: 9  local ifile=/usr/include/sys/syscall.h
//! sh:10  local -au syscalls
//! sh:12  zparseopts -D -K -E a=all n=none
//! sh:14  [[ $OSTYPE = linux* ]] && ifile=/usr/include/bits/syscall.h
//! sh:16  syscalls=( ${${${(M)${(f)"$(<$ifile)"}:#\#[[:blank:]]#define[[:blank:]]##SYS_*}#*[[:blank:]]SYS_}%%[[:blank:]]*} )
//! sh:16  [[ -n $all ]] && syscalls+=( all )
//! sh:17  [[ -n $none ]] && syscalls+=( none )
//! sh:19  _description syscalls expl 'system call'
//! sh:20  compadd "$@" "$expl[@]" -a syscalls
//! ```
//!
//! sh:10 `local -au` = array + uppercase, so each name is uppercased.
//! sh:16 the nested `${(M)…:#…}#*…SYS_}%%…` decomposition is done with
//! straight string ops (`// sh:16 approx`).

use crate::compsys::ported::_description::_description;
use crate::ported::params::getsparam;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:16 approx — extract the `SYS_<name>` token from a
/// `#define SYS_<name> <n>` line, uppercased (sh:11 `-u`).
fn parse_syscall_line(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("define")?;
    if !rest.starts_with(|c: char| c == ' ' || c == '\t') {
        return None;
    }
    let rest = rest.trim_start();
    let name = rest.strip_prefix("SYS_")?;
    let name = name
        .split(|c: char| c == ' ' || c == '\t')
        .next()
        .unwrap_or("");
    if name.is_empty() {
        None
    } else {
        Some(name.to_uppercase())
    }
}

/// `_sys_calls` — complete system-call names from `<sys/syscall.h>`.
pub fn _sys_calls(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_sys_calls");
    // sh:12  zparseopts -D -K -E a=all n=none
    let all = args.iter().any(|a| a == "-a");
    let none = args.iter().any(|a| a == "-n");
    let rest: Vec<String> = args
        .iter()
        .filter(|a| *a != "-a" && *a != "-n")
        .cloned()
        .collect();

    // sh: 9,14 — header path (linux uses bits/syscall.h).
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let ifile = if ostype.starts_with("linux") {
        "/usr/include/bits/syscall.h"
    } else {
        "/usr/include/sys/syscall.h"
    };

    // sh:16  syscalls=( … )
    let mut syscalls: Vec<String> = std::fs::read_to_string(ifile)
        .map(|c| c.lines().filter_map(parse_syscall_line).collect())
        .unwrap_or_default();

    // sh:16-17
    if all {
        syscalls.push("all".to_string());
    }
    if none {
        syscalls.push("none".to_string());
    }

    // sh:19-20  _description + compadd -a syscalls
    let _ = _description(&[
        "syscalls".to_string(),
        "expl".to_string(),
        "system call".to_string(),
    ]);
    let expl = crate::ported::params::getaparam("expl").unwrap_or_default();
    let mut cadd: Vec<String> = rest;
    cadd.extend(expl);
    cadd.push("-a".to_string());
    // compadd -a takes the ARRAY NAME; publish the list to a param.
    crate::ported::params::setaparam("_sys_calls_list", syscalls);
    cadd.push("_sys_calls_list".to_string());
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(_sys_calls(&[]), 1);
    }

    #[test]
    fn parse_extracts_uppercased_name() {
        // sh:11,16
        assert_eq!(
            parse_syscall_line("#define SYS_read 0"),
            Some("READ".into())
        );
        assert_eq!(
            parse_syscall_line("# define\tSYS_write\t1"),
            Some("WRITE".into())
        );
        assert_eq!(parse_syscall_line("#define OTHER 5"), None);
        assert_eq!(parse_syscall_line("int main"), None);
    }
}
