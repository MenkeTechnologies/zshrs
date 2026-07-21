//! Port of `_zones` from `Completion/Solaris/Type/_zones`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a type typearg expl
//! sh: 5  zparseopts -D -E -a type t+:
//! sh: 7  [[ -n $type[(r)c] ]] && typearg=-c
//! sh: 8  [[ -n $type[(r)i] ]] && typearg=-i
//! sh:10  _description zones expl zone
//! sh:11  compadd "$@" "$expl[@]" - ${="$(_call_program zones /usr/sbin/zoneadm list $typearg)"}
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam};
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

/// sh:5 — bridge for `zparseopts -D -E -a type t+:`. `-t VALUE` is
/// repeatable (`+`) and takes a mandatory argument (`:`); with no
/// `=name` override the matched option/value pairs land flattened in
/// `$type` (e.g. `-t c -t i` ⇒ `type=(-t c -t i)`). Since the later
/// checks only test membership of the literal values `c`/`i` in
/// `$type`, collecting just the argument values is equivalent. `-D`
/// removes matched flags from argv, leaving `rest` as the `"$@"`
/// passed through to `compadd`.
fn zparse_t(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut types = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            if i + 1 < args.len() {
                types.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (types, rest)
}

/// sh:7-8 — `[[ -n $type[(r)c] ]] && typearg=-c` then `...i]] && typearg=-i`.
/// Later assignment wins when both `c` and `i` were requested.
fn pick_typearg(types: &[String]) -> Option<&'static str> {
    let mut typearg = None;
    if types.iter().any(|t| t == "c") {
        typearg = Some("-c"); // sh:7
    }
    if types.iter().any(|t| t == "i") {
        typearg = Some("-i"); // sh:8
    }
    typearg
}

/// `_zones` — complete Solaris zone names via `zoneadm list`.
pub fn _zones(args: &[String]) -> i32 {
    // sh:5
    let (types, rest) = zparse_t(args);
    // sh:7-8
    let typearg = pick_typearg(&types);

    // sh:11  $(_call_program zones /usr/sbin/zoneadm list $typearg)
    let mut call_args = vec![
        "zones".to_string(),
        "/usr/sbin/zoneadm".to_string(),
        "list".to_string(),
    ];
    if let Some(t) = typearg {
        call_args.push(t.to_string());
    }
    let _ = _call_program(&call_args);
    let out = getsparam("REPLY").unwrap_or_default();

    // sh:11  ${="$(...)"} — re-split the captured output on IFS whitespace.
    let zones: Vec<String> = out.split_whitespace().map(String::from).collect();

    // sh:10  _description zones expl zone
    let _ = _description(&[
        "zones".to_string(),
        "expl".to_string(),
        "zone".to_string(),
    ]);

    // sh:11  compadd "$@" "$expl[@]" - ${=...}
    let mut cadd: Vec<String> = rest;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    cadd.extend(zones);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_pulls_repeated_t_leaving_rest() {
        let (types, rest) = zparse_t(&[
            "-t".to_string(),
            "c".to_string(),
            "-J".to_string(),
            "grp".to_string(),
            "-t".to_string(),
            "i".to_string(),
        ]);
        assert_eq!(types, vec!["c".to_string(), "i".to_string()]);
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn zparse_dangling_t_without_value_is_dropped() {
        let (types, rest) = zparse_t(&["-t".to_string()]);
        assert!(types.is_empty());
        assert!(rest.is_empty());
    }

    #[test]
    fn pick_typearg_prefers_none_when_absent() {
        assert_eq!(pick_typearg(&[]), None);
    }

    #[test]
    fn pick_typearg_selects_dash_c() {
        assert_eq!(pick_typearg(&["c".to_string()]), Some("-c"));
    }

    #[test]
    fn pick_typearg_selects_dash_i_over_c_when_both_present() {
        // sh:7-8 — the `-i` check runs after `-c`, so it wins.
        assert_eq!(
            pick_typearg(&["c".to_string(), "i".to_string()]),
            Some("-i")
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_zones(&[]), 1);
    }
}
