//! Port of `_ttys` from `Completion/Unix/Type/_ttys`.
//!
//! Full upstream body (25 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # -d strip /dev/;  -D allow with/without /dev/;  -o only attached ttys
//! sh: 8  local -a ttys expl pre
//! sh: 9  local stripdev optdev open
//! sh:11  zparseopts -D -K -E d=stripdev D=optdev o=open
//! sh:13  if [[ -n $open ]]; then
//! sh:14    ttys=( ${(u)${${(f)"$(_call_program open-ttys ps -Ao tty=)"}:#\?*}%% *} )
//! sh:15    _description open-ttys expl 'open tty'
//! sh:16  else
//! sh:17    ttys=( /dev/tty?*(N) /dev/pts/^ptmx(N) )
//! sh:18    ttys=( ${ttys#/dev/} )
//! sh:19    _description ttys expl 'tty'
//! sh:20  fi
//! sh:21  [[ -z $stripdev ]] && pre=( -p /dev/ )
//! sh:23  [[ -n $optdev ]] && compadd "$@" "$expl[@]" -M 'r:|/=* r:|=*' -a ttys && return
//! sh:24  compadd "$@" "$expl[@]" "$pre[@]" -M 'r:|/=* r:|=*' -a ttys
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::{getaparam, getsparam, setaparam};
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

/// Nullglob a pattern, dropping entries that never matched.
fn glob_n(pat: &str) -> Vec<String> {
    let mut list = {
        let mut s = pat.to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            !e.as_bytes()
                .iter()
                .any(|&b| matches!(b, b'*' | b'?' | b'[' | b']' | b'^'))
        })
        .collect()
}

/// `_ttys` — complete terminal device names.
pub fn _ttys(args: &[String]) -> i32 {
    // sh:11  zparseopts -D -K -E d=stripdev D=optdev o=open
    let stripdev = args.iter().any(|a| a == "-d");
    let optdev = args.iter().any(|a| a == "-D");
    let open = args.iter().any(|a| a == "-o");
    let rest: Vec<String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-d" | "-D" | "-o"))
        .cloned()
        .collect();

    // sh:13-20
    let ttys: Vec<String> = if open {
        // sh:14 — ps -Ao tty=, drop `?*` (no-tty) lines, strip trailing
        //   fields, unique.
        let _ = _call_program(&[
            "open-ttys".to_string(),
            "ps".to_string(),
            "-Ao".to_string(),
            "tty=".to_string(),
        ]);
        let out = getsparam("REPLY").unwrap_or_default();
        let mut seen: Vec<String> = Vec::new();
        for line in out.lines() {
            if line.starts_with('?') {
                continue;
            }
            let name = line.split(' ').next().unwrap_or("").to_string();
            if !name.is_empty() && !seen.contains(&name) {
                seen.push(name);
            }
        }
        let _ = _description(&[
            "open-ttys".to_string(),
            "expl".to_string(),
            "open tty".to_string(),
        ]);
        seen
    } else {
        // sh:17-18 — /dev/tty?*(N) /dev/pts/^ptmx(N), strip /dev/.
        let mut t = glob_n("/dev/tty?*");
        t.extend(
            glob_n("/dev/pts/*")
                .into_iter()
                .filter(|p| p.rsplit('/').next().map(|b| b != "ptmx").unwrap_or(true)),
        );
        let _ = _description(&["ttys".to_string(), "expl".to_string(), "tty".to_string()]);
        t.into_iter()
            .map(|p| p.strip_prefix("/dev/").unwrap_or(&p).to_string())
            .collect()
    };

    setaparam("_ttys_list", ttys);
    let expl = getaparam("expl").unwrap_or_default();

    // sh:23 — -D: allow with/without /dev/ prefix, no -p.
    if optdev {
        let mut cadd: Vec<String> = rest;
        cadd.extend(expl);
        cadd.push("-M".to_string());
        cadd.push("r:|/=* r:|=*".to_string());
        cadd.push("-a".to_string());
        cadd.push("_ttys_list".to_string());
        return bin_compadd("compadd", &cadd, &make_ops(), 0);
    }

    // sh:21,24 — default: prefix with /dev/ unless -d given.
    let mut cadd: Vec<String> = rest;
    cadd.extend(expl);
    if !stripdev {
        cadd.push("-p".to_string());
        cadd.push("/dev/".to_string());
    }
    cadd.push("-M".to_string());
    cadd.push("r:|/=* r:|=*".to_string());
    cadd.push("-a".to_string());
    cadd.push("_ttys_list".to_string());
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
        assert_eq!(_ttys(&[]), 1);
    }
}
